//! Who the agent admits: the desktops in `authorized` and the fleet's
//! members, less those suspended, and less every desktop with a suspended
//! or removed member's keys. A key a suspended member names never falls
//! through to a desktop: the agent fails closed.

use super::Held;
use crate::{
    Result,
    access::{Access, Id, Member, Pair},
    authorized::Authorized,
};
use sheprd_relay::wire::{Key, MAX_DIAL_KEYS};
use std::path::Path;

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

/// Everyone the agent admits.
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

    /// Reads `authorized` and the fleet's files. Blocking.
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

    /// Members admitted, with their ids: not suspended, and sharing no key
    /// with another member, suspended or not ([`Access::shared`]).
    fn members(&self) -> impl Iterator<Item = (&Id, &Member)> {
        let shared = self
            .held
            .access
            .as_ref()
            .map(Access::shared)
            .unwrap_or_default();
        self.held
            .access
            .iter()
            .flat_map(|access| &access.members)
            .filter(move |(id, _)| !self.held.suspended.contains_key(id) && !shared.contains(id))
    }

    /// The key pairs, now and before a rotation, of suspended members.
    fn suspended_pairs(&self) -> impl Iterator<Item = Pair> + '_ {
        let access = self.held.access.as_ref();
        self.held
            .suspended
            .keys()
            .filter_map(move |id| access?.members.get(id))
            .flat_map(Member::pairs)
    }

    /// A desktop with both keys of a suspended member, or of one removed
    /// for good (until enrolled again), is cut off, bound or not.
    fn cut_off(&self, dial: &Key, tunnel: &[u8; 32]) -> bool {
        let pair = Pair {
            dial: *dial,
            tunnel: *tunnel,
        };
        self.held.revoked.contains(&pair) || self.suspended_pairs().any(|named| named == pair)
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

    /// Who a handshake showing `tunnel` is, if admitted. A suspended
    /// member's key is no one's; a key two members name is still an
    /// enrolled desktop's that no removal cut off.
    #[must_use]
    pub fn peer(&self, tunnel: &[u8; 32]) -> Option<Peer> {
        if self.suspended_pairs().any(|pair| pair.tunnel == *tunnel) {
            return None;
        }
        let desktop = self.desktop(tunnel);
        match self.members().find(|(_, member)| member.tunnel == *tunnel) {
            Some((id, member)) => Some(Peer::Member {
                id: *id,
                ssh: member.allow,
                desktop,
            }),
            None => desktop.then_some(Peer::Desktop),
        }
    }

    /// Whether a session from `dial` and `tunnel` proves an unbound
    /// enrolled desktop is a member's device: what `Held::bind` records.
    #[must_use]
    pub fn binds(&self, dial: &Key, tunnel: &[u8; 32]) -> bool {
        let pair = Pair {
            dial: *dial,
            tunnel: *tunnel,
        };
        let named = self.held.access.iter().any(|access| {
            access
                .members
                .values()
                .any(|member| member.pairs().any(|named| named == pair))
        });
        let enrolled = self
            .desktops
            .entries()
            .iter()
            .any(|entry| entry.dial == *dial && entry.tunnel == *tunnel);
        named && enrolled && !self.held.bound(&pair)
    }

    /// Whether `peer`'s SSH session may go on: what a live session checks
    /// after every change.
    #[must_use]
    pub fn keeps_ssh(&self, peer: &Peer, tunnel: &[u8; 32]) -> bool {
        if self.suspended_pairs().any(|pair| pair.tunnel == *tunnel) {
            return false;
        }
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
        self.members().any(|(id, _)| id == member)
    }
}
