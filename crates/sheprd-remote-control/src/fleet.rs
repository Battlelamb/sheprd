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
//! - [`scrub`]: forgetting the fleet, with every way it let someone in.
//!
//! Every change bumps [`Fleet::changed`]; `follow` then rewrites what the
//! log grants, off the control thread.

pub(crate) mod ask;
mod error;
pub(crate) mod grants;
mod joining;
mod leaving;
mod lock;
mod me;
pub(crate) mod removals;
pub(crate) mod scrub;
mod store;

pub use error::{Error, Result};

use crate::control::fleet::{Body, HostId, Inserted, Log, MemberId, Op, OpId, Unsigned, sync};
use p256::ecdsa::SigningKey;
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
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

/// How long the SSH session that asked for a scrub keeps running after
/// it, to read the answer.
pub(crate) const SCRUB_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

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
    /// This computer joined its fleet as a shared host, not as a member
    /// (`fleet/hosted` names that fleet): only then does it act as `host`
    /// without being a member. A member that was removed never does.
    hosted: AtomicBool,
    inner: Mutex<Inner>,
    /// Parts of logs handed over while this computer is in no fleet.
    joining: joining::Handovers,
    changed: watch::Sender<u64>,
    /// Held while what the log grants is read and written, and while a
    /// scrub forgets it, so a write of the old fleet never lands after a
    /// scrub took its access away.
    writing: Mutex<()>,
    /// When this computer last forgot its fleet (`fleet scrub`), and the
    /// SSH session that asked for it, by its client port on loopback: only
    /// that session keeps [`SCRUB_GRACE`] to read the answer.
    scrubbed: Mutex<Option<(std::time::Instant, u16)>>,
    /// Removal exchanges other members dialed in for, at most
    /// `removals::CONCURRENT` at once.
    pub(crate) inbound: tokio::sync::Semaphore,
    /// The fleet folder's lock, which the daemon holds for its life.
    _lock: Option<lock::Lock>,
    /// The last write of the fleet's files failed: what is held is ahead
    /// of them, and the next change, even one that moves nothing, writes
    /// again.
    unsaved: AtomicBool,
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
        let host = match crate::tls::Authority::stored_certificate(state_dir) {
            Ok(certificate) => certificate.map(|der| HostId(Sha256::digest(&der).into())),
            Err(error) => {
                // Fails closed: with no host id, it is no fleet host here.
                tracing::warn!(%error, "could not read the certificate authority for the fleet");
                None
            }
        };
        let hosted = match (store::load_hosted(state_dir)?, &inner.log) {
            (Some(fleet), Some(log)) => fleet == log.fleet().to_string(),
            _ => false,
        };
        Ok(Self {
            state_dir: state_dir.to_owned(),
            key,
            me,
            host,
            hosted: AtomicBool::new(hosted),
            inner: Mutex::new(inner),
            joining: joining::Handovers::default(),
            changed: watch::Sender::new(0),
            writing: Mutex::new(()),
            scrubbed: Mutex::new(None),
            inbound: tokio::sync::Semaphore::new(removals::CONCURRENT),
            _lock: lock,
            unsaved: AtomicBool::new(false),
        })
    }

    pub(crate) fn me(&self) -> MemberId {
        self.me
    }

    /// This computer's host id, from its certificate authority, when it
    /// joined its fleet as a shared host: what it acts as without being a
    /// member. `None` for a computer that joined as a member, removed or
    /// not.
    pub(crate) fn host(&self) -> Option<HostId> {
        self.host.filter(|_| self.hosted.load(Ordering::SeqCst))
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
        self.release_if_left(&mut inner)?;
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
        self.release_if_left(&mut inner)?;
        let before = inner.log.as_ref().map(|log| log.heads().to_vec());
        let suspensions: Vec<OpId> = inner.unconfirmed.keys().copied().collect();
        let refused = match &mut inner.log {
            Some(log) => log
                .insert_all(ops, now_ms)
                .iter()
                .filter(|(_, result)| result.is_err())
                .count(),
            None => {
                let (log, refused) = join(ops, self.me, self.host, now_ms)?;
                // Taken because it shares this computer, not because it
                // admits it: remembered, so only such a computer ever acts
                // as a host without being a member.
                if !sync::admits(&log, &self.me) {
                    store::save_hosted(&self.state_dir, &log.fleet().to_string())?;
                    self.hosted.store(true, Ordering::SeqCst);
                }
                inner.log = Some(log);
                refused
            }
        };
        settle_unconfirmed(&mut inner, now_ms);
        let after = inner.log.as_ref().map(|log| log.heads().to_vec());
        // A suspension lifted is a change too: its target's grants return.
        let changed = after != before || !suspensions.iter().eq(inner.unconfirmed.keys());
        if changed || !inner.unconfirmed.is_empty() || self.is_unsaved() {
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

    /// Keeps `ops`, one part of `fleet`'s log handed over to this
    /// computer while it is in no fleet, until the last part arrives
    /// ([`Fleet::receive_last`]). Kept in memory: a handover needs the
    /// running daemon. Blocking.
    pub(crate) fn stage(&self, fleet: &str, ops: &[Op], now_ms: u64) -> Result<()> {
        self.joining.keep(fleet, ops, now_ms)
    }

    /// Takes `ops`, the last part of `fleet`'s log handed over, with every
    /// part kept before it, as [`Fleet::receive`] does.
    pub(crate) fn receive_last(&self, fleet: &str, ops: Vec<Op>, now_ms: u64) -> Result<usize> {
        let mut all = self.joining.take(fleet, now_ms);
        all.extend(ops);
        self.receive(all, now_ms)
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

    /// Every tunnel key a member proved here: what scrubbing the fleet
    /// revokes pairings for first, as if every member were removed.
    pub(crate) fn proven_keys(&self) -> Vec<[u8; 32]> {
        self.inner().proven.values().flatten().copied().collect()
    }

    /// Forgets the fleet: its log, which removals each peer holds, the
    /// suspensions waiting for history, and the keys members proved. The
    /// key stays: a fleet founded or joined later is another one, where it
    /// was never removed. Callers revoke the proven keys' pairings first
    /// ([`Fleet::proven_keys`]), and remove what the fleet wrote after
    /// ([`scrub::live`]). Blocking.
    ///
    /// What is held goes only once its file is gone: a log left on disk is
    /// the fleet the daemon reads at its next start, so it stays the fleet
    /// here too, and a peers file that stays keeps the keys to revoke.
    /// `session` is the client port of the fleet SSH session that asked,
    /// if one did: that one session may finish reading the answer.
    pub(crate) fn scrub(&self, session: Option<u16>) -> Result<()> {
        *self.scrubbed.lock().unwrap_or_else(PoisonError::into_inner) =
            session.map(|session| (std::time::Instant::now(), session));
        let mut inner = self.inner();
        store::remove_log(&self.state_dir)?;
        inner.log = None;
        let removed = store::remove_peers(&self.state_dir);
        if removed.is_ok() {
            *inner = Inner::default();
        }
        drop(inner);
        self.bump();
        removed
    }

    /// Records that `peer` holds the removals `ids`. Blocking.
    pub(crate) fn held(&self, peer: MemberId, ids: impl IntoIterator<Item = OpId>) -> Result<()> {
        let mut inner = self.inner();
        inner.held.entry(peer).or_default().extend(ids);
        self.save(&inner)
    }

    /// Held while what the log grants is read and written.
    pub(crate) fn writing(&self) -> MutexGuard<'_, ()> {
        self.writing.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How long the fleet SSH session whose client port on loopback is
    /// `session` may still run after a scrub cut it off: what is left of
    /// [`SCRUB_GRACE`], only for the session that asked for the scrub, and
    /// only while this computer is in no fleet again. Every other session
    /// ends at once; access is gone already and no new session starts.
    pub(crate) fn scrub_grace(&self, session: u16) -> Option<std::time::Duration> {
        let in_fleet = self.inner().log.is_some();
        let mut scrubbed = self.scrubbed.lock().unwrap_or_else(PoisonError::into_inner);
        let (at, asked) = (*scrubbed)?;
        let left = SCRUB_GRACE.checked_sub(at.elapsed()).filter(|_| !in_fleet);
        if left.is_none() {
            *scrubbed = None;
        }
        left.filter(|_| asked == session)
    }

    pub(crate) fn bump(&self) {
        self.changed
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    fn save(&self, inner: &Inner) -> Result<()> {
        let saved = store::save(&self.state_dir, inner);
        self.unsaved
            .store(saved.is_err(), Ordering::SeqCst);
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

/// `fleet scrub`, run over SSH or by hand: asks the running daemon to
/// forget its fleet, or, with none running, forgets it here and removes
/// the fleet's `authorized_keys` lines and relay records itself
/// ([`scrub`]). Succeeds only once they are gone. Blocking.
pub fn run_scrub(state_dir: &Path) -> Result<()> {
    let session = std::env::var("SSH_CONNECTION")
        .ok()
        .and_then(|connection| loopback_client_port(&connection));
    scrub_with(state_dir, session, scrub::offline)
}

/// The client port in `SSH_CONNECTION` ("client port server port") when
/// the client is on loopback, as a fleet member's session is: the daemon
/// spliced it to the SSH server from that port.
pub(crate) fn loopback_client_port(connection: &str) -> Option<u16> {
    let mut fields = connection.split_whitespace();
    let client: std::net::IpAddr = fields.next()?.parse().ok()?;
    let port = fields.next()?.parse().ok()?;
    client.is_loopback().then_some(port)
}

/// [`run_scrub`] for the fleet session `session`, with `offline`
/// forgetting the fleet when no daemon answers, run only while holding
/// the fleet folder's lock. Blocking.
fn scrub_with(
    state_dir: &Path,
    session: Option<u16>,
    offline: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    use crate::control::{Request, Response};
    let request = Request::Fleet(Box::new(sync::Ask::Scrub { session }));
    let deadline = std::time::Instant::now() + lock::WAIT;
    loop {
        match crate::control::request(&crate::control::address(state_dir), &request) {
            Ok(Response::Fleet(answer)) if *answer == sync::Answer::Done => return Ok(()),
            Ok(Response::Error { message, .. }) => return Err(Error::Daemon(message)),
            Ok(_) => return Err(Error::Daemon("an unexpected answer".to_owned())), // i18n: data
            Err(crate::control::Error::Connect(_)) => {}
            Err(error) => return Err(error.into()),
        }
        // No daemon answers. Held, the lock keeps a daemon from loading
        // the fleet, and an offline update from writing it, until the
        // files are gone; busy, a daemon is starting (it loads the fleet
        // before it answers) or an update is writing, and the socket is
        // tried again.
        if let Some(_lock) = lock::Lock::try_acquire(state_dir)? {
            return offline(state_dir);
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::Busy);
        }
        lock::pause();
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
    let point = key.verifying_key().to_sec1_point(true);
    MemberId(Sha256::digest(point.as_bytes()).into())
}

// Every test here runs members on loopback with a `USER`: Unix only.
#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
