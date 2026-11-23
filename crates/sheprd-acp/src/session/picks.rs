//! The model and thinking the person last picked for each built-in agent,
//! on this host ([`crate::known::picked`]): a chat that starts or switches
//! to an agent without settings of its own shows them at once and sets
//! them once the agent's session opens, so switching from Claude to Codex
//! brings back the Codex settings used last rather than Codex's defaults.
//!
//! Only model and thinking settings are kept. The mode and auto-accepting
//! decide what an agent may do unasked, so they follow a chat only when
//! `sheprd chat new` copies them, never from another chat's choice.

use std::path::PathBuf;

use sheprd_chat_protocol::{Change, OptionCategory, OptionKind, OptionValue, Setting, Settings};

use super::{Session, Update};

/// The kinds of setting remembered across chats.
const REMEMBERED: [OptionCategory; 3] = [
    OptionCategory::Model,
    OptionCategory::ModelConfig,
    OptionCategory::ThoughtLevel,
];

#[derive(Debug, Default)]
pub(super) struct Picks {
    /// Where the picks are kept; `None` keeps none.
    dir: Option<PathBuf>,
    /// The picks applied once the agent starting now opens its session.
    pending: Option<Settings>,
}

impl<R: FnMut(Update)> Session<R> {
    /// Keeps the person's picks in `dir`, and applies those of the agent
    /// the chat starts with, unless the chat continues a session or was
    /// given settings of its own.
    pub(crate) fn keep_picks(&mut self, dir: PathBuf) {
        self.picks.dir = Some(dir);
        if self.config.load.is_none() && self.config.settings.is_none() {
            self.recall_picks();
        }
    }

    /// Reads the picks of the agent starting now, read again on every
    /// switch so a pick made in another chat since counts, and shows them
    /// while it starts.
    pub(super) fn recall_picks(&mut self) {
        self.picks.pending = None;
        let (Some(dir), Some(agent)) = (&self.picks.dir, self.config.harness) else {
            return;
        };
        let Some(picked) = crate::known::picked(dir, agent) else {
            return;
        };
        let mut options = self.transcript.options.clone();
        for Setting { id, value } in &picked.options {
            let Some(option) = options
                .iter_mut()
                .find(|option| option.id == *id && option.accepts(value))
            else {
                continue;
            };
            match (&mut option.kind, value) {
                (OptionKind::Select { current, .. }, OptionValue::Choice { id }) => {
                    current.clone_from(id);
                }
                (OptionKind::Toggle { current }, OptionValue::Toggle { value }) => {
                    *current = *value;
                }
                _ => {}
            }
        }
        if options != self.transcript.options {
            self.emit(Change::Options { options });
        }
        self.picks.pending = Some(picked);
    }

    /// The session opened: the picks are carried into it unless the chat
    /// already carries settings, which are newer.
    pub(super) fn carry_picks(&mut self) {
        let picked = self.picks.pending.take();
        if self.config.settings.is_none() {
            self.config.settings = picked;
        }
    }

    /// The agent took a setting: what it runs with now is the agent's
    /// picks for the next chat, kept when they changed.
    pub(super) fn remember_picks(&mut self) {
        let (Some(dir), Some(agent)) = (&self.picks.dir, self.config.harness) else {
            return;
        };
        let mut picked = Settings::of(agent, &self.transcript);
        picked.auto_accept = false;
        picked.mode = None;
        let remembered = |id: &str| {
            self.transcript
                .options
                .iter()
                .any(|option| option.id == id && REMEMBERED.contains(&option.category))
        };
        picked.options.retain(|setting| remembered(&setting.id));
        if picked.options.is_empty() {
            return;
        }
        if crate::known::picked(dir, agent).as_ref() == Some(&picked) {
            return;
        }
        if let Err(error) = crate::known::save_picked(dir, agent, &picked) {
            eprintln!(
                "sheprd-acp: could not keep {}'s picked settings: {error:#}",
                agent.name()
            );
        }
    }
}
