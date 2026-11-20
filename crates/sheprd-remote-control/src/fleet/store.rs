//! The fleet's files, in the state directory's `fleet` folder, private to
//! this user: `key` (the fleet key's 32-byte secret), `log.bin` (the log,
//! `control::fleet::saved`), and `peers.json` (which removals each peer
//! holds, and which suspensions wait for their history); `joining/` (the
//! history of each fleet being joined, `FLEET.bin` in `log.bin`'s format,
//! until one admits this computer or shares it as a host); `hosted` (the
//! fleet it joined as a shared host); `lock` is `super::lock`'s.

use super::{
    Error, Inner, Result, Suspensions,
    joining::{Joining, MAX_FLEETS},
};
use crate::control::fleet::{FleetId, MemberId, Op, OpId, saved, sync};
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
/// Suspensions take most of it at worst: about 16 MiB, half of them for
/// members no longer live (`tests/folding.rs` checks the bound), with room
/// beside them for what each peer holds and the proven keys.
pub(super) const MAX_PEERS: u64 = 32 * 1024 * 1024;

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
    /// Suspensions folded per signer and target (`super::suspensions`),
    /// hex, by signer then target.
    #[serde(default)]
    folds: BTreeMap<String, BTreeMap<String, FoldedKept>>,
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
struct FoldedKept {
    /// Each folded removal's id, hex, with when it arrived.
    #[serde(default)]
    ids: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    overflowed: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    named_with: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pinned: bool,
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
        Some(_) => BTreeMap::new(),
        None => load_joining(state_dir, now_ms)?,
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
        folded: folded(&peers.folds)?,
        folded_pending: BTreeSet::new(),
        unconfirmed: unconfirmed(&peers.unconfirmed)?,
    })
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

/// Hex that no longer reads in a suspension is an error, never a
/// suspension dropped: it would let its targets back in.
fn member(hex: &str) -> Result<MemberId> {
    MemberId::from_hex(hex).ok_or(Error::Peers)
}

fn folded(
    kept: &BTreeMap<String, BTreeMap<String, FoldedKept>>,
) -> Result<super::suspensions::FoldedMap> {
    let mut folded = super::suspensions::FoldedMap::new();
    for (signer, targets) in kept {
        let into = folded.entry(member(signer)?).or_default();
        for (target, kept) in targets {
            into.insert(
                member(target)?,
                super::suspensions::FoldedTarget {
                    ids: kept
                        .ids
                        .iter()
                        .map(|(id, since)| Ok((OpId::from_hex(id).ok_or(Error::Peers)?, *since)))
                        .collect::<Result<_>>()?,
                    overflowed: kept.overflowed,
                    named_with: kept
                        .named_with
                        .iter()
                        .map(|hex| member(hex))
                        .collect::<Result<_>>()?,
                    pinned: kept.pinned,
                },
            );
        }
    }
    Ok(folded)
}

fn unconfirmed(kept: &BTreeMap<String, Kept>) -> Result<BTreeMap<OpId, super::Unconfirmed>> {
    kept.iter()
        .map(|(id, kept)| {
            let id = OpId::from_hex(id).ok_or(Error::Peers)?;
            Ok((
                id,
                super::Unconfirmed {
                    signer: member(&kept.signer)?,
                    targets: kept
                        .targets
                        .iter()
                        .map(|hex| member(hex))
                        .collect::<Result<_>>()?,
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
        .collect()
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
pub(super) fn save(state_dir: &Path, inner: &Inner, on_disk: &mut Suspensions) -> Result<()> {
    let now = Suspensions {
        unconfirmed: inner.unconfirmed.clone(),
        folded: inner.folded.clone(),
    };
    let mut first = now.clone();
    for (id, kept) in &on_disk.unconfirmed {
        first.unconfirmed.entry(*id).or_insert_with(|| kept.clone());
    }
    for (signer, targets) in &on_disk.folded {
        let folded = first.folded.entry(*signer).or_default();
        for (target, kept) in targets {
            let entry = folded.entry(*target).or_insert_with(|| kept.clone());
            for (id, since) in &kept.ids {
                entry.ids.entry(*id).or_insert(*since);
            }
            entry.overflowed = entry.overflowed.max(kept.overflowed);
            entry.named_with.extend(kept.named_with.iter().copied());
            entry.pinned |= kept.pinned;
        }
    }
    write_peers(state_dir, inner, &first)?;
    *on_disk = first;
    if let Some(log) = &inner.log {
        write_private(&path(state_dir, LOG), &saved::encode(log)?)?;
        remove_joining(state_dir)?;
    } else {
        save_joining(state_dir, &inner.joining)?;
    }
    if *on_disk != now {
        match write_peers(state_dir, inner, &now) {
            Ok(()) => *on_disk = now,
            Err(error) => {
                tracing::warn!(error = %error, "could not drop settled suspensions; kept until the next save");
            }
        }
    }
    Ok(())
}

/// Writes `peers.json` from `inner`, with `unconfirmed` as its
/// suspensions.
fn write_peers(state_dir: &Path, inner: &Inner, suspensions: &Suspensions) -> Result<()> {
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
        folds: suspensions
            .folded
            .iter()
            .map(|(signer, targets)| {
                let targets = targets
                    .iter()
                    .map(|(target, kept)| {
                        let kept = FoldedKept {
                            ids: kept
                                .ids
                                .iter()
                                .map(|(id, since)| (id.to_string(), *since))
                                .collect(),
                            overflowed: kept.overflowed,
                            named_with: kept.named_with.iter().map(ToString::to_string).collect(),
                            pinned: kept.pinned,
                        };
                        (target.to_string(), kept)
                    })
                    .collect();
                (signer.to_string(), targets)
            })
            .collect(),
        unconfirmed: suspensions
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
