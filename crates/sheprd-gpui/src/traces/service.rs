//! The app's one traces watcher: a thread that keeps this computer's index
//! of agent sessions current as agents write (`sheprd_traces::watch`) and
//! leaves the latest snapshot in a mailbox. Windows look at the mailbox's
//! generation every tick, so a new message shows within a frame of being
//! indexed; only the newest snapshot is kept, never a queue of them.

use super::host_filter;
use super::replication;
use sheprd_client::device_tags::TaggedHost;
use sheprd_client::herdr::AgentKind;
use sheprd_traces::{Activity, Host, Session, Store, redact::Redactor};
use std::{
    ops::ControlFlow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};

/// How far back the chart can look.
pub(crate) const HISTORY_DAYS: i64 = 365;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// What the index held after a pass.
#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    /// Most recently updated first, at most [`sheprd_traces::MAX_LIMIT`].
    pub sessions: Vec<Session>,
    /// Per day and agent, oldest first.
    pub activity: Vec<Activity>,
    pub hosts: Vec<TaggedHost>,
    pub local_host: Option<String>,
    pub routes: std::collections::BTreeMap<String, String>,
}

/// The watcher's state as windows see it.
#[derive(Clone, Debug)]
pub(crate) enum Status {
    Starting,
    Ready(Arc<Snapshot>),
    /// The index could not be opened or watched; why, for display.
    Failed(String),
}

pub(super) struct Mailbox {
    status: Mutex<Status>,
    generation: AtomicU64,
}

impl Mailbox {
    pub(super) fn put(&self, status: Status) {
        if let Ok(mut slot) = self.status.lock() {
            *slot = status;
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub(super) fn changed(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }
}

/// The global that owns the watcher thread; dropping it stops the thread.
pub(crate) struct Service {
    mailbox: Arc<Mailbox>,
    stop: Arc<AtomicBool>,
    replication: replication::Replication,
    tags: host_filter::Publisher,
}

impl gpui::Global for Service {}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl Service {
    /// Starts watching this computer's agent sessions.
    pub(crate) fn install(cx: &mut gpui::App) {
        let mailbox = Arc::new(Mailbox {
            status: Mutex::new(Status::Starting),
            generation: AtomicU64::new(1),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, flag) = (mailbox.clone(), stop.clone());
        let started = thread::Builder::new()
            .name("sheprd-traces".into())
            .spawn(move || {
                if let Err(error) = run(&shared, &flag) {
                    tracing::warn!(%error, "traces watcher stopped");
                    shared.put(Status::Failed(error.to_string()));
                }
            });
        if let Err(error) = started {
            mailbox.put(Status::Failed(error.to_string()));
        }
        cx.set_global(Self {
            mailbox,
            stop,
            replication: Default::default(),
            tags: Default::default(),
        });
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                cx.update(|cx| {
                    use gpui::BorrowAppContext as _;
                    let wanted = replication::wanted(cx);
                    let status = cx.global::<Self>().status();
                    let tags = host_filter::wanted(cx, &status);
                    cx.update_global::<Self, _>(|service, _| {
                        service.replication.poll(wanted, &service.mailbox);
                        service.tags.poll(tags, &service.mailbox);
                    });
                });
            }
        })
        .detach();
    }

    /// Changes each time the status does.
    pub(crate) fn generation(&self) -> u64 {
        self.mailbox.generation.load(Ordering::Acquire)
    }

    pub(crate) fn status(&self) -> Status {
        self.mailbox
            .status
            .lock()
            .map(|status| status.clone())
            .unwrap_or(Status::Starting)
    }

    pub(super) fn peers(&self) -> Vec<replication::Report> {
        self.replication.reports()
    }
}

fn run(mailbox: &Mailbox, stop: &AtomicBool) -> sheprd_traces::Result<()> {
    let path = sheprd_traces::path().ok_or(sheprd_traces::Error::NoStateDirectory)?;
    let state = path
        .parent()
        .ok_or(sheprd_traces::Error::NoStateDirectory)?;
    let home = std::env::home_dir().ok_or(sheprd_traces::Error::NoStateDirectory)?;
    let mut store = Store::open(&path)?;
    let host = Host::local(state)?;
    let roots = sheprd_traces::default_roots(&home);
    let redactor = Redactor::new(Some(&home));
    // What was indexed before, shown while the first scan runs.
    mailbox.put(Status::Ready(Arc::new(snapshot(&store)?)));
    sheprd_traces::watch(
        &mut store,
        &host,
        &roots,
        &redactor,
        stop,
        now,
        |store, _tick| {
            if stop.load(Ordering::Acquire) {
                return ControlFlow::Break(());
            }
            // Idle rescans also pick up writes made by an incoming exchange
            // in another process, even when no local agent file changed.
            match snapshot(store) {
                Ok(snapshot) => mailbox.put(Status::Ready(Arc::new(snapshot))),
                Err(error) => tracing::warn!(%error, "traces snapshot failed"),
            }
            ControlFlow::Continue(())
        },
    )
}

pub(super) fn snapshot(store: &Store) -> sheprd_traces::Result<Snapshot> {
    Ok(Snapshot {
        sessions: store.sessions(None::<AgentKind>, sheprd_traces::MAX_LIMIT)?,
        activity: store.activity_for(now() - HISTORY_DAYS * DAY_MS, now(), None)?,
        hosts: store.hosts()?,
        local_host: store.local_host_id()?,
        routes: store
            .sync_peers()?
            .into_iter()
            .filter_map(|peer| Some((peer.route, peer.host_id?)))
            .collect(),
    })
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
