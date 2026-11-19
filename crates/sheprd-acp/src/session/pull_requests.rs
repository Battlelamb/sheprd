//! The pull requests the agent worked on: found in its finished tool calls,
//! or added by `sheprd chat pr add`, and kept in the transcript every
//! client shows.

use sheprd_chat_protocol::{Change, EntryKind, MAX_PULL_REQUESTS, PullRequest, Rejection};

use super::{Session, Update};

impl<R: FnMut(Update)> Session<R> {
    /// Adds the pull request `url` names, as the agent asked.
    pub(super) fn add_pull_request(&mut self, url: &str) -> Result<(), Rejection> {
        let pull = PullRequest::parse(url.trim()).ok_or(Rejection::InvalidPullRequest)?;
        if self.transcript.pull_request(&pull).is_some() {
            return Ok(());
        }
        if self.transcript.pull_requests.len() >= MAX_PULL_REQUESTS {
            return Err(Rejection::TooManyPullRequests);
        }
        self.emit(Change::PullRequest { pull_request: pull });
        Ok(())
    }

    /// Records the pull requests a tool call `change` finished worked on,
    /// unless the transcript already has each with all it did. A full list
    /// takes no more.
    pub(super) fn find_pull_requests(&mut self, change: &Change) {
        let (Change::Push { entry } | Change::Replace { entry }) = change else {
            return;
        };
        let EntryKind::Tool(tool) = &entry.kind else {
            return;
        };
        for pull in crate::pull_requests::found(tool) {
            let known = self.transcript.pull_request(&pull);
            let new = known.is_none_or(|known| {
                pull.actions
                    .iter()
                    .any(|action| !known.actions.contains(action))
            });
            let room = known.is_some() || self.transcript.pull_requests.len() < MAX_PULL_REQUESTS;
            if new && room {
                self.emit(Change::PullRequest { pull_request: pull });
            }
        }
    }
}
