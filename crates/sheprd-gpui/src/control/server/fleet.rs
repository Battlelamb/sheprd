//! `fleet status`, `fleet open`, `fleet remove`, and `fleet remove-host`.
//! The daemon is asked on the background executor. Removing opens
//! Settings > Fleet asking, where the user picks why and confirms; nothing
//! is signed from here.
use super::{Watches, in_window, settings::answer_when_done, socket, window_for};
use crate::{
    control::protocol::{
        Call, ErrorCode, Response,
        fleet::{FleetOp, Reply},
    },
    remote::Supervisor,
    settings_window::FleetPending,
};
use gpui::App;
use sheprd_remote_control::control::{
    Request,
    fleet::{
        View,
        sync::{Answer, Ask},
    },
};
use std::time::Duration;

pub(super) fn serve(
    call: Call<FleetOp>,
    incoming: socket::Incoming,
    watches: &mut Watches,
    cx: &mut App,
) {
    let wait = Duration::from_secs(call.op.wait_seconds());
    let Call { caller, op } = call;
    if let Err(response) = window_for(&caller, cx) {
        return incoming.respond(response);
    }
    let pending = match op {
        FleetOp::Status => None,
        FleetOp::Open => Some(FleetPending::Nothing),
        FleetOp::Remove { member } => match pick_member(crate::fleet::Fleet::view(cx), &member) {
            Ok(member) => Some(FleetPending::Remove(member)),
            Err(response) => return incoming.respond(response),
        },
        FleetOp::RemoveHost { host } => match pick_host(crate::fleet::Fleet::view(cx), &host) {
            Ok(host) => Some(FleetPending::RemoveHost(host)),
            Err(response) => return incoming.respond(response),
        },
        FleetOp::Scrub { machine } => {
            let machines = crate::fleet::Fleet::machines(cx);
            let named = machines
                .iter()
                .map(|(id, machine)| (id.as_str(), machine.label.as_str()));
            match pick(named, &machine, "machine") {
                Ok(id) => Some(FleetPending::Scrub(id)),
                Err(response) => return incoming.respond(response),
            }
        }
    };
    if let Some(pending) = pending {
        let asking = !matches!(pending, FleetPending::Nothing);
        return incoming.respond(in_window(&caller, cx, move |_, _, cx| {
            crate::settings_window::open_fleet(cx.entity().downgrade(), pending, cx);
            if asking {
                Response::done("Asking in Sheprd's Settings > Fleet; the user confirms there")
            } else {
                Response::done("Showing Settings > Fleet")
            }
        }));
    }
    let Some(client) = Supervisor::client(cx) else {
        return incoming.respond(Response::error(
            ErrorCode::Unsupported,
            "This Sheprd does not run remote control",
        ));
    };
    let request = client.request(Request::Fleet(Box::new(Ask::Status)));
    let task = cx.spawn(async move |_| {
        let (view, error) = match request.await {
            Ok(sheprd_remote_control::control::Response::Fleet(answer)) => match *answer {
                Answer::Status { view } => (view, None),
                _ => (None, Some(crate::remote::Error::Unexpected.to_string())),
            },
            Ok(sheprd_remote_control::control::Response::Error { message, .. }) => {
                (None, Some(message))
            }
            Ok(_) => (None, Some(crate::remote::Error::Unexpected.to_string())),
            Err(error) => (None, Some(error.to_string())),
        };
        Response::Fleet {
            reply: Reply::Status { view, error },
        }
    });
    answer_when_done(task, wait, incoming, watches, cx);
}

/// The member `needle` names: by ID, else the one member of that name.
pub(super) fn pick_member(view: Option<&View>, needle: &str) -> Result<String, Response> {
    let view = view.ok_or_else(not_in_fleet)?;
    pick(
        view.members
            .iter()
            .map(|member| (member.id.as_str(), member.name.as_str())),
        needle,
        "member",
    )
}

/// The host `needle` names: by ID, else the one host of that name.
pub(super) fn pick_host(view: Option<&View>, needle: &str) -> Result<String, Response> {
    let view = view.ok_or_else(not_in_fleet)?;
    pick(
        view.hosts
            .iter()
            .map(|host| (host.id.as_str(), host.name.as_str())),
        needle,
        "host",
    )
}

fn pick<'a>(
    items: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    needle: &str,
    what: &str,
) -> Result<String, Response> {
    if let Some((id, _)) = items.clone().find(|(id, _)| *id == needle) {
        return Ok(id.to_owned());
    }
    let mut named = items.filter(|(_, name)| *name == needle);
    match (named.next(), named.next()) {
        (Some((id, _)), None) => Ok(id.to_owned()),
        (Some(_), Some(_)) => Err(Response::error(
            ErrorCode::InvalidRequest,
            format!("Several {what}s are named {needle}; name one by its ID from `fleet status`"),
        )),
        (None, _) => Err(Response::error(
            ErrorCode::NotFound,
            format!("No {what} is named {needle}"),
        )),
    }
}

fn not_in_fleet() -> Response {
    Response::error(
        ErrorCode::NotFound,
        "This computer is in no fleet; create one in Settings > Fleet",
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
