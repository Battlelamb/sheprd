//! A chat's conversation across the agents that hold it.
//!
//! A chat may switch agents whenever its agent is idle. The conversation
//! stays in the transcript; the agent taking over gets a handoff
//! ([`sheprd_chat_protocol::handoff`]) with the user's next prompt. An agent
//! switched back to resumes its own session and hears only what happened
//! since it last took part.

use std::collections::HashMap;

use sheprd_chat_protocol::{
    Agent, Entry, EntryId, EntryKind,
    handoff::{self, Handoff, Scope},
};

/// How the agent starting now takes over the conversation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Joining {
    /// It has never seen it: it gets the whole handoff.
    Fresh,
    /// It continues its own session, which saw everything up to `seen`.
    Resume { seen: EntryId },
}

/// Where an agent left the conversation: its session, and the last entry
/// it saw.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Left {
    seen: EntryId,
    session: String,
}

#[derive(Debug, Default)]
pub(super) struct Thread {
    /// The conversation's first prompt, kept for handoffs once the
    /// transcript drops its oldest entries.
    first_prompt: Option<String>,
    /// The text sent ahead of the next prompt, for the agent that took over.
    handoff: Option<String>,
    /// The agents that held the conversation, oldest first, each once.
    held: Vec<Agent>,
    /// How the agent starting now takes over; `None` for the chat's first.
    joining: Option<Joining>,
    /// Where each agent switched away from left the conversation.
    left: HashMap<Agent, Left>,
    /// A resumed session replays its own history, which the transcript
    /// already shows: its updates are ignored until it opens.
    replaying: bool,
}

impl Thread {
    /// The thread of a chat started with `agent`, continuing the
    /// conversation `entries` already show, if any.
    pub(super) fn new(agent: Option<Agent>, entries: &[Entry]) -> Self {
        let first_prompt = entries.iter().find_map(|entry| match &entry.kind {
            EntryKind::User { text, .. } => Some(text.clone()),
            _ => None,
        });
        Self {
            first_prompt,
            held: agent.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Whether a prompt has reached an agent: only then is there a
    /// conversation to hand over.
    pub(super) fn started(&self) -> bool {
        self.first_prompt.is_some()
    }

    /// A prompt reached the agent.
    pub(super) fn prompted(&mut self, shown: &str) {
        if self.first_prompt.is_none() {
            self.first_prompt = Some(shown.to_owned());
        }
    }

    /// `from`, whose session is `session`, leaves for `to` after seeing up
    /// to `seen`. Returns the session `to` should continue, if it took part
    /// before.
    pub(super) fn switch(
        &mut self,
        from: Option<(Agent, String)>,
        seen: Option<EntryId>,
        to: Agent,
    ) -> Option<String> {
        self.handoff = None;
        if !self.started() {
            // Nothing to hand over: the new agent simply starts.
            self.joining = None;
            self.replaying = false;
            return None;
        }
        if let (Some((agent, session)), Some(seen)) = (from, seen)
            && sheprd_chat_protocol::restore::session_id_is_plain(&session)
        {
            self.left.insert(agent, Left { seen, session });
        }
        if !self.held.contains(&to) {
            self.held.push(to);
        }
        match self.left.remove(&to) {
            Some(left) => {
                self.joining = Some(Joining::Resume { seen: left.seen });
                self.replaying = true;
                Some(left.session)
            }
            None => {
                self.joining = Some(Joining::Fresh);
                self.replaying = false;
                None
            }
        }
    }

    pub(super) fn replaying(&self) -> bool {
        self.replaying
    }

    /// The resumed session could not be continued; the agent starts a new
    /// one and needs the whole conversation.
    pub(super) fn resume_failed(&mut self) {
        if self.joining.is_some() {
            self.joining = Some(Joining::Fresh);
        }
        self.replaying = false;
    }

    /// The agent that took over, `agent`, opened its session (`loaded` when
    /// it continued one). Prepares its handoff from `entries`, and returns
    /// how much it carries, for the chat to say.
    pub(super) fn opened(
        &mut self,
        agent: Agent,
        loaded: bool,
        entries: &[Entry],
    ) -> Option<(Joining, Handoff)> {
        self.replaying = false;
        let joining = match self.joining.take()? {
            Joining::Resume { seen } if loaded => Joining::Resume { seen },
            _ => Joining::Fresh,
        };
        let others: Vec<Agent> = self
            .held
            .iter()
            .copied()
            .filter(|held| *held != agent)
            .collect();
        let built = match joining {
            Joining::Fresh => handoff::build(
                entries,
                Scope::Full {
                    first_prompt: self.first_prompt.as_deref(),
                },
                &others,
                handoff::BUDGET,
            ),
            Joining::Resume { seen } => {
                let since = entries.partition_point(|entry| entry.id <= seen);
                handoff::build(&entries[since..], Scope::Since, &others, handoff::BUDGET)
            }
        }?;
        self.handoff = Some(built.text.clone());
        Some((joining, built))
    }

    /// The handoff to send ahead of the next prompt, once.
    pub(super) fn take_handoff(&mut self) -> Option<String> {
        self.handoff.take()
    }

    /// A prompt carrying `text` did not reach the agent: the next one
    /// carries it instead.
    pub(super) fn restore_handoff(&mut self, text: String) {
        self.handoff = Some(text);
    }
}
