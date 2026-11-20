//! The fleet's files, in the state directory's `fleet` folder, private to
//! this user: `key` (the fleet key's 32-byte secret), `log.bin` (the log,
//! `control::fleet::saved`), and `peers.json` (which removals each member
//! and plain host holds, and which suspensions wait for their history);
//! `joining/` (the history of each fleet being joined, `FLEET.bin` in
//! `log.bin`'s format, until one admits this computer or shares it as a
//! host); `hosted` (the fleet it joined as a shared host); `lock` is
//! `super::lock`'s.

use super::{
    Error, Holder, Inner, Result,
    joining::{Joining, MAX_FLEETS},
};
use crate::control::fleet::{FleetId, HostId, MemberId, Op, OpId, saved, sync};
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
/// The history of each fleet being joined, until one admits this computer.
const JOINING: &str = "joining";
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
    let peers = read_peers(state_dir)?;
    let joining = match log {
        Some(_) => BTreeMap::new(),
        None => load_joining(state_dir, now_ms)?,
    };
    Ok(Inner {
        joining,
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
        // Keys an earlier build recorded on a handshake alone do not count,
        // here or in a scrub (`proven_keys`).
        proven: counted(&peers),
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

/// The proven keys that count: those a proven session recorded. An
/// earlier build's, recorded on a handshake alone, could name any
/// desktop's key, so neither a removal nor a scrub revokes a pairing for
/// them.
fn counted(peers: &Peers) -> BTreeMap<MemberId, BTreeSet<[u8; 32]>> {
    if peers.proven_version == PROVEN_VERSION {
        proven(peers)
    } else {
        BTreeMap::new()
    }
}

/// Every tunnel key a member proved, read from the peers' state alone, so
/// a damaged log does not stop a scrub from revoking their pairings.
/// Blocking.
pub(super) fn proven_keys(state_dir: &Path) -> Result<Vec<[u8; 32]>> {
    Ok(counted(&read_peers(state_dir)?)
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
    remove_joining(state_dir)?;
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
        remove_joining(state_dir)?;
    } else {
        save_joining(state_dir, &inner.joining)?;
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

/// Removes the log, and any history of a fleet being joined, so a scrub
/// leaves no fleet behind. Blocking.
pub(super) fn remove_log(state_dir: &Path) -> Result<()> {
    remove(&path(state_dir, LOG))?;
    remove_joining(state_dir)
}

/// Removes the peers' state. Blocking.
pub(super) fn remove_peers(state_dir: &Path) -> Result<()> {
    remove(&path(state_dir, PEERS))
}

/// Each history kept in `joining/`, at most [`MAX_FLEETS`]; one that no
/// longer reads, or names another fleet than its file, is passed over.
/// Blocking.
fn load_joining(state_dir: &Path, now_ms: u64) -> Result<BTreeMap<FleetId, Joining>> {
    let dir = path(state_dir, JOINING);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(source) => {
            return Err(crate::desktop::Error::File {
                operation: "read",
                path: dir,
                source,
            }
            .into());
        }
    };
    let mut joining = BTreeMap::new();
    for entry in entries.flatten().take(MAX_FLEETS * 4) {
        let file = entry.path();
        let Some(fleet) = file
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".bin"))
            .and_then(FleetId::from_hex)
        else {
            continue;
        };
        let Some(log) = read_bounded(&file, saved::MAX_SAVED as u64)?
            .and_then(|bytes| saved::decode(&bytes, now_ms).ok())
            .filter(|log| log.fleet() == fleet)
        else {
            continue;
        };
        if joining.len() < MAX_FLEETS {
            joining.insert(
                fleet,
                Joining {
                    log,
                    touched_ms: now_ms,
                },
            );
        }
    }
    Ok(joining)
}

/// Writes each history in `joining`, and removes those no longer kept.
/// Blocking.
fn save_joining(state_dir: &Path, joining: &BTreeMap<FleetId, Joining>) -> Result<()> {
    let dir = path(state_dir, JOINING);
    for (fleet, kept) in joining {
        write_private(
            &dir.join(format!("{fleet}.bin")),
            &saved::encode(&kept.log)?,
        )?;
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let file = entry.path();
        let kept = file
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".bin"))
            .and_then(FleetId::from_hex)
            .is_some_and(|fleet| joining.contains_key(&fleet));
        if !kept {
            remove(&file)?;
        }
    }
    Ok(())
}

/// Removes every history kept of fleets being joined. Blocking.
fn remove_joining(state_dir: &Path) -> Result<()> {
    let dir = path(state_dir, JOINING);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(crate::desktop::Error::File {
            operation: "remove",
            path: dir,
            source,
        }
        .into()),
    }
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
