//! A terminal agent's conversation, shown to phones as a chat. A Claude
//! Code CLI running in a Herdr pane keeps its own transcript; the same
//! `sheprd-acp observe` Sheprd's `render_existing` uses projects it, read
//! only, while the terminal keeps all input. An observer starts when a
//! phone first opens the chat and ends with the agent, when the pane's
//! agent or session changes, or when newer ones push it out.
//!
//! The pane's session comes from Herdr (`agent_session`), the process from
//! its process info, and the process's birth is read here, on the host the
//! pane runs on, so the observer can refuse a PID reused since.

mod error;

pub use error::Error;

use super::{ApiMethod, Chat, Hub, core};
use serde::Deserialize;
use serde_json::{Value, json};
use sheprd_chat_protocol::Agent;
use sheprd_client::herdr::{
    processes::{agent_pid, local_identity},
    snapshot::{Pane, ProcessInfo, SessionKind},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

/// Observers kept at once. Past it the least recently opened one that no
/// phone follows ends; one every phone still follows is never ended.
const MAX_OBSERVED: usize = 8;

/// The observers running now, by pane, with what each was started for.
#[derive(Default)]
pub(crate) struct Observed(Mutex<Vec<Running>>);

struct Running {
    binding: Binding,
    chat: Arc<Chat>,
}

/// What one observer reads: the pane's agent process and its session.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    pane: String,
    session: String,
    pid: u32,
    started: u64,
}

impl Observed {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Running>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The running observer for `binding`, ending one for the same pane
    /// that reads anything else.
    fn find(&self, binding: &Binding) -> Option<Arc<Chat>> {
        let mut running = self.lock();
        running.retain(|kept| {
            let stale = kept.chat.is_closed()
                || (kept.binding.pane == binding.pane && kept.binding != *binding);
            if stale {
                kept.chat.close();
            }
            !stale
        });
        running
            .iter()
            .find(|kept| kept.binding == *binding)
            .map(|kept| kept.chat.clone())
    }

    fn keep(&self, binding: Binding, chat: Arc<Chat>) {
        let mut running = self.lock();
        running.push(Running { binding, chat });
        while running.len() > MAX_OBSERVED {
            let Some(oldest) = running.iter().position(|kept| kept.chat.followers() == 0) else {
                break;
            };
            running.remove(oldest).chat.close();
        }
    }

    /// Ends the observers of panes that no longer show an agent that can be
    /// observed.
    pub(crate) fn retain(&self, shown: impl Fn(&str) -> bool) {
        self.lock().retain(|kept| {
            let keep = shown(&kept.binding.pane) && !kept.chat.is_closed();
            if !keep {
                kept.chat.close();
            }
            keep
        });
    }
}

/// Whether the agent named `name` in `pane` has a conversation phones can
/// read as a chat: Claude Code, which keeps its transcript under its own
/// name, in a Herdr 1 pane. Codex also needs its arguments checked against
/// the session, which this does not yet do, and a Herdr 2 terminal reports
/// its processes another way. The same on every host, so a phone sees one
/// answer; [`Hub::observe`] refuses where processes cannot be read.
pub(crate) fn observable(pane: &str, name: Option<&str>) -> bool {
    name.and_then(|name| name.parse().ok()) == Some(Agent::Claude)
        && !core::owns(&json!({"pane_id": pane}))
}

#[derive(Deserialize)]
struct PaneAnswer {
    pane: Pane,
}

#[derive(Deserialize)]
struct ProcessAnswer {
    process_info: ProcessInfo,
}

fn parse<T: for<'de> Deserialize<'de>>(answer: Value) -> Result<T> {
    serde_json::from_value(answer).map_err(Error::Answer)
}

impl Hub {
    /// Whether `pane` shows a terminal agent whose conversation can be
    /// observed, as the layout has it now.
    pub(crate) fn observable(&self, pane: &str) -> bool {
        self.layout().terminal(pane).is_some_and(|(_, terminal)| {
            terminal
                .agents
                .iter()
                .any(|agent| agent.pane == pane && observable(pane, agent.name.as_deref()))
        })
    }

    /// The chat observing `pane`'s terminal agent, started when none reads
    /// its current conversation yet.
    pub(crate) async fn observe(&self, pane: &str) -> super::Result<Arc<Chat>> {
        if !cfg!(any(target_os = "linux", target_os = "macos")) || !self.observable(pane) {
            return Err(Error::Unsupported.into());
        }
        let program = self.program().ok_or(super::Error::ProgramMissing)?;
        let binding = self.binding(pane).await?;
        if let Some(chat) = self.observed.find(&binding) {
            return Ok(chat);
        }
        let mut command = sheprd_client::new_command(program);
        command.args([
            "observe".to_owned(),
            Agent::Claude.name().to_owned(),
            binding.session.clone(),
            binding.pid.to_string(),
            binding.started.to_string(),
            "--pane".to_owned(),
            pane.to_owned(),
        ]);
        let chat = Chat::observe(pane.to_owned(), command).map_err(Error::Start)?;
        self.observed.keep(binding, chat.clone());
        Ok(chat)
    }

    /// Asks Herdr which session and process `pane`'s agent runs, then
    /// reads that process's birth here.
    async fn binding(&self, pane: &str) -> super::Result<Binding> {
        let params = json!({"pane_id": pane});
        let found: PaneAnswer = parse(self.call(ApiMethod::PaneGet, params.clone()).await?)?;
        let session = found
            .pane
            .agent_session
            .filter(|session| {
                session.kind == SessionKind::Id
                    && session.agent == Agent::Claude.name()
                    && found.pane.agent.as_deref() == Some(Agent::Claude.name())
                    && uuid::Uuid::parse_str(&session.value).is_ok()
            })
            .ok_or(Error::Session)?;
        let info: ProcessAnswer = parse(self.call(ApiMethod::PaneProcessInfo, params).await?)?;
        let identities: HashMap<u32, _> = info
            .process_info
            .foreground_processes
            .iter()
            .filter_map(|process| Some((process.pid, local_identity(process.pid)?)))
            .collect();
        let pid = agent_pid(Agent::Claude, &info.process_info, |pid| {
            identities
                .get(&pid)
                .copied()
                .or_else(|| local_identity(pid))
        })
        .ok_or(Error::Process)?;
        let started = identities.get(&pid).ok_or(Error::Process)?.started;
        Ok(Binding {
            pane: pane.to_owned(),
            session: session.value,
            pid,
            started,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
