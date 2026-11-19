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
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
};
use tokio::sync::watch;

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
}

/// What removals are passed to: a member's daemon, or a plain host's
/// agent.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Holder {
    Host(HostId),
    Member(MemberId),
}

#[derive(Debug, Default)]
pub(crate) struct Inner {
    pub(crate) log: Option<Log>,
    /// The removals each peer said it holds, so none is sent twice.
    pub(crate) held: BTreeMap<Holder, BTreeSet<OpId>>,
    /// Members cut off by a removal whose history has not arrived, by the
    /// removal: kept after the removal expires from the pending pool,
    /// until its history arrives and settles it, its signer is no longer a
    /// member, or [`UNCONFIRMED_TTL_MS`] passes.
    pub(crate) unconfirmed: BTreeMap<OpId, Unconfirmed>,
}

impl Fleet {
    /// The fleet kept in `state_dir`, making this computer's fleet key the
    /// first time. Blocking.
    pub(crate) fn load(state_dir: &Path, now_ms: u64) -> Result<Self> {
        let key = store::load_or_create_key(state_dir)?;
        let me = member_id(&key);
        let inner = store::load(state_dir, now_ms)?;
        Ok(Self {
            state_dir: state_dir.to_owned(),
            key,
            me,
            inner: Mutex::new(inner),
            changed: watch::Sender::new(0),
            inbound: tokio::sync::Semaphore::new(removals::CONCURRENT),
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
                let (log, refused) = join(ops, self.me, now_ms)?;
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

    /// Records that `peer` holds the removals `ids`. Blocking.
    pub(crate) fn held(&self, peer: Holder, ids: impl IntoIterator<Item = OpId>) -> Result<()> {
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
    use sha2::{Digest, Sha256};
    let point = key.verifying_key().to_sec1_point(true);
    MemberId(Sha256::digest(point.as_bytes()).into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
