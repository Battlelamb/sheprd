//! The parts of a log handed over to this computer while it is in no
//! fleet ([`sync::Apply::more`]), kept in memory until the last part
//! arrives: the operation that admits this computer, or shares it as a
//! host, comes last. Kept per fleet the caller names, so two members
//! handing over at once never take each other's parts, and two handing
//! over the same fleet add to one set, as both send the same log.

use super::{Error, Result};
use crate::control::fleet::{Op, OpId};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Mutex, PoisonError},
};

/// How long parts wait for the rest.
const TTL_MS: u64 = 10 * 60 * 1000;
/// The most operation bytes all parts may hold: the largest log this
/// daemon keeps.
const MAX_BYTES: usize = crate::control::fleet::saved::MAX_SAVED;
/// Handovers kept at once.
const MAX_FLEETS: usize = 4;

#[derive(Debug)]
struct Parts {
    ops: Vec<Op>,
    ids: BTreeSet<OpId>,
    bytes: usize,
    since_ms: u64,
}

/// Every handover in progress, by the fleet its caller named.
#[derive(Debug, Default)]
pub(crate) struct Handovers(Mutex<BTreeMap<String, Parts>>);

impl Handovers {
    /// Keeps `ops`, a part of `fleet`'s log, under one lock from reading
    /// what was kept to writing it back. Parts past the bound are refused,
    /// and that fleet's parts all dropped.
    pub(crate) fn keep(&self, fleet: &str, ops: &[Op], now_ms: u64) -> Result<()> {
        let mut all = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        all.retain(|_, parts| now_ms.saturating_sub(parts.since_ms) <= TTL_MS);
        if !all.contains_key(fleet) && all.len() >= MAX_FLEETS {
            return Err(Error::JoinTooLarge);
        }
        let others: usize = all
            .iter()
            .filter(|(named, _)| *named != fleet)
            .map(|(_, parts)| parts.bytes)
            .sum();
        let parts = all.entry(fleet.to_owned()).or_insert_with(|| Parts {
            ops: Vec::new(),
            ids: BTreeSet::new(),
            bytes: 0,
            since_ms: now_ms,
        });
        for op in ops {
            if parts.ids.insert(op.id()) {
                parts.bytes = parts.bytes.saturating_add(op.encode().len());
                parts.ops.push(op.clone());
            }
        }
        if others.saturating_add(parts.bytes) > MAX_BYTES {
            all.remove(fleet);
            return Err(Error::JoinTooLarge);
        }
        Ok(())
    }

    /// Takes the parts kept for `fleet`, unless they waited too long.
    pub(crate) fn take(&self, fleet: &str, now_ms: u64) -> Vec<Op> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(fleet)
            .filter(|parts| now_ms.saturating_sub(parts.since_ms) <= TTL_MS)
            .map_or_else(Vec::new, |parts| parts.ops)
    }
}
