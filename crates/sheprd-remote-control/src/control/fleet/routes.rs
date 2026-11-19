//! Hosts' self-records, and who owns each relay route.
//!
//! A route has one owner: the host that claimed it first in graph order,
//! by the intro that added it or a record carried for it. A later intro
//! naming a route another host owns holds its host; a later record
//! naming one does not count and flags its host, as a second record with
//! one `rev` does. Ownership is kept in the state, so a log pruned to a
//! checkpoint decides as the whole log would.
//!
//! A host's records and routes count from the addition whose intro it has
//! now (`Anchors::source`): a record that had not seen it speaks of an
//! intro the host no longer has, and at that addition the host gives up
//! every route it held before for the one its intro names.

use super::{Body, HostId, HostSeen, Op, State, graph::Graph, hosts::Anchors};
use std::collections::BTreeMap;

/// One claim on a route, in graph order.
enum Claim<'a> {
    /// The addition that made a host live.
    Intro(HostId),
    /// A record carried for a host.
    Seen(&'a HostSeen),
}

/// Applies the records `effective` carries to `state`'s hosts and settles
/// who owns each route, starting from `base`'s owners for the hosts whose
/// intro comes from the base.
pub(super) fn records(
    graph: &Graph<'_>,
    anchors: &BTreeMap<HostId, Anchors>,
    effective: &[(usize, &Op)],
    base: &State,
    state: &mut State,
) {
    let mut routes: BTreeMap<String, HostId> = base
        .routes
        .iter()
        .filter(|(_, owner)| {
            anchors
                .get(*owner)
                .is_some_and(|anchor| anchor.source.is_none())
        })
        .map(|(route, owner)| (route.clone(), *owner))
        .collect();
    let mut claims: Vec<(usize, Claim<'_>)> = anchors
        .iter()
        .filter_map(|(id, anchor)| anchor.source.map(|at| (at, Claim::Intro(*id))))
        .collect();
    claims.extend(effective.iter().filter_map(|&(position, op)| {
        match op.body() {
            Body::HostSeen(seen)
                if anchors
                    .get(&seen.record.host)
                    .is_some_and(|anchor| Anchors::after(graph, anchor.source, position)) =>
            {
                Some((position, Claim::Seen(seen)))
            }
            _ => None,
        }
    }));
    claims.sort_by_key(|(position, _)| *position);
    for (_, claim) in claims {
        match claim {
            Claim::Intro(id) => {
                let Some(host) = state.hosts.get_mut(&id) else {
                    continue;
                };
                routes.retain(|_, owner| *owner != id);
                let route = &host.intro.record.route;
                match routes.get(route) {
                    Some(owner) if *owner != id => host.held = true,
                    Some(_) => {}
                    None => {
                        routes.insert(route.clone(), id);
                    }
                }
            }
            Claim::Seen(seen) => apply(seen, &mut routes, state),
        }
    }
    // Only the routes a live host names stay owned, so the state stays
    // bounded and a route a host moved off is free.
    routes.retain(|route, owner| {
        state
            .hosts
            .get(owner)
            .is_some_and(|host| host.intro.record.route == *route || host.record.route == *route)
    });
    state.routes = routes;
}

/// Applies one carried record, if it is its host's word and its route is
/// the host's own or free.
fn apply(seen: &HostSeen, routes: &mut BTreeMap<String, HostId>, state: &mut State) {
    let id = seen.record.host;
    let Some(host) = state.hosts.get_mut(&id) else {
        return;
    };
    // A plain host's keys only change through a member that checked them
    // strictly, and a changed key cannot be checked that way. A record
    // that is not the host's word changes nothing, not even a flag.
    let keys_kept =
        host.intro.ca.is_some() || seen.record.ssh_host_keys == host.intro.record.ssh_host_keys;
    if !keys_kept || !host.intro.vouches(&seen.record, seen.signature.as_ref()) {
        return;
    }
    if routes
        .get(&seen.record.route)
        .is_some_and(|owner| *owner != id)
    {
        host.flagged = true;
        return;
    }
    if seen.record.rev > host.record.rev {
        // The route it moves off is free again, unless its intro names it.
        let old = std::mem::replace(&mut host.record, seen.record.clone());
        host.record_signature = seen.signature;
        if old.route != seen.record.route && old.route != host.intro.record.route {
            routes.remove(&old.route);
        }
        routes.entry(seen.record.route.clone()).or_insert(id);
    } else if seen.record.rev == host.record.rev && seen.record != host.record {
        host.flagged = true;
    }
}
