//! The name of the terminals no tab holds, beside the workspaces kept.

use super::*;

fn unlisted_label(workspaces: Value) -> String {
    let saved = serde_json::from_value(json!({"workspaces": workspaces, "tabs": []})).unwrap();
    let records = [serde_json::from_value(record("t1")).unwrap()];
    let layout = projection::project("fake-core", &records, Some(&saved), Some("s"));
    layout.workspace(UNLISTED).unwrap().label.clone()
}

#[test]
fn unlisted_terminals_are_not_named_like_the_default_workspace() {
    let label = unlisted_label(json!([
        {"id": "herdr-2", "scope": "s", "label": "Herdr 2", "cwd": ""}
    ]));
    assert_eq!(label, "Other terminals");
}

#[test]
fn unlisted_terminals_keep_their_name_with_no_namesake() {
    let label = unlisted_label(json!([
        {"id": "space-1", "scope": "s", "label": "api", "cwd": "/code/api"}
    ]));
    assert_eq!(label, "Herdr 2");
}
