//! A kept workspace follows its first driving terminal across `cd`, even
//! when an empty split group has focus. User names remain user names.

use crate::{
    browser::Store,
    core_terminals::{
        Purpose,
        tests::kept_workspaces::{core_tab, window},
    },
};
use gpui::TestAppContext;
use sheprd_backend::{RunState, Spawn, TerminalId, TerminalInfo};
use sheprd_chat_protocol::Agent;
use sheprd_client::protocol::AgentStatus;
use std::path::{Path, PathBuf};

#[gpui::test]
fn live_folder_drives_metadata_empty_groups_and_chat_tabs(cx: &mut TestAppContext) {
    let (view, cx) = window(cx, false);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let workspace = view.open_kept_workspace(None, Path::new("/old"), core_tab("t1"), window, cx).unwrap();
            let scope = crate::browser::scope(&view.endpoints[0]);
            let tab = cx.global::<Store>().in_workspace(&scope, &workspace).next().unwrap().id;
            let terminal = TerminalId::new("t1").unwrap();
            view.core_terminals.show_for_test(tab, crate::core_terminals::panes::Panes::single(terminal.clone()), false, vec![TerminalInfo {
                agent: AgentStatus::Unknown,
                cwd: Some(PathBuf::from("/work/repo")),
                foreground: None,
                id: terminal,
                labels: Default::default(),
                name: None,
                reporter: None,
                state: RunState::Running,
                title: None,
            }]);
            view.core_terminals.seed_checkout("/work/repo", Some(sheprd_vcs::git::Identity {
                branch: Some("feature".into()), key: "/work/repo/.git".into(), label: "repo".into(), linked: false,
            }));
            view.refresh_core_session(cx);
            let sessions = view.sessions(0);
            let current = sessions.workspace(&workspace).unwrap();
            assert_eq!(current.cwd, "/work/repo");
            assert_eq!(current.label, "repo");
            assert_eq!(current.branch.as_deref(), Some("feature"));
            view.rename_kept_workspace(&workspace, "Chosen name", cx);
            assert_eq!(view.sessions(0).workspace(&workspace).unwrap().label, "Chosen name");

            view.split_active_group(window, cx);
            assert!(view.shown_core_pane(cx).is_none());
            assert!(matches!(view.group_shown(view.active_group().unwrap(), cx), crate::browser::Shown::Empty),
                "an empty Herdr 2 group must not expose Herdr 1's reconnect screen");
            let mut spawn = Spawn::default();
            view.give_core_context(&Purpose::Tab { scope: scope.clone(), workspace: workspace.clone() }, &crate::core_terminals::Host::Local, &mut spawn, cx);
            assert_eq!(spawn.cwd.as_deref(), Some(Path::new("/work/repo")));

            view.config.features.acp_chats = true;
            view.chats.program = crate::chat::Program::Found("/opt/sheprd-acp".into());
            view.start_core_chat_in_tab(Agent::Codex, tab, cx).unwrap();
            let (purpose, _, spawn, _) = view.core_terminals.wanted.back().unwrap();
            assert!(matches!(purpose, Purpose::Tab { scope: actual, workspace: id } if actual == &scope && id == &workspace));
            assert_eq!(spawn.cwd.as_deref(), Some(Path::new("/work/repo")));
        })
    });
}
