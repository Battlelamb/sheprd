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
//! - [`joins`]: phones asking to join through this computer, waiting for a
//!   person to admit them in Sheprd.
//! - [`links`]: phones on this computer's API through the `Link`s they
//!   signed.
//!
//! Every change bumps [`Fleet::changed`]; `follow` then rewrites what the
//! log grants, off the control thread.

pub(crate) mod ask;
mod error;
pub(crate) mod grants;
pub(crate) mod joins;
pub(crate) mod links;
mod lock;
mod me;
pub(crate) mod removals;
mod store;
pub(crate) mod suspensions;

pub use error::{Error, Result};

use crate::control::fleet::{Body, Inserted, Log, MemberId, Op, OpId, Unsigned, sync};
use p256::ecdsa::SigningKey;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// The suspensions held, as `peers.json` keeps them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Suspensions {
    pub(crate) unconfirmed: BTreeMap<OpId, Unconfirmed>,
    pub(crate) folded: suspensions::FoldedMap,
}

impl Suspensions {
    fn of(inner: &Inner) -> Self {
        Self {
            unconfirmed: inner.unconfirmed.clone(),
            folded: inner.folded.clone(),
        }
    }
}

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
    /// Phones waiting to join, memory only.
    joins: Mutex<joins::Joins>,
    /// The fleet folder's lock, which the daemon holds for its life.
    _lock: Option<lock::Lock>,
    /// The last write of the fleet's files failed: what is held is ahead
    /// of them, and the next change, even one that moves nothing, writes
    /// again.
    unsaved: AtomicBool,
    /// The suspensions `peers.json` holds (`store::save`).
    on_disk: Mutex<Suspensions>,
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
    /// Older suspensions past what `unconfirmed` keeps per signer, folded
    /// per signer and target (`suspensions`).
    pub(crate) folded: suspensions::FoldedMap,
    /// Removals folded while they still wait in the pending pool, so they
    /// are not kept apart again; never saved, as the pool is not.
    pub(crate) folded_pending: BTreeSet<OpId>,
    /// Every tunnel key each computer member proved by finishing the
    /// tunnel's handshake here, at most [`MAX_PROVEN`] each: once it is
    /// removed, a desktop paired here one by one with any of them goes
    /// with it, keys it rotated away from included.
    pub(crate) proven: BTreeMap<MemberId, BTreeSet<[u8; 32]>>,
    /// With no fleet yet, the history of the fleet this computer is
    /// joining, as far as it arrived: a history too large for one
    /// `fleet apply` comes over several. It grants nothing; it becomes the
    /// log once it admits this computer.
    pub(crate) joining: Option<Log>,
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

    /// The fleet as the daemon starts it. A fleet that does not load stays
    /// off until the daemon restarts, and what it let in over SSH goes
    /// (`keys`), failing closed. A fleet another process holds is not
    /// that: another daemon runs on this state directory, whose lines must
    /// stay, so this one does not start (`Error::Busy`) and changes
    /// nothing. Blocking.
    pub(crate) fn for_daemon(
        state_dir: &Path,
        now_ms: u64,
        keys: Option<&crate::ssh_access::AuthorizedKeys>,
    ) -> Result<Option<Self>> {
        match Self::load_exclusive(state_dir, now_ms) {
            Ok(fleet) => Ok(Some(fleet)),
            Err(Error::Busy) => Err(Error::Busy),
            Err(error) => {
                tracing::warn!(%error, "Could not load the fleet; it stays off until the daemon restarts");
                if let Some(keys) = keys {
                    let _ = keys.set_fleet(&[]);
                }
                Ok(None)
            }
        }
    }

    fn load_with(state_dir: &Path, now_ms: u64, lock: Option<lock::Lock>) -> Result<Self> {
        let key = store::load_or_create_key(state_dir)?;
        let me = member_id(&key);
        let mut inner = store::load(state_dir, now_ms)?;
        let on_disk = Mutex::new(Suspensions::of(&inner));
        // A history `fleet apply` brought while the daemon was stopped may
        // settle a suspension.
        suspensions::settle(&mut inner, now_ms, BTreeSet::new());
        Ok(Self {
            state_dir: state_dir.to_owned(),
            key,
            me,
            inner: Mutex::new(inner),
            changed: watch::Sender::new(0),
            inbound: tokio::sync::Semaphore::new(removals::CONCURRENT),
            joins: Mutex::default(),
            _lock: lock,
            unsaved: AtomicBool::new(false),
            on_disk,
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

    /// The phones waiting to join. Lock it after [`Fleet::inner`], never
    /// before.
    pub(crate) fn joins(&self) -> MutexGuard<'_, joins::Joins> {
        self.joins.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Members cut off now: those the log's live members removed in an
    /// operation whose history has not arrived. Folded into `unconfirmed`,
    /// which only an arrived history clears.
    pub(crate) fn suspended(&self) -> Vec<MemberId> {
        let inner = self.inner();
        let Some(log) = &inner.log else {
            return Vec::new();
        };
        // Only while the removal's signer is still a member, and never this
        // computer: a removal that cannot be checked yet only takes others'
        // access away here.
        let mut members = suspensions::suspended(&inner, log, crate::web::now_ms());
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
        // Held at once even if the write fails: a removal cuts off now,
        // and the next change writes it.
        let saved = self.save(&inner);
        drop(inner);
        self.bump();
        saved?;
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
        let kept = Suspensions::of(&inner);
        // The removals waiting now, to tell which this batch's history
        // refuses.
        let waiting = inner
            .log
            .as_mut()
            .map(|log| suspensions::waiting(log, now_ms))
            .unwrap_or_default();
        let refused = match &mut inner.log {
            Some(log) => log
                .insert_all(ops, now_ms)
                .iter()
                .filter(|(_, result)| result.is_err())
                .count(),
            None => {
                let (log, refused) = join(inner.joining.take(), ops, now_ms)?;
                if !sync::admits(&log, &self.me) {
                    // Kept, granting nothing, for the rest to arrive.
                    inner.joining = Some(log);
                    let saved = self.save(&inner);
                    drop(inner);
                    saved?;
                    return Err(Error::NotAdmitted);
                }
                inner.log = Some(log);
                refused
            }
        };
        let gone = inner
            .log
            .as_mut()
            .map(|log| suspensions::refused(log, waiting, now_ms))
            .unwrap_or_default();
        suspensions::settle(&mut inner, now_ms, gone);
        let after = inner.log.as_ref().map(|log| log.heads().to_vec());
        // A suspension lifted is a change too: its target's grants return.
        let changed = after != before || kept != Suspensions::of(&inner);
        if changed || !inner.unconfirmed.is_empty() || !inner.folded.is_empty() || self.is_unsaved()
        {
            // What changed takes effect even if the write fails, which the
            // caller hears: a removal it does not see saved, it must not
            // count as held.
            let saved = self.save(&inner);
            drop(inner);
            if changed {
                self.bump();
            }
            saved?;
        }
        Ok(refused)
    }

    /// Takes what `fleet apply` carried and answers it from the log, or,
    /// while joining, from the history so far, so the member sends the
    /// rest. Blocking.
    pub(crate) fn apply(&self, apply: &sync::Apply, now_ms: u64) -> Result<sync::Applied> {
        let heads = apply.heads()?;
        let refused = match self.receive(apply.ops()?, now_ms) {
            Ok(refused) => refused,
            Err(Error::NotAdmitted) if self.inner().joining.is_some() => 0,
            Err(error) => return Err(error),
        };
        let inner = self.inner();
        let log = inner
            .log
            .as_ref()
            .or(inner.joining.as_ref())
            .ok_or(Error::NoFleet)?;
        Ok(sync::answer(log, &heads, refused))
    }

    /// Ages what waits on time alone, at `now_ms`: removals waiting for
    /// their history leave the pending pool, and suspensions past
    /// [`UNCONFIRMED_TTL_MS`] end, so their targets' grants return on an
    /// idle daemon too. Blocking.
    pub(crate) fn tick(&self, now_ms: u64) -> Result<()> {
        let before = self.suspended();
        let mut inner = self.inner();
        let Some(log) = inner.log.as_mut() else {
            return Ok(());
        };
        let _ = log.insert_all(Vec::new(), now_ms);
        let kept = Suspensions::of(&inner);
        suspensions::settle(&mut inner, now_ms, BTreeSet::new());
        let saved = if kept == Suspensions::of(&inner) {
            Ok(())
        } else {
            self.save(&inner)
        };
        drop(inner);
        if self.suspended() != before {
            self.bump();
        }
        saved
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
        // Taken only while `inner` is held, so never out of order.
        let mut on_disk = self.on_disk.lock().unwrap_or_else(PoisonError::into_inner);
        let saved = store::save(&self.state_dir, inner, &mut on_disk);
        self.unsaved.store(saved.is_err(), Ordering::SeqCst);
        saved
    }

    /// Whether the last write of the fleet's files failed.
    pub(crate) fn is_unsaved(&self) -> bool {
        self.unsaved.load(Ordering::SeqCst)
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
            return fleet.apply(&apply, now);
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

/// The history of a fleet being joined, `joining` so far with `ops` added:
/// a founding operation starts it anew unless it founded `joining`, and
/// with neither there is nothing to join (`Error::NotAdmitted`).
fn join(joining: Option<Log>, mut ops: Vec<Op>, now_ms: u64) -> Result<(Log, usize)> {
    let found = ops
        .iter()
        .position(|op| matches!(op.body(), Body::Found(_)));
    let mut log = match (joining, found) {
        (Some(log), Some(found)) if log.fleet().0 == ops[found].id().0 => {
            ops.swap_remove(found);
            log
        }
        (_, Some(found)) => Log::found(ops.swap_remove(found))?,
        (Some(log), None) => log,
        (None, None) => return Err(Error::NotAdmitted),
    };
    let refused = log
        .insert_all(ops, now_ms)
        .iter()
        .filter(|(_, result)| result.is_err())
        .count();
    Ok((log, refused))
}

/// The first bytes of a fleet session this computer opens with
/// `purpose`: the purpose byte, then the proof that the device in the
/// session whose handshake hash is `hash` holds this computer's fleet key
/// (`control::fleet::removal`). Blocking: it reads the fleet key.
pub(crate) fn opening(
    state_dir: &Path,
    purpose: u8,
    hash: &[u8; 32],
) -> crate::desktop::Result<Vec<u8>> {
    let key = store::load_or_create_key(state_dir)?;
    let mut bytes = vec![purpose];
    bytes.extend_from_slice(&crate::control::fleet::removal::prove(&key, hash));
    Ok(bytes)
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
