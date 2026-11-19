//! Phones on this computer's API through the fleet (`docs/relay-plan.md`,
//! What joining grants, A phone that joins): a live phone member that
//! signed a `Link` naming this computer's own host, with a device key it
//! made for it, is a paired device here, as a host link makes one.
//!
//! - The device is the API only: no SSH key, no relay keys until the phone
//!   sends them as any device does (`POST /api/relay`, or carried through
//!   another computer), and no GitHub sharing.
//! - Its id comes from its key ([`device_id`]), so the phone knows it
//!   without asking, and a phone that links another key is another device:
//!   the old one goes.
//! - It goes when the phone member is removed or suspended, or its link
//!   does.
//! - A device this module admitted that is gone from `devices.json` while
//!   its link stands was revoked here by hand, a local refusal: it is not
//!   admitted again. `fleet/links.json` keeps the ids admitted, so this
//!   module only ever removes its own.

mod error;

pub use error::{Error, Result};

use super::{Fleet, grants::own_host};
use crate::auth::{NewDevice, PublicKey};
use crate::control::{DeviceId, fleet::Kind};
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
}

/// Admits the devices the log links to this computer and removes those it
/// no longer does; whether any changed. Blocking: it writes
/// `devices.json` and `fleet/links.json`.
pub(crate) fn apply(state: &State, fleet: &Fleet, now_ms: u64) -> Result<bool> {
    let (wanted, via) = wanted(fleet);
    let file = fleet.state_dir().join(FILE);
    let before: BTreeSet<String> = files::read_bounded(&file, 64 * 1024)?
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut changed = false;
    let mut auth = state.auth();
    for gone in before.iter().filter(|id| !wanted.contains_key(*id)) {
        changed |= auth.revoke(&DeviceId(gone.clone()))?;
    }
    for (id, device) in &wanted {
        let id = DeviceId(id.clone());
        if before.contains(&id.0) || auth.devices().get(&id).is_some() {
            continue;
        }
        auth.devices_mut().add_with(
            NewDevice {
                app: None,
                id,
                name: device.name.clone(),
                public_key: device.public_key.clone(),
                relay: None,
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
    let kept: BTreeSet<&String> = wanted.keys().collect();
    if kept.len() != before.len() || kept.iter().any(|id| !before.contains(*id)) {
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
            Some((id.0, Wanted { public_key, name }))
        })
        .collect();
    (wanted, via)
}
