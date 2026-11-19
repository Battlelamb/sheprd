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
//! removes, removals included. A removal also reaches the members its
//! target admitted that the remover had not seen, and theirs in turn: an
//! admission it voids leaves no member behind whose trust came only from
//! it. The one exception is a counter-removal of whoever removed it. Removals that void each other in a cycle take
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
    /// The removals that took effect, by place in the graph, with every
    /// member each removes ([`Folder::reach`]).
    pub(super) removals: BTreeMap<usize, BTreeSet<MemberId>>,
    /// Where each member first named the host its bundle names: its
    /// `Found`, `Admit`, or the `Rotate` that moved it to that host. A
    /// rotation that keeps the host keeps the claim where it was. A member
    /// from the base has none.
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
        members.removals.iter().any(|(&removal, reach)| {
            removal != position && !self.graph.sees(removal, position) && reach.contains(&signer)
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
        self.permitted(op, kind, &members)?;
        if let Body::Access(access) = op.body()
            && Some(access.rev)
                != self
                    .access_floor(position, access.host, access.member)
                    .checked_add(1)
        {
            return Err(Error::Rev);
        }
        Ok(())
    }

    /// The highest `rev` of an `Access` for `host` and `member` in the
    /// history of `position`, or the base's: the one an `Access` there
    /// must be one above. Every `Access` held passed this check, so none
    /// can put the register out of reach of a later deny, and whether one
    /// follows depends on its own history alone.
    fn access_floor(&self, position: usize, host: super::HostId, member: MemberId) -> u64 {
        let floor = self
            .base
            .access
            .get(&(host, member))
            .map_or(0, |register| register.rev);
        self.graph.ancestors[position]
            .iter()
            .filter(|&ancestor| self.checks.get(ancestor).is_some_and(Result::is_ok))
            .filter_map(|ancestor| match self.graph.order[ancestor].body() {
                Body::Access(other) if other.host == host && other.member == member => {
                    Some(other.rev)
                }
                _ => None,
            })
            .fold(floor, u64::max)
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
                    || host_taken(members, &admit.bundle, id)
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
                if host_taken(members, bundle, op.signer()) {
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
            removals: BTreeMap::new(),
            origins: BTreeMap::new(),
        };
        let removed: BTreeSet<MemberId> = removals.values().flatten().copied().collect();
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
                        if host_id(&member.bundle) != host_id(bundle) {
                            members.origins.insert(op.signer(), position);
                        }
                        member.bundle = bundle.clone();
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

    /// The removals among `checked` that take effect, each with every
    /// member it removes.
    fn effective_removals(&self, checked: &[usize]) -> BTreeMap<usize, BTreeSet<MemberId>> {
        let removals: Vec<usize> = checked
            .iter()
            .copied()
            .filter(|&position| matches!(self.graph.order[position].body(), Body::Remove(_)))
            .collect();
        let admissions: Vec<usize> = checked
            .iter()
            .copied()
            .filter(|&position| matches!(self.graph.order[position].body(), Body::Admit(_)))
            .collect();
        let mut reach: Vec<BTreeSet<MemberId>> = removals
            .iter()
            .map(|&removal| self.reach(removal, targets(self.graph.order[removal]), &admissions))
            .collect();
        // Settle the removals, then let the ones in effect reach together:
        // a member every admission of which some removal in effect voids
        // has no trust left, though no one removal voids them all. Reaches
        // only grow, so this ends; a removal it leaves out of effect keeps
        // what it reached, which can only remove more (fail closed).
        loop {
            let decided = self.decide(&removals, &reach, &admissions);
            if !self.join_reach(&removals, &decided, &mut reach, &admissions) {
                return removals
                    .into_iter()
                    .zip(reach)
                    .zip(decided)
                    .filter(|(_, effective)| *effective)
                    .map(|(entry, _)| entry)
                    .collect();
            }
        }
    }

    /// Which `removals` take effect, given what each reaches.
    fn decide(
        &self,
        removals: &[usize],
        reach: &[BTreeSet<MemberId>],
        admissions: &[usize],
    ) -> Vec<bool> {
        let could_counter: Vec<bool> = (0..removals.len())
            .map(|slot| self.could_counter(slot, removals, reach, admissions))
            .collect();
        // Who could void each removal.
        let voiders: Vec<Vec<usize>> = (0..removals.len())
            .map(|of| {
                (0..removals.len())
                    .filter(|&by| self.voids(removals, reach, by, of, could_counter[of]))
                    .collect()
            })
            .collect();
        let mut decided: Vec<Option<bool>> = vec![None; removals.len()];
        loop {
            let mut changed = false;
            for slot in 0..removals.len() {
                if decided[slot].is_some() {
                    continue;
                }
                let states = voiders[slot].iter().map(|&other| decided[other]);
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
        decided
            .into_iter()
            .map(|state| state != Some(false))
            .collect()
    }

    /// Adds to the reach of the removals in effect every member each of
    /// whose admissions one of them voids: one that reaches its signer and
    /// had not seen it. The member joins the reach of each removal that
    /// voided one, so whatever it signed on that admission is void too.
    /// Says whether any reach grew.
    fn join_reach(
        &self,
        removals: &[usize],
        effective: &[bool],
        reach: &mut [BTreeSet<MemberId>],
        admissions: &[usize],
    ) -> bool {
        let mut grew = false;
        loop {
            let voiding = |admission: usize, reach: &[BTreeSet<MemberId>]| -> Vec<usize> {
                let signer = self.graph.order[admission].signer();
                (0..removals.len())
                    .filter(|&slot| {
                        effective[slot]
                            && reach[slot].contains(&signer)
                            && !self.graph.sees(removals[slot], admission)
                    })
                    .collect()
            };
            let mut joined: Vec<(usize, MemberId)> = Vec::new();
            let mut by_member: BTreeMap<MemberId, Vec<usize>> = BTreeMap::new();
            for &admission in admissions {
                if let Body::Admit(admit) = self.graph.order[admission].body() {
                    by_member
                        .entry(admit.bundle.id())
                        .or_default()
                        .push(admission);
                }
            }
            for (member, admitted_by) in by_member {
                if self.base.members.contains_key(&member) || self.base.tombstones.contains(&member)
                {
                    continue;
                }
                let voided: Vec<Vec<usize>> = admitted_by
                    .iter()
                    .map(|&admission| voiding(admission, reach))
                    .collect();
                if voided.iter().all(|slots| !slots.is_empty()) {
                    joined.extend(
                        voided
                            .into_iter()
                            .flatten()
                            .filter(|&slot| !reach[slot].contains(&member))
                            .map(|slot| (slot, member)),
                    );
                }
            }
            if joined.is_empty() {
                return grew;
            }
            grew = true;
            for (slot, member) in joined {
                reach[slot].insert(member);
            }
        }
    }

    /// Every member removal `removal` removes when it starts from `seeds`:
    /// those, and every member one of them admitted that `removal` had
    /// not seen, and so on. Such an admission is void with its signer, so
    /// whoever it admitted has no trust left of its own.
    ///
    /// Only a member every admission of which is void that way: a join
    /// signature is not bound to one admission, so a removed member can
    /// replay it in an admission of its own, and that must not take a
    /// member another admission let in. A member of the base was let in
    /// before any of these.
    fn reach<'m>(
        &self,
        removal: usize,
        seeds: impl Iterator<Item = &'m MemberId>,
        admissions: &[usize],
    ) -> BTreeSet<MemberId> {
        let mut reach: BTreeSet<MemberId> = seeds.copied().collect();
        let admitted = |admission: usize| match self.graph.order[admission].body() {
            Body::Admit(admit) => Some(admit.bundle.id()),
            _ => None,
        };
        loop {
            let before = reach.len();
            let void = |admission: usize, reach: &BTreeSet<MemberId>| {
                reach.contains(&self.graph.order[admission].signer())
                    && !self.graph.sees(removal, admission)
            };
            let joined: Vec<MemberId> = admissions
                .iter()
                .filter(|&&admission| void(admission, &reach))
                .filter_map(|&admission| admitted(admission))
                .filter(|id| {
                    !reach.contains(id)
                        && !self.base.members.contains_key(id)
                        && !self.base.tombstones.contains(id)
                        && admissions
                            .iter()
                            .filter(|&&other| admitted(other) == Some(*id))
                            .all(|&other| void(other, &reach))
                })
                .collect();
            reach.extend(joined);
            if reach.len() == before {
                return reach;
            }
        }
    }

    /// Whether removal `by`, if it takes effect, voids removal `of`: it
    /// removes `of`'s signer without having seen `of`, and `of` is not a
    /// counter-removal of `by`'s signer. `could_counter` says, for `of`,
    /// whether it could be one ([`Folder::could_counter`]).
    fn voids(
        &self,
        removals: &[usize],
        reach: &[BTreeSet<MemberId>],
        by: usize,
        of: usize,
        could_counter: bool,
    ) -> bool {
        let (by_op, of_op) = (
            self.graph.order[removals[by]],
            self.graph.order[removals[of]],
        );
        by != of
            && !self.graph.sees(removals[by], removals[of])
            && reach[by].contains(&of_op.signer())
            && !(could_counter && reach[of].contains(&by_op.signer()))
    }

    /// Whether removal `of` removes only members that removed its signer
    /// without its having seen them, each itself or through a member it
    /// admitted that `of` had not seen, and no host: what a
    /// counter-removal may do. A removed member takes with it the members
    /// that removed it, and nothing more.
    fn could_counter(
        &self,
        of: usize,
        removals: &[usize],
        reach: &[BTreeSet<MemberId>],
        admissions: &[usize],
    ) -> bool {
        let of_op = self.graph.order[removals[of]];
        let Body::Remove(remove) = of_op.body() else {
            return false;
        };
        remove.also_hosts.is_empty()
            && remove.targets().all(|target| {
                let lineage = self.reach(removals[of], std::iter::once(target), admissions);
                removals.iter().zip(reach).any(|(&other, other_reach)| {
                    lineage.contains(&self.graph.order[other].signer())
                        && !self.graph.sees(removals[of], other)
                        && other_reach.contains(&of_op.signer())
                })
            })
    }
}

/// The host a bundle names as its own.
fn host_id(bundle: &super::Bundle) -> Option<super::HostId> {
    bundle.host.as_ref().and_then(|intro| intro.id().ok())
}

/// Whether another live member already names the host `bundle` names: a
/// copied intro, perhaps with another user to log in as.
fn host_taken(members: &Members, bundle: &super::Bundle, own: MemberId) -> bool {
    let Some(id) = host_id(bundle) else {
        return false;
    };
    members
        .live
        .iter()
        .any(|(member, other)| *member != own && host_id(&other.bundle) == Some(id))
}

/// The members an operation removes; none unless it is a removal.
pub(super) fn targets(op: &Op) -> impl Iterator<Item = &MemberId> {
    let remove = match op.body() {
        Body::Remove(remove) => Some(remove),
        _ => None,
    };
    remove.into_iter().flat_map(super::Remove::targets)
}
