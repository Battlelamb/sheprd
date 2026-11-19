//! Phones on this computer's API through the fleet (`docs/relay-plan.md`,
//! What joining grants, A phone that joins): a live phone member that
//! signed a `Link` naming this computer's own host, with a device key it
//! made for it, is a paired device here, as a host link makes one.
//!
//! - The device is the API only: no SSH key and no GitHub sharing. Its
//!   relay keys are the dial and tunnel keys of the phone's own bundle,
//!   which the phone signed when it joined, so the relays let it through
//!   and the tunnel admits it to the API at once; a `Rotate` moves them.
//!   Keys are public, so keys that are another device's or another
//!   member's are never set: a bundle naming them must not divert their
//!   sessions. The device is still admitted, reachable by its other paths.
//! - Its id comes from its key ([`device_id`]), so the phone knows it
//!   without asking, and a phone that links another key is another device:
//!   the old one goes.
//! - It goes when the phone member is removed or suspended, or its link
//!   does.
//! - A device this module admitted that is gone from `devices.json` while
//!   its link stands was revoked here by hand, a local refusal: it is not
//!   admitted again. `fleet/links.json` keeps the ids admitted, with the
//!   relay keys last set on each, so this module only ever removes its own
//!   devices, and leaves relay keys a phone sent itself.

mod error;

pub use error::{Error, Result};

use super::{Fleet, grants::own_host};
use crate::auth::{DeviceRecord, NewDevice, PublicKey, devices::RelayKeys};
use crate::control::{
    DeviceId,
    fleet::{Bundle, Kind, MemberId, State as Log},
};
use crate::desktop::files;
use crate::web::State;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// The ids of the devices links admitted.
const FILE: &str = "fleet/links.json";
/// What every such device id starts with.
const PREFIX: &str = "fleet-";

/// The device id a phone's link key `key` (P-256, SEC1, either form) gets
/// on the host it names, with the key as devices keep it: `fleet-` and the
/// first 16 bytes of the SHA-256 of the key, uncompressed, in hex.
pub(crate) fn device_id(key: &[u8]) -> Option<(DeviceId, PublicKey)> {
    let point = p256::ecdsa::VerifyingKey::from_sec1_bytes(key).ok()?;
    let uncompressed = point.to_sec1_point(false);
    let hash = Sha256::digest(uncompressed.as_bytes());
    let id: String = hash[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let public_key = PublicKey::parse(&crate::auth::encode(uncompressed.as_bytes())).ok()?;
    Some((DeviceId(format!("{PREFIX}{id}")), public_key))
}

struct Wanted {
    public_key: PublicKey,
    name: String,
    /// The phone member's bundle keys, when they are no other member's.
    relay: Option<RelayKeys>,
}

/// Admits the devices the log links to this computer and removes those it
/// no longer does; whether any changed. Blocking: it writes
/// `devices.json` and `fleet/links.json`.
pub(crate) fn apply(state: &State, fleet: &Fleet, now_ms: u64) -> Result<bool> {
    let (wanted, via) = wanted(fleet);
    let file = fleet.state_dir().join(FILE);
    let before = read(&file)?;
    let mut kept: BTreeMap<String, Option<RelayKeys>> = BTreeMap::new();
    let mut changed = false;
    let mut auth = state.auth();
    for gone in before.keys().filter(|id| !wanted.contains_key(*id)) {
        changed |= auth.revoke(&DeviceId(gone.clone()))?;
    }
    for (id, device) in &wanted {
        let id = DeviceId(id.clone());
        let relay = device
            .relay
            .clone()
            .filter(|keys| !held_by_another(auth.devices().records(), &id, keys));
        if let Some(record) = auth.devices().get(&id) {
            let Some(set) = before.get(&id.0) else {
                // A device this module did not admit is not its own.
                continue;
            };
            // Its relay keys follow the bundle while they are the ones
            // this module set; keys the phone sent itself
            // (`POST /api/relay`) stay.
            if record.relay == *set && record.relay != relay {
                changed |= auth
                    .devices_mut()
                    .replace_relay(&id, relay.clone(), now_ms)?;
                kept.insert(id.0, relay);
            } else {
                kept.insert(id.0, set.clone());
            }
            continue;
        }
        if let Some(set) = before.get(&id.0) {
            // Revoked here by hand: a local refusal, kept.
            kept.insert(id.0, set.clone());
            continue;
        }
        kept.insert(id.0.clone(), relay.clone());
        auth.devices_mut().add_with(
            NewDevice {
                app: None,
                id,
                name: device.name.clone(),
                public_key: device.public_key.clone(),
                relay,
                ssh: None,
            },
            now_ms,
            Some(via.clone()),
            Vec::new(),
            // Sharing GitHub starts off on hosts the phone did not pair
            // with directly.
            false,
        )?;
        changed = true;
    }
    drop(auth);
    if kept != before {
        let json = serde_json::to_vec(&kept).map_err(|_| crate::desktop::Error::RecordFormat)?;
        files::write_private(&file, &json)?;
    }
    if changed {
        tracing::info!(
            devices = kept.len(),
            "fleet links changed this computer's devices"
        );
    }
    Ok(changed)
}

/// The devices this module admitted, by id, with the relay keys it last
/// set on each. A file from before relay keys, a list of ids, reads as
/// having set none.
fn read(file: &std::path::Path) -> Result<BTreeMap<String, Option<RelayKeys>>> {
    let Some(bytes) = files::read_bounded(file, 64 * 1024)? else {
        return Ok(BTreeMap::new());
    };
    if let Ok(map) = serde_json::from_slice(&bytes) {
        return Ok(map);
    }
    let ids: BTreeSet<String> = serde_json::from_slice(&bytes).unwrap_or_default();
    Ok(ids.into_iter().map(|id| (id, None)).collect())
}

/// The devices the log links to this computer, by id, and what they say
/// vouched for them.
fn wanted(fleet: &Fleet) -> (BTreeMap<String, Wanted>, String) {
    let suspended = fleet.suspended();
    let inner = fleet.inner();
    let Some(log) = inner.log.as_ref() else {
        return (BTreeMap::new(), String::new());
    };
    let state = log.state();
    let via = format!("fleet {}", log.fleet().short());
    let Some(host) = own_host(state, &fleet.me()) else {
        return (BTreeMap::new(), via);
    };
    let wanted = state
        .links
        .iter()
        .filter(|((phone, linked), _)| *linked == host && !suspended.contains(phone))
        .filter_map(|((phone, _), key)| {
            let member = state.members.get(phone)?;
            if member.bundle.kind != Kind::Phone {
                return None;
            }
            let (id, public_key) = device_id(key)?;
            let name = crate::auth::device_name(&member.bundle.name).ok()?;
            let relay = (!members_hold(state, &inner.proven, phone, &member.bundle))
                .then(|| bundle_keys(&member.bundle));
            Some((
                id.0,
                Wanted {
                    public_key,
                    name,
                    relay,
                },
            ))
        })
        .collect();
    (wanted, via)
}

/// A bundle's relay keys as devices keep them, in the encodings
/// `POST /api/relay` takes.
fn bundle_keys(bundle: &Bundle) -> RelayKeys {
    RelayKeys {
        dial: crate::auth::encode(&bundle.dial),
        tunnel: crate::auth::encode(&bundle.tunnel),
    }
}

/// Whether another live member's bundle, or a tunnel key another member
/// proved here, is `bundle`'s dial or tunnel key. The log already refuses
/// such a bundle among live members; this also covers keys a member proved
/// and rotated away from.
fn members_hold(
    state: &Log,
    proven: &BTreeMap<MemberId, BTreeSet<[u8; 32]>>,
    phone: &MemberId,
    bundle: &Bundle,
) -> bool {
    state.members.iter().any(|(id, member)| {
        id != phone && (member.bundle.dial == bundle.dial || member.bundle.tunnel == bundle.tunnel)
    }) || proven
        .iter()
        .any(|(id, keys)| id != phone && keys.contains(&bundle.tunnel))
}

/// Whether a device other than `id` holds either of `keys`.
fn held_by_another(records: &[DeviceRecord], id: &DeviceId, keys: &RelayKeys) -> bool {
    let (dial, tunnel) = (keys.dial_key(), keys.tunnel_key());
    records.iter().any(|record| {
        record.id != *id
            && record.relay.as_ref().is_some_and(|other| {
                (dial.is_some() && other.dial_key() == dial)
                    || (tunnel.is_some() && other.tunnel_key() == tunnel)
            })
    })
}
