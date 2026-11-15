//! The relays other computers shared with this one (`control::relays`), on
//! disk in `STATE_DIR/relay/shared.json`. A host may have the program but no
//! daemon running, so `sheprd-remote-control relays --offer` then writes the
//! file itself, and the daemon reads it when it starts.

mod error;

pub use error::Error;

use crate::control::{
    self, Request, Response,
    relays::{Offer, Shared},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Where the daemon whose state is `state_dir` keeps shared relays.
pub fn file(state_dir: &Path) -> PathBuf {
    state_dir.join("relay").join("shared.json")
}

/// The relays saved in `file`, bounded; none when it is missing or
/// unreadable, which costs only the relays a computer shares again on its
/// next visit. Blocking.
pub fn load(file: &Path) -> Shared {
    fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Shared>(&bytes).ok())
        .map(Shared::bounded)
        .unwrap_or_default()
}

/// Replaces `file` whole, readable by this user alone. Blocking.
pub fn save(file: &Path, shared: &Shared) -> Result<(), Error> {
    let dir = file.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|source| Error::Io {
        operation: "create",
        path: dir.to_owned(),
        source,
    })?;
    Ok(crate::app::edited::save(file, shared)?)
}

/// Offers `offer` to the daemon whose state is `state_dir` and returns the
/// relays it listens on now; with no daemon running, keeps it in the file
/// the daemon reads when it starts, and returns the shared relays alone.
/// Blocking.
pub fn share(state_dir: &Path, offer: Offer) -> Result<Vec<String>, Error> {
    let address = control::address(state_dir);
    match control::request(&address, &Request::ShareRelays(offer.clone())) {
        Ok(Response::Relays { relays }) => Ok(relays),
        Ok(_) => Err(Error::Unexpected),
        Err(control::Error::Connect(_)) => {
            let file = file(state_dir);
            let _lock = lock(&file)?;
            let mut shared = load(&file);
            if shared.offer(offer)? {
                save(&file, &shared)?;
            }
            Ok(shared.endpoints().iter().map(ToString::to_string).collect())
        }
        Err(error) => Err(error.into()),
    }
}

/// Holds `file`'s lock until the handle is dropped, so two offers with no
/// daemon running cannot both read the old copy. The system frees it if
/// its holder dies. Blocking.
fn lock(file: &Path) -> Result<fs::File, Error> {
    let path = file.with_extension("lock");
    let failed = |operation| {
        let path = path.clone();
        move |source| Error::Io {
            operation,
            path,
            source,
        }
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(failed("create"))?;
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(failed("open"))?;
    lock.lock().map_err(failed("lock"))?;
    Ok(lock)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
