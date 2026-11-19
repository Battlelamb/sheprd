//! The fleet, Sheprd's side (`docs/relay-plan.md`, Fleet): this computer's
//! daemon keeps the signed log and acts on it; Sheprd shows it in
//! Settings > Fleet, asks the daemon for what a person here approved, and
//! carries the log over SSH to every machine it reaches that is in the
//! same fleet.
//!
//! - [`remote`]: the halves run on another machine over SSH.
//! - [`flows`]: syncing a machine, inviting a computer, sharing a host.
//! - [`saved`]: each fleet host this computer may reach, kept as a saved
//!   device, `USER@r-ID.sheprd.invalid`.
//! - [`plain`] and [`agent`]: plain hosts, which run no Sheprd daemon:
//!   installing their relay agent and keeping its access file current.
//!
//! Syncing never hands this fleet's log to a machine outside it: a machine
//! is first asked for what it holds of this fleet, and one that answers
//! for another fleet, or for none, is left alone.

mod agent;
mod error;
pub(crate) mod flows;
pub(crate) mod plain;
mod remote;
pub(crate) mod saved;

pub use error::Error;

use crate::device_tags::Daemon;
use crate::remote::Supervisor;
use gpui::{App, BorrowAppContext, EntityId, Global};
use sheprd_client::ConnectTarget;
use sheprd_remote_control::control::{
    Request, Response,
    fleet::{
        View,
        sync::{Answer, Ask},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    time::{Duration, Instant},
};

/// How often a machine in the fleet is synced when nothing changed.
const REFRESH: Duration = Duration::from_secs(600);
/// How soon a machine that is in no fleet, or another, is asked again.
const OUTSIDE: Duration = Duration::from_secs(3600);
/// How soon a machine whose sync failed is tried again.
const RETRY: Duration = Duration::from_secs(120);
/// How often the view is read again while nothing asks for it.
const VIEW: Duration = Duration::from_secs(60);
/// How long a window's report counts.
const WINDOW_SILENCE: Duration = Duration::from_secs(30);

/// A machine some window reaches: its target and the name it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Machine {
    pub(crate) target: ConnectTarget,
    pub(crate) label: String,
}

/// Where one machine stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Link {
    /// Its heads when it last answered, so the next sync carries only what
    /// it lacks.
    heads: Vec<String>,
    /// This computer's heads when it last synced the machine.
    synced: Vec<String>,
    last: Option<Instant>,
    running: bool,
    pub(crate) standing: Standing,
}

/// What a machine said about the fleet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Standing {
    #[default]
    Unknown,
    /// In this fleet.
    Member,
    /// In no fleet or in another.
    Outside,
    /// Runs no Sheprd daemon, nor a relay agent of this fleet: it may be
    /// shared as a plain host.
    NoDaemon,
    /// A plain host of this fleet, whose relay agent is kept current.
    Agent,
    /// Its SSH host key is not checked: the fleet neither shares it as a
    /// plain host nor asks its agent anything.
    Unchecked,
    /// Could not be asked.
    Failed,
}

impl Link {
    fn due(&self, mine: &[String], now: Instant) -> bool {
        if self.running {
            return false;
        }
        let since = |wait| {
            self.last
                .is_none_or(|last| now.duration_since(last) >= wait)
        };
        match self.standing {
            Standing::Unknown => true,
            Standing::Member | Standing::Agent => self.synced != mine || since(REFRESH),
            Standing::Outside | Standing::NoDaemon | Standing::Unchecked => since(OUTSIDE),
            Standing::Failed => since(RETRY),
        }
    }
}

#[derive(Debug)]
struct Reached {
    machines: BTreeMap<String, Machine>,
    at: Instant,
}

/// The fleet as Sheprd last read it, and each machine's link.
#[derive(Default)]
pub(crate) struct Fleet {
    view: Option<View>,
    /// The daemon answered: `view` is `None` because this computer is in
    /// no fleet, not because nothing was asked yet.
    read: bool,
    asked: Option<Instant>,
    asking: bool,
    error: Option<String>,
    windows: HashMap<EntityId, Reached>,
    links: HashMap<String, Link>,
    /// Machines an invitation or share runs for.
    acting: BTreeSet<String>,
}

impl Global for Fleet {}

impl Fleet {
    pub(crate) fn install(cx: &mut App) {
        cx.set_global(Self::default());
    }

    /// The fleet as this computer's daemon last said.
    pub(crate) fn view(cx: &App) -> Option<&View> {
        cx.try_global::<Self>()?.view.as_ref()
    }

    /// Whether the daemon has answered at least once.
    pub(crate) fn read(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|fleet| fleet.read)
    }

    /// The last failure to show, if any.
    pub(crate) fn error(cx: &App) -> Option<String> {
        cx.try_global::<Self>()?.error.clone()
    }

    /// Every machine a window reaches now, by id.
    pub(crate) fn machines(cx: &App) -> BTreeMap<String, Machine> {
        cx.try_global::<Self>()
            .map(|fleet| fleet.reached(Instant::now()))
            .unwrap_or_default()
    }

    /// What `machine` said about the fleet.
    pub(crate) fn standing(cx: &App, machine: &str) -> Standing {
        cx.try_global::<Self>()
            .and_then(|fleet| fleet.links.get(machine))
            .map_or(Standing::Unknown, |link| link.standing)
    }

    /// Whether an invitation or share runs for `machine`.
    pub(crate) fn acting(cx: &App, machine: &str) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|fleet| fleet.acting.contains(machine))
    }

    /// Tells which devices `window` reaches, as `relay_sharing` hears it;
    /// this computer's own daemon is asked directly, not over SSH.
    pub(crate) fn reach(cx: &mut App, window: EntityId, reachable: &BTreeMap<String, Daemon>) {
        if !cx.has_global::<Self>() {
            return;
        }
        let machines = reachable
            .iter()
            .filter_map(|(id, daemon)| match daemon {
                Daemon::Local => None,
                Daemon::Remote(target) => Some((
                    id.clone(),
                    Machine {
                        target: target.clone(),
                        label: label(id, target),
                    },
                )),
            })
            .collect();
        let now = Instant::now();
        cx.update_global::<Self, _>(|this, _| {
            this.windows.insert(window, Reached { machines, at: now });
            this.windows
                .retain(|_, reached| now.duration_since(reached.at) < WINDOW_SILENCE);
            let reached = this.reached(now);
            this.links
                .retain(|id, link| link.running || reached.contains_key(id));
        });
        Self::tick(cx);
    }

    fn reached(&self, now: Instant) -> BTreeMap<String, Machine> {
        self.windows
            .values()
            .filter(|reached| now.duration_since(reached.at) < WINDOW_SILENCE)
            .flat_map(|reached| reached.machines.clone())
            .collect()
    }

    /// Reads the view when it is stale, then syncs the machines that are
    /// due.
    pub(crate) fn tick(cx: &mut App) {
        let Some(this) = cx.try_global::<Self>() else {
            return;
        };
        let now = Instant::now();
        if !this.asking
            && this
                .asked
                .is_none_or(|asked| now.duration_since(asked) >= VIEW)
        {
            Self::refresh(cx);
        }
        let Some(this) = cx.try_global::<Self>() else {
            return;
        };
        let Some(view) = &this.view else {
            return;
        };
        let mine = view.heads.clone();
        let fleet = view.fleet.clone();
        let due: Vec<(String, Machine)> = this
            .reached(now)
            .into_iter()
            .filter(|(id, _)| {
                !this.acting.contains(id)
                    && this.links.get(id).is_none_or(|link| link.due(&mine, now))
            })
            .collect();
        for (id, machine) in due {
            flows::sync(cx, id, machine, fleet.clone());
        }
    }

    /// Asks the daemon for the view now.
    pub(crate) fn refresh(cx: &mut App) {
        let Some(client) = Supervisor::client(cx) else {
            return;
        };
        cx.update_global::<Self, _>(|this, _| {
            this.asking = true;
            this.asked = Some(Instant::now());
        });
        let task = client.request(Request::Fleet(Box::new(Ask::Status)));
        cx.spawn(async move |cx| {
            let answer = task.await;
            cx.update(|cx| {
                let view = match answer {
                    Ok(Response::Fleet(answer)) => match *answer {
                        Answer::Status { view } => Ok(view.map(|view| *view)),
                        _ => Err(None),
                    },
                    Ok(Response::Error { message, .. }) => Err(Some(message)),
                    Ok(_) => Err(None),
                    Err(error) => Err(Some(error.to_string())),
                };
                let changed = cx.update_global::<Self, _>(|this, _| {
                    this.asking = false;
                    match view {
                        Ok(view) => {
                            let changed = this.view != view;
                            this.view = view;
                            this.read = true;
                            changed
                        }
                        // An older daemon, or none: the page says so.
                        Err(message) => {
                            this.read = true;
                            this.error = message;
                            false
                        }
                    }
                });
                if changed {
                    saved::keep(cx);
                    Self::tick(cx);
                }
                cx.refresh_windows();
            });
        })
        .detach();
    }

    /// Asks the daemon `ask`, which a person here approved, then reads the
    /// view again. `done` gets a failure to show.
    pub(crate) fn act(
        cx: &mut App,
        ask: Ask,
        done: impl FnOnce(Option<String>, &mut App) + 'static,
    ) {
        let Some(client) = Supervisor::client(cx) else {
            done(Some(Error::NoDaemon.to_string()), cx);
            return;
        };
        let task = client.request(Request::Fleet(Box::new(ask)));
        cx.spawn(async move |cx| {
            let failure = match task.await {
                Ok(Response::Fleet(_)) => None,
                Ok(Response::Error { message, .. }) => Some(message),
                Ok(_) => Some(Error::Daemon(String::new()).to_string()),
                Err(error) => Some(error.to_string()),
            };
            cx.update(|cx| {
                Self::refresh(cx);
                done(failure, cx);
            });
        })
        .detach();
    }

    /// Sets what `machine` said, as a sync that just ran would, so it is
    /// not due again.
    #[cfg(test)]
    pub(crate) fn set_standing(cx: &mut App, machine: &str, standing: Standing) {
        let heads = Self::view(cx)
            .map(|view| view.heads.clone())
            .unwrap_or_default();
        Self::set_link(cx, machine, |link| {
            link.standing = standing;
            link.synced = heads;
            link.last = Some(Instant::now());
        });
    }

    fn set_link(cx: &mut App, id: &str, change: impl FnOnce(&mut Link)) {
        cx.update_global::<Self, _>(|this, _| change(this.links.entry(id.to_owned()).or_default()));
    }

    fn set_acting(cx: &mut App, id: &str, acting: bool) {
        cx.update_global::<Self, _>(|this, _| {
            if acting {
                this.acting.insert(id.to_owned());
            } else {
                this.acting.remove(id);
            }
        });
        cx.refresh_windows();
    }
}

/// The name a machine shows: its SSH target, or a cloud machine's name.
fn label(id: &str, target: &ConnectTarget) -> String {
    match target {
        ConnectTarget::Ssh { target, .. } => target.clone(),
        #[cfg(feature = "cloud")]
        ConnectTarget::Cloud { machine, .. } => machine.clone(),
        ConnectTarget::Wsl { distro, .. } => distro.clone(),
        _ => id.to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
