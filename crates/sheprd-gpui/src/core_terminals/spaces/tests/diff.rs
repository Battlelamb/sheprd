use super::super::diff::edits;
use super::*;
use crate::browser::shared::View;
use sheprd_session::spaces::{Tab, Workspace};

fn workspace(id: &str) -> Workspace {
    Workspace {
        id: id.to_owned(),
        label: id.to_owned(),
        custom: false,
        cwd: String::new(),
    }
}

fn shared_tab(id: &str, workspace: &str, terminal: &str) -> Tab {
    Tab {
        id: id.to_owned(),
        workspace_id: workspace.to_owned(),
        title: String::new(),
        name: None,
        names: Default::default(),
        panes: Panes::single(super::id(terminal)),
        zoomed: None,
    }
}

fn view(workspaces: &[&str], tabs: Vec<Tab>) -> View {
    View {
        tabs,
        workspaces: workspaces.iter().map(|id| workspace(id)).collect(),
    }
}

#[test]
fn nothing_changed_is_no_edit() {
    let base = view(&["a", "b"], vec![shared_tab("t", "a", "t1")]);
    assert!(edits(&base, &base.clone()).is_empty());
}

#[test]
fn new_and_gone_records_are_kept_put_closed_and_forgotten() {
    let base = view(&["a", "b"], vec![shared_tab("t", "b", "t1")]);
    let local = view(&["a", "c"], vec![shared_tab("u", "c", "t2")]);
    assert_eq!(
        edits(&base, &local),
        [
            Edit::KeepWorkspace(workspace("c")),
            Edit::PutTab(shared_tab("u", "c", "t2")),
            Edit::CloseTab { id: "t".into() },
            Edit::ForgetWorkspace { id: "b".into() },
        ]
    );
}

#[test]
fn only_a_name_the_user_gave_is_written() {
    let base = view(&["a"], Vec::new());
    let mut local = base.clone();
    local.workspaces[0].label = "named".into();
    assert!(edits(&base, &local).is_empty());
    local.workspaces[0].custom = true;
    assert_eq!(
        edits(&base, &local),
        [Edit::RenameWorkspace {
            id: "a".into(),
            label: "named".into()
        }]
    );
}

#[test]
fn a_new_order_moves_each_workspace_to_its_place() {
    let base = view(&["a", "b", "c"], Vec::new());
    let local = view(&["c", "a", "b"], Vec::new());
    let made = edits(&base, &local);
    let mut spaces = Spaces::default();
    for id in ["a", "b", "c"] {
        spaces.apply(Edit::KeepWorkspace(workspace(id))).unwrap();
    }
    for edit in made {
        spaces.apply(edit).unwrap();
    }
    let order: Vec<_> = spaces.workspaces().iter().map(|w| w.id.as_str()).collect();
    assert_eq!(order, ["c", "a", "b"]);
}

#[test]
fn a_title_alone_is_not_a_change() {
    let base = view(&["a"], vec![shared_tab("t", "a", "t1")]);
    let mut local = base.clone();
    local.tabs[0].title = "vim".into();
    assert!(edits(&base, &local).is_empty());
    local.tabs[0].name = Some("editor".into());
    assert!(matches!(&edits(&base, &local)[..], [Edit::PutTab(_)]));
}
