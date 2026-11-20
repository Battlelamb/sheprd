//! The edits that turn what a core's document last held into what the app
//! holds now: workspaces kept, renamed, moved, or forgotten, and tabs
//! opened, changed, or closed.

use crate::browser::shared::View;
use crate::core_terminals::panes::{Axis, Panes};
use sheprd_backend::TerminalId;
use sheprd_session::spaces::{Edit, Tab, TabEdit};

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
    // Terminals a tab lets go of are let go first, by closing it or its
    // pane, so a pane moved to another tab is free when it lands there.
    for tab in &base.tabs {
        if local.tab(&tab.id).is_none() {
            edits.push(Edit::CloseTab { id: tab.id.clone() });
        }
    }
    let mut landing = Vec::new();
    for tab in &local.tabs {
        let Some(was) = base.tab(&tab.id) else {
            landing.push(Edit::OpenTab(tab.clone()));
            continue;
        };
        for edit in tab_edits(was, tab) {
            let edit = Edit::Tab {
                id: tab.id.clone(),
                edit,
            };
            match &edit {
                Edit::Tab {
                    edit: TabEdit::RemovePane { .. },
                    ..
                } => edits.push(edit),
                _ => landing.push(edit),
            }
        }
    }
    edits.extend(landing);
    for workspace in &base.workspaces {
        if local.workspace(&workspace.id).is_none() {
            edits.push(Edit::ForgetWorkspace {
                id: workspace.id.clone(),
            });
        }
    }
    edits
}

/// The changes from `was` to `now` of one tab, each touching only its own
/// fields, so a change another client made to the same tab stays. Its
/// program's title changes as it runs and is not written.
fn tab_edits(was: &Tab, now: &Tab) -> Vec<TabEdit> {
    let mut edits = Vec::new();
    if was.workspace_id != now.workspace_id {
        edits.push(TabEdit::Move {
            workspace_id: now.workspace_id.clone(),
        });
    }
    edits.extend(pane_edits(&was.panes, &now.panes));
    if was.name != now.name {
        edits.push(TabEdit::Rename {
            name: now.name.clone(),
        });
    }
    let renamed = (now.names.keys())
        .chain(was.names.keys())
        .filter(|terminal| was.names.get(*terminal) != now.names.get(*terminal))
        .collect::<std::collections::BTreeSet<_>>();
    for terminal in renamed {
        let Ok(id) = TerminalId::new(terminal.clone()) else {
            continue;
        };
        // A pane closed here takes its name with it.
        if now.panes.contains(&id) {
            edits.push(TabEdit::RenamePane {
                terminal: id,
                name: now.names.get(terminal).cloned(),
            });
        }
    }
    if was.zoomed != now.zoomed {
        edits.push(TabEdit::Zoom {
            zoomed: now.zoomed.clone(),
        });
    }
    edits
}

/// The pane changes from `was` to `now`: each pane closed, then a pane
/// opened by a split, so another client's split of the same tab stays. A
/// layout no such steps reach, as from a resize or a swap, is written
/// whole.
fn pane_edits(was: &Panes, now: &Panes) -> Vec<TabEdit> {
    if was == now {
        return Vec::new();
    }
    let mut edits = Vec::new();
    let mut panes = was.clone();
    for terminal in was.terminals() {
        if !now.contains(terminal) && panes.count() > 1 && panes.remove(terminal) {
            edits.push(TabEdit::RemovePane {
                terminal: terminal.clone(),
            });
        }
    }
    // One pane opened here is a split of one already there; anything
    // more is written whole.
    let opened: Vec<&TerminalId> = (now.terminals().into_iter())
        .filter(|terminal| !panes.contains(terminal))
        .collect();
    if let [new] = opened[..] {
        let split = panes.terminals().into_iter().find_map(|at| {
            [Axis::Row, Axis::Column].into_iter().find_map(|axis| {
                let mut tried = panes.clone();
                tried.split(at, axis, new.clone()).ok()?;
                (tried == *now).then(|| (at.clone(), axis, tried))
            })
        });
        if let Some((at, axis, tried)) = split {
            panes = tried;
            edits.push(TabEdit::Split {
                at,
                axis,
                new: new.clone(),
            });
        }
    }
    if panes != *now {
        edits.push(TabEdit::SetPanes { panes: now.clone() });
    }
    edits
}
