//! Which pull request a chat's running turn created or pushed to, so its
//! "finished" push can offer to open it. The chat program lists such a
//! pull request once in every turn that touches it
//! (`Change::PullRequest`), even when the transcript knows it already.

use sheprd_chat_protocol::{
    Change, EntryId, EntryKind, PullRequest, PullRequestAction, Transcript,
};

/// The last pull request the running turn created or pushed to.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TurnPull(Option<String>);

impl TurnPull {
    /// Steps past `change`; the pull request's address when `change` ends
    /// the turn that touched it.
    pub(crate) fn step(&mut self, change: &Change) -> Option<String> {
        match change {
            Change::PullRequest { pull_request } if touched(pull_request) => {
                self.0 = Some(pull_request.url.clone());
                None
            }
            Change::Push { entry } => match &entry.kind {
                EntryKind::User { .. } => {
                    self.0 = None;
                    None
                }
                EntryKind::Turn(_) => self.0.take(),
                _ => None,
            },
            _ => None,
        }
    }

    /// What is kept when the chat's copy `before` is replaced by a fresh
    /// snapshot `after`: the pull request only while `after` is still the
    /// turn `before` was in, as a snapshot taken after that turn ended or
    /// another began skipped both edges.
    pub(crate) fn carried(self, before: &Transcript, after: &Transcript) -> Self {
        match (running(before), running(after)) {
            (Some(was), Some(is)) if was == is => self,
            _ => Self::default(),
        }
    }
}

/// The user entry that began the turn still running in `transcript`: its
/// last, with no turn summary after it.
fn running(transcript: &Transcript) -> Option<EntryId> {
    for entry in transcript.entries.iter().rev() {
        match entry.kind {
            EntryKind::Turn(_) => return None,
            EntryKind::User { .. } => return Some(entry.id),
            _ => {}
        }
    }
    None
}

/// Whether the agent created or pushed to `pull`, a well-formed GitHub pull
/// request.
fn touched(pull: &PullRequest) -> bool {
    pull.fits()
        && pull.actions.iter().any(|action| {
            matches!(
                action,
                PullRequestAction::Created | PullRequestAction::Pushed
            )
        })
}
