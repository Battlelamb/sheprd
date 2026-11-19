//! One device per relay key. The relays let a device through by its dial
//! key, and the tunnel names the device by its tunnel key, so two records
//! holding one key would let either's sessions pass as the other's. Keys
//! are public: anyone may name another device's.
//!
//! A device paired here sends its keys over its own signed-in session,
//! so it takes them from a device the fleet admitted (`fleet::links`),
//! whose keys are only what a phone member's bundle names. Every other
//! clash is refused.

use super::{DeviceRecord, Devices, RelayKeys};
use crate::auth::{Error, Result};
use crate::control::DeviceId;

/// What the ids of devices the fleet admits start with (`fleet::links`).
pub(crate) const FLEET_ID: &str = "fleet-";
/// What their `via` starts with.
pub(crate) const FLEET_VIA: &str = "fleet ";

/// A record cleared to make room, as it was.
pub(super) type Cleared = Vec<(usize, Option<RelayKeys>, Option<u64>)>;

impl DeviceRecord {
    /// Admitted by a phone member's fleet link rather than paired here.
    pub(crate) fn admitted_by_fleet(&self) -> bool {
        is_fleet(&self.id, self.via.as_deref())
    }
}

fn is_fleet(id: &DeviceId, via: Option<&str>) -> bool {
    id.0.starts_with(FLEET_ID) && via.is_some_and(|via| via.starts_with(FLEET_VIA))
}

/// Whether `keys` and `other` share a dial or a tunnel key.
fn clash(keys: &RelayKeys, other: &RelayKeys) -> bool {
    let (dial, tunnel) = (keys.dial_key(), keys.tunnel_key());
    (dial.is_some() && other.dial_key() == dial)
        || (tunnel.is_some() && other.tunnel_key() == tunnel)
}

impl Devices {
    /// The records other than `id` holding any of `keys`.
    fn holders(&self, id: &DeviceId, keys: &RelayKeys) -> Vec<usize> {
        self.records
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                record.id != *id
                    && record
                        .relay
                        .as_ref()
                        .is_some_and(|other| clash(keys, other))
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Whether a record other than `id` holds any of `keys`.
    pub(crate) fn relay_taken(&self, id: &DeviceId, keys: &RelayKeys) -> bool {
        !self.holders(id, keys).is_empty()
    }

    /// Makes room for `keys` on the record `id` (whose `via` is `via`):
    /// clears them from fleet records when `id` was paired here, and
    /// refuses any other clash. Answers what it cleared, to put back if
    /// the write fails. Saves nothing.
    pub(super) fn make_room(
        &mut self,
        id: &DeviceId,
        via: Option<&str>,
        keys: &RelayKeys,
    ) -> Result<Cleared> {
        let holders = self.holders(id, keys);
        let paired_here = !is_fleet(id, via);
        if holders
            .iter()
            .any(|&index| !paired_here || !self.records[index].admitted_by_fleet())
        {
            return Err(Error::RelayKeysTaken);
        }
        Ok(holders
            .into_iter()
            .map(|index| {
                let record = &mut self.records[index];
                (index, record.relay.take(), record.relay_ms.take())
            })
            .collect())
    }

    /// Puts back what [`Devices::make_room`] cleared.
    pub(super) fn restore(&mut self, cleared: Cleared) {
        for (index, relay, relay_ms) in cleared {
            if let Some(record) = self.records.get_mut(index) {
                (record.relay, record.relay_ms) = (relay, relay_ms);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
