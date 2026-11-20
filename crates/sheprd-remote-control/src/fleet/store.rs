//! The fleet's files, in the state directory's `fleet` folder, private to
//! this user: `key` (the fleet key's 32-byte secret), `log.bin` (the log,
//! `control::fleet::saved`), and `peers.json` (which removals each peer
//! holds, and which suspensions wait for their history); `joining.bin`
//! (the history of a fleet being joined, until it admits this computer,
//! in `log.bin`'s format); `lock` is
//! `super::lock`'s.

use super::{Error, Inner, Result};
use crate::control::fleet::{MemberId, Op, OpId, saved, sync};
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
/// The history of a fleet being joined, until it admits this computer.
const JOINING: &str = "joining.bin";
const PEERS: &str = "peers.json";
/// The removals suspensions keep to check again take most of it.
pub(super) const MAX_PEERS: u64 = 16 * 1024 * 1024;

fn path(state_dir: &Path, name: &str) -> PathBuf {
    state_dir.join(DIRECTORY).join(name)
}

/// The fleet key, made and kept the first time.
pub(super) fn load_or_create_key(state_dir: &Path) -> crate::desktop::Result<SigningKey> {
    let file = path(state_dir, KEY);
    if let Some(bytes) = read_bounded(&file, 32)? {
        return SigningKey::from_slice(&bytes).map_err(|_| crate::desktop::Error::RecordFormat);
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
    #[serde(default)]
    held: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    unconfirmed: BTreeMap<String, Kept>,
    #[serde(default)]
    proven: BTreeMap<String, Proven>,
    /// What recorded `proven`: [`PROVEN_VERSION`] once keys are recorded
    /// only for a session that proved the member's fleet key. Absent, as
    /// from earlier builds, a tunnel handshake alone recorded them.
    #[serde(default)]
    proven_version: u32,
}

/// `proven` holds only keys a session proved with the member's fleet key
/// (`control::fleet::removal`). Earlier builds recorded a key on the
/// tunnel handshake alone, which any member's bundle could name, so their
/// entries are dropped on load: a key counts as a member's only once a
/// proven session records it again.
const PROVEN_VERSION: u32 = 2;

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
    let peers: Peers = read_bounded(&path(state_dir, PEERS), MAX_PEERS)?
        .map(|bytes| serde_json::from_slice(&bytes))
        .transpose()
        .map_err(|_| Error::Peers)?
        .unwrap_or_default();
    let joining = match log {
        Some(_) => None,
        None => read_bounded(&path(state_dir, JOINING), saved::MAX_SAVED as u64)?
            .and_then(|bytes| saved::decode(&bytes, now_ms).ok()),
    };
    Ok(Inner {
        joining,
        log,
        held: peers
            .held
            .iter()
            .filter_map(|(peer, ids)| {
                Some((
                    MemberId::from_hex(peer)?,
                    ids.iter().filter_map(|id| OpId::from_hex(id)).collect(),
                ))
            })
            .collect(),
        proven: peers
            .proven
            .iter()
            .filter(|_| peers.proven_version == PROVEN_VERSION)
            .filter_map(|(member, keys)| {
                let keys = keys
                    .keys()
                    .filter_map(|key| crate::control::fleet::HostId::from_hex(key).map(|key| key.0))
                    .collect();
                Some((MemberId::from_hex(member)?, keys))
            })
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

/// Writes the log and the peers' state. Blocking.
///
/// Each file is replaced whole, and the two in an order that is safe at
/// every step, even if one write fails or the daemon stops between them:
///
/// 1. `peers.json`, keeping every suspension `on_disk` still holds beside
///    the current ones: a suspension the new log settles must outlive the
///    old log on disk.
/// 2. The log, last of what a caller may roll back: a save that fails
///    before it leaves the log on disk as it was.
/// 3. `peers.json` again without the settled suspensions, once the log
///    that settles them is written. A failure here only leaves extra
///    suspensions, which the next load or save settles again.
///
/// `on_disk` is the suspensions `peers.json` holds; kept up to date.
pub(super) fn save(
    state_dir: &Path,
    inner: &Inner,
    on_disk: &mut BTreeMap<OpId, super::Unconfirmed>,
) -> Result<()> {
    let mut first = inner.unconfirmed.clone();
    for (id, kept) in on_disk.iter() {
        first.entry(*id).or_insert_with(|| kept.clone());
    }
    write_peers(state_dir, inner, &first)?;
    *on_disk = first;
    if let Some(log) = &inner.log {
        write_private(&path(state_dir, LOG), &saved::encode(log)?)?;
        remove(&path(state_dir, JOINING))?;
    } else if let Some(joining) = &inner.joining {
        write_private(&path(state_dir, JOINING), &saved::encode(joining)?)?;
    }
    if on_disk.len() != inner.unconfirmed.len() {
        match write_peers(state_dir, inner, &inner.unconfirmed) {
            Ok(()) => on_disk.clone_from(&inner.unconfirmed),
            Err(error) => {
                tracing::warn!(error = %error, "could not drop settled suspensions; kept until the next save");
            }
        }
    }
    Ok(())
}

/// Writes `peers.json` from `inner`, with `unconfirmed` as its
/// suspensions.
fn write_peers(
    state_dir: &Path,
    inner: &Inner,
    unconfirmed: &BTreeMap<OpId, super::Unconfirmed>,
) -> Result<()> {
    let peers = Peers {
        held: inner
            .held
            .iter()
            .map(|(peer, ids)| {
                (
                    peer.to_string(),
                    ids.iter().map(ToString::to_string).collect(),
                )
            })
            .collect(),
        proven: inner
            .proven
            .iter()
            .map(|(member, keys)| {
                (
                    member.to_string(),
                    Proven::Many(
                        keys.iter()
                            .map(|key| crate::control::fleet::HostId(*key).to_string())
                            .collect(),
                    ),
                )
            })
            .collect(),
        unconfirmed: unconfirmed
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
        proven_version: PROVEN_VERSION,
    };
    let json = serde_json::to_vec(&peers).map_err(|_| crate::desktop::Error::RecordFormat)?;
    write_private(&path(state_dir, PEERS), &json)?;
    Ok(())
}

/// Removes `file`, which may not exist.
fn remove(file: &Path) -> Result<()> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(crate::desktop::Error::File {
            operation: "remove",
            path: file.to_owned(),
            source,
        }
        .into()),
    }
}
