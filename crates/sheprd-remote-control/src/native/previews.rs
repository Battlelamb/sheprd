//! What each terminal agent last said, for its row on a phone. A terminal
//! has no transcript, so its screen is read and its words picked out
//! ([`screen::preview`]). Read only while a phone is connected, never while
//! a home is drawn, which only looks the pane up here: when an agent first
//! shows up, whenever it changes state, and every [`WORKING`] while it
//! works.

mod screen;

use crate::{hub::ApiMethod, web::Shared};
use serde_json::json;
use sheprd_client::protocol::AgentStatus;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

/// Agents followed; more terminal tabs than this go without.
const MAX_AGENTS: usize = 64;
/// Screens read in one pass, so a burst of changes waits its turn.
const MAX_READS: usize = 8;
/// How often a working agent's screen is read again. Longer than a
/// Herdr 2 screen's idle time, so these reads never hold a screen a phone
/// needs to open another terminal.
const WORKING: Duration = Duration::from_secs(10);
/// How often the agents are looked at, to find working ones due a read.
const TICK: Duration = Duration::from_secs(2);
/// How long a burst of layout changes settles into one pass.
const SETTLE: Duration = Duration::from_millis(500);

/// The previews read, by the pane their agent runs in.
pub(crate) struct Previews {
    known: watch::Sender<Arc<HashMap<String, String>>>,
}

impl Default for Previews {
    fn default() -> Self {
        Self {
            known: watch::Sender::new(Arc::default()),
        }
    }
}

impl Previews {
    /// The previews as last read.
    pub(crate) fn current(&self) -> Arc<HashMap<String, String>> {
        self.known.borrow().clone()
    }

    /// Changes whenever a read finds a new preview.
    pub(crate) fn watch(&self) -> watch::Receiver<Arc<HashMap<String, String>>> {
        self.known.subscribe()
    }

    #[cfg(test)]
    pub(crate) fn set_for_tests(&self, known: HashMap<String, String>) {
        self.known.send_replace(Arc::new(known));
    }
}

/// An agent as one pass sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Agent {
    pane: String,
    status: AgentStatus,
    changed_ms: Option<u64>,
}

/// An agent's state at its last read, and when that was.
#[derive(Clone, Copy, Debug)]
struct Read {
    status: AgentStatus,
    changed_ms: Option<u64>,
    at: Instant,
}

/// Whether `agent`'s screen is due a read at `now`, last read as `read`.
fn due(agent: &Agent, read: Option<&Read>, now: Instant) -> bool {
    let Some(read) = read else { return true };
    read.status != agent.status
        || read.changed_ms != agent.changed_ms
        || (agent.status == AgentStatus::Working
            && now.saturating_duration_since(read.at) >= WORKING)
}

/// The agents due a read, at most [`MAX_READS`]: never read first, then
/// the longest unread.
fn plan<'a>(agents: &'a [Agent], reads: &HashMap<String, Read>, now: Instant) -> Vec<&'a Agent> {
    let mut due: Vec<&Agent> = agents
        .iter()
        .filter(|agent| due(agent, reads.get(&agent.pane), now))
        .collect();
    due.sort_by_key(|agent| reads.get(&agent.pane).map(|read| read.at));
    due.truncate(MAX_READS);
    due
}

/// The agent each terminal tab lists, the one its row shows.
fn agents(state: &Shared) -> Vec<Agent> {
    state
        .hub
        .layout()
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.terminals)
        .filter_map(|terminal| terminal.agent.as_ref())
        .take(MAX_AGENTS)
        .map(|agent| Agent {
            pane: agent.pane.clone(),
            status: agent.status,
            changed_ms: agent.changed_ms,
        })
        .collect()
}

/// The preview on `pane`'s screen now.
async fn read(state: &Shared, pane: &str) -> crate::hub::Result<Option<String>> {
    let params = json!({"pane_id": pane, "source": "visible", "format": "ansi"});
    let answer = state.hub.call(ApiMethod::PaneRead, params).await?;
    Ok(screen::preview(
        answer["read"]["text"].as_str().unwrap_or_default(),
    ))
}

/// Reads the agents' screens while a phone is connected, when the layout
/// changes and every [`TICK`].
pub(crate) async fn follow(state: Shared) {
    let mut layout = state.hub.watch_layout();
    let mut reads: HashMap<String, Read> = HashMap::new();
    loop {
        if !state.connected().is_empty() {
            let agents = agents(&state);
            reads.retain(|pane, _| agents.iter().any(|agent| &agent.pane == pane));
            let mut found = state.previews.current().as_ref().clone();
            found.retain(|pane, _| reads.contains_key(pane));
            for agent in plan(&agents, &reads, Instant::now()) {
                // A failed read keeps the last preview; the next change or
                // tick tries again.
                match read(&state, &agent.pane).await {
                    Ok(Some(preview)) => {
                        found.insert(agent.pane.clone(), preview);
                    }
                    Ok(None) => {
                        found.remove(&agent.pane);
                    }
                    Err(error) => {
                        tracing::debug!(%error, "Could not read a terminal agent's screen");
                    }
                }
                let read = Read {
                    status: agent.status,
                    changed_ms: agent.changed_ms,
                    at: Instant::now(),
                };
                reads.insert(agent.pane.clone(), read);
            }
            state.previews.known.send_if_modified(|known| {
                if **known == found {
                    return false;
                }
                *known = Arc::new(found);
                true
            });
        }
        tokio::select! {
            changed = layout.changed() => if changed.is_err() { return },
            () = tokio::time::sleep(TICK) => {}
        }
        tokio::time::sleep(SETTLE).await;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
