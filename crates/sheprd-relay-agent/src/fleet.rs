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
//! way, never give it.

use crate::{
    Error, Result,
    access::{self, Access, Id, Member},
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
/// Removals remembered per suspended member; past that, its suspension is
/// one no access file lifts.
pub const MAX_CAUSES: usize = 8;
const MAX_SUSPENDED_BYTES: u64 = 64 * 1024;

/// Why a member is suspended.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Suspension {
    /// The removals that named it.
    pub removals: BTreeSet<Id>,
    /// More removals named it than are remembered: only its removal
    /// settles it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overflow: bool,
}

/// The access this agent holds, and who is suspended.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Held {
    pub access: Option<Access>,
    pub suspended: BTreeMap<Id, Suspension>,
}

impl Held {
    /// What `state_dir` holds; nothing for an agent no fleet reached.
    /// Blocking.
    pub fn load(state_dir: &Path) -> Result<Self> {
        let access = files::read_bounded(&state_dir.join(access::FILE), access::MAX_BYTES as u64)?
            .map(|bytes| Access::parse(&bytes))
            .transpose()?;
        let path = state_dir.join(SUSPENDED);
        let suspended = match files::read_bounded(&path, MAX_SUSPENDED_BYTES)? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|source| Error::FleetFormat { path, source })?,
            None => BTreeMap::new(),
        };
        Ok(Self { access, suspended })
    }

    fn save(&self, state_dir: &Path) -> Result<()> {
        if let Some(access) = &self.access {
            files::write_private(&state_dir.join(access::FILE), &access.to_bytes()?)?;
        }
        let path = state_dir.join(SUSPENDED);
        let json = serde_json::to_vec(&self.suspended).map_err(|source| Error::FleetFormat {
            path: path.clone(),
            source,
        })?;
        files::write_private(&path, &json)
    }

    /// `fleet apply`: merges `incoming` into what `state_dir` holds,
    /// settles suspensions, forgets the desktop lines of members removed
    /// for good, and writes the fleet's lines in `keys` (when there is
    /// one). Refuses a file for another fleet. Blocking.
    pub fn apply(state_dir: &Path, keys: Option<&Path>, incoming: Access) -> Result<Self> {
        let _lock = files::Lock::acquire(state_dir)?;
        let mut held = Self::load(state_dir)?;
        let settled = incoming.settled.clone();
        let merged = Access::merge(held.access.as_ref(), incoming)?;
        let gone: Vec<&Member> = held
            .access
            .iter()
            .flat_map(|access| &access.members)
            .filter(|(id, _)| merged.removed.contains(id))
            .map(|(_, member)| member)
            .collect();
        if !gone.is_empty() {
            let mut desktops = Authorized::load(state_dir)?;
            // Every one is forgotten, not only the first that was there.
            let mut forgot = false;
            for pair in gone.iter().flat_map(|member| member.pairs()) {
                forgot |= desktops.forget(&pair.dial, &pair.tunnel);
            }
            if forgot {
                desktops.save(state_dir)?;
            }
        }
        held.suspended.retain(|member, suspension| {
            let removed = merged.removed.contains(member);
            let voided =
                !suspension.overflow && suspension.removals.iter().all(|id| settled.contains(id));
            !removed && !voided
        });
        held.access = Some(merged);
        held.save(state_dir)?;
        held.write_keys(keys)?;
        Ok(held)
    }

    /// A removal exchange from member `sender`: suspends every target of
    /// `removals` in this fleet that is a member here, never `sender`
    /// itself. Returns the ids of the removals taken: every one for this
    /// fleet, so the sender owes them no more. Blocking.
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
        let mut taken = Vec::new();
        let mut changed = false;
        for removal in removals
            .iter()
            .filter(|removal| removal.fleet == access.fleet.0)
        {
            taken.push(removal.id);
            let targets = removal
                .targets
                .iter()
                .map(|target| Id(*target))
                .filter(|target| *target != sender && access.members.contains_key(target));
            for target in targets {
                let suspension = held.suspended.entry(target).or_default();
                let id = Id(removal.id);
                if suspension.removals.contains(&id) {
                    continue;
                }
                if suspension.removals.len() < MAX_CAUSES {
                    suspension.removals.insert(id);
                } else {
                    suspension.overflow = true;
                }
                changed = true;
            }
        }
        if changed {
            held.save(state_dir)?;
            held.write_keys(keys)?;
        }
        Ok(taken)
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

    /// What `status` reports of the fleet; none before the first file.
    pub fn report(&self) -> Result<Option<Report>> {
        let Some(access) = &self.access else {
            return Ok(None);
        };
        let allowed = access.allowed().count();
        Ok(Some(Report {
            allowed,
            denied: access.members.len() - allowed,
            digest: access.digest()?,
            fleet: access.fleet,
            removed: access.removed.len(),
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
    /// Members suspended, with the removals that named them: what Sheprd
    /// asks its log about in the next file's `settled`.
    pub suspended: BTreeMap<Id, Suspension>,
}

/// Who dialed in, as far as admitting them goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Peer {
    /// A desktop enrolled one by one in `authorized`: its session is SSH,
    /// with no purpose byte.
    Desktop,
    /// A fleet member: its session starts with a purpose byte. `desktop`
    /// when it is also enrolled one by one, whose sessions send none.
    Member { id: Id, ssh: bool, desktop: bool },
}

/// Everyone the agent admits: the desktops in `authorized` and the fleet's
/// members, less those suspended.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Admitted {
    desktops: Authorized,
    held: Held,
}

impl Admitted {
    #[must_use]
    pub fn new(desktops: Authorized, held: Held) -> Self {
        Self { desktops, held }
    }

    /// Reads `authorized`, `fleet`, and `suspended`. Blocking.
    pub fn load(state_dir: &Path) -> Result<Self> {
        Ok(Self::new(
            Authorized::load(state_dir)?,
            Held::load(state_dir)?,
        ))
    }

    #[must_use]
    pub fn desktops(&self) -> &Authorized {
        &self.desktops
    }

    #[must_use]
    pub fn held(&self) -> &Held {
        &self.held
    }

    /// Members admitted, with their ids.
    fn members(&self) -> impl Iterator<Item = (&Id, &Member)> {
        self.held
            .access
            .iter()
            .flat_map(|access| &access.members)
            .filter(|(id, _)| !self.held.suspended.contains_key(id))
    }

    /// A desktop in `authorized` with the keys of a suspended member, now
    /// or before a rotation, is that member's device: cut off with it.
    fn cut_off(&self, dial: &Key, tunnel: &[u8; 32]) -> bool {
        let Some(access) = &self.held.access else {
            return false;
        };
        self.held
            .suspended
            .keys()
            .filter_map(|id| access.members.get(id))
            .flat_map(Member::pairs)
            .any(|pair| pair.dial == *dial && pair.tunnel == *tunnel)
    }

    /// The keys the relays let dial: members first, then desktops, at
    /// most a relay's [`MAX_DIAL_KEYS`].
    #[must_use]
    pub fn dial_keys(&self) -> Vec<Key> {
        let mut keys: Vec<Key> = self.members().map(|(_, member)| member.dial).collect();
        let desktops = self
            .desktops
            .entries()
            .iter()
            .filter(|entry| !self.cut_off(&entry.dial, &entry.tunnel))
            .map(|entry| entry.dial);
        for key in desktops {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys.truncate(MAX_DIAL_KEYS);
        keys
    }

    fn desktop(&self, tunnel: &[u8; 32]) -> bool {
        self.desktops
            .entries()
            .iter()
            .any(|entry| entry.tunnel == *tunnel && !self.cut_off(&entry.dial, &entry.tunnel))
    }

    /// Who a handshake showing `tunnel` is, if admitted.
    #[must_use]
    pub fn peer(&self, tunnel: &[u8; 32]) -> Option<Peer> {
        let desktop = self.desktop(tunnel);
        let mut holders = self
            .members()
            .filter(|(_, member)| member.tunnel == *tunnel);
        match (holders.next(), holders.next()) {
            (Some((id, member)), None) => Some(Peer::Member {
                id: *id,
                ssh: member.allow,
                desktop,
            }),
            // Two members naming one key: neither is told apart.
            (Some(_), Some(_)) => None,
            (None, _) => desktop.then_some(Peer::Desktop),
        }
    }

    /// Whether `peer`'s SSH session may go on: what a live session checks
    /// after every change.
    #[must_use]
    pub fn keeps_ssh(&self, peer: &Peer, tunnel: &[u8; 32]) -> bool {
        match peer {
            Peer::Desktop => self.desktop(tunnel),
            Peer::Member { id, .. } => self
                .members()
                .any(|(member, entry)| member == id && entry.allow && entry.tunnel == *tunnel),
        }
    }

    /// Whether `member` may still pass removals.
    #[must_use]
    pub fn admits_member(&self, member: &Id) -> bool {
        self.held.admits(member)
    }
}

#[cfg(test)]
mod tests;
