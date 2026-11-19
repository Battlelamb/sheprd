//! The access file a plain host's agent takes (`sheprd_relay_agent::
//! access`), as the log computes it: what Sheprd hands
//! `sheprd-relay-agent fleet apply` over SSH (`docs/relay-plan.md`, Fleet,
//! Spreading a host).
//!
//! The format and the agent's merge live in `sheprd-relay-agent`, built
//! without its runtime, so the agent never depends on this crate and this
//! crate brings no Tokio to the app.

use super::{
    Body, Bundle, Error, HostId, HostKind, Kind, Log, MAX_TOMBSTONES, MemberId, Op, OpId, Result,
    State,
};
use sheprd_relay_agent::fleet::Suspension;
use std::collections::{BTreeMap, BTreeSet};

pub use sheprd_relay_agent::access::{Access, Id, MAX_SETTLED, Member};

/// A removal a plain host's agent reported suspending `target` for: its
/// id, and the signer and sequence number the agent read in it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Asked {
    pub removal: OpId,
    pub signer: MemberId,
    pub seq: u64,
    pub target: MemberId,
}

/// Whether removal `op` may be passed to a plain host's agent, which acts
/// on it without checking it: every member it names has been removed.
/// A removal the log voided may name a member another removal took, but
/// it also names one that is still a member, and the agent would suspend
/// that one. One that names only removed members suspends no one the log
/// still lets in.
#[must_use]
pub fn for_agents(state: &State, op: &Op) -> bool {
    let Body::Remove(remove) = op.body() else {
        return false;
    };
    remove
        .targets()
        .all(|target| state.tombstones.contains(target))
}

/// Every removal the agent's `status --json` reports suspending for.
pub fn asked(suspended: &BTreeMap<Id, Suspension>) -> Vec<Asked> {
    suspended
        .iter()
        .flat_map(|(target, suspension)| {
            suspension.removals.iter().map(|(removal, cause)| Asked {
                removal: OpId(removal.0),
                signer: MemberId(cause.signer.0),
                seq: cause.seq,
                target: MemberId(target.0),
            })
        })
        .collect()
}

/// Whether `asked` no longer stands in `log`, so the agent may lift what it
/// suspended for it. It does when the log holds the operation (a `Remove`
/// whose target is still a member was voided; anything else is no
/// removal). One the log does not hold stands while it waits for its
/// parents; otherwise it is void when its signer or target is no member,
/// when the log holds a later operation of its signer (the log synced with
/// the signer, and the signer never made it), or once `graced` says it
/// stayed unknown long enough.
fn settles(log: &Log, asked: &Asked, graced: &impl Fn(&OpId) -> bool) -> bool {
    if log.get(&asked.removal).is_some() {
        return true;
    }
    if log
        .pending_removals()
        .iter()
        .any(|(id, _, _)| *id == asked.removal)
    {
        return false;
    }
    let state = log.state();
    let Some(signer) = state.members.get(&asked.signer) else {
        return true;
    };
    !state.members.contains_key(&asked.target) || signer.seq >= asked.seq || graced(&asked.removal)
}

// The agent remembers as many removed members as the log does.
const _: () = assert!(sheprd_relay_agent::access::MAX_REMOVED == MAX_TOMBSTONES);

/// How new `member`'s keys (`bundle`) are, as the agent orders key
/// updates: the sequence number of its own latest `Rotate` that put them
/// in place. A member's operations form one chain, so a later rotation
/// always has a higher number, whatever else it signed in between; the
/// number of operations it signed says nothing about its keys. Keys from
/// its admission are 0; keys rotated before the checkpoint the log was
/// pruned to take the member's sequence number there, which no rotation
/// pruned away exceeds and every later one does.
fn key_version(log: &Log, member: &MemberId, bundle: &Bundle) -> u64 {
    let rotated = log
        .ops()
        .filter(|op| op.signer() == *member)
        .filter(|op| matches!(op.body(), Body::Rotate(rotated) if rotated == bundle))
        .map(|op| op.unsigned().seq)
        .max();
    rotated
        .or_else(|| {
            let (_, base) = log.base()?;
            let kept = base.members.get(member)?;
            (kept.bundle == *bundle).then_some(kept.seq)
        })
        .unwrap_or(0)
}

/// The access file for plain host `host` as `log` has it: every live
/// computer member, allowed or not, with the `rev` of its `Access`
/// register, and every removed member. `suspended` are the removals the
/// host's agent reported suspending for (`asked`); each goes in `settled`
/// once it no longer stands ([`settles`], with `graced` for one the log
/// has not heard of), so the agent lifts a suspension the log voided and
/// no member can keep another suspended by naming removals that do not
/// exist. Refuses a host that is not a plain host, or that is held.
pub fn compute(
    log: &Log,
    host: &HostId,
    suspended: impl IntoIterator<Item = Asked>,
    graced: impl Fn(&OpId) -> bool,
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
                keys: key_version(log, id, &member.bundle),
                former: Vec::new(),
            };
            Some((Id(id.0), member))
        })
        .collect();
    // One removal may suspend several members: each id counts once
    // against the bound, so repeats cannot crowd out another's.
    let settled: BTreeSet<Id> = suspended
        .into_iter()
        .filter(|asked| settles(log, asked, &graced))
        .map(|asked| Id(asked.removal.0))
        .collect();
    let settled = settled.into_iter().take(MAX_SETTLED).collect();
    let access = Access {
        fleet: Id(log.fleet().0),
        members,
        removed: state.tombstones.iter().map(|id| Id(id.0)).collect(),
        settled,
    };
    access.check()?;
    Ok(access)
}
