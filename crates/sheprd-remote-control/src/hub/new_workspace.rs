//! A phone's new workspace: in a folder, on a new worktree when a branch
//! is given, with a chat in it.
//!
//! A worktree of a repository Herdr 1 already shows is made by Herdr 1 from
//! that repository's workspace, as Sheprd's own new worktree is, so it
//! joins the repository's group in every client. Its chat still starts on
//! the core while one answers, in that workspace. Any other new workspace
//! goes to the core, which Sheprd keeps a workspace for, or to Herdr 1 when
//! no core answers.

use super::{
    ApiMethod, CreatedWorkspace, Error, Hub, Layout, Result, command_line, core, created_tab,
    direct_command_line, on_core, place, sessions, tab_params,
};
use serde_json::{Value, json};
use sheprd_chat_protocol::Agent;
use std::path::Path;

/// What a phone's new workspace asks for.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NewWorkspace<'a> {
    pub(crate) agent: Agent,
    /// A new worktree on this branch, else the folder itself.
    pub(crate) branch: Option<&'a str>,
    pub(crate) cwd: &'a str,
    /// The Herdr 1 session this daemon follows by that name; the primary
    /// otherwise.
    pub(crate) session: Option<&'a str>,
    /// The name the person gave the workspace; else it is named for its
    /// folder or branch.
    pub(crate) title: Option<&'a str>,
}

impl Hub {
    /// Creates `new`, answering its workspace and its chat's pane. While a
    /// core answers the chat starts there.
    pub(crate) async fn create_workspace(&self, new: NewWorkspace<'_>) -> Result<CreatedWorkspace> {
        // As in [`Hub::start_chat`]: the program here cannot run in a
        // distribution, and a core would open the chat on this computer.
        if new.session.is_some_and(sessions::is_wsl) {
            return Err(Error::WslChat);
        }
        let program = self.program().ok_or(Error::ProgramMissing)?;
        let core = self.core_for_work().await?;
        if let Some(branch) = new.branch {
            let session = new.session.filter(|name| self.sessions.get(name).is_some());
            let repository = herdr_1_repository(&self.layout(), new.cwd, session, |id| {
                self.kept_workspace(id)
            });
            if let Some(source) = repository {
                let worktree = Worktree {
                    base: source.base.as_deref(),
                    branch,
                    repository: &source.workspace,
                    title: new.title,
                };
                return self
                    .herdr_1_worktree(core, worktree, new.agent, &program)
                    .await;
            }
        }
        if let Some(core) = core {
            return self.core_workspace(core, new, &program).await;
        }
        self.herdr_1_workspace(new, &program).await
    }

    /// `new` on the core, in its unlisted group until Sheprd keeps a
    /// workspace for it: the core has no workspaces or worktrees, so a
    /// branch's checkout is made here.
    async fn core_workspace(
        &self,
        core: std::sync::Arc<core::Source>,
        new: NewWorkspace<'_>,
        program: &Path,
    ) -> Result<CreatedWorkspace> {
        // Herdr 2 keeps no Herdr 1 session: asked for a named one, the
        // workspace could not be where it was asked for.
        if new
            .session
            .is_some_and(|name| self.sessions.get(name).is_some())
        {
            return Err(Error::SessionOnHerdr2);
        }
        let (cwd, program, agent) = (new.cwd.to_owned(), program.to_owned(), new.agent);
        let (branch, root) = (new.branch.map(str::to_owned), self.worktrees.clone());
        let title = new.title.map(str::to_owned);
        let (pane, folder) = on_core(move || {
            let title = title.as_deref();
            match branch {
                Some(branch) => {
                    core.open_worktree_chat(agent, &program, &cwd, &branch, title, root.as_deref())
                }
                // Sheprd keeps a workspace for it, in this folder.
                None => core
                    .open_chat(
                        agent,
                        &program,
                        &cwd,
                        Some(core::Place::New { cwd: &cwd, title }),
                    )
                    .map(|pane| (pane, cwd)),
            }
        })
        .await?;
        Ok(CreatedWorkspace {
            workspace: core::new_workspace_id(&folder),
            pane,
        })
    }

    /// A worktree Herdr 1 makes from `worktree.repository`'s workspace, in a
    /// workspace of its own there, with the chat on `core` while one
    /// answers, else typed into the worktree's first pane.
    async fn herdr_1_worktree(
        &self,
        core: Option<std::sync::Arc<core::Source>>,
        worktree: Worktree<'_>,
        agent: Agent,
        program: &Path,
    ) -> Result<CreatedWorkspace> {
        let created = self
            .call(ApiMethod::WorktreeCreate, worktree.params())
            .await?;
        let workspace = answered(&created["workspace"]["workspace_id"])?;
        let Some(core) = core else {
            let pane = answered(&created["root_pane"]["pane_id"])?;
            // The worktree's first pane is a shell in its folder; the program
            // path rides in this line because the pane was not made with it.
            self.run_line(&pane, &direct_command_line(program, agent))
                .await?;
            return Ok(CreatedWorkspace { workspace, pane });
        };
        let checkout = answered(&created["worktree"]["path"])?;
        let (labelled, program) = (workspace.clone(), program.to_owned());
        // Labelled for the new workspace, so Sheprd puts it in a tab there.
        let started = on_core(move || {
            core.open_chat(
                agent,
                &program,
                &checkout,
                Some(place(&labelled, &checkout)),
            )
        })
        .await;
        let pane = started.inspect_err(|error| {
            tracing::warn!(%error, %workspace, "A new worktree's chat did not start; the worktree stays");
        })?;
        Ok(CreatedWorkspace { workspace, pane })
    }

    /// `new` in Herdr 1, which no core stands in for: a workspace in its
    /// folder, and a worktree from it when a branch is given.
    async fn herdr_1_workspace(
        &self,
        new: NewWorkspace<'_>,
        program: &Path,
    ) -> Result<CreatedWorkspace> {
        let mut params = json!({"cwd": new.cwd, "focus": false});
        if let (Some(title), None) = (new.title, new.branch) {
            params["label"] = title.into();
        }
        let opened = self
            .call_in(new.session, ApiMethod::WorkspaceCreate, params)
            .await?;
        let workspace = answered(&opened["workspace"]["workspace_id"])?;
        let Some(branch) = new.branch else {
            let response = self
                .call(
                    ApiMethod::TabCreate,
                    tab_params(&workspace, new.cwd, new.agent, program),
                )
                .await?;
            let (_, pane) = created_tab(&response).ok_or(Error::Response)?;
            self.run_line(&pane, &command_line(new.agent)).await?;
            return Ok(CreatedWorkspace { workspace, pane });
        };
        let worktree = Worktree {
            base: None,
            branch,
            repository: &workspace,
            title: new.title,
        };
        self.herdr_1_worktree(None, worktree, new.agent, program)
            .await
    }
}

/// A new worktree Herdr 1 makes.
#[derive(Clone, Copy, Debug)]
struct Worktree<'a> {
    /// The full ref it starts from; `None` for the repository's `HEAD`.
    base: Option<&'a str>,
    branch: &'a str,
    /// The workspace of the repository it is made from.
    repository: &'a str,
    title: Option<&'a str>,
}

impl Worktree<'_> {
    /// Its `worktree.create` parameters; a title names the new workspace.
    fn params(self) -> Value {
        let mut params = json!({
            "workspace_id": self.repository,
            "branch": self.branch,
            "focus": false,
        });
        if let Some(title) = self.title {
            params["label"] = title.into();
        }
        if let Some(base) = self.base {
            params["base"] = base.into();
        }
        params
    }
}

/// A non-empty id or path Herdr answered.
fn answered(value: &Value) -> Result<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .ok_or(Error::Response)
}

/// Where Herdr 1 makes a new worktree from: the workspace of the
/// repository's own checkout, and the ref it starts from when that is not
/// the checkout's `HEAD`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Source {
    pub(super) workspace: String,
    pub(super) base: Option<String>,
}

/// Where Herdr 1 makes a new worktree in `cwd` from, among `session`'s
/// workspaces (the primary's when `None`):
///
/// - the repository checkout whose folder is `cwd`, or whose root is;
/// - for a linked worktree at `cwd`, its repository's own checkout, from
///   the linked worktree's branch, as the desktop's new worktree does:
///   Herdr 1 refuses a linked checkout as the source.
///
/// `None` when Herdr 1 shows no such checkout, and the core makes it as
/// before. Workspaces Sheprd keeps for Herdr 2, a core's new workspaces,
/// and WSL's are not Herdr 1's to grow here.
pub(super) fn herdr_1_repository(
    layout: &Layout,
    cwd: &str,
    session: Option<&str>,
    kept: impl Fn(&str) -> bool,
) -> Option<Source> {
    let cwd = Path::new(cwd);
    let candidates: Vec<_> = layout
        .workspaces
        .iter()
        .filter(|workspace| {
            sessions::split_id(&workspace.id).0 == session
                && !sessions::in_wsl(&workspace.id)
                && !core::is_new_workspace(&workspace.id)
                && !kept(&workspace.id)
        })
        .filter_map(|workspace| Some((workspace, workspace.worktree.as_ref()?)))
        .collect();
    let main_of = |key: &str| {
        candidates
            .iter()
            .find(|(_, tree)| !tree.is_linked_worktree && tree.key == key)
            .map(|(workspace, _)| workspace.id.clone())
    };
    let here = candidates
        .iter()
        .filter(|(workspace, _)| !workspace.cwd.is_empty() && Path::new(&workspace.cwd) == cwd)
        .min_by_key(|(_, tree)| tree.is_linked_worktree);
    if let Some((workspace, tree)) = here {
        if !tree.is_linked_worktree {
            return Some(Source {
                workspace: workspace.id.clone(),
                base: None,
            });
        }
        let branch = workspace.branch.as_deref().filter(|b| !b.is_empty())?;
        return Some(Source {
            workspace: main_of(&tree.key)?,
            base: Some(format!("refs/heads/{branch}")),
        });
    }
    // A repository's key is its Git directory, `.git` in its root.
    candidates
        .iter()
        .find(|(_, tree)| {
            !tree.is_linked_worktree
                && Path::new(&tree.key).ends_with(".git")
                && Path::new(&tree.key).parent() == Some(cwd)
        })
        .map(|(workspace, _)| Source {
            workspace: workspace.id.clone(),
            base: None,
        })
}

#[cfg(test)]
mod tests;
