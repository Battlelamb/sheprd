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
mod lock;
mod me;
pub(crate) mod removals;
mod store;

pub use error::{Error, Result};

use crate::control::fleet::{Body, Inserted, Log, MemberId, Op, OpId, Unsigned, sync};
use p256::ecdsa::SigningKey;
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

/// A suspension waiting for its removal's history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Unconfirmed {
    pub(crate) signer: MemberId,
    pub(crate) targets: Vec<MemberId>,
    pub(crate) since_ms: u64,
    /// The removal itself, put to the log again once its history is
    /// held: the log refusing it ends the suspension. Kept for at most
    /// [`MAX_KEPT_REMOVALS`] suspensions, newest first; one without it
    /// lasts until a settled history, its signer, or the TTL ends it.
    pub(crate) op: Option<Op>,
}

/// Suspensions that keep their removal to check again.
pub(crate) const MAX_KEPT_REMOVALS: usize = crate::control::fleet::MAX_PENDING;

/// The daemon's fleet: its key, and its log once it founded or joined one.
#[derive(Debug)]
pub(crate) struct Fleet {
    state_dir: PathBuf,
    key: SigningKey,
    me: MemberId,
    inner: Mutex<Inner>,
    changed: watch::Sender<u64>,
    /// Removal exchanges other members dialed in for, at most
    /// `removals::CONCURRENT` at once.
    pub(crate) inbound: tokio::sync::Semaphore,
    /// The fleet folder's lock, which the daemon holds for its life.
    _lock: Option<lock::Lock>,
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
        Self::load_with(state_dir, now_ms, None)
    }

    /// The fleet kept in `state_dir`, as the daemon loads it: holding the
    /// fleet folder's lock for as long as it lives, so `fleet apply` with
    /// no daemon cannot write the log under it. Waits for one already
    /// writing. Blocking.
    pub(crate) fn load_exclusive(state_dir: &Path, now_ms: u64) -> Result<Self> {
        let lock = lock::Lock::acquire(state_dir)?;
        Self::load_with(state_dir, now_ms, Some(lock))
    }

    fn load_with(state_dir: &Path, now_ms: u64, lock: Option<lock::Lock>) -> Result<Self> {
        let key = store::load_or_create_key(state_dir)?;
        let me = member_id(&key);
        let mut inner = store::load(state_dir, now_ms)?;
        // A history `fleet apply` brought while the daemon was stopped may
        // settle a suspension.
        settle_unconfirmed(&mut inner, now_ms);
        Ok(Self {
            state_dir: state_dir.to_owned(),
            key,
            me,
            inner: Mutex::new(inner),
            changed: watch::Sender::new(0),
            inbound: tokio::sync::Semaphore::new(removals::CONCURRENT),
            _lock: lock,
        })
    }

    pub(crate) fn me(&self) -> MemberId {
        self.me
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
        for (_, targets) in log.pending_removals() {
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

    /// Signs what `bodies` makes of the log as this computer, in order,
    /// each on the one before, and keeps them all or none: one the log
    /// refuses leaves the log, and the file, as they were. Blocking: it
    /// writes the log.
    pub(crate) fn sign_all(
        &self,
        now_ms: u64,
        bodies: impl FnOnce(&Log) -> Vec<Body>,
    ) -> Result<Vec<Op>> {
        let mut inner = self.inner();
        let log = inner.log.as_mut().ok_or(Error::NoFleet)?;
        let bodies = bodies(log);
        let mut draft = log.clone();
        let mut signed = Vec::with_capacity(bodies.len());
        for body in bodies {
            let op = draft.draft(self.me, now_ms, body).sign(&self.key)?;
            match draft.insert(op.clone(), now_ms)? {
                Inserted::Applied | Inserted::Known => {}
                Inserted::Pending => return Err(Error::NotApplied),
            }
            signed.push(op);
        }
        let kept = std::mem::replace(log, draft);
        if let Err(error) = self.save(&inner) {
            // Unsaved, so not kept: what is held matches the file.
            if let Some(log) = inner.log.as_mut() {
                *log = kept;
            }
            return Err(error);
        }
        drop(inner);
        self.bump();
        Ok(signed)
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
        let suspensions: Vec<OpId> = inner.unconfirmed.keys().copied().collect();
        let refused = match &mut inner.log {
            Some(log) => log
                .insert_all(ops, now_ms)
                .iter()
                .filter(|(_, result)| result.is_err())
                .count(),
            None => {
                let (log, refused) = join(ops, self.me, now_ms)?;
                inner.log = Some(log);
                refused
            }
        };
        settle_unconfirmed(&mut inner, now_ms);
        let after = inner.log.as_ref().map(|log| log.heads().to_vec());
        // A suspension lifted is a change too: its target's grants return.
        if after != before
            || !inner.unconfirmed.is_empty()
            || !suspensions.iter().eq(inner.unconfirmed.keys())
        {
            self.save(&inner)?;
            drop(inner);
            self.bump();
        }
        Ok(refused)
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
    let deadline = std::time::Instant::now() + lock::WAIT;
    loop {
        match crate::control::request(&crate::control::address(state_dir), &request) {
            Ok(Response::Fleet(answer)) => match *answer {
                sync::Answer::Applied(applied) => return Ok(applied),
                _ => return Err(Error::Daemon("an unexpected answer".to_owned())), // i18n: data
            },
            Ok(Response::Error { message, .. }) => return Err(Error::Daemon(message)),
            Ok(_) => return Err(Error::Daemon("an unexpected answer".to_owned())), // i18n: data
            Err(crate::control::Error::Connect(_)) => {}
            Err(error) => return Err(error.into()),
        }
        // No daemon answers. Held, the lock keeps a daemon from starting
        // and another update from writing until this one has; busy, a
        // daemon is starting, or another update is writing, and the
        // socket is tried again.
        if let Some(_lock) = lock::Lock::try_acquire(state_dir)? {
            let now = crate::web::now_ms();
            let fleet = Fleet::load(state_dir, now)?;
            let heads = apply.heads()?;
            let refused = fleet.receive(apply.ops()?, now)?;
            let inner = fleet.inner();
            let log = inner.log.as_ref().ok_or(Error::NoFleet)?;
            return Ok(sync::answer(log, &heads, refused));
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::Busy);
        }
        lock::pause();
    }
}

/// `fleet bundle`, run over SSH: this computer's bundle, signed for
/// `fleet`, from the running daemon. Blocking.
pub fn run_bundle(state_dir: &Path, fleet: String) -> Result<sync::Answer> {
    use crate::control::{Request, Response};
    let request = Request::Fleet(Box::new(sync::Ask::Bundle { fleet }));
    match crate::control::request(&crate::control::address(state_dir), &request)? {
        Response::Fleet(answer) => Ok(*answer),
        Response::Error { message, .. } => Err(Error::Daemon(message)),
        _ => Err(Error::Daemon("an unexpected answer".to_owned())), // i18n: data
    }
}

/// A log from operations that found a fleet, or `Error::NotAdmitted` when
/// they do not admit `me`.
fn join(mut ops: Vec<Op>, me: MemberId, now_ms: u64) -> Result<(Log, usize)> {
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
    if !sync::admits(&log, &me) {
        return Err(Error::NotAdmitted);
    }
    Ok((log, refused))
}

/// Moves the log's pending removals into `unconfirmed`, and drops those
/// the log settled: taken in, or refused once their history arrived.
fn settle_unconfirmed(inner: &mut Inner, now_ms: u64) {
    let Some(log) = &mut inner.log else {
        return;
    };
    for (op, targets) in log.pending_removals() {
        inner.unconfirmed.entry(op.id()).or_insert(Unconfirmed {
            signer: op.signer(),
            targets,
            since_ms: now_ms,
            op: Some(op.clone()),
        });
    }
    // A removal whose history is now held, and that the log does not
    // hold, either expired from the pending pool before its history came
    // or was refused with it: put it to the log again. Only a refusal the
    // history proves ends the suspension; a full log keeps it.
    let ready: Vec<Op> = inner
        .unconfirmed
        .iter()
        .filter_map(|(id, kept)| kept.op.as_ref().filter(|_| log.get(id).is_none()))
        .filter(|op| op.parents().iter().all(|parent| log.get(parent).is_some()))
        .cloned()
        .collect();
    let mut refused = BTreeSet::new();
    for op in ready {
        let id = op.id();
        match log.insert(op, now_ms) {
            Err(
                crate::control::fleet::Error::Full
                | crate::control::fleet::Error::Bounds
                | crate::control::fleet::Error::ParentMissing,
            )
            | Ok(_) => {}
            Err(_) => {
                refused.insert(id);
            }
        }
    }
    inner.unconfirmed.retain(|id, kept| {
        log.get(id).is_none()
            && !refused.contains(id)
            && standing(log, kept)
            && now_ms.saturating_sub(kept.since_ms) <= UNCONFIRMED_TTL_MS
    });
    // Bounded: the oldest give up their removal, never their suspension.
    let mut kept: Vec<(u64, OpId)> = inner
        .unconfirmed
        .iter()
        .filter(|(_, kept)| kept.op.is_some())
        .map(|(id, kept)| (kept.since_ms, *id))
        .collect();
    if kept.len() > MAX_KEPT_REMOVALS {
        kept.sort_unstable();
        for (_, id) in &kept[..kept.len() - MAX_KEPT_REMOVALS] {
            if let Some(entry) = inner.unconfirmed.get_mut(id) {
                entry.op = None;
            }
        }
    }
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
    use sha2::{Digest, Sha256};
    let point = key.verifying_key().to_sec1_point(true);
    MemberId(Sha256::digest(point.as_bytes()).into())
}

// Every test here runs members on loopback with a `USER`: Unix only.
#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
