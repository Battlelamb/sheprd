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
//!   admitted again, even after a suspension of the phone comes and goes.
//!   `fleet/links.json` keeps the ids admitted, with the relay keys last
//!   set on each, and those refused, so this module only ever removes its
//!   own devices, and leaves relay keys a phone sent itself.

mod error;

pub use error::{Error, Result};

use super::{Fleet, grants::own_host};
use crate::auth::{
    NewDevice, PublicKey,
    devices::{
        RelayKeys,
        relay_keys::{FLEET_ID, FLEET_VIA},
    },
};
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
    Some((DeviceId(format!("{FLEET_ID}{id}")), public_key))
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
    let (wanted, linked, via) = wanted(fleet);
    let file = fleet.state_dir().join(FILE);
    let stored = read(&file)?;
    let mut before = stored.clone();
    let mut kept = Kept::default();
    let mut changed = false;
    let mut auth = state.auth();
    // A device marked as the fleet's (its id and `via`) that the file does
    // not name was added before a write of the file that never landed: it
    // is this module's, so a removal still reaches it.
    for record in auth.devices().records() {
        let marked = record.id.0.starts_with(FLEET_ID)
            && record
                .via
                .as_deref()
                .is_some_and(|via| via.starts_with(FLEET_VIA));
        if marked && !before.refused.contains(&record.id.0) {
            before
                .admitted
                .entry(record.id.0.clone())
                .or_insert_with(|| record.relay.clone());
        }
    }
    // A device this module admitted that is gone from `devices.json` was
    // revoked here by hand: a local refusal, kept while its link stands,
    // suspended or not, so a suspension lifted later never brings it back.
    kept.refused = before
        .admitted
        .keys()
        .filter(|id| auth.devices().get(&DeviceId((*id).clone())).is_none())
        .chain(&before.refused)
        .filter(|id| linked.contains(*id))
        .cloned()
        .collect();
    for gone in before
        .admitted
        .keys()
        .filter(|id| !wanted.contains_key(*id))
    {
        changed |= auth.revoke(&DeviceId(gone.clone()))?;
    }
    for (id, device) in &wanted {
        let id = DeviceId(id.clone());
        if kept.refused.contains(&id.0) {
            continue;
        }
        let relay = device
            .relay
            .clone()
            .filter(|keys| !auth.devices().relay_taken(&id, keys));
        if let Some(record) = auth.devices().get(&id) {
            let Some(set) = before.admitted.get(&id.0) else {
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
                kept.admitted.insert(id.0, relay);
            } else {
                kept.admitted.insert(id.0, set.clone());
            }
            continue;
        }
        kept.admitted.insert(id.0.clone(), relay.clone());
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
    if kept != stored {
        let json = serde_json::to_vec(&kept).map_err(|_| crate::desktop::Error::RecordFormat)?;
        files::write_private(&file, &json)?;
    }
    if changed {
        tracing::info!(
            devices = kept.admitted.len(),
            "fleet links changed this computer's devices"
        );
    }
    Ok(changed)
}

/// What `fleet/links.json` keeps.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
struct Kept {
    /// The devices this module admitted, with the relay keys it last set
    /// on each.
    admitted: BTreeMap<String, Option<RelayKeys>>,
    /// Devices revoked here by hand while their link stood: never admitted
    /// again while it stands, whatever suspensions come and go.
    #[serde(default)]
    refused: BTreeSet<String>,
}

/// What this module kept. Older files read too: a map of admitted ids to
/// relay keys, or before that a list of ids, with no refusals recorded.
fn read(file: &std::path::Path) -> Result<Kept> {
    let Some(bytes) = files::read_bounded(file, 64 * 1024)? else {
        return Ok(Kept::default());
    };
    if let Ok(kept) = serde_json::from_slice(&bytes) {
        return Ok(kept);
    }
    if let Ok(admitted) = serde_json::from_slice(&bytes) {
        return Ok(Kept {
            admitted,
            refused: BTreeSet::new(),
        });
    }
    let ids: BTreeSet<String> = serde_json::from_slice(&bytes).unwrap_or_default();
    Ok(Kept {
        admitted: ids.into_iter().map(|id| (id, None)).collect(),
        refused: BTreeSet::new(),
    })
}

/// The devices the log links to this computer, by id, and what they say
/// vouched for them.
fn wanted(fleet: &Fleet) -> (BTreeMap<String, Wanted>, BTreeSet<String>, String) {
    let suspended = fleet.suspended();
    let inner = fleet.inner();
    let Some(log) = inner.log.as_ref() else {
        return (BTreeMap::new(), BTreeSet::new(), String::new());
    };
    let state = log.state();
    let via = format!("{FLEET_VIA}{}", log.fleet().short());
    let Some(host) = own_host(state, &fleet.me()) else {
        return (BTreeMap::new(), BTreeSet::new(), via);
    };
    let phones: Vec<(&MemberId, &Vec<u8>)> = state
        .links
        .iter()
        .filter(|((phone, linked), _)| {
            *linked == host
                && state
                    .members
                    .get(phone)
                    .is_some_and(|member| member.bundle.kind == Kind::Phone)
        })
        .map(|((phone, _), key)| (phone, key))
        .collect();
    // Every device the log links here, suspended phones' included.
    let linked = phones
        .iter()
        .filter_map(|(_, key)| device_id(key).map(|(id, _)| id.0))
        .collect();
    let wanted = phones
        .into_iter()
        .filter(|(phone, _)| !suspended.contains(phone))
        .filter_map(|(phone, key)| {
            let member = state.members.get(phone)?;
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
    (wanted, linked, via)
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
