//! Who is a member, in the history of any operation.
//!
//! Each operation is checked once, against the members of its own history
//! ([`Folder::check`]): its signer must have been live there, its signature
//! must check out with that signer's key, its sequence number must follow
//! the signer's earlier ones, and the signer must be allowed to sign that
//! body. That check depends only on the operation's ancestors, so it never
//! changes: an operation that fails it is refused for good.
//!
//! Which removals take effect depends on the whole set being folded,
//! because a removal voids every concurrent operation by the member it
//! removes, removals included. The one exception is a counter-removal of
//! whoever removed it. Removals that void each other in a cycle take
//! effect, every one: when the rules cannot say who wins, everyone involved
//! is removed.

use super::{
    Body, Error, FleetId, Kind, MAX_COMPUTERS, MAX_MEMBERSHIP_OPS, MAX_PHONES, MAX_TOMBSTONES,
    MemberId, Op, OpId, Result,
    graph::{Bits, Graph},
    state::{Member, State},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    rc::Rc,
};

/// The members of one history.
#[derive(Debug, Default)]
pub(super) struct Members {
    pub(super) live: BTreeMap<MemberId, Member>,
    pub(super) tombstones: BTreeSet<MemberId>,
    /// The removals that took effect, by place in the graph.
    pub(super) removals: Vec<usize>,
    /// Where each member's bundle came from: its `Found`, `Admit`, or
    /// latest `Rotate`. A member from the base has none.
    pub(super) origins: BTreeMap<MemberId, usize>,
}

impl Members {
    fn kind_count(&self, kind: Kind) -> usize {
        self.live
            .values()
            .filter(|member| member.bundle.kind == kind)
            .count()
    }
}

/// Checks operations and folds membership over a graph.
pub(super) struct Folder<'g, 'a> {
    graph: &'g Graph<'a>,
    base: &'g State,
    fleet: FleetId,
    /// The membership operations.
    mask: Bits,
    checks: Vec<Result<()>>,
    cache: RefCell<HashMap<Bits, Rc<Members>>>,
}

impl<'g, 'a> Folder<'g, 'a> {
    /// Checks every operation of `graph`, in order. `base` is the state of
    /// the checkpoint the log was pruned to, or empty.
    /// Operations in `known` passed their check before, and are not
    /// checked again.
    pub(super) fn new(
        graph: &'g Graph<'a>,
        base: &'g State,
        fleet: FleetId,
        known: &HashSet<OpId>,
    ) -> Result<Self> {
        let mut mask = Bits::new(graph.len());
        for (position, op) in graph.order.iter().enumerate() {
            if op.body().membership() {
                mask.insert(position);
            }
        }
        let costly = mask
            .iter()
            .filter(|&position| !matches!(graph.order[position].body(), Body::Remove(_)))
            .count();
        if costly > MAX_MEMBERSHIP_OPS {
            return Err(Error::Full);
        }
        let mut folder = Self {
            graph,
            base,
            fleet,
            mask,
            checks: Vec::with_capacity(graph.len()),
            cache: RefCell::default(),
        };
        for position in 0..graph.len() {
            let check = if known.contains(&graph.order[position].id()) {
                Ok(())
            } else {
                folder.check_one(position)
            };
            folder.checks.push(check);
        }
        Ok(folder)
    }

    pub(super) fn graph(&self) -> &Graph<'a> {
        self.graph
    }

    /// Whether the operation at `position` passed its check, for good.
    pub(super) fn check(&self, position: usize) -> &Result<()> {
        &self.checks[position]
    }

    /// The members of the down-closed set `set`.
    pub(super) fn members(&self, set: &Bits) -> Rc<Members> {
        let key = set.masked(&self.mask);
        if let Some(members) = self.cache.borrow().get(&key) {
            return Rc::clone(members);
        }
        let members = Rc::new(self.fold(&key));
        self.cache.borrow_mut().insert(key, Rc::clone(&members));
        members
    }

    /// Whether `position` is void because an effective removal of its
    /// signer, in `members`, had not seen it.
    pub(super) fn voided(&self, members: &Members, position: usize) -> bool {
        let signer = self.graph.order[position].signer();
        members.removals.iter().any(|&removal| {
            removal != position
                && !self.graph.sees(removal, position)
                && targets(self.graph.order[removal]).any(|target| *target == signer)
        })
    }

    fn check_one(&self, position: usize) -> Result<()> {
        let op = self.graph.order[position];
        let members = self.members(&self.graph.ancestors[position]);
        let (key, kind) = match op.body() {
            Body::Found(bundle) => {
                if !self.base.members.is_empty()
                    || !self.base.tombstones.is_empty()
                    || bundle.id() != op.signer()
                    || bundle.kind != Kind::Computer
                    || FleetId(op.id().0) != self.fleet
                {
                    return Err(Error::Invalid);
                }
                (bundle.verifying_key()?, bundle.kind)
            }
            _ => {
                if op.unsigned().fleet != self.fleet {
                    return Err(Error::Fleet);
                }
                let member = members.live.get(&op.signer()).ok_or(Error::Signer)?;
                (member.bundle.verifying_key()?, member.bundle.kind)
            }
        };
        op.verify(&key)?;
        if op.unsigned().seq <= self.last_seq(position, op.signer()) {
            return Err(Error::Sequence);
        }
        self.permitted(op, kind, &members)
    }

    /// The highest sequence number the signer used before `position`.
    fn last_seq(&self, position: usize, signer: MemberId) -> u64 {
        let base = self
            .base
            .members
            .get(&signer)
            .map_or(0, |member| member.seq);
        self.graph.ancestors[position]
            .iter()
            .map(|ancestor| self.graph.order[ancestor])
            .filter(|op| op.signer() == signer)
            .map(|op| op.unsigned().seq)
            .fold(base, u64::max)
    }

    fn permitted(&self, op: &Op, kind: Kind, members: &Members) -> Result<()> {
        let computer = kind == Kind::Computer;
        match op.body() {
            Body::Found(_) | Body::HostRemove(_) => Ok(()),
            Body::Admit(admit) => {
                let id = admit.bundle.id();
                if members.live.contains_key(&id)
                    || members.tombstones.contains(&id)
                    || taken(members, &admit.bundle, id)
                {
                    return Err(Error::Invalid);
                }
                let limit = match admit.bundle.kind {
                    Kind::Computer => MAX_COMPUTERS,
                    Kind::Phone => MAX_PHONES,
                };
                if members.kind_count(admit.bundle.kind) >= limit
                    || members.tombstones.len() >= MAX_TOMBSTONES
                {
                    return Err(Error::Full);
                }
                let message = admit
                    .bundle
                    .join_message(&self.fleet)
                    .ok_or(Error::Invalid)?;
                let signature = p256::ecdsa::Signature::from_slice(&admit.joiner_signature)
                    .map_err(|_| Error::JoinSignature)?;
                p256::ecdsa::signature::Verifier::verify(
                    &admit.bundle.verifying_key()?,
                    &message,
                    &signature,
                )
                .map_err(|_| Error::JoinSignature)
            }
            Body::Remove(remove) => {
                let mut seen = BTreeSet::new();
                if remove
                    .targets()
                    .all(|target| members.live.contains_key(target) && seen.insert(*target))
                {
                    Ok(())
                } else {
                    Err(Error::Unknown)
                }
            }
            Body::Rotate(bundle) => {
                let current = members.live.get(&op.signer()).ok_or(Error::Signer)?;
                if bundle.key != current.bundle.key || bundle.kind != kind {
                    return Err(Error::Permission);
                }
                if taken(members, bundle, op.signer()) {
                    return Err(Error::Invalid);
                }
                Ok(())
            }
            Body::HostAdd(_) | Body::Access(_) | Body::Checkpoint(_) if computer => Ok(()),
            Body::HostSeen(seen) if computer || seen.signature.is_some() => Ok(()),
            Body::Link(_) if kind == Kind::Phone => Ok(()),
            Body::HostAdd(_)
            | Body::Access(_)
            | Body::Checkpoint(_)
            | Body::HostSeen(_)
            | Body::Link(_) => Err(Error::Permission),
        }
    }

    /// Membership of the membership operations in `set`.
    fn fold(&self, set: &Bits) -> Members {
        let checked: Vec<usize> = set
            .iter()
            .filter(|&position| self.checks.get(position).is_some_and(Result::is_ok))
            .collect();
        let removals = self.effective_removals(&checked);
        let mut members = Members {
            live: self.base.members.clone(),
            tombstones: self.base.tombstones.clone(),
            removals: Vec::new(),
            origins: BTreeMap::new(),
        };
        let removed: BTreeSet<MemberId> = removals
            .iter()
            .flat_map(|&removal| targets(self.graph.order[removal]).copied())
            .collect();
        let partial = Members {
            removals: removals.clone(),
            ..Members::default()
        };
        for &position in &checked {
            let op = self.graph.order[position];
            match op.body() {
                Body::Found(bundle) => {
                    members.origins.insert(bundle.id(), position);
                    members.live.insert(
                        bundle.id(),
                        Member {
                            bundle: bundle.clone(),
                            admitted_by: None,
                            seq: 0,
                        },
                    );
                }
                Body::Admit(admit) if !self.voided(&partial, position) => {
                    let id = admit.bundle.id();
                    let limit = match admit.bundle.kind {
                        Kind::Computer => MAX_COMPUTERS,
                        Kind::Phone => MAX_PHONES,
                    };
                    // Concurrent admissions can pass the bound each on
                    // its own; the first in order are kept.
                    if !removed.contains(&id)
                        && !members.tombstones.contains(&id)
                        && !members.live.contains_key(&id)
                        && members.kind_count(admit.bundle.kind) < limit
                    {
                        members.origins.insert(id, position);
                        members.live.entry(id).or_insert(Member {
                            bundle: admit.bundle.clone(),
                            admitted_by: Some(op.signer()),
                            seq: 0,
                        });
                    }
                }
                Body::Rotate(bundle) if !self.voided(&partial, position) => {
                    if let Some(member) = members.live.get_mut(&op.signer()) {
                        member.bundle = bundle.clone();
                        members.origins.insert(op.signer(), position);
                    }
                }
                _ => {}
            }
        }
        for id in removed {
            members.live.remove(&id);
            members.tombstones.insert(id);
        }
        members.removals = removals;
        members
    }

    /// The removals among `checked` that take effect.
    fn effective_removals(&self, checked: &[usize]) -> Vec<usize> {
        let removals: Vec<usize> = checked
            .iter()
            .copied()
            .filter(|&position| matches!(self.graph.order[position].body(), Body::Remove(_)))
            .collect();
        // Who could void each removal.
        let voiders: Vec<Vec<usize>> = removals
            .iter()
            .map(|&removal| {
                let could_counter = self.could_counter(removal, &removals);
                removals
                    .iter()
                    .copied()
                    .filter(|&other| self.voids(other, removal, could_counter))
                    .collect()
            })
            .collect();
        let mut decided: Vec<Option<bool>> = vec![None; removals.len()];
        let place: HashMap<usize, usize> = removals
            .iter()
            .enumerate()
            .map(|(slot, &position)| (position, slot))
            .collect();
        loop {
            let mut changed = false;
            for slot in 0..removals.len() {
                if decided[slot].is_some() {
                    continue;
                }
                let states = voiders[slot].iter().map(|other| decided[place[other]]);
                if states.clone().any(|state| state == Some(true)) {
                    decided[slot] = Some(false);
                    changed = true;
                } else if states.clone().all(|state| state == Some(false)) {
                    decided[slot] = Some(true);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // What is left voids itself in a cycle: fail closed.
        removals
            .into_iter()
            .zip(decided)
            .filter(|(_, state)| *state != Some(false))
            .map(|(position, _)| position)
            .collect()
    }

    /// Whether removal `by`, if it takes effect, voids removal `of`: it
    /// removes `of`'s signer without having seen `of`, and `of` is not a
    /// counter-removal of `by`'s signer. `counters` says, for `of`,
    /// whether it could be one ([`Folder::could_counter`]).
    fn voids(&self, by: usize, of: usize, could_counter: bool) -> bool {
        let (by_op, of_op) = (self.graph.order[by], self.graph.order[of]);
        by != of
            && !self.graph.sees(by, of)
            && targets(by_op).any(|target| *target == of_op.signer())
            && !(could_counter && targets(of_op).any(|target| *target == by_op.signer()))
    }

    /// Whether `of` removes only members that removed its signer without
    /// its having seen them, and no host: what a counter-removal may do. A
    /// removed member takes with it the members that removed it, and
    /// nothing more.
    fn could_counter(&self, of: usize, removals: &[usize]) -> bool {
        let of_op = self.graph.order[of];
        let Body::Remove(remove) = of_op.body() else {
            return false;
        };
        remove.also_hosts.is_empty()
            && remove.targets().all(|target| {
                removals.iter().any(|&other| {
                    let other_op = self.graph.order[other];
                    other_op.signer() == *target
                        && !self.graph.sees(of, other)
                        && targets(other_op).any(|removed| *removed == of_op.signer())
                })
            })
    }
}

/// Whether another live member already holds a key or the host `bundle`
/// names: its tunnel key, its dial key, or its own host (a copied intro,
/// perhaps with another user to log in as). Keys are public, so a bundle
/// naming another member's would make that member's sessions look like
/// its own.
fn taken(members: &Members, bundle: &super::Bundle, own: MemberId) -> bool {
    let host = bundle.host.as_ref().and_then(|intro| intro.id().ok());
    members.live.iter().any(|(member, other)| {
        *member != own
            && (other.bundle.tunnel == bundle.tunnel
                || other.bundle.dial == bundle.dial
                || host.is_some_and(|host| {
                    other.bundle.host.as_ref().and_then(|intro| intro.id().ok()) == Some(host)
                }))
    })
}

/// The members an operation removes; none unless it is a removal.
pub(super) fn targets(op: &Op) -> impl Iterator<Item = &MemberId> {
    let remove = match op.body() {
        Body::Remove(remove) => Some(remove),
        _ => None,
    };
    remove.into_iter().flat_map(super::Remove::targets)
}
