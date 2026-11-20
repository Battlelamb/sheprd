//! The workspaces and tabs Sheprd keeps for one herdr-core, held in that
//! core's [`DOC`] document so every client of the core reads and changes
//! the same ones: the app on the core's own computer, the app on another
//! computer that reaches it over SSH, and the remote control daemon.
//!
//! The document holds what every client shows alike: which workspaces
//! there are, their names, folders and order, and each tab's terminals and
//! split layout. What one window shows (which tab is in front, the selected
//! pane, a tab watching another's terminal) stays with that window.
//!
//! Clients change the document with [`Edit`]s applied to what they last
//! read and written back over that version, so two clients editing at once
//! each keep their change: the later write is refused, and it applies its
//! edit again to the newer document (`sheprd_core_client::spaces`).

mod error;

pub use error::Error;

use crate::panes::Panes;
use crate::saved::{MAX_CWD_BYTES, MAX_ID_BYTES, MAX_LABEL_CHARS, MAX_SAVED_TABS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sheprd_backend::TerminalId;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The herdr-core document holding a core's [`Spaces`].
pub const DOC: &str = "sheprd.spaces";
/// The format this version writes. A document of a newer format is read
/// for what this version knows, and never written.
pub const FORMAT: u32 = 1;
/// The largest document read or written.
pub const MAX_BYTES: usize = 256 * 1024;
/// The most workspaces, and the most tabs, a document holds.
pub const MAX_ENTRIES: usize = MAX_SAVED_TABS;
/// How a workspace id the document hands out starts, apart from Herdr 1's
/// and from the `space-N` ids the app kept in its own file.
pub const ID_PREFIX: &str = "space-";

/// One core's workspaces and tabs, in the user's order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spaces {
    #[serde(default)]
    format: u32,
    #[serde(default, deserialize_with = "readable")]
    workspaces: Vec<Workspace>,
    #[serde(default, deserialize_with = "readable")]
    tabs: Vec<Tab>,
}

/// A workspace Sheprd keeps for the core's terminals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// Unique in the document; tabs name it.
    pub id: String,
    pub label: String,
    /// Whether the user named it; otherwise it is named for its folder, and
    /// a linked worktree's row shows its branch instead.
    #[serde(default)]
    pub custom: bool,
    /// Where its new terminals start, on the core's computer; empty for none.
    #[serde(default)]
    pub cwd: String,
}

/// A tab of the core's terminals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    /// Unique in the document, and never reused: clients map it to tabs of
    /// their own.
    pub id: String,
    pub workspace_id: String,
    /// Its program's title when the document was last written.
    #[serde(default)]
    pub title: String,
    /// The name the user gave it, shown over its title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Names the user gave its panes, by terminal address.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    pub panes: Panes,
    /// The pane filling it, when one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoomed: Option<TerminalId>,
}

/// One change to a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Closes a tab; one already gone is no change.
    CloseTab { id: String },
    /// Forgets a workspace and every tab in it; one already gone is no
    /// change.
    ForgetWorkspace { id: String },
    /// Keeps a new workspace at the end. An id already kept is refused.
    KeepWorkspace(Workspace),
    /// Moves a workspace before another, or to the end when `before` is
    /// `None` or gone.
    MoveWorkspace { id: String, before: Option<String> },
    /// Adds a tab at the end, or replaces the tab of that id: its layout,
    /// names, title, or workspace changed.
    PutTab(Tab),
    /// Renames a workspace, which the user then named.
    RenameWorkspace { id: String, label: String },
}

/// What [`Spaces::adopt`] changed: the document ids the adopted workspaces
/// and tabs took, by the ids the caller knew them by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adopted {
    pub tabs: HashMap<String, String>,
    pub workspaces: HashMap<String, String>,
}

impl Spaces {
    /// The document's `value` as `doc.get` answers it: `Null` for one never
    /// written. A record that fails to read or breaks a bound is left out
    /// rather than costing the rest.
    pub fn from_value(value: Value) -> Result<Self, Error> {
        if value.is_null() {
            return Ok(Self::default());
        }
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Large);
        }
        let mut spaces: Self = serde_json::from_value(value)?;
        spaces.workspaces.retain(Workspace::valid);
        spaces.workspaces.truncate(MAX_ENTRIES);
        let mut seen = HashSet::new();
        spaces
            .workspaces
            .retain(|workspace| seen.insert(workspace.id.clone()));
        let kept: HashSet<&str> = spaces.workspaces.iter().map(|w| w.id.as_str()).collect();
        let mut tab_ids = HashSet::new();
        let tabs = std::mem::take(&mut spaces.tabs)
            .into_iter()
            .filter(|tab| {
                tab.valid()
                    && kept.contains(tab.workspace_id.as_str())
                    && tab_ids.insert(tab.id.clone())
            })
            .take(MAX_ENTRIES)
            .collect();
        spaces.tabs = tabs;
        Ok(spaces)
    }

    /// The value to write with `doc.set`, refused when this version cannot
    /// write the document's format or it outgrew its bounds.
    pub fn to_value(&self) -> Result<Value, Error> {
        if self.format > FORMAT {
            return Err(Error::Newer(self.format));
        }
        let mut written = self.clone();
        written.format = FORMAT;
        let value = serde_json::to_value(&written)?;
        if serde_json::to_vec(&value)?.len() > MAX_BYTES {
            return Err(Error::Large);
        }
        Ok(value)
    }

    /// Whether this version may write the document.
    pub fn writable(&self) -> bool {
        self.format <= FORMAT
    }

    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }

    pub fn workspace(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.iter().find(|workspace| workspace.id == id)
    }

    /// Every tab, in the order they were opened.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn tab(&self, id: &str) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// `workspace`'s tabs, in the order they were opened.
    pub fn tabs_of<'a>(&'a self, workspace: &'a str) -> impl Iterator<Item = &'a Tab> + 'a {
        self.tabs
            .iter()
            .filter(move |tab| tab.workspace_id == workspace)
    }

    /// The tab holding `terminal`, if any.
    pub fn tab_holding(&self, terminal: &TerminalId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.panes.contains(terminal))
    }

    /// Makes `edit`, keeping every bound; on an error nothing changed.
    pub fn apply(&mut self, edit: Edit) -> Result<(), Error> {
        if !self.writable() {
            return Err(Error::Newer(self.format));
        }
        match edit {
            Edit::CloseTab { id } => self.tabs.retain(|tab| tab.id != id),
            Edit::ForgetWorkspace { id } => {
                self.workspaces.retain(|workspace| workspace.id != id);
                self.tabs.retain(|tab| tab.workspace_id != id);
            }
            Edit::KeepWorkspace(workspace) => {
                if !workspace.valid() {
                    return Err(Error::Invalid);
                }
                if self.workspace(&workspace.id).is_some() {
                    return Err(Error::Taken(workspace.id));
                }
                if self.workspaces.len() >= MAX_ENTRIES {
                    return Err(Error::Full);
                }
                self.workspaces.push(workspace);
            }
            Edit::MoveWorkspace { id, before } => {
                let Some(from) = self.workspaces.iter().position(|w| w.id == id) else {
                    return Err(Error::Missing(id));
                };
                let moved = self.workspaces.remove(from);
                let to = before
                    .and_then(|before| self.workspaces.iter().position(|w| w.id == before))
                    .unwrap_or(self.workspaces.len());
                self.workspaces.insert(to, moved);
            }
            Edit::PutTab(tab) => {
                if !tab.valid() {
                    return Err(Error::Invalid);
                }
                if self.workspace(&tab.workspace_id).is_none() {
                    return Err(Error::Missing(tab.workspace_id));
                }
                if let Some(held) = self.tabs.iter_mut().find(|held| held.id == tab.id) {
                    *held = tab;
                } else if self.tabs.len() >= MAX_ENTRIES {
                    return Err(Error::Full);
                } else {
                    self.tabs.push(tab);
                }
            }
            Edit::RenameWorkspace { id, label } => {
                let label = clean_label(&label);
                if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
                    return Err(Error::Invalid);
                }
                let workspace = self
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.id == id)
                    .ok_or(Error::Missing(id))?;
                workspace.label = label;
                workspace.custom = true;
            }
        }
        Ok(())
    }

    /// Adds what one client kept for this core before the document held it,
    /// as when the app first finds a core's document, without losing what
    /// is there:
    ///
    /// - a workspace whose folder a kept one has joins that one;
    /// - any other keeps its id when free, else takes `new_id()`;
    /// - a tab whose terminals another tab already holds is left out, and
    ///   any other keeps its id when free, else takes `new_id()`.
    ///
    /// What does not fit the bounds is left out. Answers the ids taken.
    pub fn adopt(
        &mut self,
        workspaces: impl IntoIterator<Item = Workspace>,
        tabs: impl IntoIterator<Item = Tab>,
        mut new_id: impl FnMut() -> String,
    ) -> Result<Adopted, Error> {
        if !self.writable() {
            return Err(Error::Newer(self.format));
        }
        let mut adopted = Adopted::default();
        for workspace in workspaces {
            if !workspace.valid() {
                continue;
            }
            let joined = self
                .workspaces
                .iter()
                .find(|kept| !kept.cwd.is_empty() && kept.cwd == workspace.cwd)
                .map(|kept| kept.id.clone());
            if let Some(joined) = joined {
                adopted.workspaces.insert(workspace.id, joined);
                continue;
            }
            if self.workspaces.len() >= MAX_ENTRIES {
                continue;
            }
            let id = self.free_workspace_id(&workspace.id, &mut new_id);
            adopted.workspaces.insert(workspace.id.clone(), id.clone());
            self.workspaces.push(Workspace { id, ..workspace });
        }
        for tab in tabs {
            let Some(workspace_id) = adopted.workspaces.get(&tab.workspace_id).cloned() else {
                continue;
            };
            let held = tab
                .panes
                .terminals()
                .into_iter()
                .any(|terminal| self.tab_holding(terminal).is_some());
            if held || !tab.valid() || self.tabs.len() >= MAX_ENTRIES {
                continue;
            }
            let id = if self.tab(&tab.id).is_some() {
                fresh(&mut new_id, |id| self.tab(id).is_none())
            } else {
                tab.id.clone()
            };
            adopted.tabs.insert(tab.id.clone(), id.clone());
            self.tabs.push(Tab {
                id,
                workspace_id,
                ..tab
            });
        }
        Ok(adopted)
    }

    fn free_workspace_id(&self, wanted: &str, new_id: &mut impl FnMut() -> String) -> String {
        if self.workspace(wanted).is_none() {
            return wanted.to_owned();
        }
        fresh(new_id, |id| self.workspace(id).is_none())
    }
}

/// An id from `new_id` that `free` accepts. Callers hand out random ids, so
/// a second try is rare; the bound only stops a broken generator.
fn fresh(new_id: &mut impl FnMut() -> String, free: impl Fn(&str) -> bool) -> String {
    let mut id = new_id();
    for _ in 0..16 {
        if free(&id) {
            break;
        }
        id = new_id();
    }
    id
}

impl Workspace {
    /// Whether a workspace is one a client could have written.
    pub fn valid(&self) -> bool {
        valid_id(&self.id)
            && !self.label.trim().is_empty()
            && self.label.chars().count() <= MAX_LABEL_CHARS
            && self.cwd.len() <= MAX_CWD_BYTES
            && (self.cwd.is_empty() || !self.cwd.contains('\0'))
    }
}

impl Tab {
    /// Whether a tab is one a client could have written: bounded names, and
    /// a zoomed pane that is one of its own.
    pub fn valid(&self) -> bool {
        valid_id(&self.id)
            && valid_id(&self.workspace_id)
            && self.title.len() <= MAX_ID_BYTES * 4
            && self
                .name
                .as_ref()
                .is_none_or(|name| name.len() <= MAX_ID_BYTES * 4)
            && self.names.len() <= MAX_ENTRIES
            && self.names.iter().all(|(terminal, name)| {
                terminal.len() <= MAX_ID_BYTES && name.len() <= MAX_ID_BYTES * 4
            })
            && self
                .zoomed
                .as_ref()
                .is_none_or(|zoomed| self.panes.contains(zoomed))
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES && !id.chars().any(char::is_control)
}

/// A name on one line, without the spaces around it.
fn clean_label(label: &str) -> String {
    label
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The records of a list that read, each on its own.
fn readable<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let values = Vec::<Value>::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect())
}
