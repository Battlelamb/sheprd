//! Suspensions: members cut off by a removal whose history has not
//! arrived (`docs/relay-plan.md`, Suspend first, confirm after).
//!
//! Each removal is kept apart ([`Unconfirmed`]), with every target it
//! named, until its history settles it, its signer no longer stands for
//! it, or its TTL passes: at most [`MAX_UNCONFIRMED_PER_SIGNER`] for a live
//! signer and none for a signer no longer live. Older ones fold into their
//! signer's [`Folded`] suspensions: each target with the latest arrival of
//! the removals that named it, and every member those removals named
//! beside it with the latest arrival of a removal naming it, so its
//! signer's standing is judged by the same rule as for each removal on its
//! own, and a name ends with the removals that gave it. However many removals a member sends, what is
//! kept is bounded by the members, and no target is let back in before a
//! removal that named it would have let it in.

use super::{Inner, MAX_KEPT_REMOVALS, UNCONFIRMED_TTL_MS, Unconfirmed};
use crate::control::fleet::{Error, Inserted, Log, MemberId, Op, OpId};
use std::collections::{BTreeMap, BTreeSet};

/// Removals a live signer keeps apart, each with its targets and, for the
/// newest, the operation to check again: each is settled by its own
/// history.
pub(crate) const MAX_UNCONFIRMED_PER_SIGNER: usize = 16;

/// One target's suspension by one signer's removals past those kept apart.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FoldedTarget {
    /// When the latest removal that named it arrived: it ends on that
    /// TTL, no sooner than any of them.
    pub(crate) since_ms: u64,
    /// Every member those removals named, each with when the latest
    /// removal naming it arrived, so a name ends with the removals that
    /// gave it: the signer stands for this target while it is a member,
    /// or once a named member removed it, as for each removal on its own.
    pub(crate) named: BTreeMap<MemberId, u64>,
    /// The signer is no longer a member, and a removal naming a member
    /// that removed it stood: when the latest such removal arrived. It
    /// stands until that removal's TTL, and `named` is dropped.
    pub(crate) pinned: Option<u64>,
}

/// One signer's folded suspensions, by target.
pub(crate) type Folded = BTreeMap<MemberId, FoldedTarget>;

/// Folded suspensions, by signer.
pub(crate) type FoldedMap = BTreeMap<MemberId, Folded>;

/// The removals waiting for their history now: taken before a batch,
/// after expiring the pool at the batch's time, so a removal that leaves
/// the pool during the batch was taken in, refused, or dropped for room,
/// never expired.
pub(super) fn waiting(log: &mut Log, now_ms: u64) -> Vec<Op> {
    let _ = log.insert_all(Vec::new(), now_ms);
    log.pending_removals()
        .into_iter()
        .map(|(op, _)| op.clone())
        .collect()
}

/// Of the removals `waiting` before a batch, those the log refused once
/// their history arrived: no longer waiting, not held, every parent held,
/// and refused again when put to the log once more for a reason that
/// judges the removal itself. Room (a full log, the bounds) or a missing
/// parent says nothing of it, and keeps its suspension.
pub(super) fn refused(log: &mut Log, waiting: Vec<Op>, now_ms: u64) -> BTreeSet<OpId> {
    let still: BTreeSet<OpId> = log
        .pending_removals()
        .into_iter()
        .map(|(op, _)| op.id())
        .collect();
    let mut refused = BTreeSet::new();
    for op in waiting {
        let id = op.id();
        if still.contains(&id)
            || log.get(&id).is_some()
            || !op.parents().iter().all(|parent| log.get(parent).is_some())
        {
            continue;
        }
        if refuses(&log.insert(op, now_ms)) {
            refused.insert(id);
        }
    }
    refused
}

/// Whether putting a removal to the log refused it for what it is, not for
/// want of room or of its parents.
pub(super) fn refuses(result: &crate::control::fleet::Result<Inserted>) -> bool {
    !matches!(
        result,
        Ok(_) | Err(Error::Full | Error::Bounds | Error::ParentMissing)
    )
}

/// Moves the log's pending removals into `unconfirmed`, drops those the
/// log settled (taken in, or refused once their history arrived, as
/// `refused` says or checking a kept removal again shows), and folds what
/// is past the bounds.
pub(super) fn settle(inner: &mut Inner, now_ms: u64, mut refused: BTreeSet<OpId>) {
    let Some(log) = &mut inner.log else {
        return;
    };
    let pending: Vec<(Op, Vec<MemberId>)> = log
        .pending_removals()
        .into_iter()
        .map(|(op, targets)| (op.clone(), targets))
        .collect();
    // A removal folded while it still waits is not kept apart again: that
    // would restart its time.
    inner
        .folded_pending
        .retain(|id| pending.iter().any(|(op, _)| op.id() == *id));
    for (op, targets) in pending {
        let id = op.id();
        if inner.folded_pending.contains(&id) {
            continue;
        }
        inner.unconfirmed.entry(id).or_insert(Unconfirmed {
            signer: op.signer(),
            targets,
            since_ms: now_ms,
            op: Some(op),
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
        if refuses(&log.insert(op, now_ms)) {
            refused.insert(id);
        }
    }
    let log: &Log = log;
    let fresh = |since: u64| now_ms.saturating_sub(since) <= UNCONFIRMED_TTL_MS;
    inner.unconfirmed.retain(|id, kept| {
        log.get(id).is_none()
            && !refused.contains(id)
            && standing(log, kept)
            && fresh(kept.since_ms)
    });
    for (signer, folded) in &mut inner.folded {
        let live = log.is_member(signer);
        let removers = log.removers(signer);
        for kept in folded.values_mut() {
            kept.named.retain(|_, since| fresh(*since));
            kept.pinned = kept.pinned.filter(|since| fresh(*since));
            // No longer a member: a name that removed it stands for good,
            // until the removal that gave it ends; the rest never will.
            if !live {
                let stood = kept
                    .named
                    .iter()
                    .filter(|(name, _)| removers.contains(name))
                    .map(|(_, since)| *since)
                    .max();
                kept.pinned = kept.pinned.max(stood);
                kept.named.clear();
            }
        }
        folded.retain(|target, kept| {
            log.is_member(target)
                && fresh(kept.since_ms)
                && target_standing(log, signer, kept, now_ms)
        });
    }
    inner.folded.retain(|_, folded| !folded.is_empty());
    fold(
        &mut inner.unconfirmed,
        &mut inner.folded,
        &mut inner.folded_pending,
        log,
    );
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
    let fresh = |since: u64| now_ms.saturating_sub(since) <= UNCONFIRMED_TTL_MS;
    let kept = inner
        .unconfirmed
        .values()
        .filter(|kept| standing(log, kept) && fresh(kept.since_ms))
        .flat_map(|kept| kept.targets.iter().copied());
    let folded = inner.folded.iter().flat_map(|(signer, folded)| {
        folded
            .iter()
            .filter(move |(_, kept)| {
                fresh(kept.since_ms) && target_standing(log, signer, kept, now_ms)
            })
            .map(|(target, _)| *target)
    });
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
    stands(log, &kept.signer, kept.targets.iter())
}

/// The rule [`standing`] applies to a removal by `signer` of `targets`.
fn stands<'a>(
    log: &Log,
    signer: &MemberId,
    targets: impl IntoIterator<Item = &'a MemberId>,
) -> bool {
    if log.is_member(signer) {
        return true;
    }
    let removers = log.removers(signer);
    targets.into_iter().any(|target| removers.contains(target))
}

/// Whether `signer`'s folded suspension of one target stands at `now_ms`:
/// some removal that named it, still within its TTL, stands by [`stands`]
/// exactly when the signer's removers meet the members such removals
/// named, which is `named`; or it was pinned within that time.
fn target_standing(log: &Log, signer: &MemberId, kept: &FoldedTarget, now_ms: u64) -> bool {
    let fresh = |since: &u64| now_ms.saturating_sub(*since) <= UNCONFIRMED_TTL_MS;
    kept.pinned.as_ref().is_some_and(fresh)
        || stands(
            log,
            signer,
            kept.named
                .iter()
                .filter(|(_, since)| fresh(since))
                .map(|(name, _)| name),
        )
}

/// Bounds what is kept: targets no longer members go (a removed member
/// never comes back), and past [`MAX_UNCONFIRMED_PER_SIGNER`] for a live
/// signer, or any for a signer no longer live, its oldest removals fold
/// into its [`Folded`] suspensions, each target keeping the latest
/// arrival.
fn fold(
    unconfirmed: &mut BTreeMap<OpId, Unconfirmed>,
    folded_map: &mut FoldedMap,
    folded_pending: &mut BTreeSet<OpId>,
    log: &Log,
) {
    for kept in unconfirmed.values_mut() {
        kept.targets.retain(|target| log.is_member(target));
    }
    unconfirmed.retain(|_, kept| !kept.targets.is_empty());
    let mut by_signer: BTreeMap<MemberId, Vec<(u64, OpId)>> = BTreeMap::new();
    for (id, kept) in unconfirmed.iter() {
        by_signer
            .entry(kept.signer)
            .or_default()
            .push((kept.since_ms, *id));
    }
    let waiting: BTreeSet<OpId> = log
        .pending_removals()
        .into_iter()
        .map(|(op, _)| op.id())
        .collect();
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
            if waiting.contains(id) {
                folded_pending.insert(*id);
            }
            let live = log.is_member(&signer);
            let folded = folded_map.entry(signer).or_default();
            for target in &kept.targets {
                let entry = folded.entry(*target).or_default();
                entry.since_ms = entry.since_ms.max(kept.since_ms);
                if live {
                    for name in &kept.targets {
                        let since = entry.named.entry(*name).or_insert(kept.since_ms);
                        *since = (*since).max(kept.since_ms);
                    }
                } else {
                    // Kept apart, it stood: it stands until its own TTL.
                    entry.pinned = entry.pinned.max(Some(kept.since_ms));
                }
            }
        }
    }
}
