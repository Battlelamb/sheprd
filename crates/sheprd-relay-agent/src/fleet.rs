//! The fleet as this agent holds it (`docs/relay-plan.md`, Fleet): the
//! merge of every access file Sheprd gave it (`access`, kept in `fleet`),
//! and the members a removal passed by another member suspended (kept in
//! `suspended`), both private to this user in the state directory.
//!
//! The agent checks no signatures, so a removal passed inside the tunnel
//! only suspends its targets: their tunnel and dial keys stop being
//! admitted and their `authorized_keys` lines go at once. The next access
//! file, computed over SSH from the signed log, settles each suspension:
//! a removed target becomes a tombstone for good, and a target still a
//! member is readmitted once every removal it was suspended for is one
//! that log holds, which means the log voided it. A suspension the agent
//! cannot settle stays: a member can only ever take access away here this
//! way, never give it. A member suspends only in its own name (the
//! removal's signer must be the member that passed it), and a member
//! denied SSH here suspends only members denied too.
//!
//! A desktop enrolled one by one in `authorized` with both keys of a
//! member (now or before a rotation) fails closed with it: it is cut off
//! while the member is suspended, and after its removal. It is never
//! forgotten: a member may name any desktop's public keys, and nothing a
//! session proves shows the desktop is that member. It stays enrolled,
//! cut off and listed in `status --json` (`revoked`), so a person can
//! enroll it again (`authorize`) when a member named its keys falsely.

mod admitted;

pub use admitted::{Admitted, Peer};

use crate::{
    Error, Result,
    access::{self, Access, Id, Member, Pair},
    authorized::Authorized,
    authorized_keys, files,
    removal::Removed,
};
use serde::{Deserialize, Serialize};
use sheprd_relay::wire::{Key, MAX_DIAL_KEYS};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// The members suspended, in the state directory.
pub const SUSPENDED: &str = "suspended";
/// The key pairs of members removed for good that an enrolled desktop
/// still has, in the state directory.
pub const REVOKED: &str = "revoked";
/// The members whose new keys wait for room on the relays, in the state
/// directory.
pub const CROWDED: &str = "crowded";
/// Removals remembered per suspended member; past that, its suspension is
/// one no access file lifts.
pub const MAX_CAUSES: usize = 8;
const MAX_SUSPENDED_BYTES: u64 = 64 * 1024;

/// Who signed a removal that suspended a member, and with what sequence
/// number: what Sheprd's daemon holds to its log to tell a removal that is
/// on its way from one no member made.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Cause {
    pub signer: Id,
    pub seq: u64,
}

/// Why a member is suspended.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Suspension {
    /// The removals that named it, by id.
    pub removals: BTreeMap<Id, Cause>,
    /// More removals named it than are remembered: only its removal
    /// settles it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overflow: bool,
}

/// The access this agent holds, who is suspended, and the key pairs of
/// members removed for good.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Held {
    pub access: Option<Access>,
    pub suspended: BTreeMap<Id, Suspension>,
    /// Key pairs of members removed for good: a desktop with one stays
    /// cut off until enrolled again.
    pub revoked: BTreeSet<Pair>,
    /// Members whose rotated keys do not fit in the relay's dial keys yet:
    /// kept apart from the fleet's own allow and deny, which the access
    /// file holds as the log decided, so the block lifts as soon as there
    /// is room, with the same file applied again.
    pub crowded: BTreeSet<Id>,
}

impl Held {
    /// What `state_dir` holds; nothing for an agent no fleet reached.
    /// Blocking.
    pub fn load(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join(access::FILE);
        let access = files::read_bounded(&path, access::MAX_STORED_BYTES as u64)?
            .map(|bytes| Access::parse_stored(&bytes))
            .transpose()?;
        Ok(Self {
            access,
            suspended: read_json(state_dir, SUSPENDED)?,
            revoked: read_json(state_dir, REVOKED)?,
            crowded: read_json(state_dir, CROWDED)?,
        })
    }

    /// Writes each file whole, in the order [`Held::files`] gives.
    fn save(&self, state_dir: &Path) -> Result<()> {
        for (name, bytes) in self.files()? {
            files::write_private(&state_dir.join(name), &bytes)?;
        }
        Ok(())
    }

    /// The files kept, by name, in the order that fails closed when saving
    /// stops part way: the revoked pairs before the access file that drops
    /// the members they come from (a retry finds them there again), the
    /// members waiting for room before the file that gives them new keys,
    /// and
    /// the suspensions after it, so one is lifted only once the file that
    /// lifts it, or removes its member, is kept.
    fn files(&self) -> Result<Vec<(&'static str, Vec<u8>)>> {
        let mut files = vec![
            (REVOKED, json(REVOKED, &self.revoked)?),
            (CROWDED, json(CROWDED, &self.crowded)?),
        ];
        if let Some(access) = &self.access {
            files.push((access::FILE, access.to_stored_bytes()?));
        }
        files.push((SUSPENDED, json(SUSPENDED, &self.suspended)?));
        Ok(files)
    }

    /// `fleet apply`: merges `incoming` into what `state_dir` holds,
    /// settles suspensions, cuts off the desktops with the keys of members
    /// removed for good, and writes the fleet's lines in `keys` (when there is
    /// one). Refuses a file for another fleet. Blocking.
    pub fn apply(state_dir: &Path, keys: Option<&Path>, incoming: Access) -> Result<Self> {
        let _lock = files::Lock::acquire(state_dir)?;
        let mut held = Self::load(state_dir)?;
        let settled = incoming.settled.clone();
        let mut merged = Access::merge(held.access.as_ref(), incoming)?;
        // A desktop with both keys of a member removed for good stays
        // enrolled, cut off: never forgotten on a member's word.
        let removed = |id: &Id| merged.removed.contains(id);
        let pairs: Vec<Pair> = held
            .access
            .iter()
            .flat_map(|access| &access.members)
            .filter(|(id, _)| removed(id))
            .flat_map(|(_, member)| member.pairs())
            .collect();
        let desktops = Authorized::load(state_dir)?;
        let enrolled = |pair: &Pair| {
            desktops
                .entries()
                .iter()
                .any(|entry| entry.dial == pair.dial && entry.tunnel == pair.tunnel)
        };
        // A pair no desktop has any more blocks nothing: forgotten, so the
        // set holds at most one pair per enrolled desktop and a required
        // block always fits.
        held.revoked.retain(|pair| enrolled(pair));
        for pair in pairs.into_iter().filter(|pair| enrolled(pair)) {
            if !held.revoked.contains(&pair) && held.revoked.len() >= crate::authorized::MAX_ENTRIES
            {
                return Err(Error::Revoked);
            }
            held.revoked.insert(pair);
        }
        // Taking access away always applies; only what adds a dial key
        // waits for room, counted after the removals.
        let refused = hold_back_additions(
            held.access.as_ref(),
            &mut merged,
            &desktops,
            &mut held.crowded,
        );
        // Each settled removal is forgotten as it is settled, so files
        // that each settle some of a suspension's removals, as the bound
        // on `settled` may make them, lift it between them.
        held.suspended.retain(|member, suspension| {
            if merged.removed.contains(member) {
                return false;
            }
            suspension.removals.retain(|id, _| !settled.contains(id));
            suspension.overflow || !suspension.removals.is_empty()
        });
        held.access = Some(merged);
        // The SSH lines first: a write that fails leaves the file it was
        // computed from unsaved, so the digest still shows the host behind
        // and Sheprd applies it again.
        held.write_keys(keys)?;
        held.save(state_dir)?;
        match refused {
            Some(error) => Err(error),
            None => Ok(held),
        }
    }

    /// `fleet scrub`: forgets the fleet, its `authorized_keys` lines in
    /// `keys`, its access file, and its suspensions, so no member of it
    /// reaches this account through the agent. Desktops in `authorized`
    /// stay. Blocking.
    pub fn scrub(state_dir: &Path, keys: Option<&Path>) -> Result<()> {
        let _lock = files::Lock::acquire(state_dir)?;
        let held = Self::load(state_dir)?;
        if let (Some(path), Some(access)) = (keys, &held.access) {
            authorized_keys::set_fleet(path, &access.fleet.short(), &[])?;
        }
        for name in [access::FILE, SUSPENDED, REVOKED, CROWDED] {
            let path = state_dir.join(name);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(Error::Io { path, source }),
            }
        }
        Ok(())
    }

    /// A removal exchange from member `sender`: suspends every target of
    /// `removals` in this fleet that `sender` signed and that is a member
    /// here, never `sender` itself, and, when `sender` is denied SSH here,
    /// only members denied too. Returns the ids of the removals taken:
    /// every one for this fleet, acted on or not, so the sender owes them
    /// no more. Blocking.
    pub fn suspend(
        state_dir: &Path,
        keys: Option<&Path>,
        sender: Id,
        removals: &[Removed],
    ) -> Result<Vec<[u8; 32]>> {
        let _lock = files::Lock::acquire(state_dir)?;
        let mut held = Self::load(state_dir)?;
        let Some(access) = &held.access else {
            return Ok(Vec::new());
        };
        // The sender may have been suspended since it was let in.
        if !access.members.contains_key(&sender) || held.suspended.contains_key(&sender) {
            return Ok(Vec::new());
        }
        let sender_allowed = held.reaches_ssh(&sender);
        let reaching: BTreeSet<Id> = access
            .members
            .keys()
            .filter(|id| held.reaches_ssh(id))
            .copied()
            .collect();
        let mut taken = Vec::new();
        let mut changed = false;
        for removal in removals
            .iter()
            .filter(|removal| removal.fleet == access.fleet.0)
        {
            taken.push(removal.id);
            // Only in its own name: the agent checks no signature, so a
            // removal another member signed is the log's to bring here.
            if removal.signer != sender.0 {
                continue;
            }
            let targets = removal
                .targets
                .iter()
                .map(|target| Id(*target))
                .filter(|target| *target != sender)
                .filter(|target| {
                    access.members.contains_key(target)
                        && (sender_allowed || !reaching.contains(target))
                });
            let cause = Cause {
                signer: sender,
                seq: removal.seq,
            };
            for target in targets {
                let suspension = held.suspended.entry(target).or_default();
                let id = Id(removal.id);
                if suspension.removals.contains_key(&id) {
                    continue;
                }
                if suspension.removals.len() < MAX_CAUSES {
                    suspension.removals.insert(id, cause);
                } else {
                    suspension.overflow = true;
                }
                changed = true;
            }
        }
        if changed {
            held.save(state_dir)?;
        }
        // Even when nothing changed: a suspension saved by an earlier try
        // whose `authorized_keys` write failed is finished before the
        // removal is acknowledged. Rewriting is a no-op when it is current.
        if !taken.is_empty() {
            held.write_keys(keys)?;
        }
        Ok(taken)
    }

    /// `authorize` enrolled the desktop with `dial` and `tunnel` again: a
    /// person vouches for it, so a removed member's having had its keys no
    /// longer cuts it off. A suspended member's still does. Blocking.
    pub fn release(state_dir: &Path, dial: Key, tunnel: [u8; 32]) -> Result<()> {
        let _lock = files::Lock::acquire(state_dir)?;
        let mut held = Self::load(state_dir)?;
        if held.revoked.remove(&Pair { dial, tunnel }) {
            held.save(state_dir)?;
        }
        Ok(())
    }

    /// Whether `member` is admitted at all: a member here, not suspended.
    #[must_use]
    pub fn admits(&self, member: &Id) -> bool {
        self.access
            .as_ref()
            .is_some_and(|access| access.members.contains_key(member))
            && !self.suspended.contains_key(member)
    }

    /// Whether member `id` may reach SSH here as far as the fleet and room
    /// go: allowed by the access file (not removed, sharing no key with
    /// another), and not waiting for room for its keys. Suspension is
    /// apart. What `suspend`, `Admitted`, and `report` all ask.
    #[must_use]
    pub fn reaches_ssh(&self, id: &Id) -> bool {
        !self.crowded.contains(id)
            && self
                .access
                .as_ref()
                .is_some_and(|access| access.allowed().any(|(member, _)| member == id))
    }

    /// The members that may reach SSH here now: [`Held::reaches_ssh`],
    /// and not suspended.
    pub fn ssh_members(&self) -> impl Iterator<Item = (&Id, &Member)> {
        self.access
            .iter()
            .flat_map(|access| &access.members)
            .filter(|(id, _)| self.reaches_ssh(id) && !self.suspended.contains_key(id))
    }

    fn write_keys(&self, keys: Option<&Path>) -> Result<()> {
        let (Some(path), Some(access)) = (keys, &self.access) else {
            return Ok(());
        };
        let lines: Vec<(String, String)> = self
            .ssh_members()
            .map(|(id, member)| (id.short(), member.ssh.clone()))
            .collect();
        authorized_keys::set_fleet(path, &access.fleet.short(), &lines)
    }

    /// What `status` reports of the fleet, with `desktops`' entries a
    /// removed member's keys cut off; none before the first file.
    pub fn report(&self, desktops: &Authorized) -> Result<Option<Report>> {
        let Some(access) = &self.access else {
            return Ok(None);
        };
        let allowed = access
            .members
            .keys()
            .filter(|id| self.reaches_ssh(id))
            .count();
        let revoked = desktops
            .entries()
            .iter()
            .filter(|entry| {
                let pair = Pair {
                    dial: entry.dial,
                    tunnel: entry.tunnel,
                };
                self.revoked.contains(&pair)
            })
            .map(|entry| Revoked {
                label: entry.label.clone(),
                tunnel: Id(entry.tunnel),
            })
            .collect();
        Ok(Some(Report {
            allowed,
            denied: access.members.len() - allowed,
            digest: access.digest()?,
            fleet: access.fleet,
            removed: access.removed.len(),
            revoked,
            suspended: self.suspended.clone(),
        }))
    }
}

/// When `merged` would need more dial keys than a relay takes, holds back
/// what it adds beside `held`: members `held` lacked are left out, and a
/// member whose dial key changed takes its new keys, the old pair gone as
/// a rotation always removes it, but waits in `crowded` until there is
/// room: old keys are never kept, and the fleet's allow is left as the
/// log decided. Removals, denials, and suspensions stay. Once everything
/// fits, no one waits. The refusal, to report once the rest is applied;
/// none when it fits.
fn hold_back_additions(
    held: Option<&Access>,
    merged: &mut Access,
    desktops: &Authorized,
    crowded: &mut BTreeSet<Id>,
) -> Option<Error> {
    crowded.retain(|id| merged.members.contains_key(id));
    let Err(error) = fits(desktops, Some(merged)) else {
        crowded.clear();
        return None;
    };
    let before = |id: &Id| held.and_then(|held| held.members.get(id));
    let added: Vec<Id> = merged
        .members
        .iter()
        .filter(|(id, member)| before(id).is_none_or(|old| old.dial != member.dial))
        .map(|(id, _)| *id)
        .collect();
    if added.is_empty() {
        // Nothing here adds a key: what was held was already over, and
        // what this file changes only takes access away.
        return None;
    }
    for id in added {
        if before(&id).is_some() {
            crowded.insert(id);
        } else {
            merged.members.remove(&id);
        }
    }
    Some(error)
}

/// Whether every enrolled desktop and every member of `access` fits in
/// the dial keys a relay takes for one route ([`MAX_DIAL_KEYS`]). An
/// update that would crowd one out is refused rather than leave someone
/// enrolled who can no longer dial.
pub fn fits(desktops: &Authorized, access: Option<&Access>) -> Result<()> {
    let mut keys: BTreeSet<Key> = desktops.entries().iter().map(|entry| entry.dial).collect();
    keys.extend(
        access
            .iter()
            .flat_map(|access| access.members.values())
            .map(|member| member.dial),
    );
    if keys.len() > MAX_DIAL_KEYS {
        return Err(Error::DialKeys { needed: keys.len() });
    }
    Ok(())
}

/// The fleet in `status --json` and `fleet apply`'s answer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Report {
    /// Members that may reach SSH here, suspended ones included.
    pub allowed: usize,
    /// Members that may only pass removals.
    pub denied: usize,
    /// [`Access::digest`] of the access held: what Sheprd compares with
    /// the file it would write, to see whether this host is behind.
    pub digest: String,
    /// The fleet this account belongs to.
    pub fleet: Id,
    /// Members removed for good.
    pub removed: usize,
    /// Enrolled desktops cut off because a member removed for good named
    /// their keys: `authorize` enrolls one again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revoked: Vec<Revoked>,
    /// Members suspended, with the removals that named them: what Sheprd
    /// asks its log about in the next file's `settled`.
    pub suspended: BTreeMap<Id, Suspension>,
}

/// An enrolled desktop a removed member's keys cut off.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Revoked {
    pub label: String,
    pub tunnel: Id,
}

fn read_json<T: serde::de::DeserializeOwned + Default>(state_dir: &Path, name: &str) -> Result<T> {
    let path = state_dir.join(name);
    match files::read_bounded(&path, MAX_SUSPENDED_BYTES)? {
        Some(bytes) => {
            serde_json::from_slice(&bytes).map_err(|source| Error::FleetFormat { path, source })
        }
        None => Ok(T::default()),
    }
}

fn json(name: &str, value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|source| Error::FleetFormat {
        path: name.into(),
        source,
    })
}

#[cfg(test)]
mod tests;
