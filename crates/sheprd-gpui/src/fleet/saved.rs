//! Each fleet host this computer may reach, kept as a saved device,
//! `USER@r-ID.sheprd.invalid`, which `ssh` reaches through `relay-connect`
//! and the record the daemon wrote from the signed log. A device the fleet
//! saved is forgotten when the fleet no longer lets this computer in
//! there; devices a person saved are never touched.

use super::Fleet;
use gpui::{App, BorrowAppContext};
use sheprd_client::SavedHost;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

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

/// Orders the updates [`keep`] starts: each runs alone, and one started
/// before another that already ran does nothing, so an older view never
/// saves again a device a newer one forgot.
#[derive(Clone, Debug, Default)]
pub(super) struct Saving {
    /// The last update started, on the UI thread.
    started: u64,
    /// The last update that ran.
    ran: Arc<Mutex<u64>>,
}

impl Saving {
    /// Runs `update`, the `number`th started, unless a later one ran.
    /// Blocking: it waits for one that runs now.
    pub(super) fn run(&self, number: u64, update: impl FnOnce()) {
        let mut ran = self.ran.lock().unwrap_or_else(PoisonError::into_inner);
        if *ran >= number {
            return;
        }
        update();
        *ran = number;
    }
}

/// What to do with the saved devices: the fleet's devices to save, and
/// those it saved before and to forget.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Plan {
    /// New devices, or ones whose target moved.
    pub(super) save: Vec<SavedHost>,
    /// The fleet's devices to forget first: gone, or about to be saved
    /// again with a new target.
    pub(super) forget: Vec<String>,
    /// The fleet's devices afterwards.
    pub(super) kept: BTreeSet<String>,
}

/// The plan from `before`, the ids the fleet saved, `saved`, every saved
/// device, and `wanted`. A device a person saved under a wanted id is left
/// alone and never becomes the fleet's.
pub(super) fn plan(before: &BTreeSet<String>, saved: &[SavedHost], wanted: &[SavedHost]) -> Plan {
    let mut plan = Plan::default();
    for host in wanted {
        let ours = before.contains(&host.id);
        match saved.iter().find(|saved| saved.id == host.id) {
            Some(saved) if saved.target == host.target && saved.session == host.session => {
                if ours {
                    plan.kept.insert(host.id.clone());
                }
            }
            Some(_) if ours => {
                plan.forget.push(host.id.clone());
                plan.save.push(host.clone());
                plan.kept.insert(host.id.clone());
            }
            Some(_) => {}
            None => {
                plan.save.push(host.clone());
                plan.kept.insert(host.id.clone());
            }
        }
    }
    plan.forget.extend(
        before
            .iter()
            .filter(|id| !wanted.iter().any(|host| host.id == **id))
            .cloned(),
    );
    plan
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
    let (saving, number) = cx.update_global::<Fleet, _>(|fleet, _| {
        fleet.saving.started += 1;
        (fleet.saving.clone(), fleet.saving.started)
    });
    cx.background_executor()
        .spawn(async move {
            saving.run(number, || {
                if let Err(error) = keep_blocking(&wanted) {
                    tracing::warn!(%error, "Could not keep the fleet's saved devices");
                }
            });
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
    let saved = sheprd_client::load_saved_hosts(false)?;
    let mut plan = plan(&before, &saved, wanted);
    for id in &plan.forget {
        if let Err(error) = crate::menu::devices::setup::remove(id) {
            tracing::warn!(%error, "Could not forget a fleet host");
            // Tried again on the next change.
            plan.kept.insert(id.clone());
        }
    }
    for host in &plan.save {
        // A failure keeps the id: the device is the fleet's either way,
        // and saving it is tried again on the next change.
        if let Err(error) = sheprd_client::store_saved_host(false, host) {
            tracing::warn!(%error, "Could not save a fleet host");
        }
    }
    crate::state_file::write(&file, &plan.kept)
}
