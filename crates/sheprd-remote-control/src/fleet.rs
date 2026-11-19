//! This daemon's part in the fleet (`docs/relay-plan.md`, Fleet): its own
//! fleet key, its copy of the signed log (`control::fleet`), and what the
//! log grants it and asks of it.
//!
//! - [`store`]: the key and the log on disk, private to this user.
//! - [`me`]: this computer's member bundle and its own host record.
//! - [`ask`]: what Sheprd asks over the control socket.
//! - [`grants`]: what the log makes of this computer: the members' SSH
//!   lines, their keys on the relays, and records for the hosts it may
//!   reach.
//! - [`removals`]: removals passed between daemons inside the relay
//!   tunnel, so they land while every Sheprd window is shut.
//!
//! Every change bumps [`Fleet::changed`]; `follow` then rewrites what the
//! log grants, off the control thread.

pub(crate) mod ask;
mod error;
pub(crate) mod grants;
mod me;
pub(crate) mod removals;
mod store;

pub use error::{Error, Result};

use crate::control::fleet::{Body, HostId, Inserted, Log, MemberId, Op, OpId, Unsigned, sync};
use p256::ecdsa::SigningKey;
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
};
use tokio::sync::watch;

/// Tunnel keys remembered per member. A member that proves more has the
/// pairing with each further key revoked at once rather than remembered.
pub(crate) const MAX_PROVEN: usize = 8;

/// How long a removal whose history never arrives keeps its targets cut
/// off: any member that syncs brings the history long before, so only a
/// removal whose history does not exist outlives it.
pub(crate) const UNCONFIRMED_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// How long the parts of a log handed over to a computer with no fleet
/// wait for the rest ([`sync::Apply::more`]).
const JOINING_TTL_MS: u64 = 10 * 60 * 1000;
/// The most operation bytes those parts may hold: the largest log this
/// daemon keeps.
const MAX_JOINING_BYTES: usize = crate::control::fleet::saved::MAX_SAVED;

/// The parts of a log handed over so far to a computer with no fleet,
/// kept in memory until the last arrives.
#[derive(Debug)]
struct Joining {
    ops: Vec<Op>,
    bytes: usize,
    since_ms: u64,
}

/// A suspension waiting for its removal's history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Unconfirmed {
    pub(crate) signer: MemberId,
    pub(crate) targets: Vec<MemberId>,
    pub(crate) since_ms: u64,
}

/// The daemon's fleet: its key, and its log once it founded or joined one.
#[derive(Debug)]
pub(crate) struct Fleet {
    state_dir: PathBuf,
    key: SigningKey,
    me: MemberId,
    /// This computer as a host, from its certificate authority's
    /// certificate: a computer that is not a member holds the log as a host
    /// the fleet shares, and lets in the members allowed there.
    host: Option<HostId>,
    inner: Mutex<Inner>,
    joining: Mutex<Option<Joining>>,
    changed: watch::Sender<u64>,
    /// Removal exchanges other members dialed in for, at most
    /// `removals::CONCURRENT` at once.
    pub(crate) inbound: tokio::sync::Semaphore,
}

#[derive(Debug, Default)]
pub(crate) struct Inner {
    pub(crate) log: Option<Log>,
    /// The removals each peer said it holds, so none is sent twice.
    pub(crate) held: BTreeMap<MemberId, BTreeSet<OpId>>,
    /// Members cut off by a removal whose history has not arrived, by the
    /// removal: kept after the removal expires from the pending pool,
    /// until its history arrives and settles it, its signer is no longer a
    /// member, or [`UNCONFIRMED_TTL_MS`] passes.
    pub(crate) unconfirmed: BTreeMap<OpId, Unconfirmed>,
    /// Every tunnel key each computer member proved by finishing the
    /// tunnel's handshake here, at most [`MAX_PROVEN`] each: once it is
    /// removed, a desktop paired here one by one with any of them goes
    /// with it, keys it rotated away from included.
    pub(crate) proven: BTreeMap<MemberId, BTreeSet<[u8; 32]>>,
}

impl Fleet {
    /// The fleet kept in `state_dir`, making this computer's fleet key the
    /// first time. Blocking.
    pub(crate) fn load(state_dir: &Path, now_ms: u64) -> Result<Self> {
        let key = store::load_or_create_key(state_dir)?;
        let me = member_id(&key);
        let inner = store::load(state_dir, now_ms)?;
        let host = match crate::tls::Authority::stored_certificate(state_dir) {
            Ok(certificate) => certificate.map(|der| HostId(Sha256::digest(&der).into())),
            Err(error) => {
                // Fails closed: with no host id, it is no fleet host here.
                tracing::warn!(%error, "could not read the certificate authority for the fleet");
                None
            }
        };
        Ok(Self {
            state_dir: state_dir.to_owned(),
            key,
            me,
            host,
            inner: Mutex::new(inner),
            joining: Mutex::new(None),
            changed: watch::Sender::new(0),
            inbound: tokio::sync::Semaphore::new(removals::CONCURRENT),
        })
    }

    pub(crate) fn me(&self) -> MemberId {
        self.me
    }

    /// This computer's host id, from its certificate authority.
    pub(crate) fn host(&self) -> Option<HostId> {
        self.host
    }

    pub(crate) fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub(crate) fn key(&self) -> &SigningKey {
        &self.key
    }

    /// Bumped on every change to the log or to who is suspended.
    pub(crate) fn watch(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub(crate) fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Members cut off now: those the log's live members removed in an
    /// operation whose history has not arrived. Folded into `unconfirmed`,
    /// which only an arrived history clears.
    pub(crate) fn suspended(&self) -> Vec<MemberId> {
        let inner = self.inner();
        let Some(log) = &inner.log else {
            return Vec::new();
        };
        let now = crate::web::now_ms();
        // Only while the removal's signer is still a member, and never this
        // computer: a removal that cannot be checked yet only takes others'
        // access away here.
        let mut members: BTreeSet<MemberId> = inner
            .unconfirmed
            .values()
            .filter(|kept| {
                standing(log, kept) && now.saturating_sub(kept.since_ms) <= UNCONFIRMED_TTL_MS
            })
            .flat_map(|kept| kept.targets.iter().copied())
            .collect();
        for (_, _, targets) in log.pending_removals() {
            members.extend(targets);
        }
        members.retain(|member| *member != self.me && log.is_member(member));
        members.into_iter().collect()
    }

    /// Signs `body` as this computer, on everything the log holds, and
    /// keeps it. Blocking: it writes the log.
    pub(crate) fn sign(&self, body: Body, now_ms: u64) -> Result<Op> {
        let mut inner = self.inner();
        let log = inner.log.as_mut().ok_or(Error::NoFleet)?;
        let op = log.draft(self.me, now_ms, body).sign(&self.key)?;
        match log.insert(op.clone(), now_ms)? {
            Inserted::Applied | Inserted::Known => {}
            Inserted::Pending => return Err(Error::NotApplied),
        }
        self.save(&inner)?;
        drop(inner);
        self.bump();
        Ok(op)
    }

    /// Founds a fleet with `bundle`, this computer's, as its only member.
    pub(crate) fn found(&self, bundle: crate::control::fleet::Bundle, now_ms: u64) -> Result<()> {
        let mut inner = self.inner();
        if inner.log.is_some() {
            return Err(Error::InFleet);
        }
        if bundle.id() != self.me {
            return Err(Error::NotMine);
        }
        let op = Unsigned {
            fleet: crate::control::fleet::FleetId([0; 32]),
            signer: self.me,
            seq: 1,
            issued_ms: now_ms,
            parents: Vec::new(),
            body: Body::Found(bundle),
        }
        .sign(&self.key)?;
        inner.log = Some(Log::found(op)?);
        self.save(&inner)?;
        drop(inner);
        self.bump();
        Ok(())
    }

    /// Takes operations another member or a phone carried, as they arrived
    /// at `now_ms`; says how many were refused. With no fleet yet, it joins
    /// one only when the operations found it and admit this computer: a
    /// member admitted it, after a person's approval there. Blocking.
    pub(crate) fn receive(&self, ops: Vec<Op>, now_ms: u64) -> Result<usize> {
        let mut inner = self.inner();
        let before = inner.log.as_ref().map(|log| log.heads().to_vec());
        let refused = match &mut inner.log {
            Some(log) => log
                .insert_all(ops, now_ms)
                .iter()
                .filter(|(_, result)| result.is_err())
                .count(),
            None => {
                let mut all = self
                    .take_joining(now_ms)
                    .map_or_else(Vec::new, |kept| kept.ops);
                all.extend(ops);
                let (log, refused) = join(all, self.me, self.host, now_ms)?;
                inner.log = Some(log);
                refused
            }
        };
        settle_unconfirmed(&mut inner, now_ms);
        let after = inner.log.as_ref().map(|log| log.heads().to_vec());
        if after != before || !inner.unconfirmed.is_empty() {
            self.save(&inner)?;
            drop(inner);
            self.bump();
        }
        Ok(refused)
    }

    /// Keeps `ops`, one part of a log handed over to this computer while it
    /// is in no fleet, until the last part arrives and [`Fleet::receive`]
    /// joins with them all: the operation that admits this computer comes
    /// last. Parts past [`MAX_JOINING_BYTES`] are refused and all dropped.
    /// Kept in memory: a handover needs the running daemon. Blocking.
    pub(crate) fn stage(&self, ops: Vec<Op>, now_ms: u64) -> Result<()> {
        if self.inner().log.is_some() {
            return Err(Error::InFleet);
        }
        let mut kept = self.take_joining(now_ms).unwrap_or(Joining {
            ops: Vec::new(),
            bytes: 0,
            since_ms: now_ms,
        });
        kept.bytes = ops.iter().fold(kept.bytes, |bytes, op| {
            bytes.saturating_add(op.encode().len())
        });
        if kept.bytes > MAX_JOINING_BYTES {
            return Err(Error::JoinTooLarge);
        }
        kept.ops.extend(ops);
        *self.joining.lock().unwrap_or_else(PoisonError::into_inner) = Some(kept);
        Ok(())
    }

    /// The parts kept by [`Fleet::stage`], unless they waited too long.
    fn take_joining(&self, now_ms: u64) -> Option<Joining> {
        self.joining
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .filter(|kept| now_ms.saturating_sub(kept.since_ms) <= JOINING_TTL_MS)
    }

    /// Records that `member` proved `tunnel` by finishing the tunnel's
    /// handshake here. Blocking.
    /// Whether the key is kept: past [`MAX_PROVEN`] it is not, and the
    /// caller revokes its pairing at once instead, so no key a member
    /// proved is ever forgotten while a pairing with it stands.
    pub(crate) fn proved(&self, member: MemberId, tunnel: [u8; 32]) -> Result<bool> {
        let mut inner = self.inner();
        let keys = inner.proven.entry(member).or_default();
        if keys.contains(&tunnel) {
            return Ok(true);
        }
        if keys.len() >= MAX_PROVEN {
            return Ok(false);
        }
        keys.insert(tunnel);
        self.save(&inner)?;
        Ok(true)
    }

    /// Forgets what `member` proved, once its desktops are revoked.
    /// Blocking.
    pub(crate) fn forget_proven(&self, member: &MemberId) -> Result<()> {
        let mut inner = self.inner();
        if inner.proven.remove(member).is_none() {
            return Ok(());
        }
        self.save(&inner)
    }

    /// Records that `peer` holds the removals `ids`. Blocking.
    pub(crate) fn held(&self, peer: MemberId, ids: impl IntoIterator<Item = OpId>) -> Result<()> {
        let mut inner = self.inner();
        inner.held.entry(peer).or_default().extend(ids);
        self.save(&inner)
    }

    pub(crate) fn bump(&self) {
        self.changed
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    fn save(&self, inner: &Inner) -> Result<()> {
        store::save(&self.state_dir, inner)
    }
}

/// `fleet apply`, run over SSH: hands `apply` to the running daemon, or,
/// with none running, applies it to the log on disk, which the daemon
/// reads when it starts. Blocking.
pub fn run_apply(state_dir: &Path, apply: sync::Apply) -> Result<sync::Applied> {
    use crate::control::{Request, Response};
    let request = Request::Fleet(Box::new(sync::Ask::Apply(apply.clone())));
    match crate::control::request(&crate::control::address(state_dir), &request) {
        Ok(Response::Fleet(answer)) => match *answer {
            sync::Answer::Applied(applied) => Ok(applied),
            _ => Err(Error::Daemon("an unexpected answer".to_owned())), // i18n: data
        },
        Ok(Response::Error { message, .. }) => Err(Error::Daemon(message)),
        Ok(_) => Err(Error::Daemon("an unexpected answer".to_owned())), // i18n: data
        Err(crate::control::Error::Connect(_)) => {
            let now = crate::web::now_ms();
            let fleet = Fleet::load(state_dir, now)?;
            let heads = apply.heads()?;
            let refused = fleet.receive(apply.ops()?, now)?;
            let inner = fleet.inner();
            let log = inner.log.as_ref().ok_or(Error::NoFleet)?;
            Ok(sync::answer(log, &heads, refused))
        }
        Err(error) => Err(error.into()),
    }
}

/// `fleet bundle` or `fleet intro`, run over SSH: this computer's bundle
/// signed for a fleet, or its host intro, from the running daemon, which
/// holds the certificate authority's key. Blocking.
pub fn run_ask(state_dir: &Path, ask: sync::Ask) -> Result<sync::Answer> {
    use crate::control::{Request, Response};
    let request = Request::Fleet(Box::new(ask));
    match crate::control::request(&crate::control::address(state_dir), &request)? {
        Response::Fleet(answer) => Ok(*answer),
        Response::Error { message, .. } => Err(Error::Daemon(message)),
        _ => Err(Error::Daemon("an unexpected answer".to_owned())), // i18n: data
    }
}

/// A log from operations that found a fleet, or `Error::NotAdmitted` when
/// they neither admit `me` nor share `host`, this computer as a host. A
/// host's record is signed by its own certificate authority, so only a
/// member that read it from this computer (over SSH, as `fleet intro`)
/// can share it, and only a caller that can run `fleet apply` here can
/// hand the log over: someone who controls this account already.
fn join(mut ops: Vec<Op>, me: MemberId, host: Option<HostId>, now_ms: u64) -> Result<(Log, usize)> {
    let found = ops
        .iter()
        .position(|op| matches!(op.body(), Body::Found(_)))
        .ok_or(Error::NotAdmitted)?;
    let mut log = Log::found(ops.swap_remove(found))?;
    let refused = log
        .insert_all(ops, now_ms)
        .iter()
        .filter(|(_, result)| result.is_err())
        .count();
    if !sync::admits(&log, &me) && !host.is_some_and(|host| sync::shares(&log, &host)) {
        return Err(Error::NotAdmitted);
    }
    Ok((log, refused))
}

/// Moves the log's pending removals into `unconfirmed`, and drops those
/// whose history arrived: once in the log, the log settles them.
fn settle_unconfirmed(inner: &mut Inner, now_ms: u64) {
    let Some(log) = &inner.log else {
        return;
    };
    for (id, signer, targets) in log.pending_removals() {
        inner.unconfirmed.entry(id).or_insert(Unconfirmed {
            signer,
            targets,
            since_ms: now_ms,
        });
    }
    inner.unconfirmed.retain(|id, kept| {
        log.get(id).is_none()
            && standing(log, kept)
            && now_ms.saturating_sub(kept.since_ms) <= UNCONFIRMED_TTL_MS
    });
}

/// Whether a suspension still stands: its signer is a member, or was
/// removed by one of the members it suspends. A suspended member cannot
/// lift its own suspension by removing whoever signed it; the history,
/// once it arrives, settles both removals.
fn standing(log: &Log, kept: &Unconfirmed) -> bool {
    log.is_member(&kept.signer)
        || log
            .removers(&kept.signer)
            .iter()
            .any(|remover| kept.targets.contains(remover))
}

fn member_id(key: &SigningKey) -> MemberId {
    let point = key.verifying_key().to_sec1_point(true);
    MemberId(Sha256::digest(point.as_bytes()).into())
}

// Every test here runs members on loopback with a `USER`: Unix only.
#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
