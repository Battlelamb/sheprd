//! A member's copy of the fleet log: the operations it holds, those
//! waiting for their parents, and the state they compute.

use super::{
    Body, Error, FleetId, MAX_OPS, MAX_PENDING, MAX_SKEW_MS, MemberId, Op, OpId, PENDING_TTL_MS,
    Result, State, Unsigned,
    fold::fold,
    graph::{Bits, Graph},
    membership::Folder,
};
use std::collections::{BTreeMap, HashSet};

/// What inserting an operation came to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Inserted {
    /// It passed its checks and is part of the log.
    Applied,
    /// The log already held it.
    Known,
    /// Its parents have not arrived; it waits for them.
    Pending,
}

/// The fleet log.
#[derive(Debug)]
pub struct Log {
    fleet: FleetId,
    /// The checkpoint the log was pruned to, which every kept operation
    /// descends from, and the state it names.
    root: Option<Op>,
    base: State,
    ops: BTreeMap<OpId, Op>,
    /// Operations waiting for parents, with when each arrived.
    pending: BTreeMap<OpId, (Op, u64)>,
    state: State,
    heads: Vec<OpId>,
    /// Operations whose check passed, which never changes.
    checked: HashSet<OpId>,
}

impl Log {
    /// A new fleet, founded by `found`.
    pub fn found(found: Op) -> Result<Self> {
        if !matches!(found.body(), Body::Found(_)) {
            return Err(Error::Invalid);
        }
        let mut log = Self {
            fleet: FleetId(found.id().0),
            root: None,
            base: State::default(),
            ops: BTreeMap::from([(found.id(), found)]),
            pending: BTreeMap::new(),
            state: State::default(),
            heads: Vec::new(),
            checked: HashSet::new(),
        };
        let graph = Graph::new(&log.ops, None)?;
        let folder = Folder::new(&graph, &log.base, log.fleet, &HashSet::new())?;
        folder.check(0).clone()?;
        log.state = fold(&folder, &Bits::full(graph.len()), &log.base);
        log.heads = graph.heads();
        drop(folder);
        drop(graph);
        log.checked = log.ops.keys().copied().collect();
        Ok(log)
    }

    /// A log that starts at `checkpoint`, whose history computed `state`,
    /// as a joining member is handed it. The joiner trusts the member that
    /// approved it for that history; everything after is checked.
    pub fn from_checkpoint(checkpoint: Op, state: &[u8]) -> Result<Self> {
        let Body::Checkpoint(hash) = checkpoint.body() else {
            return Err(Error::Invalid);
        };
        let base = State::decode(state)?;
        if base.hash()? != *hash {
            return Err(Error::Checkpoint);
        }
        let signer = base
            .members
            .get(&checkpoint.signer())
            .ok_or(Error::Signer)?;
        if signer.bundle.kind != super::Kind::Computer {
            return Err(Error::Permission);
        }
        checkpoint.verify(&signer.bundle.verifying_key()?)?;
        let mut base = base;
        counted(&mut base, &checkpoint);
        Ok(Self {
            fleet: checkpoint.unsigned().fleet,
            heads: vec![checkpoint.id()],
            root: Some(checkpoint),
            state: base.clone(),
            base,
            ops: BTreeMap::new(),
            pending: BTreeMap::new(),
            checked: HashSet::new(),
        })
    }

    #[must_use]
    pub fn fleet(&self) -> FleetId {
        self.fleet
    }

    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The operations nothing in the log names as a parent.
    #[must_use]
    pub fn heads(&self) -> &[OpId] {
        &self.heads
    }

    /// The checkpoint the log was pruned to.
    #[must_use]
    pub fn root(&self) -> Option<&Op> {
        self.root.as_ref()
    }

    #[must_use]
    pub fn get(&self, id: &OpId) -> Option<&Op> {
        self.ops
            .get(id)
            .or_else(|| self.root.as_ref().filter(|root| root.id() == *id))
    }

    /// Every operation held, in id order.
    pub fn ops(&self) -> impl Iterator<Item = &Op> {
        self.ops.values()
    }

    /// Operations waiting for their parents.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// An operation for `signer` to sign next: on every head, with the
    /// next sequence number.
    #[must_use]
    pub fn draft(&self, signer: MemberId, issued_ms: u64, body: Body) -> Unsigned {
        let mut parents = self.heads.clone();
        parents.sort_unstable();
        let seq = self
            .ops
            .values()
            .chain(self.root.as_ref())
            .filter(|op| op.signer() == signer)
            .map(|op| op.unsigned().seq)
            .chain(self.base.members.get(&signer).map(|member| member.seq))
            .max()
            .unwrap_or(0)
            + 1;
        Unsigned {
            fleet: self.fleet,
            signer,
            seq,
            issued_ms,
            parents,
            body,
        }
    }

    /// Inserts one operation that arrived at `now_ms`.
    pub fn insert(&mut self, op: Op, now_ms: u64) -> Result<Inserted> {
        let id = op.id();
        self.insert_all(vec![op], now_ms)
            .into_iter()
            .find(|(inserted, _)| *inserted == id)
            .map_or(Ok(Inserted::Known), |(_, result)| result)
    }

    /// Inserts operations that arrived together at `now_ms`, in any order,
    /// folding once. Says what each came to; one refused never stops the
    /// rest, though its descendants are refused with it.
    pub fn insert_all(&mut self, ops: Vec<Op>, now_ms: u64) -> Vec<(OpId, Result<Inserted>)> {
        let mut results = Vec::with_capacity(ops.len());
        let mut fresh = Vec::new();
        for op in ops {
            let id = op.id();
            let result = self.admissible(&op, now_ms);
            match result {
                Ok(Inserted::Pending) => {
                    self.pending.entry(id).or_insert((op, now_ms));
                    fresh.push(id);
                }
                other => results.push((id, other)),
            }
        }
        self.expire(now_ms);
        let mut added = Vec::new();
        loop {
            let ready: Vec<OpId> = self
                .pending
                .iter()
                .filter(|(_, (op, _))| op.parents().iter().all(|parent| self.present(parent)))
                .map(|(id, _)| *id)
                .collect();
            if ready.is_empty() {
                break;
            }
            for id in ready {
                if let Some((op, _)) = self.pending.remove(&id) {
                    self.ops.insert(id, op);
                    added.push(id);
                }
            }
        }
        let refused = match self.settle(&added) {
            Ok(refused) => refused,
            Err(error) => {
                // The log cannot take these at all (full): put it back as
                // it was.
                for id in &added {
                    self.ops.remove(id);
                }
                added.iter().map(|id| (*id, error.clone())).collect()
            }
        };
        for id in fresh {
            let result = match refused.get(&id) {
                Some(error) => Err(error.clone()),
                None if self.ops.contains_key(&id) => Ok(Inserted::Applied),
                None => Ok(Inserted::Pending),
            };
            results.push((id, result));
        }
        results
    }

    /// The state of the history of `id`, which a checkpoint at `id` names
    /// and a joining member is handed with it.
    pub fn state_at(&self, id: &OpId) -> Result<State> {
        if self.root.as_ref().is_some_and(|root| root.id() == *id) {
            return Ok(self.base.clone());
        }
        let graph = self.graph()?;
        let folder = self.folder(&graph)?;
        let position = *graph.index.get(id).ok_or(Error::Unknown)?;
        Ok(fold(&folder, &graph.ancestors[position], &self.base))
    }

    /// Prunes the log to the checkpoint `id`: drops its history and keeps
    /// its state. Refused while any operation held is concurrent with it,
    /// since that one's history could not be recomputed after.
    pub fn prune(&mut self, id: &OpId) -> Result<()> {
        let graph = self.graph()?;
        let position = *graph.index.get(id).ok_or(Error::Unknown)?;
        if !matches!(graph.order[position].body(), Body::Checkpoint(_))
            || (0..graph.len()).any(|other| graph.concurrent(position, other))
        {
            return Err(Error::Checkpoint);
        }
        let folder = self.folder(&graph)?;
        let mut base = fold(&folder, &graph.ancestors[position], &self.base);
        counted(&mut base, graph.order[position]);
        let dropped: Vec<OpId> = graph.ancestors[position]
            .iter()
            .map(|ancestor| graph.order[ancestor].id())
            .collect();
        drop(folder);
        drop(graph);
        for ancestor in dropped {
            self.ops.remove(&ancestor);
        }
        self.root = self.ops.remove(id);
        self.base = base;
        self.recompute()
    }

    fn admissible(&self, op: &Op, now_ms: u64) -> Result<Inserted> {
        if self.present(&op.id()) || self.pending.contains_key(&op.id()) {
            return Ok(Inserted::Known);
        }
        if matches!(op.body(), Body::Found(_)) || op.unsigned().fleet != self.fleet {
            return Err(Error::Fleet);
        }
        if op.unsigned().issued_ms > now_ms.saturating_add(MAX_SKEW_MS) {
            return Err(Error::Future);
        }
        Ok(Inserted::Pending)
    }

    fn present(&self, id: &OpId) -> bool {
        self.ops.contains_key(id) || self.root.as_ref().is_some_and(|root| root.id() == *id)
    }

    fn expire(&mut self, now_ms: u64) {
        self.pending
            .retain(|_, (_, since)| now_ms.saturating_sub(*since) <= PENDING_TTL_MS);
        while self.pending.len() > MAX_PENDING {
            let oldest = self
                .pending
                .iter()
                .min_by_key(|(id, (_, since))| (*since, **id))
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                self.pending.remove(&oldest);
            }
        }
    }

    /// Checks the operations just `added`, removes those refused and their
    /// descendants, and folds. Says why each was refused.
    fn settle(&mut self, added: &[OpId]) -> Result<BTreeMap<OpId, Error>> {
        if self.ops.len() > MAX_OPS {
            return Err(Error::Full);
        }
        let mut refused = BTreeMap::new();
        {
            let graph = self.graph()?;
            let folder = self.folder(&graph)?;
            let fresh: HashSet<OpId> = added.iter().copied().collect();
            for position in 0..graph.len() {
                let op = graph.order[position];
                if !fresh.contains(&op.id()) {
                    continue;
                }
                let parent_refused = op
                    .parents()
                    .iter()
                    .any(|parent| refused.contains_key(parent));
                let result = if parent_refused {
                    Err(Error::ParentMissing)
                } else {
                    folder
                        .check(position)
                        .clone()
                        .and_then(|()| checkpoint_matches(&folder, position, &self.base))
                };
                if let Err(error) = result {
                    refused.insert(op.id(), error);
                }
            }
        }
        for id in refused.keys() {
            self.ops.remove(id);
        }
        self.checked = self.ops.keys().copied().collect();
        self.recompute()?;
        Ok(refused)
    }

    fn recompute(&mut self) -> Result<()> {
        let graph = self.graph()?;
        let folder = self.folder(&graph)?;
        let state = fold(&folder, &Bits::full(graph.len()), &self.base);
        let mut heads = graph.heads();
        if heads.is_empty()
            && let Some(root) = &self.root
        {
            heads.push(root.id());
        }
        drop(folder);
        drop(graph);
        self.state = state;
        self.heads = heads;
        Ok(())
    }

    fn graph(&self) -> Result<Graph<'_>> {
        Graph::new(&self.ops, self.root.as_ref().map(Op::id))
    }

    fn folder<'g, 'a>(&'g self, graph: &'g Graph<'a>) -> Result<Folder<'g, 'a>> {
        Folder::new(graph, &self.base, self.fleet, &self.checked)
    }
}

/// Counts the checkpoint `root` itself in a state computed from its
/// history, so its signer's next sequence number follows it.
fn counted(base: &mut State, root: &Op) {
    if let Some(member) = base.members.get_mut(&root.signer()) {
        member.seq = member.seq.max(root.unsigned().seq);
    }
}

/// A checkpoint must name the state of its own history.
fn checkpoint_matches(folder: &Folder<'_, '_>, position: usize, base: &State) -> Result<()> {
    let graph = folder.graph();
    let Body::Checkpoint(hash) = graph.order[position].body() else {
        return Ok(());
    };
    let state = fold(folder, &graph.ancestors[position], base);
    if state.hash()? == *hash {
        Ok(())
    } else {
        Err(Error::Checkpoint)
    }
}
