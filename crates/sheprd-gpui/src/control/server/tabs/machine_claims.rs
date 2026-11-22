//! Pane-owned reservations, serialized by the app thread across all windows.
use crate::control::{
    protocol::{Caller, ErrorCode, Response, tabs::TabOp},
    server::socket,
};
use gpui::App;

#[cfg(not(target_os = "macos"))]
pub(super) fn serve(_: Caller, _: TabOp, incoming: socket::Incoming, _: &mut App) {
    incoming.respond(Response::error(
        ErrorCode::Unsupported,
        "Virtual Macs need an Apple silicon Mac",
    ));
}

#[cfg(target_os = "macos")]
pub(super) fn serve(caller: Caller, op: TabOp, incoming: socket::Incoming, cx: &mut App) {
    use crate::machine;
    let (name, release) = match op {
        TabOp::MachineClaim { name } => (name, false),
        TabOp::MachineRelease { name } => (name, true),
        _ => return,
    };
    if owner(&caller).is_none() {
        return incoming.respond(Response::error(
            ErrorCode::InvalidRequest,
            "Reserve from a Herdr pane or core terminal",
        ));
    }
    let name = match machine::Name::try_from(name) {
        Ok(name) => name,
        Err(error) => {
            return incoming.respond(Response::error(
                ErrorCode::InvalidRequest,
                error.to_string(),
            ));
        }
    };
    let read = cx.background_executor().spawn(async { machine::list() });
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let listed = read.await;
        let response = cx.update(|cx| match listed {
            Ok(listed) if listed.iter().any(|item| item.name == name) => cx
                .default_global::<Reservations>()
                .change(&caller, name.as_str(), release, Instant::now()),
            Ok(_) => Response::error(ErrorCode::NotFound, "No such virtual Mac"),
            Err(error) => Response::error(ErrorCode::Failed, error.to_string()),
        });
        incoming.respond(response);
    })
    .detach();
}

#[cfg(any(target_os = "macos", test))]
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[cfg(any(target_os = "macos", test))]
const IDLE: Duration = Duration::from_secs(15 * 60);

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, PartialEq, Eq)]
struct Owner {
    daemon: Option<String>,
    pane: Option<String>,
    terminal: Option<String>,
}

#[cfg(any(target_os = "macos", test))]
fn owner(caller: &Caller) -> Option<Owner> {
    if caller.pane_id.is_none() && caller.terminal_id.is_none() {
        return None;
    }
    Some(Owner {
        daemon: caller.daemon_socket.clone(),
        pane: caller.pane_id.clone(),
        terminal: caller.terminal_id.clone(),
    })
}

#[cfg(any(target_os = "macos", test))]
struct Reservation {
    owner: Owner,
    until: Instant,
}

#[cfg(any(target_os = "macos", test))]
impl Reservation {
    fn describe(&self, now: Instant) -> String {
        format!(
            "busy: pane {}, terminal {} ({}s remaining)",
            self.owner.pane.as_deref().unwrap_or("-"),
            self.owner.terminal.as_deref().unwrap_or("-"),
            self.until.saturating_duration_since(now).as_secs()
        )
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct Reservations(BTreeMap<String, Reservation>);
#[cfg(any(target_os = "macos", test))]
impl gpui::Global for Reservations {}

#[cfg(any(target_os = "macos", test))]
impl Reservations {
    fn prune(&mut self, now: Instant) {
        self.0.retain(|_, lease| lease.until > now);
    }

    fn change(&mut self, caller: &Caller, name: &str, release: bool, now: Instant) -> Response {
        self.prune(now);
        let Some(owner) = owner(caller) else {
            return Response::error(ErrorCode::InvalidRequest, "Reserve from a pane or terminal");
        };
        if let Some(lease) = self.0.get(name)
            && lease.owner != owner
        {
            return Response::error(
                ErrorCode::Busy,
                format!("{name} is {}", lease.describe(now)),
            );
        }
        if release {
            self.0.remove(name);
            Response::done(format!("Released {name}"))
        } else {
            // Bound even malformed clients; installed machine count is normally tiny.
            if self.0.len() >= 64 && !self.0.contains_key(name) {
                return Response::error(ErrorCode::Busy, "Too many machine reservations");
            }
            self.0.insert(
                name.to_owned(),
                Reservation {
                    owner,
                    until: now + IDLE,
                },
            );
            Response::done(format!(
                "Reserved {name} for this pane for 15 idle minutes; release when finished"
            ))
        }
    }

    fn check(&mut self, caller: &Caller, name: &str, now: Instant) -> Option<Response> {
        self.prune(now);
        let lease = self.0.get_mut(name)?;
        if owner(caller).as_ref() != Some(&lease.owner) {
            return Some(Response::error(
                ErrorCode::Busy,
                format!("{name} is {}", lease.describe(now)),
            ));
        }
        // Guest commands take at most 600s, less than the idle lease.
        lease.until = now + IDLE;
        None
    }
}

#[cfg(target_os = "macos")]
pub(super) fn check(caller: &Caller, name: &str, cx: &mut App) -> Option<Response> {
    cx.default_global::<Reservations>()
        .check(caller, name, Instant::now())
}

#[cfg(target_os = "macos")]
pub(super) fn describe(name: &str, cx: &mut App) -> Option<String> {
    let now = Instant::now();
    let reservations = cx.default_global::<Reservations>();
    reservations.prune(now);
    reservations.0.get(name).map(|lease| lease.describe(now))
}

#[cfg(test)]
mod tests;
