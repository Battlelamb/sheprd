//! Which Herdr 1 workspace a phone's new worktree grows from.

use super::{Worktree, herdr_1_repository};
use crate::hub::{Layout, Workspace};
use sheprd_client::protocol::ClientShellWorktree;

fn workspace(id: &str, cwd: &str, linked: Option<bool>) -> Workspace {
    Workspace {
        branch: None,
        chats: Vec::new(),
        cwd: cwd.to_owned(),
        id: id.to_owned(),
        label: id.to_owned(),
        session: None,
        terminals: Vec::new(),
        worktree: linked.map(|linked| ClientShellWorktree {
            key: "/code/app/.git".to_owned(),
            label: "app".to_owned(),
            is_linked_worktree: linked,
        }),
    }
}

fn layout(workspaces: Vec<Workspace>) -> Layout {
    Layout {
        workspaces,
        ..Layout::default()
    }
}

fn kept_none(_: &str) -> bool {
    false
}

#[test]
fn the_repository_checkout_at_the_folder_grows_the_worktree() {
    let layout = layout(vec![
        workspace("w2", "/code/app", Some(true)),
        workspace("w1", "/code/app", Some(false)),
        workspace("w3", "/code/other", Some(false)),
    ]);
    assert_eq!(
        herdr_1_repository(&layout, "/code/app", None, kept_none).as_deref(),
        Some("w1"),
        "the repository's own checkout ahead of a linked worktree there"
    );
    assert_eq!(
        herdr_1_repository(&layout, "/code/app/", None, kept_none).as_deref(),
        Some("w1"),
        "the same folder written with a trailing slash"
    );
}

#[test]
fn the_repository_root_finds_its_checkout_shown_from_a_subfolder() {
    let layout = layout(vec![workspace("w1", "/code/app/crates", Some(false))]);
    assert_eq!(
        herdr_1_repository(&layout, "/code/app", None, kept_none).as_deref(),
        Some("w1")
    );
    let linked = layout_of_linked();
    assert_eq!(
        herdr_1_repository(&linked, "/code/app", None, kept_none),
        None
    );
}

/// Only a linked worktree of the repository at `/code/app` is shown.
fn layout_of_linked() -> Layout {
    layout(vec![workspace("w2", "/code/worktrees/topic", Some(true))])
}

#[test]
fn a_folder_herdr_1_shows_as_no_repository_grows_nothing() {
    let layout = layout(vec![
        workspace("w1", "/code/plain", None),
        workspace("space-3", "/code/kept", Some(false)),
        workspace("herdr:dev:w4", "/code/elsewhere", Some(false)),
    ]);
    let kept = |id: &str| id == "space-3";
    assert_eq!(herdr_1_repository(&layout, "/code/plain", None, kept), None);
    assert_eq!(herdr_1_repository(&layout, "/code/kept", None, kept), None);
    assert_eq!(herdr_1_repository(&layout, "/code/none", None, kept), None);
}

#[test]
fn a_repository_is_looked_for_in_the_session_asked_for() {
    let layout = layout(vec![
        workspace("w1", "/code/app", Some(false)),
        workspace("herdr:dev:w1", "/code/app", Some(false)),
    ]);
    assert_eq!(
        herdr_1_repository(&layout, "/code/app", Some("dev"), kept_none).as_deref(),
        Some("herdr:dev:w1")
    );
    assert_eq!(
        herdr_1_repository(&layout, "/code/app", None, kept_none).as_deref(),
        Some("w1")
    );
    assert_eq!(
        herdr_1_repository(&layout, "/code/app", Some("other"), kept_none),
        None
    );
}

#[test]
fn a_title_names_the_herdr_1_worktree_workspace() {
    let worktree = Worktree {
        branch: "topic",
        repository: "w1",
        title: Some("Review"),
    };
    let params = worktree.params();
    assert_eq!(params["workspace_id"], "w1");
    assert_eq!(params["branch"], "topic");
    assert_eq!(params["focus"], false);
    assert_eq!(params["label"], "Review");
    let untitled = Worktree {
        title: None,
        ..worktree
    };
    assert!(untitled.params().get("label").is_none());
}
