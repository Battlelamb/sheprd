//! Bridge between `SheprdWindow` and `InteractionStore`: ingests newly arrived
//! chat interactions, focuses source chat panes, sends user replies from modal
//! choices/runbooks/diffs, and locates the current workspace root for the vault.

use super::{
    EscalationLevel, InteractionStore, ParsedInteraction, Presence, escalate, sync_hud_deferred,
};
use crate::{
    NavigationTarget, SheprdWindow,
    chat::prepared::{Body, Part},
};
use gpui::{Context, Window};
use sheprd_chat_protocol::Command;
use sheprd_control_protocol::protocol::modal::CueKind;
use std::{path::PathBuf, time::Instant};

impl SheprdWindow {
    /// Syncs newly arrived interaction directives & cues from every live chat
    /// pane in this window into [`InteractionStore`].
    pub(crate) fn sync_chat_interactions(&mut self, window: &Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let win_id = window.window_handle().window_id();
        let volume = self.config.sound.volume;
        let window_modals = self.config.chat.agent_modals;
        let global_modals = self.config.chat.global_modals;
        let progress_pill = self.config.chat.progress_pill;

        let mut pending = Vec::new();
        for pane in self.chats.panes() {
            let pane_id = pane.target.pane_id.clone();
            let agent = pane.session.transcript().agent.clone();
            let workspace = self
                .live
                .snapshot
                .as_deref()
                .and_then(|s| {
                    s.panes
                        .iter()
                        .find(|p| p.pane_id == pane_id)
                        .and_then(|p| {
                            s.workspaces
                                .iter()
                                .find(|w| w.workspace_id == p.workspace_id)
                        })
                        .map(|w| w.label.clone())
                })
                .unwrap_or_else(|| "workspace".to_owned());
            let keys = pane.session.keys();
            for (entry_id, revision) in keys {
                let Some(body) = pane.prepared.body(entry_id) else {
                    continue;
                };
                let (interactions, cues) = extract_interactions(&body);
                pending.push((
                    pane_id.clone(),
                    agent.clone(),
                    workspace.clone(),
                    entry_id,
                    revision,
                    interactions,
                    cues,
                ));
            }
        }
        if pending.is_empty() {
            return;
        }
        let store = InteractionStore::global(cx);
        let focused_pane = self
            .live
            .snapshot
            .as_deref()
            .and_then(|s| s.focused_pane_id.clone());
        let mut any_changed = false;
        store.update(cx, |s, cx| {
            for (pane_id, agent, workspace, entry_id, revision, mut interactions, cues) in pending {
                let presence = if self.active && focused_pane.as_deref() == Some(pane_id.as_str()) {
                    Presence::InChat
                } else if self.active {
                    Presence::SameWindowOtherTab
                } else {
                    Presence::OtherApp
                };
                // Route each interaction through the Smart Escalation Ladder
                // based on whether the user is in this chat, another tab, or another app.
                let mut routed = Vec::with_capacity(interactions.len());
                for mut item in interactions.drain(..) {
                    if !matches!(item.payload, super::Payload::Progress { .. }) {
                        let level = escalate(item.scope, presence, 0, global_modals);
                        match level {
                            EscalationLevel::InlineOnly
                                if item.scope == sheprd_control_protocol::protocol::modal::ModalScope::Window =>
                            {
                                continue;
                            }
                            EscalationLevel::GlobalHud | EscalationLevel::PhoneEscalation => {
                                item.scope =
                                    sheprd_control_protocol::protocol::modal::ModalScope::All;
                            }
                            EscalationLevel::InlineOnly | EscalationLevel::WindowModal => {}
                        }
                    }
                    routed.push(item);
                }
                // Only treat the first pass of an unseen pane as history if it
                // already holds multiple entries; otherwise process live.
                let is_initial_history = !s.seen_entries.contains_key(&pane_id) && entry_id.0 == 0;
                any_changed |= s.sync_chat_entry(
                    win_id,
                    &pane_id,
                    &agent,
                    &workspace,
                    entry_id,
                    revision,
                    is_initial_history,
                    &routed,
                    &cues,
                    now,
                    volume,
                    window_modals,
                    global_modals,
                    progress_pill,
                );
            }
            if any_changed {
                sync_hud_deferred(cx);
                cx.notify();
            }
        });
        if any_changed {
            cx.notify();
        }
    }

    /// Focuses the chat pane that originated an interaction.
    pub(crate) fn focus_chat_pane_by_id(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        let target = NavigationTarget::Pane(pane_id);
        let _ = self.navigate(target, cx);
    }

    /// Sends a user response (choice click, runbook completion, diff approval,
    /// or redacted vault receipt) back into the originating chat pane.
    pub(crate) fn send_interaction_reply(
        &mut self,
        pane_id: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = self.chats.pane_mut(pane_id) {
            let origin = Some(pane.target.origin());
            let _ = pane.send(Command::Prompt {
                attachments: Vec::new(),
                origin,
                text: text.to_owned(),
            });
            cx.notify();
        }
    }

    /// Best-effort root directory of the active workspace, used to confine
    /// secret-safe `.env` vault writes.
    pub(crate) fn selected_workspace_path(&self) -> Option<PathBuf> {
        let snapshot = self.live.snapshot.as_deref()?;
        let focused_ws = snapshot.focused_workspace_id.as_deref()?;
        snapshot
            .workspaces
            .iter()
            .find(|w| w.workspace_id == focused_ws)
            .map(|w| PathBuf::from(&w.new_workspace_cwd))
    }
}

fn extract_interactions(body: &Body) -> (Vec<ParsedInteraction>, Vec<CueKind>) {
    let Body::Message(parts) = body else {
        return (Vec::new(), Vec::new());
    };
    let mut interactions = Vec::new();
    let mut cues = Vec::new();
    for part in parts {
        match part {
            Part::Interaction(parsed) => interactions.push((**parsed).clone()),
            Part::Cue(cue) => cues.push(*cue),
            _ => {}
        }
    }
    (interactions, cues)
}
