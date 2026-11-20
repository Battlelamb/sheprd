//! The edits that turn what a core's document last held into what the app
//! holds now: workspaces kept, renamed, moved, or forgotten, and tabs
//! opened, changed, or closed.

use crate::browser::shared::View;
use sheprd_session::spaces::{Edit, Tab};

/// The edits from `base`, the document as last read, to `local`, the
/// store's view of the same scope now. None when nothing changed.
pub(crate) fn edits(base: &View, local: &View) -> Vec<Edit> {
    let mut edits = Vec::new();
    for workspace in &local.workspaces {
        match base.workspace(&workspace.id) {
            None => edits.push(Edit::KeepWorkspace(workspace.clone())),
            // Only the user renames: a folder-named workspace keeps the
            // name every client gave it.
            Some(was) if workspace.custom && (workspace.label != was.label || !was.custom) => {
                edits.push(Edit::RenameWorkspace {
                    id: workspace.id.clone(),
                    label: workspace.label.clone(),
                });
            }
            Some(_) => {}
        }
    }
    // The order the document would hold after these edits; the store's
    // when they agree, else every workspace moves to its place.
    let mut order: Vec<&str> = (base.workspaces.iter())
        .map(|workspace| workspace.id.as_str())
        .filter(|id| local.workspace(id).is_some())
        .collect();
    order.extend(
        (local.workspaces.iter())
            .filter(|workspace| base.workspace(&workspace.id).is_none())
            .map(|workspace| workspace.id.as_str()),
    );
    let wanted: Vec<&str> = (local.workspaces.iter())
        .map(|workspace| workspace.id.as_str())
        .collect();
    if order != wanted {
        for (at, workspace) in local.workspaces.iter().enumerate().rev() {
            edits.push(Edit::MoveWorkspace {
                id: workspace.id.clone(),
                before: local.workspaces.get(at + 1).map(|next| next.id.clone()),
            });
        }
    }
    for tab in &local.tabs {
        if base.tab(&tab.id).is_none_or(|was| !same(was, tab)) {
            edits.push(Edit::PutTab(tab.clone()));
        }
    }
    for tab in &base.tabs {
        if local.tab(&tab.id).is_none() {
            edits.push(Edit::CloseTab { id: tab.id.clone() });
        }
    }
    for workspace in &base.workspaces {
        if local.workspace(&workspace.id).is_none() {
            edits.push(Edit::ForgetWorkspace {
                id: workspace.id.clone(),
            });
        }
    }
    edits
}

/// Whether two tabs share the same layout, names, zoom, and workspace. Its
/// program's title changes as it runs and is written only with them.
fn same(a: &Tab, b: &Tab) -> bool {
    a.workspace_id == b.workspace_id
        && a.panes == b.panes
        && a.name == b.name
        && a.names == b.names
        && a.zoomed == b.zoomed
}
