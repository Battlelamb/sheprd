//! The access file a plain host's agent takes (`sheprd_relay_agent::
//! access`), as the log computes it: what Sheprd hands
//! `sheprd-relay-agent fleet apply` over SSH (`docs/relay-plan.md`, Fleet,
//! Spreading a host).
//!
//! The format and the agent's merge live in `sheprd-relay-agent`, built
//! without its runtime, so the agent never depends on this crate and this
//! crate brings no Tokio to the app.

use super::{Body, Error, HostId, HostKind, Kind, Log, MAX_TOMBSTONES, OpId, Result};

pub use sheprd_relay_agent::access::{Access, Id, MAX_SETTLED, Member};

// The agent remembers as many removed members as the log does.
const _: () = assert!(sheprd_relay_agent::access::MAX_REMOVED == MAX_TOMBSTONES);

/// The access file for plain host `host` as `log` has it: every live
/// computer member, allowed or not, with the `rev` of its `Access`
/// register, and every removed member. `suspended` are the removals the
/// host's agent reported suspending for (`status --json`); each goes in
/// `settled` once the log can say it no longer stands: it holds it as a
/// `Remove` (so a target still a member was not removed by it), or, with
/// nothing waiting for its parents, it holds no such `Remove` at all (an
/// id no member's removal has, or one that is no removal). The agent then
/// lifts a suspension the log voided, and no member can keep another
/// suspended by naming removals that do not exist. Refuses a host that is
/// not a plain host, or that is held.
pub fn compute(
    log: &Log,
    host: &HostId,
    suspended: impl IntoIterator<Item = OpId>,
) -> Result<Access> {
    let state = log.state();
    let entry = state.hosts.get(host).ok_or(Error::Unknown)?;
    if entry.intro.kind != HostKind::Agent {
        return Err(Error::Invalid);
    }
    if entry.held {
        return Err(Error::HostHeld);
    }
    let members = state
        .members
        .iter()
        .filter(|(_, member)| member.bundle.kind == Kind::Computer)
        .filter_map(|(id, member)| {
            let ssh = sheprd_relay_agent::authorized_keys::ed25519(member.bundle.ssh.as_deref()?)?;
            let rev = state
                .access
                .get(&(*host, *id))
                .map_or(0, |register| register.rev);
            let member = Member {
                allow: state.allowed(host, id),
                dial: member.bundle.dial,
                name: member.bundle.name.clone(),
                rev,
                ssh,
                tunnel: member.bundle.tunnel,
                keys: member.seq,
                former: Vec::new(),
            };
            Some((Id(id.0), member))
        })
        .collect();
    let current = log.pending() == 0;
    let settled = suspended
        .into_iter()
        .filter(|id| match log.get(id) {
            Some(op) => matches!(op.body(), Body::Remove(_)) || current,
            None => current,
        })
        .take(MAX_SETTLED)
        .map(|id| Id(id.0))
        .collect();
    let access = Access {
        fleet: Id(log.fleet().0),
        members,
        removed: state.tombstones.iter().map(|id| Id(id.0)).collect(),
        settled,
    };
    access.check()?;
    Ok(access)
}
