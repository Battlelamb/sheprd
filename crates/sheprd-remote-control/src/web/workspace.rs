//! `POST /api/v2/workspaces/{workspace}/close` and `.../remove-worktree`: a
//! phone closes a workspace, or deletes its linked worktree and closes it,
//! as Sheprd's workspace menu does. Herdr 1 does both through its API; a
//! workspace Sheprd keeps for Herdr 2 is Sheprd's to change, while it runs.
//!
//! Deleting is never forced, so a checkout with uncommitted changes stays,
//! and one whose `.herdr/worktree.toml` has an archive script is left for
//! Sheprd, which runs that script first.

use super::{ApiError, Device, Shared};
use crate::{control::layout::Op, hub::ApiMethod};
use axum::{
    Json,
    extract::{Path, State},
};
use serde_json::{Value, json};
use std::io::Read as _;

/// Where a checkout's worktree scripts live, as Sheprd reads them.
const SCRIPTS: &str = ".herdr/worktree.toml";
/// Larger script files are not read, as Sheprd refuses them.
const MAX_SCRIPTS: u64 = 64 * 1024;

/// What a phone does to a workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Close,
    RemoveWorktree,
}

impl Action {
    /// The change Sheprd makes to a workspace it keeps.
    fn op(self, workspace: String) -> Op {
        match self {
            Self::Close => Op::CloseWorkspace { workspace },
            Self::RemoveWorktree => Op::RemoveWorktree { workspace },
        }
    }
}

/// The Herdr 1 method and parameters for `action` on `workspace`. A close
/// closes this workspace alone, never its repository's other checkouts.
pub(crate) fn herdr_call(action: Action, workspace: &str) -> (ApiMethod, Value) {
    match action {
        Action::Close => (
            ApiMethod::WorkspaceClose,
            json!({ "workspace_id": workspace, "close_group": false }),
        ),
        Action::RemoveWorktree => (
            ApiMethod::WorktreeRemove,
            json!({ "workspace_id": workspace, "force": false, "trust_repository": false }),
        ),
    }
}

/// Whether the worktree scripts at `checkout` name an archive script, or
/// cannot be read to tell: either way Sheprd should delete it.
pub(crate) fn needs_archive(checkout: &std::path::Path) -> bool {
    #[derive(serde::Deserialize, Default)]
    struct File {
        #[serde(default)]
        scripts: Scripts,
    }
    #[derive(serde::Deserialize, Default)]
    struct Scripts {
        archive: Option<String>,
    }
    let file = match std::fs::File::open(checkout.join(SCRIPTS)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    let mut text = String::new();
    if file
        .take(MAX_SCRIPTS + 1)
        .read_to_string(&mut text)
        .is_err()
        || text.len() as u64 > MAX_SCRIPTS
    {
        return true;
    }
    toml::from_str::<File>(&text).map_or(true, |file| {
        file.scripts
            .archive
            .is_some_and(|script| !script.trim().is_empty())
    })
}

pub(super) async fn close(
    State(state): State<Shared>,
    _device: Device,
    Path(workspace): Path<String>,
) -> Result<Json<Value>, ApiError> {
    act(&state, workspace, Action::Close).await
}

pub(super) async fn remove_worktree(
    State(state): State<Shared>,
    _device: Device,
    Path(workspace): Path<String>,
) -> Result<Json<Value>, ApiError> {
    act(&state, workspace, Action::RemoveWorktree).await
}

async fn act(state: &Shared, workspace: String, action: Action) -> Result<Json<Value>, ApiError> {
    let (cwd, worktree) = {
        let layout = state.hub.layout();
        let listed = layout.workspace(&workspace).ok_or(ApiError::NotFound)?;
        (listed.cwd.clone(), listed.worktree.clone())
    };
    if action == Action::RemoveWorktree {
        if worktree.is_some_and(|tree| !tree.is_linked_worktree) {
            return Err(ApiError::Invalid("only a linked worktree can be deleted"));
        }
        if cwd.is_empty() {
            return Err(ApiError::Invalid(
                "this workspace has no checkout to delete",
            ));
        }
        let checkout = std::path::PathBuf::from(&cwd);
        let archive = tokio::task::spawn_blocking(move || needs_archive(&checkout))
            .await
            .unwrap_or(true);
        if archive {
            return Err(ApiError::Invalid(
                "this worktree has an archive script; delete it in Sheprd, which runs it first",
            ));
        }
    }
    if state.hub.kept_workspace(&workspace) {
        let keeper = crate::hub::Keeper::Saved {
            tab: 0,
            terminal: String::new(),
        };
        crate::native::changes::arrange(state, keeper, action.op(workspace)).await?;
        return Ok(Json(json!({})));
    }
    let (method, params) = herdr_call(action, &workspace);
    state.hub.call(method, params).await?;
    Ok(Json(json!({})))
}
