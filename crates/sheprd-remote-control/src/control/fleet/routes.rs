//! Hosts' self-records, and who owns each relay route.
//!
//! One pass over what touched hosts, in graph order, from the state the
//! base kept: additions (a `HostAdd`, or a computer's `Found`, `Admit`, or
//! `Rotate` naming its host), removals, and carried records. A route has
//! one owner: the host that claimed it first. An addition naming a route
//! another live host owns is held; a record naming one does not count and
//! flags its host, as a second record with one `rev` does. A removal frees
//! the host's routes and starts it over.
//!
//! Every step depends only on what came before it in graph order, never
//! on what came after. Everything before a checkpoint comes before it in
//! any log that holds it, and the state keeps where the pass stood there
//! (`State::routes`, and each host's record and flags), so a log pruned
//! to it decides as the whole log would.

use super::{
    Body, Bundle, HostId, HostIntro, HostKind, HostSeen, MemberId, Op, SelfRecord, State,
    body::RawSignature, graph::Graph, membership::Members,
};
use std::collections::{BTreeMap, BTreeSet};

/// A host as the pass has it so far.
struct Tracked<'a> {
    intro: &'a HostIntro,
    record: SelfRecord,
    signature: Option<RawSignature>,
    flagged: bool,
    /// Its intro named a route another host owned.
    held: bool,
}

impl<'a> Tracked<'a> {
    fn new(intro: &'a HostIntro) -> Self {
        Self {
            intro,
            record: intro.record.clone(),
            signature: intro.record_signature,
            flagged: false,
            held: false,
        }
    }
}

/// What the pass has seen so far.
struct Pass<'a> {
    hosts: BTreeMap<HostId, Tracked<'a>>,
    routes: BTreeMap<String, HostId>,
    /// The removals of each host so far, by place in the graph.
    removed: BTreeMap<HostId, Vec<usize>>,
    /// The host each computer names.
    own: BTreeMap<MemberId, HostId>,
}

impl<'a> Pass<'a> {
    fn new(base: &'a State) -> Self {
        let hosts = base
            .hosts
            .iter()
            .map(|(id, host)| {
                let tracked = Tracked {
                    intro: &host.intro,
                    record: host.record.clone(),
                    signature: host.record_signature,
                    flagged: host.flagged,
                    held: false,
                };
                (*id, tracked)
            })
            .collect();
        let own = base
            .members
            .iter()
            .filter_map(|(member, entry)| host_id(&entry.bundle).map(|host| (*member, host)))
            .collect();
        Self {
            hosts,
            routes: base.routes.clone(),
            removed: BTreeMap::new(),
            own,
        }
    }

    /// An addition of `id` at `position`. A computer's own (`member`)
    /// gives the host the intro it names, as its bundle does; another
    /// addition of a live host changes nothing.
    fn add(
        &mut self,
        graph: &Graph<'_>,
        position: usize,
        id: HostId,
        intro: &'a HostIntro,
        member: Option<MemberId>,
    ) {
        // An addition concurrent with a removal of the host before it is
        // void, as `hosts` has it.
        let survives = self
            .removed
            .get(&id)
            .is_none_or(|removals| removals.iter().all(|&at| graph.sees(position, at)));
        if !survives {
            return;
        }
        if let Some(member) = member
            && let Some(old) = self.own.insert(member, id)
            && old != id
        {
            self.remove(old);
        }
        let tracked = match self.hosts.get_mut(&id) {
            Some(_) if member.is_none() => return,
            Some(tracked) => {
                if tracked.intro != intro {
                    *tracked = Tracked {
                        held: tracked.held,
                        ..Tracked::new(intro)
                    };
                }
                tracked
            }
            None => self.hosts.entry(id).or_insert(Tracked::new(intro)),
        };
        let route = &tracked.intro.record.route;
        let record = &tracked.record.route;
        self.routes
            .retain(|kept, owner| *owner != id || kept == route || kept == record);
        match self.routes.get(route) {
            Some(owner) if *owner != id => tracked.held = true,
            Some(_) => {}
            None => {
                self.routes.insert(route.clone(), id);
            }
        }
    }

    /// A removal of `id`: it frees the host's routes, and the host starts
    /// over when added again.
    fn remove(&mut self, id: HostId) {
        self.hosts.remove(&id);
        self.routes.retain(|_, owner| *owner != id);
    }

    /// A carried record, if it is its host's word and its route is the
    /// host's own or free.
    fn seen(&mut self, seen: &HostSeen) {
        let id = seen.record.host;
        let Some(host) = self.hosts.get_mut(&id) else {
            return;
        };
        // A plain host's keys only change through a member that checked
        // them strictly, and a changed key cannot be checked that way. A
        // record that is not the host's word changes nothing, not even a
        // flag.
        let keys_kept =
            host.intro.ca.is_some() || seen.record.ssh_host_keys == host.intro.record.ssh_host_keys;
        if !keys_kept || !host.intro.vouches(&seen.record, seen.signature.as_ref()) {
            return;
        }
        if self
            .routes
            .get(&seen.record.route)
            .is_some_and(|owner| *owner != id)
        {
            host.flagged = true;
            return;
        }
        if seen.record.rev > host.record.rev {
            // The route it moves off is free again, unless its intro
            // names it.
            let old = std::mem::replace(&mut host.record, seen.record.clone());
            host.signature = seen.signature;
            if old.route != seen.record.route && old.route != host.intro.record.route {
                self.routes.remove(&old.route);
            }
            self.routes.entry(seen.record.route.clone()).or_insert(id);
        } else if seen.record.rev == host.record.rev && seen.record != host.record {
            host.flagged = true;
        }
    }
}

/// Applies the records `effective` carries to `state`'s hosts, holds those
/// whose intro names a route another host owns, and settles who owns each
/// route. `bundles` are the places of the `Found`, `Admit`, and `Rotate`
/// operations that count.
pub(super) fn records(
    graph: &Graph<'_>,
    members: &Members,
    bundles: &BTreeSet<usize>,
    effective: &[(usize, &Op)],
    base: &State,
    state: &mut State,
) {
    let mut pass = Pass::new(base);
    for &(position, op) in effective {
        match op.body() {
            Body::HostAdd(intro)
                if intro.kind != HostKind::Member && !state.equivocating.contains(&op.signer()) =>
            {
                if let Ok(id) = intro.id() {
                    pass.add(graph, position, id, intro, None);
                }
            }
            Body::Found(bundle) | Body::Rotate(bundle) if bundles.contains(&position) => {
                own(&mut pass, graph, position, bundle, bundle.id());
            }
            Body::Admit(admit) if bundles.contains(&position) => {
                own(&mut pass, graph, position, &admit.bundle, admit.bundle.id());
            }
            Body::HostRemove(id) => {
                pass.removed.entry(*id).or_default().push(position);
                pass.remove(*id);
            }
            Body::Remove(remove) => {
                let Some(reach) = members.removals.get(&position) else {
                    continue;
                };
                let hosts = remove
                    .also_hosts
                    .iter()
                    .copied()
                    .chain(reach.iter().filter_map(|member| pass.own.remove(member)));
                for id in hosts.collect::<Vec<_>>() {
                    pass.removed.entry(id).or_default().push(position);
                    pass.remove(id);
                }
            }
            Body::HostSeen(seen) => pass.seen(seen),
            _ => {}
        }
    }
    for (id, host) in &mut state.hosts {
        let Some(tracked) = pass.hosts.get(id) else {
            continue;
        };
        if *tracked.intro != host.intro {
            continue;
        }
        host.record = tracked.record.clone();
        host.record_signature = tracked.signature;
        host.flagged = tracked.flagged;
        host.held |= tracked.held;
    }
    // Only the routes a live host names stay owned, so the state stays
    // bounded and a route a host moved off is free.
    pass.routes.retain(|route, owner| {
        state
            .hosts
            .get(owner)
            .is_some_and(|host| host.intro.record.route == *route || host.record.route == *route)
    });
    state.routes = pass.routes;
}

/// A computer's bundle naming its own host.
fn own<'a>(
    pass: &mut Pass<'a>,
    graph: &Graph<'_>,
    position: usize,
    bundle: &'a Bundle,
    member: MemberId,
) {
    if let Some(intro) = bundle.host.as_ref()
        && let Ok(id) = intro.id()
    {
        pass.add(graph, position, id, intro, Some(member));
    }
}

/// The host a bundle names as its own.
fn host_id(bundle: &Bundle) -> Option<HostId> {
    bundle.host.as_ref().and_then(|intro| intro.id().ok())
}
