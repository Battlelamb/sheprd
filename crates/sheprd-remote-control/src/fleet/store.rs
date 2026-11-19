//! The fleet's files, in the state directory's `fleet` folder, private to
//! this user: `key` (the fleet key's 32-byte secret), `log.bin` (the log,
//! `control::fleet::saved`), and `peers.json` (which removals each member
//! and plain host holds, and which suspensions wait for their history);
//! `lock` is `super::lock`'s.

use super::{Error, Holder, Inner, Result};
use crate::control::fleet::{HostId, MemberId, Op, OpId, saved, sync};
use crate::desktop::files::{read_bounded, write_private};
use p256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub(super) const DIRECTORY: &str = "fleet";
const KEY: &str = "key";
const LOG: &str = "log.bin";
const PEERS: &str = "peers.json";
/// The fleet id this computer joined as a host the fleet shares, not as a
/// member.
const HOSTED: &str = "hosted";
/// The removals suspensions keep to check again take most of it.
const MAX_PEERS: u64 = 16 * 1024 * 1024;

fn path(state_dir: &Path, name: &str) -> PathBuf {
    state_dir.join(DIRECTORY).join(name)
}

/// The fleet key, made and kept the first time.
pub(super) fn load_or_create_key(state_dir: &Path) -> Result<SigningKey> {
    let file = path(state_dir, KEY);
    if let Some(bytes) = read_bounded(&file, 32)? {
        return SigningKey::from_slice(&bytes)
            .map_err(|_| Error::Desktop(crate::desktop::Error::RecordFormat));
    }
    let mut secret = [0; 32];
    // A scalar out of range is drawn again; vanishingly rare.
    let key = loop {
        getrandom::fill(&mut secret).map_err(|_| crate::desktop::Error::RecordFormat)?;
        if let Ok(key) = SigningKey::from_slice(&secret) {
            break key;
        }
    };
    write_private(&file, &secret)?;
    Ok(key)
}

#[derive(Default, Serialize, Deserialize)]
struct Peers {
    /// By member.
    #[serde(default)]
    held: BTreeMap<String, BTreeSet<String>>,
    /// By plain host, whose agent holds them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    held_hosts: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    unconfirmed: BTreeMap<String, Kept>,
    #[serde(default)]
    proven: BTreeMap<String, Proven>,
    /// Removals plain hosts reported that the log has not heard of.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    unknown: BTreeMap<String, super::plain::Unknown>,
}

/// A member's proven keys: one, as an earlier build kept it, or several.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Proven {
    Many(BTreeSet<String>),
    One(String),
}

impl Proven {
    fn keys(&self) -> impl Iterator<Item = &String> {
        let (many, one) = match self {
            Self::Many(keys) => (Some(keys), None),
            Self::One(key) => (None, Some(key)),
        };
        many.into_iter().flatten().chain(one)
    }
}

#[derive(Serialize, Deserialize)]
struct Kept {
    signer: String,
    targets: Vec<String>,
    since_ms: u64,
    /// The removal, base64url; absent from files an earlier build wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    op: Option<String>,
}

/// What the fleet folder keeps, read at `now_ms`. A log that no longer
/// reads is an error rather than an empty fleet, so a damaged file never
/// looks like having left.
pub(super) fn load(state_dir: &Path, now_ms: u64) -> Result<Inner> {
    let log = read_bounded(&path(state_dir, LOG), saved::MAX_SAVED as u64)?
        .map(|bytes| saved::decode(&bytes, now_ms))
        .transpose()?;
    // A file that no longer reads is an error, never an empty one: it may
    // hold suspensions, and dropping them would let their targets back in.
    let peers = read_peers(state_dir)?;
    Ok(Inner {
        log,
        held: peers
            .held
            .iter()
            .filter_map(|(peer, ids)| Some((Holder::Member(MemberId::from_hex(peer)?), ids)))
            .chain(
                peers
                    .held_hosts
                    .iter()
                    .filter_map(|(host, ids)| Some((Holder::Host(HostId::from_hex(host)?), ids))),
            )
            .map(|(holder, ids)| {
                let ids = ids.iter().filter_map(|id| OpId::from_hex(id)).collect();
                (holder, ids)
            })
            .collect(),
        proven: proven(&peers),
        unknown: peers
            .unknown
            .iter()
            .filter_map(|(id, seen)| Some((OpId::from_hex(id)?, *seen)))
            .collect(),
        unconfirmed: peers
            .unconfirmed
            .iter()
            .filter_map(|(id, kept)| {
                let id = OpId::from_hex(id)?;
                Some((
                    id,
                    super::Unconfirmed {
                        signer: MemberId::from_hex(&kept.signer)?,
                        targets: kept
                            .targets
                            .iter()
                            .filter_map(|member| MemberId::from_hex(member))
                            .collect(),
                        since_ms: kept.since_ms,
                        // One that no longer reads keeps its suspension.
                        op: kept
                            .op
                            .as_deref()
                            .and_then(|op| sync::bytes(op).ok())
                            .and_then(|op| Op::decode(&op).ok())
                            .filter(|op| op.id() == id),
                    },
                ))
            })
            .collect(),
    })
}

fn read_peers(state_dir: &Path) -> Result<Peers> {
    Ok(read_bounded(&path(state_dir, PEERS), MAX_PEERS)?
        .map(|bytes| serde_json::from_slice(&bytes))
        .transpose()
        .map_err(|_| Error::Peers)?
        .unwrap_or_default())
}

fn proven(peers: &Peers) -> BTreeMap<MemberId, BTreeSet<[u8; 32]>> {
    peers
        .proven
        .iter()
        .filter_map(|(member, keys)| {
            let keys = keys
                .keys()
                .filter_map(|key| HostId::from_hex(key).map(|key| key.0))
                .collect();
            Some((MemberId::from_hex(member)?, keys))
        })
        .collect()
}

/// Every tunnel key a member proved, read from the peers' state alone, so
/// a damaged log does not stop a scrub from revoking their pairings.
/// Blocking.
pub(super) fn proven_keys(state_dir: &Path) -> Result<Vec<[u8; 32]>> {
    Ok(proven(&read_peers(state_dir)?)
        .into_values()
        .flatten()
        .collect())
}

/// The fleet id this computer joined as a shared host, if it did. Blocking.
pub(super) fn load_hosted(state_dir: &Path) -> Result<Option<String>> {
    Ok(read_bounded(&path(state_dir, HOSTED), 64)?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
}

/// Records that this computer joined `fleet` as a shared host. Blocking.
pub(super) fn save_hosted(state_dir: &Path, fleet: &str) -> Result<()> {
    write_private(&path(state_dir, HOSTED), fleet.as_bytes())?;
    Ok(())
}

/// Removes the log and how this computer joined it, once it left the
/// fleet for good; `save` then writes the peers' state empty. Blocking.
pub(super) fn forget(state_dir: &Path) -> Result<()> {
    for name in [LOG, HOSTED] {
        let file = path(state_dir, name);
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(crate::desktop::Error::File {
                    operation: "remove",
                    path: file,
                    source,
                }
                .into());
            }
        }
    }
    Ok(())
}

/// Writes the log and the peers' state. Blocking.
pub(super) fn save(state_dir: &Path, inner: &Inner) -> Result<()> {
    if let Some(log) = &inner.log {
        write_private(&path(state_dir, LOG), &saved::encode(log)?)?;
    }
    let held = |key: fn(&Holder) -> Option<String>| {
        inner
            .held
            .iter()
            .filter_map(|(peer, ids)| {
                Some((key(peer)?, ids.iter().map(ToString::to_string).collect()))
            })
            .collect()
    };
    let peers = Peers {
        held: held(|peer| match peer {
            Holder::Member(member) => Some(member.to_string()),
            Holder::Host(_) => None,
        }),
        held_hosts: held(|peer| match peer {
            Holder::Host(host) => Some(host.to_string()),
            Holder::Member(_) => None,
        }),
        proven: inner
            .proven
            .iter()
            .map(|(member, keys)| {
                (
                    member.to_string(),
                    Proven::Many(keys.iter().map(|key| HostId(*key).to_string()).collect()),
                )
            })
            .collect(),
        unknown: inner
            .unknown
            .iter()
            .map(|(id, seen)| (id.to_string(), *seen))
            .collect(),
        unconfirmed: inner
            .unconfirmed
            .iter()
            .map(|(id, kept)| {
                (
                    id.to_string(),
                    Kept {
                        signer: kept.signer.to_string(),
                        targets: kept.targets.iter().map(ToString::to_string).collect(),
                        since_ms: kept.since_ms,
                        op: kept.op.as_ref().map(|op| sync::text(&op.encode())),
                    },
                )
            })
            .collect(),
    };
    let json = serde_json::to_vec(&peers).map_err(|_| crate::desktop::Error::RecordFormat)?;
    write_private(&path(state_dir, PEERS), &json)?;
    Ok(())
}

/// Removes the log. Blocking.
pub(super) fn remove_log(state_dir: &Path) -> Result<()> {
    remove(state_dir, LOG)
}

/// Removes the peers' state. Blocking.
pub(super) fn remove_peers(state_dir: &Path) -> Result<()> {
    remove(state_dir, PEERS)
}

fn remove(state_dir: &Path, name: &str) -> Result<()> {
    let file = path(state_dir, name);
    match std::fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Desktop(crate::desktop::Error::File {
            operation: "remove",
            path: file,
            source,
        })),
    }
}
