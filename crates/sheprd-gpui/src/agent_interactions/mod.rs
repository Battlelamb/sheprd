//! Persistent agent interactions: window modals, global always-on-top HUD,
//! dynamic progress pill, secret-safe vault, shared runbooks, drop-zones,
//! spotlights, mini-diffs, smart escalation, and semantic cues.

pub(crate) mod audio;
pub(crate) mod error;
pub(crate) mod hud_window;
pub(crate) mod parse;
pub(crate) mod render_window;
pub(crate) mod vault;
mod window_bridge;

#[cfg(test)]
mod tests;

pub(crate) use parse::{BlockDirective, ParsedInteraction, Payload};

use gpui::{App, AppContext, Context, Entity, Global, WindowId};
use sheprd_chat_protocol::EntryId;
use sheprd_control_protocol::protocol::modal::{CueKind, ModalScope, ModalSummary, RunbookStep};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

/// Maximum concurrent active interactions retained across all chats/windows.
pub(crate) const MAX_ACTIVE: usize = 16;
/// How long an explicit Edge Light cue override pulses.
const CUE_DURATION: Duration = Duration::from_secs(4);
/// Duration of a green "Done ✓" flash before auto-clearing a progress pill.
const DONE_FLASH: Duration = Duration::from_secs(3);

/// Where the user currently is, used by the Smart Escalation Ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    /// Looking directly at the chat pane that emitted this interaction.
    InChat,
    /// Active inside the same Sheprd window, on a different tab/workspace.
    SameWindowOtherTab,
    /// Sheprd is not frontmost (or user is on another Space/app).
    OtherApp,
    /// User has been idle for >= `idle_secs` seconds (escalate to paired phone).
    #[allow(dead_code)]
    Away { idle_secs: u64 },
}

/// Surface selected by the Smart Escalation Ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EscalationLevel {
    /// Quiet inline chip in the active chat only.
    InlineOnly,
    /// Persistent modal/sheet over the chat's Sheprd window.
    WindowModal,
    /// Always-on-top global floating HUD across all apps & Spaces + Dock bounce.
    GlobalHud,
    /// Global HUD + paired iPhone Live Activity / time-sensitive push.
    PhoneEscalation,
}

/// Pure routing function for the Smart Escalation Ladder.
pub(crate) fn escalate(
    scope: ModalScope,
    presence: Presence,
    unread_secs: u64,
    global_enabled: bool,
) -> EscalationLevel {
    match presence {
        Presence::InChat => {
            if scope == ModalScope::All {
                EscalationLevel::InlineOnly
            } else {
                EscalationLevel::InlineOnly
            }
        }
        Presence::SameWindowOtherTab => EscalationLevel::WindowModal,
        Presence::OtherApp => {
            if global_enabled && (scope == ModalScope::All || unread_secs >= 10) {
                EscalationLevel::GlobalHud
            } else {
                EscalationLevel::WindowModal
            }
        }
        Presence::Away { idle_secs } => {
            if global_enabled && idle_secs >= 20 {
                EscalationLevel::PhoneEscalation
            } else if global_enabled && scope == ModalScope::All {
                EscalationLevel::GlobalHud
            } else {
                EscalationLevel::WindowModal
            }
        }
    }
}

/// Origin of an active interaction: a chat pane or an explicit CLI call.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SourceKey {
    pub(crate) window_id: Option<WindowId>,
    pub(crate) pane_id: Option<String>,
}

/// One live persistent interaction tracked by [`InteractionStore`].
#[derive(Clone, Debug)]
pub(crate) struct ActiveInteraction {
    pub(crate) id: String,
    pub(crate) source: SourceKey,
    pub(crate) agent_label: String,
    pub(crate) workspace_label: String,
    pub(crate) entry_id: Option<EntryId>,
    pub(crate) parsed: ParsedInteraction,
    pub(crate) created_at: Instant,
    pub(crate) snoozed_until: Option<Instant>,
    pub(crate) expires_at: Option<Instant>,
    /// Transient status line shown after a user clicks an action or vault submit.
    pub(crate) feedback: Option<String>,
}

impl ActiveInteraction {
    pub(crate) fn is_progress(&self) -> bool {
        matches!(self.parsed.payload, Payload::Progress { .. })
    }

    pub(crate) fn is_visible(&self, now: Instant) -> bool {
        if self.expires_at.is_some_and(|exp| now >= exp) {
            return false;
        }
        self.snoozed_until.is_none_or(|until| now >= until)
    }

    pub(crate) fn summary(&self) -> ModalSummary {
        let percent = match &self.parsed.payload {
            Payload::Progress { percent, .. } => *percent,
            _ => None,
        };
        ModalSummary {
            id: self.id.clone(),
            kind: self.parsed.directive.kind_name().to_owned(),
            scope: self.parsed.scope,
            title: self.parsed.title.clone(),
            source: Some(format!("{} · {}", self.agent_label, self.workspace_label)),
            percent,
        }
    }
}

/// Temporary Edge Light cue override active on windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EdgeCue {
    pub(crate) cue: CueKind,
    pub(crate) until: Instant,
}

/// App-global store of all active persistent modals, progress pills, runbooks,
/// spotlights, and semantic cues across windows and the floating global HUD.
#[derive(Default)]
pub(crate) struct InteractionStore {
    next_id: u64,
    items: Vec<ActiveInteraction>,
    /// Chat entries already ingested so history loads and re-renders never
    /// re-open dismissed modals.
    seen_entries: HashMap<String, HashSet<(EntryId, u64)>>,
    /// Currently selected card index inside the stacked deck.
    pub(crate) deck_index: usize,
    /// Whether the global Dynamic Progress Pill is expanded on hover/click.
    pub(crate) pill_expanded: bool,
    /// Last explicit audio/Edge Light cue fired.
    edge_cue: Option<EdgeCue>,
    /// Handle to the floating always-on-top HUD window, if open.
    pub(crate) hud_open: bool,
}

/// Global GPUI wrapper holding the shared `Entity<InteractionStore>`.
pub(crate) struct GlobalInteractions(pub(crate) Entity<InteractionStore>);

impl Global for GlobalInteractions {}

impl InteractionStore {
    /// Returns the shared app-wide store, initializing it on first use.
    pub(crate) fn global(cx: &mut App) -> Entity<Self> {
        if let Some(existing) = cx.try_global::<GlobalInteractions>() {
            return existing.0.clone();
        }
        let entity = cx.new(|_| Self::default());
        cx.set_global(GlobalInteractions(entity.clone()));
        entity
    }

    /// Reads the shared store if initialized.
    pub(crate) fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalInteractions>().map(|g| g.0.clone())
    }

    /// Inserts or replaces the active interaction of the same category (modal
    /// vs progress pill) for `source`, returning its assigned ID.
    pub(crate) fn upsert(
        &mut self,
        source: SourceKey,
        agent_label: String,
        workspace_label: String,
        entry_id: Option<EntryId>,
        parsed: ParsedInteraction,
        now: Instant,
        volume: u8,
    ) -> String {
        self.prune_expired(now);
        let is_progress = matches!(parsed.payload, Payload::Progress { .. });
        let done_flash = matches!(parsed.payload, Payload::Progress { done: true, .. });
        // At most one modal and one progress pill per source pane so agents
        // can update in place without flooding the user's screen.
        if source.pane_id.is_some() {
            self.items.retain(|item| {
                !(item.source.pane_id == source.pane_id && item.is_progress() == is_progress)
            });
        }
        if let Some(cue) = parsed.cue {
            self.trigger_cue(cue, now, volume);
        }
        self.next_id += 1;
        let id = format!("m{}", self.next_id);
        let expires_at = if done_flash {
            Some(now + DONE_FLASH)
        } else {
            None
        };
        self.items.push(ActiveInteraction {
            id: id.clone(),
            source,
            agent_label,
            workspace_label,
            entry_id,
            parsed,
            created_at: now,
            snoozed_until: None,
            expires_at,
            feedback: None,
        });
        while self.items.len() > MAX_ACTIVE {
            self.items.remove(0);
        }
        if self.deck_index >= self.items.len() {
            self.deck_index = self.items.len().saturating_sub(1);
        }
        id
    }

    /// Records a semantic cue (audio + Edge Light override).
    pub(crate) fn trigger_cue(&mut self, cue: CueKind, now: Instant, volume: u8) {
        self.edge_cue = Some(EdgeCue {
            cue,
            until: now + CUE_DURATION,
        });
        audio::play_async(cue, volume);
    }

    /// Active Edge Light cue override at `now`, if any.
    pub(crate) fn active_edge_cue(&self, now: Instant) -> Option<CueKind> {
        self.edge_cue
            .filter(|edge| now < edge.until)
            .map(|edge| edge.cue)
    }

    /// Ingests newly arrived chat interactions from `pane_id`. Historical
    /// transcript loads are recorded as seen without popping modals; a newer
    /// agent entry auto-clears prior `auto_clear` modals for that pane.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sync_chat_entry(
        &mut self,
        window_id: WindowId,
        pane_id: &str,
        agent_label: &str,
        workspace_label: &str,
        entry_id: EntryId,
        revision: u64,
        history: bool,
        interactions: &[ParsedInteraction],
        cues: &[CueKind],
        now: Instant,
        volume: u8,
        window_modals_enabled: bool,
        global_modals_enabled: bool,
        progress_pill_enabled: bool,
    ) -> bool {
        let seen = self.seen_entries.entry(pane_id.to_owned()).or_default();
        if history {
            seen.insert((entry_id, revision));
            return false;
        }
        if !seen.insert((entry_id, revision)) {
            return false;
        }
        // A newer entry from the same chat auto-clears earlier auto_clear
        // modals (e.g., git push succeeded after YubiKey touch).
        let before = self.items.len();
        self.items.retain(|item| {
            !(item.source.pane_id.as_deref() == Some(pane_id)
                && item.parsed.auto_clear
                && item.entry_id.is_some_and(|prev| prev < entry_id))
        });
        let mut changed = self.items.len() != before;

        for &cue in cues {
            self.trigger_cue(cue, now, volume);
            changed = true;
        }
        for parsed in interactions {
            let is_progress = matches!(parsed.payload, Payload::Progress { .. });
            if is_progress && !progress_pill_enabled {
                continue;
            }
            if !is_progress && parsed.scope == ModalScope::Window && !window_modals_enabled {
                continue;
            }
            let mut parsed = parsed.clone();
            if !is_progress && parsed.scope == ModalScope::All && !global_modals_enabled {
                if !window_modals_enabled {
                    continue;
                }
                parsed.scope = ModalScope::Window;
            }
            self.upsert(
                SourceKey {
                    window_id: Some(window_id),
                    pane_id: Some(pane_id.to_owned()),
                },
                agent_label.to_owned(),
                workspace_label.to_owned(),
                Some(entry_id),
                parsed,
                now,
                volume,
            );
            changed = true;
        }
        changed
    }

    pub(crate) fn dismiss(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != id);
        if self.deck_index >= self.items.len() {
            self.deck_index = self.items.len().saturating_sub(1);
        }
        self.items.len() != before
    }

    pub(crate) fn snooze(&mut self, id: &str, duration: Duration, now: Instant) -> bool {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.snoozed_until = Some(now + duration);
            return true;
        }
        false
    }

    pub(crate) fn toggle_runbook_step(&mut self, id: &str, index: usize) -> Option<RunbookStep> {
        let item = self.items.iter_mut().find(|item| item.id == id)?;
        let Payload::Runbook { steps } = &mut item.parsed.payload else {
            return None;
        };
        let step = steps.get_mut(index)?;
        step.done = !step.done;
        Some(step.clone())
    }

    pub(crate) fn set_feedback(&mut self, id: &str, text: String) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.feedback = Some(text);
        }
    }

    pub(crate) fn clear_for_pane(&mut self, pane_id: &str) -> usize {
        let before = self.items.len();
        self.items
            .retain(|item| item.source.pane_id.as_deref() != Some(pane_id));
        before - self.items.len()
    }

    pub(crate) fn clear_all(&mut self) -> usize {
        let count = self.items.len();
        self.items.clear();
        self.deck_index = 0;
        count
    }

    pub(crate) fn prune_expired(&mut self, now: Instant) {
        self.items
            .retain(|item| item.expires_at.is_none_or(|exp| now < exp));
    }

    /// Visible items belonging to `window_id` (plus mirrored global items).
    pub(crate) fn for_window(&self, window_id: WindowId, now: Instant) -> Vec<&ActiveInteraction> {
        self.items
            .iter()
            .filter(|item| {
                item.is_visible(now)
                    && !item.is_progress()
                    && (item.parsed.scope == ModalScope::All
                        || item.source.window_id == Some(window_id)
                        || item.source.window_id.is_none())
            })
            .collect()
    }

    /// Visible global items (`ModalScope::All`) for the always-on-top HUD.
    pub(crate) fn global_hud_items(&self, now: Instant) -> Vec<&ActiveInteraction> {
        self.items
            .iter()
            .filter(|item| item.is_visible(now) && item.parsed.scope == ModalScope::All)
            .collect()
    }

    /// Visible progress pills across all chats.
    pub(crate) fn progress_items(&self, now: Instant) -> Vec<&ActiveInteraction> {
        self.items
            .iter()
            .filter(|item| item.is_visible(now) && item.is_progress())
            .collect()
    }

    pub(crate) fn summaries(&self, now: Instant) -> Vec<ModalSummary> {
        self.items
            .iter()
            .filter(|item| item.is_visible(now))
            .map(ActiveInteraction::summary)
            .collect()
    }

    pub(crate) fn has_active_for_entry(&self, pane_id: &str, entry_id: EntryId) -> bool {
        self.items.iter().any(|item| {
            item.source.pane_id.as_deref() == Some(pane_id) && item.entry_id == Some(entry_id)
        })
    }

    pub(crate) fn step_deck(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.deck_index = 0;
            return;
        }
        let next = (self.deck_index as isize + delta).rem_euclid(len as isize);
        self.deck_index = next as usize;
    }

    /// Ensures the floating global HUD window is open whenever any global
    /// modal or progress pill is active.
    pub(crate) fn ensure_hud(store: &Entity<Self>, cx: &mut App) {
        let now = Instant::now();
        let needs_hud = {
            let s = store.read(cx);
            !s.hud_open && !s.global_hud_items(now).is_empty()
        };
        if needs_hud {
            hud_window::open(store.clone(), cx);
        }
    }
}

/// Helper used by `Context<InteractionStore>` callers to notify observers and
/// bring up the global HUD when needed.
pub(crate) fn sync_hud_deferred(cx: &mut Context<InteractionStore>) {
    let entity = cx.entity();
    cx.defer(move |cx| {
        InteractionStore::ensure_hud(&entity, cx);
    });
}
