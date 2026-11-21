//! What a rename dialog's subject holds that an agent can read or be asked:
//! the chats in its panes, and its other terminals, gathered on the UI
//! thread from what the window already keeps.

use super::{
    Field,
    work::{Reach, Reading, Terminal},
};
use crate::{SheprdWindow, browser::TabId};
use sheprd_chat_protocol::excerpt;

/// Chats read at most; the first is the one asked.
const MAX_CHATS: usize = 4;
/// Terminals read at most.
const MAX_TERMINALS: usize = 6;

/// A chat in the subject.
#[derive(Debug)]
struct Chat {
    /// The window's key for it (`Chats::pane`).
    key: String,
    agent: String,
    excerpt: String,
}

/// What a subject holds, most relevant first: the focused pane's.
#[derive(Debug, Default)]
pub(super) struct Sources {
    chats: Vec<Chat>,
    terminals: Vec<Terminal>,
}

/// The agent [`super::Way::Agent`] asks.
#[derive(Debug)]
pub(super) enum Asked {
    Chat { key: String, agent: String },
    Terminal(Terminal),
}

impl Sources {
    /// What the agent asked is called, when the subject has one: its first
    /// chat, else its first terminal agent.
    pub(super) fn asked_agent(&self) -> Option<&str> {
        self.chats
            .first()
            .map(|chat| chat.agent.as_str())
            .or_else(|| self.terminals.iter().find_map(|t| t.agent.as_deref()))
    }

    pub(super) fn into_asked(self) -> Option<Asked> {
        if let Some(chat) = self.chats.into_iter().next() {
            return Some(Asked::Chat {
                key: chat.key,
                agent: chat.agent,
            });
        }
        self.terminals
            .into_iter()
            .find(|terminal| terminal.agent.is_some())
            .map(Asked::Terminal)
    }

    pub(super) fn into_reading(self) -> Reading {
        Reading {
            chats: self
                .chats
                .into_iter()
                .map(|chat| (chat.agent, chat.excerpt))
                .collect(),
            terminals: self.terminals,
        }
    }

    /// Adds the chat the window keeps under `key`; false when it keeps none.
    fn add_chat(&mut self, window: &SheprdWindow, key: &str) -> bool {
        let Some(pane) = window.chats.pane(key) else {
            return false;
        };
        if self.chats.len() < MAX_CHATS {
            let transcript = pane.session.transcript();
            self.chats.push(Chat {
                key: key.to_owned(),
                agent: transcript.agent.clone(),
                excerpt: excerpt::transcript(transcript),
            });
        }
        true
    }

    fn add_terminal(&mut self, terminal: Terminal) {
        if self.terminals.len() < MAX_TERMINALS {
            self.terminals.push(terminal);
        }
    }
}

impl SheprdWindow {
    /// What the open rename dialog for `field` names holds, if it is still
    /// open on something this window shows.
    pub(super) fn naming_sources(&self, field: Field) -> Option<Sources> {
        match field {
            Field::Tab => {
                let panes = self.tab_menu_panes()?;
                self.herdr_naming_sources(|pane| panes.contains(&pane.pane_id))
            }
            Field::Workspace => {
                let target = self.menu.workspace_target()?;
                let workspace = target.id();
                if target.keeper() == sheprd_session::Keeper::App {
                    let scope = crate::browser::scope(self.endpoints.get(self.selected_endpoint)?);
                    let tabs = self.core_terminals.workspace_tabs(&scope, workspace);
                    return Some(self.core_naming_sources(&tabs));
                }
                self.herdr_naming_sources(|pane| pane.workspace_id == workspace)
            }
            Field::CoreTab => {
                let tab = self.menu.core.as_ref()?.target().tab();
                Some(self.core_naming_sources(&[tab]))
            }
        }
    }

    /// The Herdr 1 panes `wanted` picks from the selected host's snapshot,
    /// the focused one first.
    fn herdr_naming_sources(
        &self,
        wanted: impl Fn(&sheprd_client::protocol::ClientShellPane) -> bool,
    ) -> Option<Sources> {
        let snapshot = self.live.snapshot.as_deref()?;
        let endpoint = self.endpoints.get(self.selected_endpoint)?;
        // A host the `herdr` CLI cannot reach, or a client that cannot run
        // its scripts, still has its chats read.
        let host = crate::pane_move::refusal(&endpoint.connection.target)
            .is_none()
            .then(|| crate::pane_move::host_for(&endpoint.connection.target).ok())
            .flatten();
        let mut panes: Vec<_> = snapshot.panes.iter().filter(|pane| wanted(pane)).collect();
        panes.sort_by_key(|pane| !pane.focused);
        let mut sources = Sources::default();
        for pane in panes {
            if sources.add_chat(self, &pane.pane_id) {
                continue;
            }
            let Some(host) = &host else {
                continue;
            };
            let agent = snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane.pane_id && agent.agent.is_some())
                .map(|agent| display_name(crate::sidebar::agent_name(agent)));
            sources.add_terminal(Terminal {
                agent,
                reach: Reach::Herdr {
                    host: host.clone(),
                    pane: pane.pane_id.clone(),
                },
            });
        }
        Some(sources)
    }

    /// The herdr-core terminals of `tabs`, each read through its own host.
    fn core_naming_sources(&self, tabs: &[TabId]) -> Sources {
        let core = &self.core_terminals;
        let mut sources = Sources::default();
        for &tab in tabs {
            let host = core.host_of(tab);
            let backend = core.host_backend(&host);
            for terminal in core.tab_terminals(tab) {
                let key = crate::chat::core_pane::key_on(&host, terminal);
                if sources.add_chat(self, &key) {
                    continue;
                }
                let Some(backend) = backend else {
                    continue;
                };
                let reported = core
                    .info(tab, terminal)
                    .and_then(sheprd_core_client::agent_name);
                sources.add_terminal(Terminal {
                    agent: reported.as_deref().map(display_name),
                    reach: Reach::Core {
                        backend: backend.clone(),
                        terminal: terminal.clone(),
                        reported,
                    },
                });
            }
        }
        sources
    }
}

/// What a terminal agent is called on a button: a known agent's own name,
/// as its chats show it (`claude` is Claude), else the name it goes by.
pub(super) fn display_name(name: &str) -> String {
    name.parse::<sheprd_chat_protocol::Agent>()
        .map_or_else(|_| name.to_owned(), |agent| agent.display_name().to_owned())
}
