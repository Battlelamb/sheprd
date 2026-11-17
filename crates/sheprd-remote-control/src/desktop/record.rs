//! What this computer keeps about each machine it added: where it listens
//! (`hosts/ID.json`), and its SSH host keys (`known_hosts`, under the
//! machine's `r-ID.sheprd.invalid` name, which `ssh` checks strictly).

use super::{Error, Result, files};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sheprd_client::relay_host::{self, KNOWN_HOSTS, directory, host_name};
use sheprd_relay::RouteId;
use std::path::{Path, PathBuf};

/// The largest record or known hosts file read.
const MAX_FILE: u64 = 256 * 1024;

/// A machine reached through relays.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    /// What the machine calls itself.
    pub(crate) computer: String,
    /// Its tunnel identity to pin: X25519 then ML-KEM-768, base64url.
    pub(crate) identity: String,
    /// The relays it listens on, nearest first when paired.
    pub(crate) relays: Vec<String>,
    /// Its route, base64url.
    pub(crate) route: String,
}

impl Record {
    /// The id its host name carries: the route's first eight bytes, hex.
    pub(crate) fn id(&self) -> Result<String> {
        let route = RouteId::parse(&self.route).ok_or(Error::RelayInfo)?;
        Ok(route.as_bytes()[..relay_host::ID_LEN / 2]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    /// The machine with `id`. Blocking.
    pub(crate) fn load(state_dir: &Path, id: &str) -> Result<Self> {
        let bytes = files::read_bounded(&path(state_dir, id), MAX_FILE)?
            .ok_or_else(|| Error::Record(id.to_owned()))?;
        let record: Self = serde_json::from_slice(&bytes).map_err(|_| Error::RecordFormat)?;
        if record.id()? != id {
            return Err(Error::RecordFormat);
        }
        Ok(record)
    }

    /// Keeps this machine, with its SSH `host_keys`, replacing what was
    /// kept for it. Returns its id. Blocking.
    pub(crate) fn save(&self, state_dir: &Path, host_keys: &[String]) -> Result<String> {
        let id = self.id()?;
        // The id is only the route's first eight bytes: another machine
        // whose route shares them must not take over this one's record
        // and host keys. The same machine paired again may update both.
        match Self::load(state_dir, &id) {
            Ok(kept) if kept.route != self.route || kept.identity != self.identity => {
                return Err(Error::RecordConflict(id));
            }
            Ok(_) | Err(Error::Record(_)) => {}
            Err(error) => return Err(error),
        }
        let json = serde_json::to_vec_pretty(self).map_err(|_| Error::RecordFormat)?;
        files::write_private(&path(state_dir, &id), &json)?;
        let known = directory(state_dir).join(KNOWN_HOSTS);
        let text = files::read_bounded(&known, MAX_FILE)?.unwrap_or_default();
        let text = String::from_utf8(text).map_err(|_| Error::RecordFormat)?;
        let updated = known_hosts(&text, &host_name(&id), host_keys);
        files::write_private(&known, updated.as_bytes())?;
        Ok(id)
    }

    /// The tunnel identity, decoded.
    pub(crate) fn tunnel_identity(&self) -> Result<sheprd_tunnel::Identity> {
        let bytes = URL_SAFE_NO_PAD
            .decode(&self.identity)
            .map_err(|_| Error::RelayInfo)?;
        Ok(sheprd_tunnel::Identity::from_bytes(&bytes)?)
    }
}

fn path(state_dir: &Path, id: &str) -> PathBuf {
    directory(state_dir)
        .join("hosts")
        .join(format!("{id}.json"))
}

/// `text` with `host`'s lines replaced by one per key in `keys`.
pub(crate) fn known_hosts(text: &str, host: &str, keys: &[String]) -> String {
    let mut lines: Vec<String> = text
        .lines()
        .filter(|line| line.split_ascii_whitespace().next() != Some(host))
        .map(str::to_owned)
        .collect();
    lines.extend(keys.iter().map(|key| format!("{host} {key}")));
    let mut text = lines.join("\n");
    text.push('\n');
    text
}
