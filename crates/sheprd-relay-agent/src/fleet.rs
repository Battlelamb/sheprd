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
use sheprd_relay::wire::Key;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// The members suspended, in the state directory.
pub const SUSPENDED: &str = "suspended";
/// The key pairs of members removed for good that an enrolled desktop
/// still has, in the state directory.
pub const REVOKED: &str = "revoked";
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
        })
    }

    /// Writes each file whole, in the order that fails closed when it
    /// stops part way: the revoked pairs before the access file that drops
    /// the members they come from (a retry finds them there again), and
    /// the suspensions after it, so one is lifted only once the file that
    /// lifts it, or removes its member, is kept.
    fn save(&self, state_dir: &Path) -> Result<()> {
        write_json(state_dir, REVOKED, &self.revoked)?;
        if let Some(access) = &self.access {
            files::write_private(&state_dir.join(access::FILE), &access.to_stored_bytes()?)?;
        }
        write_json(state_dir, SUSPENDED, &self.suspended)
    }

    /// `fleet apply`: merges `incoming` into what `state_dir` holds,
    /// settles suspensions, cuts off the desktops with the keys of members
    /// removed for good, and writes the fleet's lines in `keys` (when there is
    /// one). Refuses a file for another fleet. Blocking.
    pub fn apply(state_dir: &Path, keys: Option<&Path>, incoming: Access) -> Result<Self> {
        let _lock = files::Lock::acquire(state_dir)?;
        let mut held = Self::load(state_dir)?;
        let settled = incoming.settled.clone();
        let merged = Access::merge(held.access.as_ref(), incoming)?;
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
        if !pairs.is_empty() {
            let desktops = Authorized::load(state_dir)?;
            for pair in pairs {
                let enrolled = desktops
                    .entries()
                    .iter()
                    .any(|entry| entry.dial == pair.dial && entry.tunnel == pair.tunnel);
                if enrolled && held.revoked.len() < crate::authorized::MAX_ENTRIES {
                    held.revoked.insert(pair);
                }
            }
        }
        held.suspended.retain(|member, suspension| {
            let removed = merged.removed.contains(member);
            let voided =
                !suspension.overflow && suspension.removals.keys().all(|id| settled.contains(id));
            !removed && !voided
        });
        held.access = Some(merged);
        // The SSH lines first: a write that fails leaves the file it was
        // computed from unsaved, so the digest still shows the host behind
        // and Sheprd applies it again.
        held.write_keys(keys)?;
        held.save(state_dir)?;
        Ok(held)
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
        for name in [access::FILE, SUSPENDED, REVOKED] {
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
        let sender_allowed = access
            .members
            .get(&sender)
            .is_some_and(|member| member.allow);
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
                    access
                        .members
                        .get(target)
                        .is_some_and(|member| sender_allowed || !member.allow)
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

    /// The members that may reach SSH here now.
    pub fn ssh_members(&self) -> impl Iterator<Item = (&Id, &Member)> {
        self.access
            .iter()
            .flat_map(Access::allowed)
            .filter(|(id, _)| !self.suspended.contains_key(id))
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
        let allowed = access.allowed().count();
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

fn write_json(state_dir: &Path, name: &str, value: &impl Serialize) -> Result<()> {
    let path = state_dir.join(name);
    let json = serde_json::to_vec(value).map_err(|source| Error::FleetFormat {
        path: path.clone(),
        source,
    })?;
    files::write_private(&path, &json)
}

#[cfg(test)]
mod tests;
