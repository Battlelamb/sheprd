//! Computing the state of a set of operations: members first
//! (`membership`), then hosts, access, and links, each resolved toward the
//! safer side when concurrent operations disagree.

use super::{
    Body, HostId, HostIntro, Kind, MAX_HOSTS, MemberId, Op,
    graph::Bits,
    membership::{Folder, Members},
    state::{AccessRegister, Host, Member, State},
};
use std::collections::{BTreeMap, BTreeSet};

/// The state of the down-closed set `set`, on top of `base`.
pub(super) fn fold(folder: &Folder<'_, '_>, set: &Bits, base: &State) -> State {
    let graph = folder.graph();
    let members = folder.members(set);
    let positions: Vec<usize> = set
        .iter()
        .filter(|&position| folder.check(position).is_ok())
        .collect();
    let equivocating = equivocating(folder, &positions, base);
    // Operations that count: checked, and not voided by a removal of their
    // signer that had not seen them. Removals were settled with members.
    let effective: Vec<(usize, &Op)> = positions
        .iter()
        .map(|&position| (position, graph.order[position]))
        .filter(|(position, op)| op.body().membership() || !folder.voided(&members, *position))
        .collect();
    let mut state = State {
        members: members.live.clone(),
        tombstones: members.tombstones.clone(),
        equivocating,
        ..State::default()
    };
    for (_, op) in &effective {
        if let Some(member) = state.members.get_mut(&op.signer()) {
            member.seq = member.seq.max(op.unsigned().seq);
        }
    }
    hosts(folder, &members, &effective, base, &mut state);
    access(folder, &effective, base, &mut state);
    links(folder, &effective, base, &mut state);
    state
}

/// Members with two operations neither had seen.
fn equivocating(folder: &Folder<'_, '_>, positions: &[usize], base: &State) -> BTreeSet<MemberId> {
    let graph = folder.graph();
    let mut by_signer: BTreeMap<MemberId, Vec<usize>> = BTreeMap::new();
    for &position in positions {
        by_signer
            .entry(graph.order[position].signer())
            .or_default()
            .push(position);
    }
    let mut found = base.equivocating.clone();
    for (signer, mut ops) in by_signer {
        // A signer's sequence numbers climb along its own chain, so its
        // operations in sequence order each see the one before, unless
        // the chain forked: then two neighbours are concurrent.
        ops.sort_by_key(|&position| (graph.order[position].unsigned().seq, position));
        if ops.windows(2).any(|pair| !graph.sees(pair[1], pair[0])) {
            found.insert(signer);
        }
    }
    found
}

/// One way a host was added: from the base, which every operation
/// descends from, or by an operation.
struct Add<'a> {
    position: Option<usize>,
    intro: &'a HostIntro,
    added_by: MemberId,
    order: [u8; 32],
}

fn hosts(
    folder: &Folder<'_, '_>,
    members: &Members,
    effective: &[(usize, &Op)],
    base: &State,
    state: &mut State,
) {
    let graph = folder.graph();
    let mut member_hosts: BTreeMap<HostId, Vec<(&HostIntro, MemberId)>> = BTreeMap::new();
    for (id, member) in &members.live {
        if let Some(intro) = member.bundle.host.as_ref()
            && let Ok(host) = intro.id()
        {
            member_hosts.entry(host).or_default().push((intro, *id));
        }
    }
    let mut adds: BTreeMap<HostId, Vec<Add<'_>>> = BTreeMap::new();
    for (id, host) in &base.hosts {
        if host.intro.kind != super::HostKind::Member {
            adds.entry(*id).or_default().push(Add {
                position: None,
                intro: &host.intro,
                added_by: host.added_by,
                order: [0; 32],
            });
        }
    }
    let mut removals: BTreeMap<HostId, Vec<usize>> = BTreeMap::new();
    for &(position, op) in effective {
        match op.body() {
            Body::HostAdd(intro) if !state.equivocating.contains(&op.signer()) => {
                if let Ok(id) = intro.id()
                    && intro.kind != super::HostKind::Member
                {
                    adds.entry(id).or_default().push(Add {
                        position: Some(position),
                        intro,
                        added_by: op.signer(),
                        order: op.id().0,
                    });
                }
            }
            Body::HostRemove(host) => removals.entry(*host).or_default().push(position),
            Body::Remove(remove) if members.removals.contains(&position) => {
                for host in &remove.also_hosts {
                    removals.entry(*host).or_default().push(position);
                }
            }
            _ => {}
        }
    }
    let mut live: Vec<(usize, HostId, Host)> = Vec::new();
    for (id, mut candidates) in adds {
        let removed_by = removals.get(&id).map(Vec::as_slice).unwrap_or_default();
        // An addition survives only when it saw every removal: a removal
        // wins over every addition it is concurrent with.
        candidates.retain(|add| {
            removed_by
                .iter()
                .all(|&removal| add.position.is_some_and(|at| graph.sees(at, removal)))
        });
        candidates.sort_by_key(|add| add.order);
        let Some(first) = candidates.first() else {
            continue;
        };
        let held = candidates.iter().any(|add| {
            add.intro.ca != first.intro.ca
                || add.intro.record.ssh_host_keys != first.intro.record.ssh_host_keys
        });
        let earliest = candidates
            .iter()
            .map(|add| add.position.map_or(0, |at| at + 1))
            .min()
            .unwrap_or_default();
        live.push((
            earliest,
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
    // A computer is its own host. Two members naming one host, or a member
    // naming a host added before with another account or other SSH host
    // keys, is a copied intro: held until a person picks.
    for (id, claims) in member_hosts {
        let (intro, member) = claims[0];
        let added = state.hosts.get(&id).map(|host| &host.intro);
        let held = claims.len() > 1
            || added.is_some_and(|added| {
                added.user != intro.user || added.record.ssh_host_keys != intro.record.ssh_host_keys
            });
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
    // Keep the base's newer records, then apply carried ones.
    for (id, host) in &mut state.hosts {
        if let Some(kept) = base.hosts.get(id)
            && kept.intro == host.intro
            && kept.record.rev > host.record.rev
        {
            host.record = kept.record.clone();
            host.record_signature = kept.record_signature;
            host.flagged = kept.flagged;
        }
    }
    for &(_, op) in effective {
        let Body::HostSeen(seen) = op.body() else {
            continue;
        };
        let Some(host) = state.hosts.get_mut(&seen.record.host) else {
            continue;
        };
        // A plain host's keys only change through a member that checked
        // them strictly, and a changed key cannot be checked that way.
        let keys_kept =
            host.intro.ca.is_some() || seen.record.ssh_host_keys == host.intro.record.ssh_host_keys;
        if !keys_kept || !host.intro.vouches(&seen.record, seen.signature.as_ref()) {
            continue;
        }
        if seen.record.rev > host.record.rev {
            host.record = seen.record.clone();
            host.record_signature = seen.signature;
        } else if seen.record.rev == host.record.rev && seen.record != host.record {
            host.flagged = true;
        }
    }
}

/// One `Access` operation: its place, and what it says.
type Grant = (usize, bool, u64);

fn access(folder: &Folder<'_, '_>, effective: &[(usize, &Op)], base: &State, state: &mut State) {
    let graph = folder.graph();
    let mut by_pair: BTreeMap<(HostId, MemberId), Vec<Grant>> = BTreeMap::new();
    for &(position, op) in effective {
        if let Body::Access(access) = op.body()
            && !state.equivocating.contains(&op.signer())
        {
            by_pair
                .entry((access.host, access.member))
                .or_default()
                .push((position, access.allow, access.rev));
        }
    }
    let mut registers = base.access.clone();
    for (pair, entries) in by_pair {
        let floor = base.access.get(&pair).map_or(0, |register| register.rev);
        // A `rev` is one above the highest it saw, so an agent merging by
        // `rev` alone orders it rightly, and no single operation can put
        // the register out of reach of a later deny.
        // `entries` are in graph order, so each one's counted ancestors
        // are known before it; one that did not count sets no floor.
        let mut counted: Vec<&Grant> = Vec::new();
        for entry in &entries {
            let (position, _, rev) = entry;
            let seen = counted
                .iter()
                .filter(|(other, ..)| graph.sees(*position, *other))
                .map(|(_, _, other_rev)| *other_rev)
                .fold(floor, u64::max);
            if seen.checked_add(1) == Some(*rev) {
                counted.push(entry);
            }
        }
        let latest: Vec<&&Grant> = counted
            .iter()
            .filter(|(position, ..)| {
                !counted
                    .iter()
                    .any(|(other, ..)| graph.sees(*other, *position))
            })
            .collect();
        if latest.is_empty() {
            continue;
        }
        registers.insert(
            pair,
            AccessRegister {
                // Concurrent allow and deny: deny.
                allow: latest.iter().all(|(_, allow, _)| *allow),
                rev: latest.iter().map(|(_, _, rev)| *rev).max().unwrap_or(floor),
            },
        );
    }
    registers.retain(|(host, member), _| {
        state.hosts.contains_key(host)
            && state
                .members
                .get(member)
                .is_some_and(|person| person.bundle.kind == Kind::Computer)
    });
    state.access = registers;
}

fn links(folder: &Folder<'_, '_>, effective: &[(usize, &Op)], base: &State, state: &mut State) {
    let graph = folder.graph();
    let mut links = base.links.clone();
    let mut latest: BTreeMap<(MemberId, HostId), usize> = BTreeMap::new();
    for &(position, op) in effective {
        let Body::Link(link) = op.body() else {
            continue;
        };
        let key = (op.signer(), link.host);
        // One phone signs every link it has; the later one wins, and of
        // two it signed concurrently, the higher sequence number.
        let newer = latest.get(&key).is_none_or(|&kept| {
            graph.sees(position, kept)
                || (!graph.sees(kept, position)
                    && op.unsigned().seq > graph.order[kept].unsigned().seq)
        });
        if newer {
            latest.insert(key, position);
            links.insert(key, link.device_key.clone());
        }
    }
    links.retain(|(phone, host), _| {
        state.hosts.contains_key(host)
            && state
                .members
                .get(phone)
                .is_some_and(|member: &Member| member.bundle.kind == Kind::Phone)
    });
    state.links = links;
}
