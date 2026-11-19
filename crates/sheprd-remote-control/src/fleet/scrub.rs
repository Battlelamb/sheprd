//! `fleet scrub`: forgets the fleet here, without signing anything, with
//! every way it let someone in: the pairings of desktops whose keys
//! members proved, the members' `authorized_keys` lines, and the relay
//! records it wrote. The fleet key stays.
//!
//! It fails closed: it answers only once access is gone, and what it could
//! not read (a damaged peers file, whose proven keys it could not revoke)
//! stays on disk and is reported, so asking again finishes the job. A
//! damaged log or record list never stops the rest.

use super::{Fleet, Result, grants, store};
use crate::ssh_access::{self, AuthorizedKeys};
use crate::web::State;
use std::path::Path;

/// The daemon's answer to `Ask::Scrub`. Blocking.
pub(crate) fn live(state: &State, fleet: &Fleet) -> Result<()> {
    // As if every member were removed: pairings with keys they proved here
    // go first.
    for tunnel in fleet.proven_keys() {
        grants::supersede(state, &tunnel)?;
    }
    // Access goes even when a file stays; the failure is still reported.
    let forgotten = fleet.scrub();
    forget_access(daemon_keys(state).as_ref(), fleet.state_dir())?;
    forgotten
}

/// The daemon's answer to `Ask::Scrub` when its fleet did not load: a
/// damaged file, which a scrub must still clear. Blocking.
pub(crate) fn unloaded(state: &State) -> Result<()> {
    let state_dir = state.auth().state_dir().to_owned();
    on_disk(&state_dir, daemon_keys(state).as_ref(), |tunnels| {
        for tunnel in tunnels {
            grants::supersede(state, tunnel)?;
        }
        Ok(())
    })
}

/// `fleet scrub` with no daemon running. Blocking.
pub(crate) fn offline(state_dir: &Path) -> Result<()> {
    let keys = match AuthorizedKeys::of_user() {
        Ok(keys) => Some(keys),
        // Where the fleet never writes lines, there are none to remove.
        Err(ssh_access::Error::Unsupported) => None,
        Err(error) => return Err(error.into()),
    };
    on_disk(state_dir, keys.as_ref(), |tunnels| {
        if tunnels.is_empty() {
            return Ok(());
        }
        let mut auth = crate::auth::Auth::load(state_dir, crate::auth::Limits::default())?;
        for tunnel in tunnels {
            grants::revoke_paired(&mut auth, tunnel)?;
        }
        Ok(())
    })
}

/// The `authorized_keys` the daemon lets members in through, if any.
fn daemon_keys(state: &State) -> Option<AuthorizedKeys> {
    let auth = state.auth();
    auth.ssh_access().ok().map(|access| access.keys.clone())
}

/// Removes the members' lines from `keys` and the fleet's relay records.
fn forget_access(keys: Option<&AuthorizedKeys>, state_dir: &Path) -> Result<()> {
    if let Some(keys) = keys {
        keys.set_fleet(&[])?;
    }
    grants::forget_records(state_dir)
}

/// Forgets the fleet kept in `state_dir` without reading its log, with
/// `revoke` taking the pairings of the keys members proved. Blocking.
fn on_disk(
    state_dir: &Path,
    keys: Option<&AuthorizedKeys>,
    revoke: impl FnOnce(&[[u8; 32]]) -> Result<()>,
) -> Result<()> {
    // Read before anything goes; a peers file that does not read is kept,
    // and reported once the rest is gone.
    let proven = store::proven_keys(state_dir);
    let removed = store::remove_log(state_dir);
    forget_access(keys, state_dir)?;
    removed?;
    revoke(&proven?)?;
    store::remove_peers(state_dir)
}
