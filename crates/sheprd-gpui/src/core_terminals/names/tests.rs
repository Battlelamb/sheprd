use crate::core_terminals::{checkouts::Checkouts, names::apply};
use sheprd_session::{CoreWorkspace, Session, Worktree};
use sheprd_vcs::git::Identity;
use std::collections::BTreeMap;

fn session(custom_label: bool, linked: bool) -> Session {
    Session::core(
        [CoreWorkspace {
            branch: Some("main"),
            cwd: "/original",
            custom_label,
            id: "root",
            label: "original",
            worktree: Some(Worktree {
                key: "/original/.git".into(),
                label: "original".into(),
                linked,
            }),
        }],
        [],
        None,
        |_| None,
    )
}

#[test]
fn root_names_follow_checkouts_and_plain_folders_without_changing_identity() {
    let mut checkouts = Checkouts::default();
    for (cwd, root) in [
        ("/code/herdr-app/src", "/code/herdr-app"),
        ("/trees/feature/src", "/trees/feature"),
    ] {
        checkouts.seed(
            cwd,
            Some(Identity {
                branch: Some("feature".into()),
                key: "/code/herdr-app/.git".into(),
                label: "herdr-app".into(),
                linked: root.starts_with("/trees"),
                root: root.into(),
            }),
        );
    }
    for custom in [false, true] {
        for linked in [false, true] {
            let mut session = session(custom, linked);
            let original = session.workspaces[0].clone();
            for (cwd, expected) in [
                ("/code/herdr-app/src", "herdr-app"),
                ("/trees/feature/src", "feature"),
                ("/tmp/notes", "notes"),
                ("/Users/me", "~"),
            ] {
                let folders = BTreeMap::from([("root".into(), Some(cwd.into()))]);
                apply(&mut session, &folders, Some(&checkouts), "/Users/me");
                let mut expected_workspace = original.clone();
                if !custom && !linked {
                    expected_workspace.label = expected.into();
                }
                assert_eq!(session.workspaces[0], expected_workspace);
            }
        }
    }
}

#[test]
fn missing_live_folders_use_the_saved_folder_and_stale_answers_are_not_used() {
    let mut checkouts = Checkouts::default();
    checkouts.seed(
        "/old/src",
        Some(Identity {
            branch: None,
            key: "/old/.git".into(),
            label: "old".into(),
            linked: false,
            root: "/old".into(),
        }),
    );
    let mut session = session(false, false);
    for cwd in [None, Some("/new/src")] {
        let folders = BTreeMap::from([("root".into(), cwd.map(str::to_owned))]);
        apply(&mut session, &folders, Some(&checkouts), "");
        assert_eq!(
            session.workspaces[0].label,
            if cwd.is_some() { "src" } else { "original" }
        );
    }
}
