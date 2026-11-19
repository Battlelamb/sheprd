//! Which hosts are live, and since when.
//!
//! Every host is live from an operation that added it: a `HostAdd`, or for
//! a computer's own host the `Found`, `Admit`, or `Rotate` that named it.
//! A removal of the host (`HostRemove`, or `Remove::also_hosts`) wins over
//! every addition it had not seen, a computer's own included, and only an
//! addition that saw every removal brings it back. Two anchors come out:
//!
//! - `since`, the first addition that counts: what was said about the host
//!   before it (exclusions, links) went with the removal.
//! - `source`, the addition whose intro the host has now: its records and
//!   routes count from there.
//!
//! Either is `None` when the host was live in the base and nothing since
//! removed it, so a log pruned to a checkpoint decides as the whole log.

use super::{
    Body, HostId, HostIntro, HostKind, MAX_HOSTS, MemberId, Op,
    graph::Graph,
    membership::Members,
    state::{Host, State},
};
use std::collections::BTreeMap;

/// Where a live host's history starts ([module docs](self)).
#[derive(Clone, Copy, Debug)]
pub(super) struct Anchors {
    pub(super) since: Option<usize>,
    pub(super) source: Option<usize>,
}

impl Anchors {
    /// Whether the operation at `position` came after `anchor`, and so
    /// speaks of the host as it is.
    pub(super) fn after(graph: &Graph<'_>, anchor: Option<usize>, position: usize) -> bool {
        anchor.is_none_or(|anchor| anchor == position || graph.sees(position, anchor))
    }
}

/// One way a host was added: from the base, which every operation
/// descends from, or by an operation.
struct Add<'a> {
    position: Option<usize>,
    intro: &'a HostIntro,
    added_by: MemberId,
    order: [u8; 32],
    /// The base held it: the additions that disagreed went with the
    /// history a checkpoint replaced, and still disagree.
    held: bool,
}

/// Sets `state.hosts` from the additions and removals in `effective`, on
/// top of `base`, and says where each live host's history starts.
pub(super) fn hosts(
    graph: &Graph<'_>,
    members: &Members,
    effective: &[(usize, &Op)],
    base: &State,
    state: &mut State,
) -> BTreeMap<HostId, Anchors> {
    let mut adds: BTreeMap<HostId, Vec<Add<'_>>> = BTreeMap::new();
    for (id, host) in &base.hosts {
        if host.intro.kind != HostKind::Member {
            adds.entry(*id).or_default().push(Add {
                position: None,
                intro: &host.intro,
                added_by: host.added_by,
                order: [0; 32],
                held: host.held,
            });
        }
    }
    let mut removals: BTreeMap<HostId, Vec<usize>> = BTreeMap::new();
    for &(position, op) in effective {
        match op.body() {
            Body::HostAdd(intro) if !state.equivocating.contains(&op.signer()) => {
                if let Ok(id) = intro.id()
                    && intro.kind != HostKind::Member
                {
                    adds.entry(id).or_default().push(Add {
                        position: Some(position),
                        intro,
                        added_by: op.signer(),
                        order: op.id().0,
                        held: false,
                    });
                }
            }
            Body::HostRemove(host) => removals.entry(*host).or_default().push(position),
            Body::Remove(remove) if members.removals.contains_key(&position) => {
                for host in &remove.also_hosts {
                    removals.entry(*host).or_default().push(position);
                }
            }
            _ => {}
        }
    }
    // An addition survives only when it saw every removal: a removal wins
    // over every addition it is concurrent with.
    let survives = |id: &HostId, at: Option<usize>| {
        removals.get(id).is_none_or(|removed_by| {
            at.is_some_and(|at| removed_by.iter().all(|&removal| graph.sees(at, removal)))
        })
    };
    let mut anchors = BTreeMap::new();
    let mut live: Vec<(usize, HostId, Host)> = Vec::new();
    // Where each surviving addition was made; `None` for the base.
    let mut preceding: BTreeMap<HostId, Vec<(Option<usize>, &HostIntro)>> = BTreeMap::new();
    for (id, mut candidates) in adds {
        candidates.retain(|add| survives(&id, add.position));
        // The first added gives the host its intro: the base's before any
        // other, so a pruned log picks the same one.
        candidates.sort_by_key(|add| (add.position.map_or(0, |at| at + 1), add.order));
        let Some(first) = candidates.first() else {
            continue;
        };
        preceding.insert(
            id,
            candidates
                .iter()
                .map(|add| (add.position, add.intro))
                .collect(),
        );
        let held = candidates.iter().any(|add| {
            add.held
                || add.intro.ca != first.intro.ca
                || add.intro.record.ssh_host_keys != first.intro.record.ssh_host_keys
        });
        anchors.insert(
            id,
            Anchors {
                since: first.position,
                source: first.position,
            },
        );
        live.push((
            first.position.map_or(0, |at| at + 1),
            id,
            Host {
                intro: first.intro.clone(),
                added_by: first.added_by,
                record: first.intro.record.clone(),
                record_signature: first.intro.record_signature,
                held,
                flagged: false,
            },
        ));
    }
    // Concurrent additions can pass the bound each on its own; the first
    // added are kept.
    live.sort_by_key(|(earliest, id, _)| (*earliest, *id));
    live.truncate(MAX_HOSTS);
    state.hosts = live.into_iter().map(|(_, id, host)| (id, host)).collect();
    anchors.retain(|id, _| state.hosts.contains_key(id));
    member_hosts(
        graph,
        members,
        base,
        &preceding,
        &survives,
        state,
        &mut anchors,
    );
    // Keep the base's newer records and its flags while the host still has
    // the intro it had there.
    for (id, host) in &mut state.hosts {
        let Some(kept) = base.hosts.get(id) else {
            continue;
        };
        if anchors
            .get(id)
            .is_some_and(|anchor| anchor.source.is_some())
            || kept.intro != host.intro
        {
            continue;
        }
        if kept.record.rev > host.record.rev {
            host.record = kept.record.clone();
            host.record_signature = kept.record_signature;
        }
        host.flagged |= kept.flagged;
    }
    anchors
}

/// A computer is its own host, from the operations that named it.
///
/// Two members naming one host is a copied intro: held until a person
/// picks. So is a member naming an added host with another account or
/// other SSH host keys, unless the addition came after the member first
/// named the host and saw it: only then is the addition the copy, and it
/// changes nothing, so no member can hold another's host by adding a copy
/// of it. An addition before or concurrent with it is weighed, so a bundle
/// cannot silently take over a host another member added.
fn member_hosts(
    graph: &Graph<'_>,
    members: &Members,
    base: &State,
    preceding: &BTreeMap<HostId, Vec<(Option<usize>, &HostIntro)>>,
    survives: &impl Fn(&HostId, Option<usize>) -> bool,
    state: &mut State,
    anchors: &mut BTreeMap<HostId, Anchors>,
) {
    let mut named: BTreeMap<HostId, Vec<(&HostIntro, MemberId)>> = BTreeMap::new();
    for (id, member) in &members.live {
        if let Some(intro) = member.bundle.host.as_ref()
            && let Ok(host) = intro.id()
        {
            named.entry(host).or_default().push((intro, *id));
        }
    }
    for (id, claimants) in named {
        let (intro, member) = claimants[0];
        let origin = members.origins.get(&member).copied();
        // Where it named this host: the base, while the base had it live,
        // then each operation since. The host lives from the first that
        // saw every removal of it; its intro is the last's.
        let from_base = origin.is_none() && base.hosts.contains_key(&id);
        let claims: Vec<Option<usize>> = from_base
            .then_some(None)
            .into_iter()
            .chain(
                members
                    .claims
                    .get(&member)
                    .into_iter()
                    .flatten()
                    .map(|&at| Some(at)),
            )
            .collect();
        let Some(&since) = claims.iter().find(|at| survives(&id, **at)) else {
            continue;
        };
        let source = claims.last().copied().flatten();
        // Every addition weighed must name the same machine the same way:
        // one that differs, in account, CA, or SSH host keys, holds it.
        let weighed =
            preceding
                .get(&id)
                .into_iter()
                .flatten()
                .filter(|(at, _)| match (at, origin) {
                    (None, _) => true,
                    (Some(at), Some(origin)) => !graph.sees(*at, origin),
                    // A member from the base came before every addition here.
                    (Some(_), None) => false,
                });
        // A host the base held for an addition that disagreed stays held:
        // the addition went with the history a checkpoint replaced, and a
        // removal of the host is what would have dropped it. When two of
        // the base's members claimed it, that is why it was held, and the
        // claims are counted again from the members live.
        let carried = since.is_none()
            && base.hosts.get(&id).is_some_and(|kept| {
                kept.held
                    && kept.intro.kind == HostKind::Member
                    && base
                        .members
                        .values()
                        .filter(|other| {
                            other.bundle.host.as_ref().and_then(|intro| intro.id().ok()) == Some(id)
                        })
                        .count()
                        <= 1
            });
        let held = claimants.len() > 1
            || carried
            || weighed.into_iter().any(|(_, added)| {
                added.user != intro.user
                    || added.ca != intro.ca
                    || added.record.ssh_host_keys != intro.record.ssh_host_keys
            });
        anchors.insert(id, Anchors { since, source });
        state.hosts.insert(
            id,
            Host {
                intro: intro.clone(),
                added_by: member,
                record: intro.record.clone(),
                record_signature: intro.record_signature,
                held,
                flagged: false,
            },
        );
    }
}
