use super::*;
use crate::core_terminals::panes::Panes;
use sheprd_backend::TerminalId;

fn terminal(id: &str) -> Option<Location> {
    let id = TerminalId::new(id).unwrap();
    Some(Location::core_terminal(Panes::single(id), false))
}

#[test]
fn only_tabs_that_type_into_a_kept_workspaces_terminals_are_shared() {
    let scope = Scope::endpoint("ssh:box");
    let mut store = Store::default();
    let kept = store
        .keep_workspace(&scope, "api", "/src/api", false)
        .unwrap();
    let shared = store
        .open(scope.clone(), &kept, terminal("t1"), None)
        .unwrap();
    let mirror = Location::core_terminal(Panes::single(TerminalId::new("t1").unwrap()), true);
    store
        .open(scope.clone(), &kept, Some(mirror), None)
        .unwrap();
    store
        .open(scope.clone(), &kept, url("http://localhost:3000"), None)
        .unwrap();
    // A Herdr 1 workspace's terminal tab is not the document's.
    store
        .open(scope.clone(), "w_1", terminal("t2"), None)
        .unwrap();

    assert!(store.unnamed_shared_tabs(&scope));
    assert!(store.name_shared_tabs(&scope, || "tab-1".to_owned()));
    assert!(!store.unnamed_shared_tabs(&scope));
    assert!(!store.name_shared_tabs(&scope, || "tab-2".to_owned()));
    let view = store.shared_view(&scope);
    assert_eq!(view.workspaces.len(), 1);
    assert_eq!(view.tabs.len(), 1);
    assert_eq!(view.tabs[0].id, "tab-1");
    assert_eq!(store.get(shared).unwrap().space.as_deref(), Some("tab-1"));
    assert!(store.shared_view(&Scope::endpoint("other")).tabs.is_empty());
}

#[test]
fn a_tabs_document_id_and_what_was_synced_survive_a_restart() {
    let scope = Scope::endpoint("ssh:box");
    let mut store = Store::default();
    let kept = store
        .keep_workspace(&scope, "api", "/src/api", false)
        .unwrap();
    store
        .open(scope.clone(), &kept, terminal("t1"), None)
        .unwrap();
    store.name_shared_tabs(&scope, || "tab-1".to_owned());
    let synced = store.shared_view(&scope).synced(3);
    store.set_synced(&scope, Some(synced.clone()));
    let bytes = serde_json::to_vec(&Saved {
        synced: store.synced.clone(),
        tabs: store.tabs.clone(),
        workspaces: store.workspaces.clone(),
    })
    .unwrap();
    let read = parse(&bytes).unwrap();
    assert_eq!(read.tabs[0].space.as_deref(), Some("tab-1"));
    assert_eq!(read.synced.get(scope.as_str()), Some(&synced));
    // Files from before carry neither.
    let old = br#"{"tabs": []}"#;
    assert!(parse(old).unwrap().synced.is_empty());
}
