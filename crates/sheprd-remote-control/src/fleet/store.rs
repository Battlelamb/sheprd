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
/// The removals suspensions keep to check again take most of it.
const MAX_PEERS: u64 = 16 * 1024 * 1024;

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
    /// What recorded `proven`: [`PROVEN_VERSION`] once keys are recorded
    /// only for a session that proved the member's fleet key. Absent, as
    /// from earlier builds, a tunnel handshake alone recorded them.
    #[serde(default)]
    proven_version: u32,
    /// Removals plain hosts reported that the log has not heard of.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    unknown: BTreeMap<String, super::plain::Unknown>,
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
        proven: peers
            .proven
            .iter()
            .filter(|_| peers.proven_version == PROVEN_VERSION)
            .filter_map(|(member, keys)| {
                let keys = keys
                    .keys()
                    .filter_map(|key| HostId::from_hex(key).map(|key| key.0))
                    .collect();
                Some((MemberId::from_hex(member)?, keys))
            })
            .collect(),
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
        proven_version: PROVEN_VERSION,
    };
    let json = serde_json::to_vec(&peers).map_err(|_| crate::desktop::Error::RecordFormat)?;
    write_private(&path(state_dir, PEERS), &json)?;
    Ok(())
}
