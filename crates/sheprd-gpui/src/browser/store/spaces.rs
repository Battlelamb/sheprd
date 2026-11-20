//! The store's side of a core's shared document (`sheprd_session::spaces`):
//! what one scope's kept workspaces and herdr-core tabs look like as the
//! document holds them, and taking a document's in. What only one window
//! shows stays in the store alone: which pane is in use, mirror tabs, and
//! pages.

use super::{Location, MAX_TABS, Place, Scope, Store, Tab, TabId, Zoom};
use crate::core_terminals::panes::Panes;
use serde::{Deserialize, Serialize};
use sheprd_backend::TerminalId;
use sheprd_session::{
    SavedWorkspace,
    spaces::{self, Adopted},
};
use std::collections::{BTreeSet, HashSet};

/// One scope's kept workspaces and shared tabs, as a core's document holds
/// them, in the user's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct View {
    pub(crate) tabs: Vec<spaces::Tab>,
    pub(crate) workspaces: Vec<spaces::Workspace>,
}

impl View {
    pub(crate) fn of(spaces: &spaces::Spaces) -> Self {
        Self {
            tabs: spaces.tabs().to_vec(),
            workspaces: spaces.workspaces().to_vec(),
        }
    }

    pub(crate) fn workspace(&self, id: &str) -> Option<&spaces::Workspace> {
        self.workspaces.iter().find(|workspace| workspace.id == id)
    }

    pub(crate) fn tab(&self, id: &str) -> Option<&spaces::Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// The ids it holds, as the store keeps them across restarts.
    pub(crate) fn synced(&self) -> Synced {
        Synced {
            tabs: self.tabs.iter().map(|tab| tab.id.clone()).collect(),
            workspaces: (self.workspaces.iter())
                .map(|workspace| workspace.id.clone())
                .collect(),
        }
    }

    /// The view less what `document` does not hold: what the store took of
    /// it, without records only this app has made so far.
    pub(crate) fn within(mut self, document: &Self) -> Self {
        self.workspaces
            .retain(|workspace| document.workspace(&workspace.id).is_some());
        self.tabs.retain(|tab| document.tab(&tab.id).is_some());
        self
    }
}

/// The ids a core's document held when last read, kept with the tabs so a
/// record that went from the document while the app was closed is told
/// from one the app made and has not written yet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Synced {
    #[serde(default)]
    pub(crate) tabs: BTreeSet<String>,
    #[serde(default)]
    pub(crate) workspaces: BTreeSet<String>,
}

impl Store {
    /// `scope`'s kept workspaces and the herdr-core tabs that type into
    /// their terminals, as the document holds them. A tab with no document
    /// id yet is left out ([`Self::name_shared_tabs`]).
    pub(crate) fn shared_view(&self, scope: &Scope) -> View {
        let workspaces: Vec<spaces::Workspace> = self
            .kept_workspaces(scope)
            .map(|workspace| spaces::Workspace {
                cwd: workspace.cwd.clone(),
                custom: workspace.custom,
                id: workspace.id.clone(),
                label: workspace.label.clone(),
            })
            .collect();
        let tabs = self
            .in_scope(scope)
            .filter(|tab| workspaces.iter().any(|kept| kept.id == tab.workspace_id))
            .filter_map(shared_tab)
            .collect();
        View { tabs, workspaces }
    }

    /// Whether a herdr-core tab of `scope`'s kept workspaces has no
    /// document id yet.
    pub(crate) fn unnamed_shared_tabs(&self, scope: &Scope) -> bool {
        self.in_scope(scope).any(|tab| {
            tab.space.is_none()
                && drives(tab)
                && self.kept_workspace(scope, &tab.workspace_id).is_some()
        })
    }

    /// Gives each herdr-core tab of `scope`'s kept workspaces that types
    /// into its terminals a document id of its own, from `new_id`. Whether
    /// any took one.
    pub(crate) fn name_shared_tabs(
        &mut self,
        scope: &Scope,
        mut new_id: impl FnMut() -> String,
    ) -> bool {
        let kept: HashSet<String> = self
            .kept_workspaces(scope)
            .map(|workspace| workspace.id.clone())
            .collect();
        let mut named = false;
        for tab in &mut self.tabs {
            if tab.space.is_none()
                && &tab.scope == scope
                && kept.contains(&tab.workspace_id)
                && drives(tab)
            {
                tab.space = Some(new_id());
                named = true;
            }
        }
        if named {
            self.save();
        }
        named
    }

    /// What `scope`'s document held when last read; empty when the app
    /// never read it.
    pub(crate) fn synced(&self, scope: &Scope) -> Option<&Synced> {
        self.synced.get(scope.as_str())
    }

    pub(crate) fn set_synced(&mut self, scope: &Scope, synced: Option<Synced>) {
        let key = scope.as_str();
        if self.synced.get(key) == synced.as_ref() {
            return;
        }
        match synced {
            Some(synced) => self.synced.insert(key.to_owned(), synced),
            None => self.synced.remove(key),
        };
        self.save();
    }

    /// Takes the ids [`Spaces::adopt`](spaces::Spaces::adopt) gave what the
    /// app kept for `scope`: a workspace and the tabs in it under its id in
    /// the document, and each tab under its tab id.
    pub(crate) fn take_adopted(&mut self, scope: &Scope, adopted: &Adopted) {
        let mut changed = false;
        for workspace in &mut self.workspaces {
            if workspace.scope != scope.as_str() {
                continue;
            }
            if let Some(id) = adopted.workspaces.get(&workspace.id)
                && *id != workspace.id
            {
                workspace.id.clone_from(id);
                changed = true;
            }
        }
        // A workspace that joined one kept already appears twice now.
        let mut seen = HashSet::new();
        self.workspaces.retain(|workspace| {
            workspace.scope != scope.as_str() || seen.insert(workspace.id.clone())
        });
        for tab in &mut self.tabs {
            if &tab.scope != scope {
                continue;
            }
            if let Some(id) = adopted.workspaces.get(&tab.workspace_id)
                && *id != tab.workspace_id
            {
                tab.workspace_id.clone_from(id);
                changed = true;
            }
            if let Some(id) = (tab.space.as_ref()).and_then(|space| adopted.tabs.get(space))
                && Some(id) != tab.space.as_ref()
            {
                tab.space = Some(id.clone());
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }

    /// Takes `document` in as `scope`'s kept workspaces and shared tabs,
    /// keeping what one window shows: the pane in use and a tab's title.
    /// A workspace or tab the document does not hold goes only when
    /// `gone` says the document held it before; it ends nothing itself,
    /// as the client that closed it does. Whether anything changed.
    pub(crate) fn mirror_shared(
        &mut self,
        scope: &Scope,
        document: &View,
        gone: impl Fn(Record<'_>) -> bool,
    ) -> bool {
        let before = (self.workspaces.clone(), self.tabs.clone());
        self.mirror_workspaces(scope, document, &gone);
        self.mirror_tabs(scope, document, &gone);
        let changed = before != (self.workspaces.clone(), self.tabs.clone());
        if changed {
            self.save();
        }
        changed
    }

    fn mirror_workspaces(
        &mut self,
        scope: &Scope,
        document: &View,
        gone: &impl Fn(Record<'_>) -> bool,
    ) {
        let ours = |workspace: &SavedWorkspace| workspace.scope == scope.as_str();
        let at = self.workspaces.iter().position(ours);
        let mine: Vec<SavedWorkspace> = self
            .workspaces
            .iter()
            .filter(|w| ours(w))
            .cloned()
            .collect();
        let others = self.workspaces.len() - mine.len();
        let mut room = super::kept::MAX_WORKSPACES.saturating_sub(others);
        let mut taken: Vec<SavedWorkspace> = Vec::new();
        for workspace in &document.workspaces {
            if room == 0 {
                break;
            }
            room -= 1;
            taken.push(SavedWorkspace {
                cwd: workspace.cwd.clone(),
                custom: workspace.custom,
                id: workspace.id.clone(),
                label: workspace.label.clone(),
                scope: scope.as_str().to_owned(),
            });
        }
        // Workspaces made here and not written yet stay, after the
        // document's.
        for workspace in mine {
            if document.workspace(&workspace.id).is_none()
                && !gone(Record::Workspace(&workspace.id))
                && room > 0
            {
                room -= 1;
                taken.push(workspace);
            }
        }
        let gone_ids: HashSet<String> = (self.workspaces.iter())
            .filter(|workspace| {
                ours(workspace) && !taken.iter().any(|kept| kept.id == workspace.id)
            })
            .map(|workspace| workspace.id.clone())
            .collect();
        self.tabs
            .retain(|tab| &tab.scope != scope || !gone_ids.contains(&tab.workspace_id));
        self.workspaces.retain(|workspace| !ours(workspace));
        let at = at
            .unwrap_or(self.workspaces.len())
            .min(self.workspaces.len());
        self.workspaces.splice(at..at, taken);
    }

    fn mirror_tabs(&mut self, scope: &Scope, document: &View, gone: &impl Fn(Record<'_>) -> bool) {
        self.tabs.retain(|tab| {
            let Some(space) = tab.space.as_deref().filter(|_| &tab.scope == scope) else {
                return true;
            };
            document.tab(space).is_some() || !gone(Record::Tab(space))
        });
        for shared in &document.tabs {
            if self.kept_workspace(scope, &shared.workspace_id).is_none() {
                continue;
            }
            let room = self.tabs.len() < MAX_TABS;
            let held = self
                .tabs
                .iter_mut()
                .find(|tab| &tab.scope == scope && tab.space.as_deref() == Some(&shared.id));
            match held {
                Some(tab) => {
                    let active = match &tab.location {
                        Some(Location::CoreTerminal { active, .. }) => active.clone(),
                        _ => None,
                    };
                    tab.location = Some(location_of(shared, active));
                    tab.workspace_id.clone_from(&shared.workspace_id);
                    if tab.title.is_empty() && !shared.title.is_empty() {
                        tab.title.clone_from(&shared.title);
                    }
                }
                None if room => {
                    let location = location_of(shared, None);
                    let title = Some(shared.title.clone())
                        .filter(|title| !title.is_empty())
                        .unwrap_or_else(|| location.default_title());
                    let id = TabId(self.next);
                    self.next += 1;
                    self.tabs.push(Tab {
                        id,
                        scope: scope.clone(),
                        workspace_id: shared.workspace_id.clone(),
                        location: Some(location),
                        title,
                        origin: None,
                        place: Place::Group,
                        space: Some(shared.id.clone()),
                        zoom: Zoom::default(),
                    });
                }
                None => tracing::warn!("No room for a shared herdr-core tab"),
            }
        }
    }
}

/// A record of a shared document, by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Record<'a> {
    Tab(&'a str),
    Workspace(&'a str),
}

/// Whether `tab` types into herdr-core terminals: a shared tab, unlike a
/// page or a mirror that only watches.
fn drives(tab: &Tab) -> bool {
    tab.place == Place::Group
        && matches!(
            tab.location,
            Some(Location::CoreTerminal { observe: false, .. })
        )
}

/// `tab` as the document holds it, once it has a document id.
fn shared_tab(tab: &Tab) -> Option<spaces::Tab> {
    if !drives(tab) {
        return None;
    }
    let Some(Location::CoreTerminal {
        panes,
        zoomed,
        name,
        names,
        ..
    }) = &tab.location
    else {
        return None;
    };
    Some(spaces::Tab {
        id: tab.space.clone()?,
        workspace_id: tab.workspace_id.clone(),
        title: tab.title.clone(),
        name: name.clone(),
        names: names
            .iter()
            .filter(|(terminal, _)| panes.contains(terminal))
            .map(|(terminal, name)| (terminal.as_str().to_owned(), name.clone()))
            .collect(),
        panes: panes.clone(),
        zoomed: zoomed.clone().filter(|zoomed| panes.contains(zoomed)),
    })
}

/// The location a shared tab saves with, keeping the pane this app had in
/// use when the document still holds it.
fn location_of(shared: &spaces::Tab, active: Option<TerminalId>) -> Location {
    let panes: Panes = shared.panes.clone();
    Location::CoreTerminal {
        active: active.filter(|active| panes.contains(active)),
        zoomed: shared
            .zoomed
            .clone()
            .filter(|zoomed| panes.contains(zoomed)),
        name: shared.name.clone(),
        names: shared
            .names
            .iter()
            .filter_map(|(terminal, name)| {
                let terminal = TerminalId::new(terminal.clone()).ok()?;
                panes.contains(&terminal).then(|| (terminal, name.clone()))
            })
            .collect(),
        observe: false,
        panes,
    }
}

/// How the document's tab ids start.
pub(crate) const TAB_PREFIX: &str = "tab-";

/// A new document id: `prefix` and 16 random hex digits, so ids two
/// clients hand out at once never meet, as the `space-N` counter did.
pub(crate) fn new_id(prefix: &str) -> String {
    let mut bytes = [0u8; 8];
    let random = match getrandom::fill(&mut bytes) {
        Ok(()) => u64::from_le_bytes(bytes),
        // No system randomness: a hasher seeded per process, over the time
        // and a count, still keeps ids apart.
        Err(_) => {
            use std::hash::{BuildHasher, Hasher};
            static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            hasher.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |time| time.as_nanos()),
            );
            hasher.finish()
        }
    };
    format!("{prefix}{random:016x}")
}
