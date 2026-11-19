//! A pull request's place in a GitHub stack, and in its base branch's merge
//! queue. GitHub merges a stack from the bottom up, so merging one layer also
//! merges every open layer under it: the [`Verdict`] is about those layers
//! together, and the lowest one that cannot merge is what blocks the rest.
//!
//! Read-only. Titles are remote text, cleaned on the way in like the pull
//! request's own; layers past [`LAYER_LIMIT`] are not read.

use super::{MergeState, Outcome, ReviewDecision, Standing, State, Tone, clean};
use serde::Deserialize;

/// The most layers a lookup reads of a stack. The query names the same
/// number; a test holds them together.
pub const LAYER_LIMIT: usize = 16;

/// A GitHub stack, bottom layer first.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "RawStack")]
pub struct Stack {
    pub number: u64,
    /// The branch the bottom layer merges into.
    pub base: String,
    /// How many layers GitHub says the stack has, read or not.
    pub size: u64,
    /// The layers read, by ascending position, which starts at 1.
    pub layers: Vec<Layer>,
}

/// One pull request in a stack, with what decides whether it can merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub position: u64,
    pub number: u64,
    pub title: String,
    pub state: State,
    pub is_draft: bool,
    pub merge: MergeState,
    pub review: ReviewDecision,
    /// GitHub's rollup of the head commit's checks; `None` when none ran.
    pub checks: Option<Outcome>,
}

/// Why a layer cannot merge, most fundamental first: [`Layer::blocker`]
/// reports the first that applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    Closed,
    Draft,
    Conflicts,
    ChecksFailed,
    ChangesRequested,
    ChecksRunning,
    ReviewRequired,
    Behind,
    /// Branch protection holds it for a reason GitHub does not say.
    Held,
    /// GitHub has not worked out whether it can merge yet.
    Undecided,
}

impl Blocker {
    /// A stable word for scripts, as `sheprd pr status --json` reports it.
    pub fn key(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Draft => "draft",
            Self::Conflicts => "conflicts",
            Self::ChecksFailed => "checks_failed",
            Self::ChangesRequested => "changes_requested",
            Self::ChecksRunning => "checks_running",
            Self::ReviewRequired => "review_required",
            Self::Behind => "behind",
            Self::Held => "held",
            Self::Undecided => "undecided",
        }
    }
}

/// What merging a stack at one layer would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// That layer and every one under it merged already.
    Merged,
    /// `count` open layers, up to and including that one, can merge.
    Ready { count: usize },
    /// The lowest layer read up to that one that cannot merge, and why.
    Blocked { number: u64, blocker: Blocker },
    /// No layer read blocks, but some layer up to that one was not read
    /// (past the read limit, or hidden from this reader), so whether it can
    /// merge, and what it would land, is not known.
    Unread,
}

impl Layer {
    fn standing(&self) -> Standing {
        Standing {
            merge: self.merge,
            review: self.review,
            failed: self.checks == Some(Outcome::Failed),
            running: self.checks == Some(Outcome::Pending),
        }
    }

    /// The badge tone, as the pull request's own would be.
    pub fn tone(&self) -> Tone {
        super::model::lifecycle_tone(self.state, self.is_draft, || self.standing().tone())
    }

    /// The layer's status as a stable word: `merged`, `ready`, or why it
    /// cannot merge ([`Blocker::key`]).
    pub fn status_key(&self) -> &'static str {
        match self.blocker() {
            None if self.state == State::Merged => "merged",
            None => "ready",
            Some(blocker) => blocker.key(),
        }
    }

    /// Why this layer cannot merge, or `None` when it can or already did.
    /// Agrees with [`Self::tone`]: an open layer is unblocked exactly when it
    /// is [`Tone::Ready`].
    pub fn blocker(&self) -> Option<Blocker> {
        let standing = self.standing();
        match self.state {
            State::Merged => return None,
            State::Closed => return Some(Blocker::Closed),
            State::Unknown => return Some(Blocker::Undecided),
            State::Open if self.is_draft => return Some(Blocker::Draft),
            State::Open => {}
        }
        Some(match standing.merge {
            MergeState::Dirty => Blocker::Conflicts,
            _ if standing.failed
                || (standing.merge == MergeState::Unstable && !standing.running) =>
            {
                Blocker::ChecksFailed
            }
            _ if standing.review == ReviewDecision::ChangesRequested => Blocker::ChangesRequested,
            _ if standing.running => Blocker::ChecksRunning,
            _ if standing.review == ReviewDecision::ReviewRequired => Blocker::ReviewRequired,
            MergeState::Clean => return None,
            MergeState::Behind => Blocker::Behind,
            MergeState::Blocked => Blocker::Held,
            _ => Blocker::Undecided,
        })
    }
}

impl Stack {
    /// The layers merging at `position` would land: the open ones at or
    /// below it, bottom first.
    pub fn landing(&self, position: u64) -> impl Iterator<Item = &Layer> {
        self.layers
            .iter()
            .filter(move |layer| layer.position <= position && layer.state != State::Merged)
    }

    /// What merging at `position` would do. Ready and merged need every
    /// layer up to it read: an unread one may be open, and blocked.
    pub fn verdict(&self, position: u64) -> Verdict {
        let mut count = 0;
        for layer in self.landing(position) {
            if let Some(blocker) = layer.blocker() {
                return Verdict::Blocked {
                    number: layer.number,
                    blocker,
                };
            }
            count += 1;
        }
        if !self.read_through(position) {
            return Verdict::Unread;
        }
        match count {
            0 => Verdict::Merged,
            count => Verdict::Ready { count },
        }
    }

    /// Whether every layer from the bottom up to `position` was read.
    pub fn read_through(&self, position: u64) -> bool {
        (1..=position).all(|wanted| self.layers.iter().any(|layer| layer.position == wanted))
    }

    /// Whether some layers were not read.
    pub fn truncated(&self) -> bool {
        self.size > self.layers.len() as u64
    }
}

impl super::PullRequest {
    /// The pull requests merging or queueing this one lands, bottom first:
    /// on a stack, every open layer up to it as read; else itself.
    pub fn lands(&self) -> Vec<u64> {
        match self.stack() {
            Some((stack, position)) => stack.landing(position).map(|layer| layer.number).collect(),
            None => vec![self.number],
        }
    }

    /// Whether a merge may be offered as far as its stack goes: in no
    /// stack, or every layer up to it read and ready, so a confirmation can
    /// name all that lands.
    pub fn stack_mergeable(&self) -> bool {
        self.stack().is_none_or(|(stack, position)| {
            matches!(stack.verdict(position), Verdict::Ready { .. })
        })
    }

    /// Whether queueing may be offered as far as its stack goes: in no
    /// stack, or every layer up to it read.
    pub fn stack_known(&self) -> bool {
        self.stack()
            .is_none_or(|(stack, position)| stack.read_through(position))
    }
}

/// A pull request's entry in its base branch's merge queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct QueueEntry {
    pub state: QueueState,
    /// Its place in the queue, from 1.
    pub position: u64,
    /// GitHub's estimate of the seconds until it merges, when it has one.
    #[serde(rename = "estimatedTimeToMerge")]
    pub eta: Option<u64>,
}

/// GitHub's `MergeQueueEntryState`. Advisory like [`MergeState`], so a value
/// added upstream degrades to `Unknown` rather than failing the lookup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum QueueState {
    // keep-sorted start
    AwaitingChecks,
    Locked,
    Mergeable,
    Queued,
    Unmergeable,
    // keep-sorted end
    #[default]
    Unknown,
}

impl QueueState {
    /// A stable word for scripts, as `sheprd pr status --json` reports it.
    pub fn key(self) -> &'static str {
        match self {
            Self::AwaitingChecks => "awaiting_checks",
            Self::Locked => "locked",
            Self::Mergeable => "mergeable",
            Self::Queued => "queued",
            Self::Unmergeable => "unmergeable",
            Self::Unknown => "unknown",
        }
    }
}

impl From<String> for QueueState {
    fn from(value: String) -> Self {
        match value.as_str() {
            "AWAITING_CHECKS" => Self::AwaitingChecks,
            "LOCKED" => Self::Locked,
            "MERGEABLE" => Self::Mergeable,
            "QUEUED" => Self::Queued,
            "UNMERGEABLE" => Self::Unmergeable,
            _ => Self::Unknown,
        }
    }
}

/// `stackEntry`: where the pull request itself sits.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(super) struct Entry {
    pub(super) position: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStack {
    number: u64,
    base_ref_name: String,
    size: u64,
    entries: Nodes<RawEntry>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<T>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEntry {
    position: u64,
    /// Null when the reader may not see that pull request.
    pull_request: Option<RawLayer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLayer {
    number: u64,
    title: String,
    state: State,
    is_draft: bool,
    merge_state_status: MergeState,
    #[serde(default)]
    review_decision: ReviewDecision,
    commits: Nodes<RawCommit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCommit {
    commit: RawRollupHolder,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRollupHolder {
    status_check_rollup: Option<RawRollup>,
}

#[derive(Deserialize)]
struct RawRollup {
    state: Option<String>,
}

impl From<RawStack> for Stack {
    fn from(raw: RawStack) -> Self {
        let mut layers: Vec<Layer> = raw
            .entries
            .nodes
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let pr = entry.pull_request?;
                let checks = pr
                    .commits
                    .nodes
                    .into_iter()
                    .flatten()
                    .next()
                    .and_then(|commit| commit.commit.status_check_rollup)
                    .and_then(|rollup| rollup.state)
                    .map(|state| Outcome::from(Some(state)));
                Some(Layer {
                    position: entry.position,
                    number: pr.number,
                    title: clean(&pr.title),
                    state: pr.state,
                    is_draft: pr.is_draft,
                    merge: pr.merge_state_status,
                    review: pr.review_decision,
                    checks,
                })
            })
            .take(LAYER_LIMIT)
            .collect();
        layers.sort_by_key(|layer| layer.position);
        layers.dedup_by_key(|layer| layer.position);
        Self {
            number: raw.number,
            base: clean(&raw.base_ref_name),
            size: raw.size,
            layers,
        }
    }
}

/// Layer 3 of 4 of a stack onto `main`: #1361 merged, #1362 with a failed
/// check, this one (#1364) waiting on a review, and #1366 a draft.
#[cfg(any(test, feature = "test-support"))]
pub fn stacked_fixture() -> crate::Result<super::PullRequest> {
    stacked(false)
}

/// [`stacked_fixture`] once #1362's checks pass and #1364 is approved, so
/// both can merge, on a branch with a merge queue.
#[cfg(any(test, feature = "test-support"))]
pub fn ready_stacked_fixture() -> crate::Result<super::PullRequest> {
    stacked(true)
}

#[cfg(any(test, feature = "test-support"))]
fn stacked(ready: bool) -> crate::Result<super::PullRequest> {
    let (below, here, review) = if ready {
        ("SUCCESS", "CLEAN", "APPROVED")
    } else {
        ("FAILURE", "BLOCKED", "REVIEW_REQUIRED")
    };
    let layer = |position: u64,
                 number: u64,
                 state: &str,
                 draft: bool,
                 merge: &str,
                 review: Option<&str>,
                 rollup: &str| {
        serde_json::json!({"position": position, "pullRequest": {
            "number": number, "title": format!("Layer {position} of the T3 stack"),
            "state": state, "isDraft": draft, "mergeStateStatus": merge, "reviewDecision": review,
            "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": rollup}}}]}
        }})
    };
    super::parse::parse(&serde_json::json!([{
        "number": 1364, "url": "https://github.com/example/project/pull/1364",
        "title": "Show T3 sessions in the sidebar", "state": "OPEN", "isDraft": false,
        "headRefName": "feat/t3-sidebar", "baseRefName": "feat/t3-client",
        "additions": 120, "deletions": 4, "changedFiles": 6, "updatedAt": "2026-11-17T09:00:00Z",
        "mergeStateStatus": here, "reviewDecision": review, "isMergeQueueEnabled": ready,
        "headRepositoryOwner": {"login": "example"},
        "id": "PR_kwDOstacked", "headRefOid": "0123456789abcdef0123456789abcdef01234567",
        "stackEntry": {"position": 3},
        "stack": {"number": 950, "baseRefName": "main", "size": 4, "entries": {"nodes": [
            layer(1, 1361, "MERGED", false, "UNKNOWN", None, "SUCCESS"),
            layer(2, 1362, "OPEN", false, here, Some("APPROVED"), below),
            layer(3, 1364, "OPEN", false, here, Some(review), "SUCCESS"),
            layer(4, 1366, "OPEN", true, "DRAFT", None, "PENDING"),
        ]}}
    }]).to_string(), "example", "project", "feat/t3-sidebar")?
    .ok_or(super::Error::Repository.into())
}

#[cfg(test)]
mod tests;
