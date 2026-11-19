//! Each fleet host this computer may reach, kept as a saved device,
//! `USER@r-ID.sheprd.invalid`, which `ssh` reaches through `relay-connect`
//! and the record the daemon wrote from the signed log. A device the fleet
//! saved is forgotten when the fleet no longer lets this computer in
//! there; devices a person saved are never touched.

use super::Fleet;
use gpui::App;
use sheprd_client::SavedHost;
use std::{collections::BTreeSet, path::PathBuf};

/// The session saved devices use.
const SESSION: &str = "default";
/// The ids of the devices the fleet saved.
const FILE: &str = "fleet-saved.json";
const MAX_FILE: u64 = 64 * 1024;

/// A fleet host's saved device id: the first 16 bytes of its host id, so
/// saving it again finds the same device.
pub(crate) fn device_id(host: &str) -> Option<String> {
    let id = host.get(..32)?.to_ascii_lowercase();
    sheprd_client::valid_profile_id(&id).then_some(id)
}

fn file() -> Option<PathBuf> {
    crate::remote::state_dir().ok().map(|dir| dir.join(FILE))
}

/// Saves the fleet hosts this computer may reach and forgets those it no
/// longer may, on the background executor.
pub(super) fn keep(cx: &mut App) {
    let Some(view) = Fleet::view(cx) else {
        return;
    };
    let wanted: Vec<SavedHost> = view
        .hosts
        .iter()
        .filter(|host| !host.held)
        .filter_map(|host| {
            Some(SavedHost {
                id: device_id(&host.id)?,
                label: host.name.clone(),
                target: host.target.clone()?,
                session: SESSION.to_owned(),
                enabled: true,
            })
        })
        .collect();
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = keep_blocking(&wanted) {
                tracing::warn!(%error, "Could not keep the fleet's saved devices");
            }
        })
        .detach();
}

fn keep_blocking(wanted: &[SavedHost]) -> crate::Result<()> {
    let Some(file) = file() else {
        return Ok(());
    };
    let before: BTreeSet<String> = crate::state_file::read(&file, MAX_FILE)?
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut saved = BTreeSet::new();
    for host in wanted {
        match sheprd_client::store_saved_host(false, host) {
            Ok(_) => {
                saved.insert(host.id.clone());
            }
            Err(error) => tracing::warn!(%error, "Could not save a fleet host"),
        }
    }
    let gone: Vec<String> = before.difference(&saved).cloned().collect();
    for id in gone {
        if let Err(error) = crate::menu::devices::setup::remove(&id) {
            tracing::warn!(%error, "Could not forget a fleet host");
            // Tried again on the next change.
            saved.insert(id);
        }
    }
    crate::state_file::write(&file, &saved)
}
