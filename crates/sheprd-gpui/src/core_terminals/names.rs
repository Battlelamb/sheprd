//! Automatic root-space names, following Herdr's first-terminal rule.

use crate::{
    SheprdWindow,
    browser::{Location, Scope, Store},
    core_terminals::{Host, WORKSPACE, checkouts::Checkouts},
};
use gpui::App;
use sheprd_session::Session;
use std::collections::BTreeMap;

impl SheprdWindow {
    /// Names use the last listed directory even while disconnected; unlike
    /// agent status, a cached folder does not claim a process is still running.
    /// Keep an entry even when the first terminal has no live folder: a
    /// later tab must not become the naming terminal while it reconnects.
    pub(super) fn core_name_folders(
        &self,
        store: &Store,
        scope: &Scope,
        cx: &App,
    ) -> BTreeMap<String, Option<String>> {
        let host = Host::of(scope);
        let mut folders = BTreeMap::new();
        for tab in store.in_scope(scope) {
            if let Some(Location::CoreTerminal {
                panes,
                observe: false,
                ..
            }) = &tab.location
            {
                folders.entry(tab.workspace_id.clone()).or_insert_with(|| {
                    self.core_terminals
                        .listed(&host, panes.first())
                        .and_then(|info| info.cwd.as_ref())
                        .map(|cwd| cwd.to_string_lossy().into_owned())
                        .filter(|cwd| !cwd.is_empty())
                });
            }
        }
        if host == Host::Local
            && let Some(view) = self.core_fallback_view()
        {
            folders.entry(WORKSPACE.to_owned()).or_insert_with(|| {
                self.core_terminals
                    .listed(&host, view.read(cx).terminal())
                    .and_then(|info| info.cwd.as_ref())
                    .map(|cwd| cwd.to_string_lossy().into_owned())
                    .filter(|cwd| !cwd.is_empty())
            });
        }
        folders
    }
}

pub(super) fn apply(
    session: &mut Session,
    folders: &BTreeMap<String, Option<String>>,
    checkouts: Option<&Checkouts>,
    home: &str,
) {
    for workspace in &mut session.workspaces {
        if workspace.custom_label || workspace.worktree.as_ref().is_some_and(|tree| tree.linked) {
            continue;
        }
        let cwd = folders
            .get(&workspace.id)
            .and_then(Option::as_deref)
            .unwrap_or(&workspace.cwd);
        let folder = checkouts
            .and_then(|known| known.get(cwd))
            .map(|checkout| checkout.root.as_str())
            .filter(|root| !root.is_empty())
            .unwrap_or(cwd);
        workspace.label = workspace.label_for_folder(folder, home);
    }
}

#[cfg(test)]
mod tests;
