//! Suspensions: members cut off by a removal whose history has not
//! arrived (`docs/relay-plan.md`, Suspend first, confirm after).
//!
//! Each removal is kept apart ([`Unconfirmed`]) until its history settles
//! it, at most [`MAX_UNCONFIRMED_PER_SIGNER`] for a live signer. Older ones
//! fold per signer and target ([`Folded`]), keeping the ids of the
//! removals behind them, so each is still settled by its own history,
//! checked against its own signer's standing, and ended by its own TTL,
//! never moved by a later one. However many removals a member
//! sends, what is kept is bounded by the members, and no suspension ends
//! sooner than it would have kept apart.

use super::{Inner, MAX_KEPT_REMOVALS, UNCONFIRMED_TTL_MS, Unconfirmed};
use crate::control::fleet::{Error, Log, MemberId, Op, OpId};
use std::collections::{BTreeMap, BTreeSet};

/// Removals a live signer keeps apart, each with its targets and, for the
/// newest, the operation to check again.
pub(crate) const MAX_UNCONFIRMED_PER_SIGNER: usize = 8;
/// Removal ids a folded suspension keeps for one target of a live signer;
/// one for a signer no longer live. Past it, the suspension is
/// `overflowed`.
pub(crate) const MAX_FOLDED_IDS: usize = 8;

/// One target's suspension by one signer's removals that were folded.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Folded {
    /// The removals that cut this target off, with when each arrived:
    /// each is settled by its own history and ends on its own TTL.
    pub(crate) ids: BTreeMap<OpId, u64>,
    /// More removals cut it off than `ids` keeps: when the first of those
    /// arrived. They end on its TTL, or once the signer no longer stands
    /// for this target.
    pub(crate) overflowed: Option<u64>,
}

impl Folded {
    /// Whether anything still holds the target at `now_ms`.
    fn live(&self, now_ms: u64) -> bool {
        let fresh = |since: &u64| now_ms.saturating_sub(*since) <= UNCONFIRMED_TTL_MS;
        self.ids.values().any(fresh) || self.overflowed.as_ref().is_some_and(fresh)
    }

    /// Adds removal `id`, which arrived at `since_ms`, within `cap` ids.
    fn add(&mut self, id: OpId, since_ms: u64, cap: usize) {
        if self.ids.contains_key(&id) {
            return;
        }
        if self.ids.len() < cap {
            self.ids.insert(id, since_ms);
        } else {
            self.overflow(since_ms);
        }
    }

    /// Counts a removal that arrived at `since_ms` past the ids kept.
    fn overflow(&mut self, since_ms: u64) {
        self.overflowed = Some(self.overflowed.map_or(since_ms, |kept| kept.min(since_ms)));
    }
}

/// Folded suspensions, by signer and target.
pub(crate) type FoldedMap = BTreeMap<(MemberId, MemberId), Folded>;

/// The removals waiting for their history now, with their parents: taken
/// before a batch, after expiring the pool at the batch's time, so a
/// removal that leaves the pool during the batch was taken or refused,
/// never expired.
pub(super) fn waiting(log: &mut Log, now_ms: u64) -> Vec<(OpId, Vec<OpId>)> {
    let _ = log.insert_all(Vec::new(), now_ms);
    log.pending_removals()
        .into_iter()
        .map(|(op, _)| (op.id(), op.parents().to_vec()))
        .collect()
}

/// Of the removals `waiting` before a batch, those the log refused once
/// their history arrived: no longer waiting, not held, every parent held.
pub(super) fn refused(log: &Log, waiting: &[(OpId, Vec<OpId>)]) -> BTreeSet<OpId> {
    let still: BTreeSet<OpId> = log
        .pending_removals()
        .into_iter()
        .map(|(op, _)| op.id())
        .collect();
    waiting
        .iter()
        .filter(|(id, parents)| {
            !still.contains(id)
                && log.get(id).is_none()
                && parents.iter().all(|parent| log.get(parent).is_some())
        })
        .map(|(id, _)| *id)
        .collect()
}

/// Moves the log's pending removals into `unconfirmed`, drops those the
/// log settled (taken in, or refused once their history arrived, as
/// `refused` says or checking a kept removal again shows), and folds what
/// is past the bounds.
pub(super) fn settle(inner: &mut Inner, now_ms: u64, mut refused: BTreeSet<OpId>) {
    let Some(log) = &mut inner.log else {
        return;
    };
    for (op, targets) in log.pending_removals() {
        let id = op.id();
        if inner
            .folded
            .values()
            .any(|folded| folded.ids.contains_key(&id))
        {
            continue;
        }
        inner.unconfirmed.entry(id).or_insert(Unconfirmed {
            signer: op.signer(),
            targets,
            since_ms: now_ms,
            op: Some(op.clone()),
        });
    }
    // A removal whose history is now held, and that the log does not
    // hold, either expired from the pending pool before its history came
    // or was refused with it: put it to the log again. Only a refusal the
    // history proves ends the suspension; a full log keeps it.
    let ready: Vec<Op> = inner
        .unconfirmed
        .iter()
        .filter_map(|(id, kept)| kept.op.as_ref().filter(|_| log.get(id).is_none()))
        .filter(|op| op.parents().iter().all(|parent| log.get(parent).is_some()))
        .cloned()
        .collect();
    for op in ready {
        let id = op.id();
        match log.insert(op, now_ms) {
            Err(Error::Full | Error::Bounds | Error::ParentMissing) | Ok(_) => {}
            Err(_) => {
                refused.insert(id);
            }
        }
    }
    let log: &Log = log;
    inner.unconfirmed.retain(|id, kept| {
        log.get(id).is_none()
            && !refused.contains(id)
            && standing(log, kept)
            && now_ms.saturating_sub(kept.since_ms) <= UNCONFIRMED_TTL_MS
    });
    let fresh = |since: &u64| now_ms.saturating_sub(*since) <= UNCONFIRMED_TTL_MS;
    for folded in inner.folded.values_mut() {
        folded
            .ids
            .retain(|id, since| log.get(id).is_none() && !refused.contains(id) && fresh(since));
        folded.overflowed = folded.overflowed.filter(fresh);
    }
    let lifted: Vec<(MemberId, MemberId)> = inner
        .folded
        .iter()
        .filter(|(key, folded)| {
            (folded.ids.is_empty() && folded.overflowed.is_none())
                || !folded_standing(log, &inner.folded, key)
        })
        .map(|(key, _)| *key)
        .collect();
    for key in lifted {
        inner.folded.remove(&key);
    }
    fold(&mut inner.unconfirmed, &mut inner.folded, log);
    // Bounded: the oldest give up their removal, never their suspension.
    let mut kept: Vec<(u64, OpId)> = inner
        .unconfirmed
        .iter()
        .filter(|(_, kept)| kept.op.is_some())
        .map(|(id, kept)| (kept.since_ms, *id))
        .collect();
    if kept.len() > MAX_KEPT_REMOVALS {
        kept.sort_unstable();
        for (_, id) in &kept[..kept.len() - MAX_KEPT_REMOVALS] {
            if let Some(entry) = inner.unconfirmed.get_mut(id) {
                entry.op = None;
            }
        }
    }
}

/// The members suspended at `now_ms`, before leaving out this computer and
/// those no longer members.
pub(super) fn suspended(inner: &Inner, log: &Log, now_ms: u64) -> BTreeSet<MemberId> {
    let kept = inner
        .unconfirmed
        .values()
        .filter(|kept| {
            standing(log, kept) && now_ms.saturating_sub(kept.since_ms) <= UNCONFIRMED_TTL_MS
        })
        .flat_map(|kept| kept.targets.iter().copied());
    let folded = inner
        .folded
        .iter()
        .filter(|(key, folded)| folded.live(now_ms) && folded_standing(log, &inner.folded, key))
        .map(|((_, target), _)| *target);
    let pending = log
        .pending_removals()
        .into_iter()
        .flat_map(|(_, targets)| targets);
    kept.chain(folded).chain(pending).collect()
}

/// Whether a suspension still stands: its signer is a member, or was
/// removed by one of the members it suspends. A suspended member cannot
/// lift its own suspension by removing whoever signed it; the history,
/// once it arrives, settles both removals.
fn standing(log: &Log, kept: &Unconfirmed) -> bool {
    stands(log, &kept.signer, &kept.targets)
}

/// The rule [`standing`] applies to one removal by `signer` of `targets`.
fn stands(log: &Log, signer: &MemberId, targets: &[MemberId]) -> bool {
    log.is_member(signer)
        || log
            .removers(signer)
            .iter()
            .any(|remover| targets.contains(remover))
}

/// Whether the folded suspension of `key`'s target by its signer stands:
/// by the rule for one removal, for any removal folded into it, with the
/// targets that removal named (every target whose folded suspension by
/// the same signer keeps its id); once overflowed, by the same rule for
/// this target alone.
fn folded_standing(log: &Log, folded: &FoldedMap, key: &(MemberId, MemberId)) -> bool {
    let (signer, target) = key;
    let Some(this) = folded.get(key) else {
        return false;
    };
    if this.overflowed.is_some() && stands(log, signer, &[*target]) {
        return true;
    }
    this.ids.keys().any(|id| {
        let targets: Vec<MemberId> = folded
            .range((*signer, MemberId([0; 32]))..=(*signer, MemberId([0xff; 32])))
            .filter(|(_, other)| other.ids.contains_key(id))
            .map(|((_, target), _)| *target)
            .collect();
        stands(log, signer, &targets)
    })
}

/// Bounds what is kept: targets no longer members go (a removed member
/// never comes back), and past [`MAX_UNCONFIRMED_PER_SIGNER`] a signer's
/// oldest removals fold, target by target, into [`Folded`] suspensions
/// that keep their ids and when each arrived.
fn fold(unconfirmed: &mut BTreeMap<OpId, Unconfirmed>, folded_map: &mut FoldedMap, log: &Log) {
    for kept in unconfirmed.values_mut() {
        kept.targets.retain(|target| log.is_member(target));
    }
    unconfirmed.retain(|_, kept| !kept.targets.is_empty());
    folded_map.retain(|(_, target), _| log.is_member(target));
    let mut by_signer: BTreeMap<MemberId, Vec<(u64, OpId)>> = BTreeMap::new();
    for (id, kept) in unconfirmed.iter() {
        by_signer
            .entry(kept.signer)
            .or_default()
            .push((kept.since_ms, *id));
    }
    for (signer, mut ids) in by_signer {
        let limit = if log.is_member(&signer) {
            MAX_UNCONFIRMED_PER_SIGNER
        } else {
            0
        };
        if ids.len() <= limit {
            continue;
        }
        ids.sort_unstable();
        for (_, id) in &ids[..ids.len() - limit] {
            let Some(kept) = unconfirmed.remove(id) else {
                continue;
            };
            for target in kept.targets {
                folded_map.entry((signer, target)).or_default().add(
                    *id,
                    kept.since_ms,
                    MAX_FOLDED_IDS,
                );
            }
        }
    }
    // A signer no longer live keeps one id per target.
    for ((signer, _), folded) in folded_map.iter_mut() {
        if log.is_member(signer) {
            continue;
        }
        while folded.ids.len() > 1 {
            if let Some((_, since)) = folded.ids.pop_last() {
                folded.overflow(since);
            }
        }
    }
}
