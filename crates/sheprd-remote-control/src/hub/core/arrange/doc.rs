//! A phone's change to a tab or workspace of the core's shared document
//! (`sheprd_session::spaces`), made by the daemon while Sheprd is closed:
//! written to the document as [`Edit`]s, which every client of the core
//! then reads. Each is made again over what another client wrote first.

use super::{Error, axis};
use crate::hub::core::kept;
use crate::{
    control::layout::Op,
    hub::core::{HubError, Result, Source, TabRef, checkouts, lock},
};
use sheprd_backend::TerminalId;
use sheprd_core_client::{Core, spaces};
use sheprd_session::{
    WORKSPACE_LABEL,
    spaces::{Edit, Error as SpacesError, Spaces, Tab},
};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

/// How many times closing a workspace looks again for tabs another client
/// added to it while their terminals were ended.
const CLOSE_ATTEMPTS: usize = 4;

impl Source {
    /// Makes `op` at the terminal at `address` in the document's tab `tab`,
    /// or to the workspace it names. Answers a split's new terminal.
    pub(in crate::hub::core) fn arrange_doc(
        &self,
        tab: &TabRef,
        address: &str,
        op: &Op,
    ) -> Result<Option<String>> {
        let core = lock(&self.state)
            .core
            .clone()
            .ok_or(crate::hub::core::Error::Disconnected)?;
        match op {
            Op::RenameWorkspace { workspace, name } => {
                let edit = Edit::RenameWorkspace {
                    id: workspace.clone(),
                    label: name.trim().to_owned(),
                };
                spaces::update(&core, &[edit])?;
                return Ok(None);
            }
            Op::CloseWorkspace { workspace } => {
                self.close_workspace(&core, workspace)?;
                return Ok(None);
            }
            Op::RemoveWorktree { workspace } => {
                self.remove_worktree(&core, workspace)?;
                return Ok(None);
            }
            _ => {}
        }
        let TabRef::Doc(id) = tab else {
            return Err(crate::hub::core::Error::Unsupported.into());
        };
        let at = TerminalId::new(address).map_err(Error::from)?;
        // The phone names a recovered chat's replacement, which a document
        // written before the recovery does not: rebind it within the edit.
        let replacements = lock(&self.poller).replacements.clone();
        if let Some(axis) = axis(op) {
            let record = self.record(address)?;
            let workspace = lock(&self.poller)
                .shared()
                .and_then(|shared| shared.spaces.tab(id))
                .map(|tab| tab.workspace_id.clone());
            // Labelled for its workspace, so a client that has not read the
            // document yet still lists it there.
            let labels = workspace
                .as_deref()
                .filter(|id| sheprd_session::tab_workspace(id))
                .map(|id| BTreeMap::from([(WORKSPACE_LABEL, id)]))
                .unwrap_or_default();
            let new = self.start_beside(&record, labels)?;
            let split = TerminalId::new(new.address.as_str())
                .map_err(|error| HubError::from(Error::from(error)))
                .and_then(|new| {
                    edit_tab(&core, id, &replacements, |tab| {
                        tab.panes.split(&at, axis, new.clone())?;
                        tab.zoomed = None;
                        Ok(())
                    })
                });
            if let Err(error) = split {
                self.end(&new.address);
                return Err(error);
            }
            return Ok(Some(new.address));
        }
        let present = |tab: &Tab| {
            tab.panes
                .contains(&at)
                .then_some(())
                .ok_or_else(|| sheprd_session::panes::Error::Missing(at.clone()))
        };
        match op {
            Op::Zoom => edit_tab(&core, id, &replacements, |tab| {
                present(tab)?;
                tab.zoomed = match tab.zoomed {
                    Some(_) => None,
                    None => Some(at.clone()),
                };
                Ok(())
            })?,
            Op::RenameTab(name) => edit_tab(&core, id, &replacements, |tab| {
                tab.name = named(name.as_deref());
                Ok(())
            })?,
            Op::RenamePane(name) => edit_tab(&core, id, &replacements, |tab| {
                present(tab)?;
                match named(name.as_deref()) {
                    Some(name) => tab.names.insert(at.as_str().to_owned(), name),
                    None => tab.names.remove(at.as_str()),
                };
                Ok(())
            })?,
            Op::Close => {
                spaces::update(&core, &[Edit::CloseTab { id: id.clone() }])?;
            }
            Op::SplitDown
            | Op::SplitRight
            | Op::RenameWorkspace { .. }
            | Op::CloseWorkspace { .. }
            | Op::RemoveWorktree { .. } => {
                return Err(crate::hub::core::Error::Unsupported.into());
            }
        }
        Ok(None)
    }

    /// Ends the terminals of `workspace`'s tabs, and those started for it,
    /// then forgets it, as its menu's Close does. A tab another client adds
    /// meanwhile has its terminals ended too before the workspace goes.
    fn close_workspace(&self, core: &Core, workspace: &str) -> Result<()> {
        let mut ended = HashSet::new();
        for attempt in 0..CLOSE_ATTEMPTS {
            let (spaces, _) = spaces::read(core)?;
            if attempt == 0 {
                editable(&spaces, workspace)?;
            }
            let mut addresses: Vec<String> = spaces
                .tabs_of(workspace)
                .flat_map(|tab| tab.panes.terminals())
                .map(|terminal| terminal.as_str().to_owned())
                .collect();
            addresses.extend(
                lock(&self.state)
                    .terminals
                    .iter()
                    .filter(|terminal| {
                        sheprd_session::labelled_workspace(&terminal.labels).is_some_and(
                            |labelled| labelled.session.is_none() && labelled.id == workspace,
                        )
                    })
                    .map(|terminal| terminal.address.clone()),
            );
            addresses.sort_unstable();
            addresses.dedup();
            addresses.retain(|address| !ended.contains(address));
            self.end_terminals(core, &addresses)?;
            ended.extend(addresses);
            let mut missed = false;
            spaces::change(core, |spaces| {
                missed = spaces
                    .tabs_of(workspace)
                    .flat_map(|tab| tab.panes.terminals())
                    .any(|terminal| !ended.contains(terminal.as_str()));
                if !missed {
                    spaces.apply(Edit::ForgetWorkspace {
                        id: workspace.to_owned(),
                    })?;
                }
                Ok(())
            })?;
            if !missed {
                return Ok(());
            }
        }
        Err(Error::Changed.into())
    }

    /// Deletes `workspace`'s linked worktree, never forced, so a checkout
    /// with changes stays, then closes the workspace. The branch stays.
    fn remove_worktree(&self, core: &Core, workspace: &str) -> Result<()> {
        let (spaces, _) = spaces::read(core)?;
        // Nothing is deleted for a workspace the document would not forget.
        editable(&spaces, workspace)?;
        let cwd = spaces
            .workspace(workspace)
            .map(|workspace| workspace.cwd.clone())
            .filter(|cwd| !cwd.is_empty())
            .ok_or(Error::NotWorktree)?;
        let checkout = checkouts::read(Path::new(&cwd))
            .filter(|checkout| checkout.worktree.is_linked_worktree)
            .ok_or(Error::NotWorktree)?;
        // A workspace may be in a folder of the checkout: Git removes it by
        // its root.
        let root =
            crate::native::changes::local::root(Path::new(&cwd)).map_err(|_| Error::NotWorktree)?;
        let status = sheprd_client::new_command("git")
            .arg("--git-dir")
            .arg(&checkout.worktree.key)
            .args(["worktree", "remove", "--"])
            .arg(&root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(Error::from)?;
        if !status.success() {
            return Err(Error::WorktreeKept.into());
        }
        self.close_workspace(core, workspace)
    }
}

/// Makes `change` to the document's tab `id`, over whatever another client
/// wrote first. A change the tab refuses leaves the document as it was.
fn edit_tab(
    core: &Core,
    id: &str,
    replacements: &BTreeMap<TerminalId, TerminalId>,
    mut change: impl FnMut(&mut Tab) -> std::result::Result<(), sheprd_session::panes::Error>,
) -> Result<()> {
    let mut refused = None;
    spaces::change(core, |spaces| {
        refused = None;
        let mut tab = spaces
            .tab(id)
            .cloned()
            .ok_or_else(|| sheprd_session::spaces::Error::Missing(id.to_owned()))?;
        kept::remap(&mut tab, replacements);
        if let Err(error) = change(&mut tab) {
            refused = Some(error);
            return Ok(());
        }
        spaces.apply(Edit::PutTab(tab))?;
        Ok(())
    })?;
    match refused {
        Some(error) => Err(Error::from(error).into()),
        None => Ok(()),
    }
}

/// Refuses a workspace the document does not keep, or a document this
/// version may not write, before anything is ended or deleted for it.
fn editable(spaces: &Spaces, workspace: &str) -> Result<()> {
    // Writing it refuses a newer format the way an edit would, naming it.
    if !spaces.writable() {
        spaces.to_value().map_err(sheprd_core_client::Error::from)?;
    }
    if spaces.workspace(workspace).is_none() {
        let missing = SpacesError::Missing(workspace.to_owned());
        return Err(sheprd_core_client::Error::from(missing).into());
    }
    Ok(())
}

/// A name given, or `None` for a blank one, which gives back the title.
fn named(name: Option<&str>) -> Option<String> {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}
