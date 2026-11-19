//! Plain hosts, Sheprd's side (`docs/relay-plan.md`, Fleet, Spreading a
//! host, step 3): sharing a machine with no Sheprd daemon by installing
//! its relay agent, and keeping each agent's access file current. The
//! daemon computes the file (`Ask::AgentAccess`), so the log never leaves
//! it; this app only carries the file over SSH.

use super::{Error, Fleet, Machine, Standing, agent};
use crate::{remote::supervisor::Client, teleport::Host};
use gpui::{App, BackgroundExecutor};
use sheprd_client::ConnectTarget;
use sheprd_relay_agent::access::Access;
use sheprd_remote_control::control::fleet::{
    HostKind, OpId, View,
    sync::{self, Answer, Ask},
};
use std::sync::atomic::AtomicBool;

/// This computer's relays, from its own host record: the agent listens on
/// them, so a member that reaches this computer reaches the host.
pub(crate) fn relays(view: &View) -> Vec<String> {
    view.hosts
        .iter()
        .find(|host| host.kind == HostKind::Member && host.added_by == view.me)
        .map(|host| {
            host.relays
                .iter()
                .filter(|relay| agent::plain_relay(relay))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// The SSH target a plain host is reached by, when its sessions check the
/// host key strictly: the host keys they accept come from the known hosts
/// of that name. A target the user chose not to check
/// (`sheprd_client::HostKeys::Skip`) is refused, since nothing it reports
/// about itself was said over a session that proved who it is.
pub(crate) fn strict_target(target: &ConnectTarget) -> Result<String, Error> {
    let ConnectTarget::Ssh { target, .. } = target else {
        return Err(Error::Unsupported);
    };
    match sheprd_client::HostKeys::of(target) {
        sheprd_client::HostKeys::Verify => Ok(target.clone()),
        sheprd_client::HostKeys::Skip => Err(Error::Unchecked),
    }
}

fn known_keys(target: &str) -> Result<Vec<String>, Error> {
    sheprd_client::known_host_keys(target).map_err(Error::Trust)
}

/// Shares `machine`, which runs no Sheprd daemon, as a plain host, leaving
/// out the computer members in `deny` (a person here asked for it, after
/// being told the agent is installed): installs or updates its agent on
/// this computer's relays, checks the host key it reports against this
/// computer's known hosts, has the daemon add it, and hands it its first
/// access file.
pub(crate) fn share(
    cx: &mut App,
    id: String,
    machine: Machine,
    deny: Vec<String>,
    done: impl FnOnce(Option<String>, &mut App) + 'static,
) {
    let (Some(client), Some(view)) = (
        crate::remote::Supervisor::client(cx),
        Fleet::view(cx).cloned(),
    ) else {
        done(Some(Error::NoDaemon.to_string()), cx);
        return;
    };
    Fleet::set_acting(cx, &id, true);
    let executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        let outcome = async {
            // Before anything runs on the machine.
            let target = strict_target(&machine.target)?;
            let relays = relays(&view);
            if relays.is_empty() {
                return Err(Error::NoRelays);
            }
            let (connect, label) = (machine.target.clone(), machine.label.clone());
            let installing = relays.clone();
            let (report, user, known) = executor
                .spawn(async move {
                    let host = Host::new(&connect).map_err(|_| Error::Unsupported)?;
                    let cancelled = AtomicBool::new(false);
                    let user = agent::user(&host, &cancelled)?;
                    let report = agent::install(&host, &installing, &cancelled)?;
                    let known = known_keys(&target)?;
                    // The key that names the host must be the one it
                    // negotiates, not only one it says it has.
                    let first = report.ssh_host_keys.first().ok_or(Error::HostKey)?;
                    if !sheprd_client::confirm_host_key(&target, first).map_err(Error::Trust)? {
                        return Err(Error::HostKey);
                    }
                    Ok::<_, Error>((report, user, known))
                })
                .await?;
            let intro = agent::intro(&report, &known, &label, &user, &relays, now_ms())?;
            let host = intro.id().map_err(Error::Intro)?;
            let bytes = intro.to_bytes().map_err(Error::Intro)?;
            daemon(
                &client,
                Ask::AddHost {
                    intro: sync::text(&bytes),
                    deny,
                },
            )
            .await?;
            keep(&client, &executor, &machine, &host.to_string(), &report).await
        }
        .await;
        cx.update(|cx| {
            Fleet::set_acting(cx, &id, false);
            if outcome.is_ok() {
                Fleet::set_link(cx, &id, |link| link.standing = Standing::Agent);
            }
            Fleet::refresh(cx);
            done(outcome.err().map(|error| error.to_string()), cx);
        });
    })
    .detach();
}

/// Where a machine with no Sheprd daemon stands: a plain host of this
/// fleet, kept current; or one with no agent of this fleet, which may be
/// shared as one. Runs from the sync worker.
pub(super) async fn sync(
    client: &Client,
    executor: &BackgroundExecutor,
    machine: &Machine,
    view: &View,
) -> Result<Standing, Error> {
    // Nothing is asked of a machine whose key is not checked.
    let target = match strict_target(&machine.target) {
        Ok(target) => target,
        Err(Error::Unchecked) => return Ok(Standing::Unchecked),
        Err(_) => return Ok(Standing::NoDaemon),
    };
    let connect = machine.target.clone();
    let report = executor
        .spawn(async move {
            let host = Host::new(&connect).map_err(|_| Error::Unsupported)?;
            agent::status(&host, &AtomicBool::new(false))
        })
        .await?;
    let Some(report) = report else {
        return Ok(Standing::NoDaemon);
    };
    if report
        .fleet
        .as_ref()
        .is_some_and(|held| held.fleet.0 != fleet_bytes(&view.fleet))
    {
        return Ok(Standing::Outside);
    }
    let reported = report.ssh_host_keys.clone();
    let known = executor.spawn(async move { known_keys(&target) }).await?;
    let host = agent::host_id(&reported, &known);
    let shared = host.filter(|host| {
        let host = host.to_string();
        view.hosts
            .iter()
            .any(|entry| entry.id == host && entry.kind == HostKind::Agent)
    });
    let Some(host) = shared else {
        return Ok(Standing::NoDaemon);
    };
    keep(client, executor, machine, &host.to_string(), &report).await?;
    Ok(Standing::Agent)
}

/// Hands the agent on `machine` the access file the daemon computes for
/// `host`, settling what it reported suspending, unless it holds that file
/// already and has nothing to settle.
async fn keep(
    client: &Client,
    executor: &BackgroundExecutor,
    machine: &Machine,
    host: &str,
    report: &sheprd_relay_agent::status::Report,
) -> Result<(), Error> {
    let held = report.fleet.as_ref();
    let suspended: Vec<String> = held
        .into_iter()
        .flat_map(|held| held.suspended.values())
        .flat_map(|suspension| &suspension.removals)
        .map(|id| OpId(id.0).to_string())
        .collect();
    let access = match daemon(
        client,
        Ask::AgentAccess {
            host: host.to_owned(),
            suspended,
        },
    )
    .await?
    {
        Answer::AgentAccess { access } => access,
        _ => return Err(Error::Daemon(String::new())),
    };
    let file = Access::parse(access.as_bytes())?;
    let digest = file.digest().map_err(sheprd_relay_agent::Error::from)?;
    if held.is_some_and(|held| held.digest == digest) && file.settled.is_empty() {
        return Ok(());
    }
    let connect = machine.target.clone();
    executor
        .spawn(async move {
            let host = Host::new(&connect).map_err(|_| Error::Unsupported)?;
            agent::apply(&host, &access, &AtomicBool::new(false))
        })
        .await?;
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// A fleet id's bytes from its hex; zeros for one that is not.
fn fleet_bytes(fleet: &str) -> [u8; 32] {
    sync::fleet(fleet).map(|id| id.0).unwrap_or_default()
}

async fn daemon(client: &Client, ask: Ask) -> Result<Answer, Error> {
    super::flows::daemon(client, ask).await
}
