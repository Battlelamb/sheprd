//! Sheprd's own relays, for Settings to offer: the list at
//! `sheprd.org/relays.json`, taken only when one of the signers compiled
//! into `sheprd_relay::directory::signed` vouched for its exact bytes. A
//! list nobody signed, or an expired one, offers nothing.

mod error;

pub use error::Error;
use sheprd_relay::directory::{
    self, Directory,
    signed::{self, Signatures, Signer},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long the list and its signatures may take, each.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The relays Sheprd lists, and who vouched for the list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Listed {
    pub(crate) relays: Vec<Public>,
    pub(crate) signer: Signer,
}

/// One of Sheprd's relays.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Public {
    /// Its address, as `[remote] relays` and the daemon's status name it.
    pub(crate) address: String,
    /// A name to show, such as `Frankfurt`.
    pub(crate) name: Option<String>,
    /// A short region code, such as `fra1`.
    pub(crate) region: Option<String>,
}

/// Downloads the list and its signatures, and checks them. Blocking: run
/// it off the UI thread.
pub(crate) fn fetch() -> Result<Listed, Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .max_redirects(0)
        .build()
        .into();
    let bytes = download(&agent, directory::URL, directory::MAX_BYTES)?.ok_or(Error::Missing)?;
    let openpgp = download(&agent, signed::OPENPGP_URL, signed::MAX_SIGNATURE)?;
    let ed25519 = download(&agent, signed::ED25519_URL, signed::MAX_SIGNATURE)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    listed(
        &bytes,
        Signatures {
            openpgp: openpgp.as_deref(),
            ed25519: ed25519.as_deref(),
        },
        now,
    )
}

/// What a downloaded list offers at `now` (Unix seconds), once checked.
pub(crate) fn listed(bytes: &[u8], signatures: Signatures<'_>, now: u64) -> Result<Listed, Error> {
    Ok(Directory::verify(bytes, signatures, now, None)?.into())
}

impl From<signed::Signed> for Listed {
    fn from(signed: signed::Signed) -> Self {
        Self {
            relays: signed
                .directory
                .relays
                .into_iter()
                .map(|relay| Public {
                    address: relay.endpoint.to_string(),
                    name: relay.name,
                    region: relay.region,
                })
                .collect(),
            signer: signed.signer,
        }
    }
}

/// The file at `url`, at most `limit` bytes of it, or `None` when the site
/// has none: a signature either signer may leave out.
fn download(agent: &ureq::Agent, url: &str, limit: usize) -> Result<Option<Vec<u8>>, Error> {
    let mut response = match agent.get(url).call() {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(source) => return Err(Error::Download(source)),
    };
    // One byte past the limit, so a longer file is refused, not cut short.
    let bytes = response
        .body_mut()
        .with_config()
        .limit(limit as u64 + 1)
        .read_to_vec()
        .map_err(Error::Download)?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests;
