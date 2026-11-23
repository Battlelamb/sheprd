#![allow(clippy::unwrap_used)]

use super::*;

fn herdr(pane: &str) -> Place {
    Place::Herdr {
        endpoint: "local".to_owned(),
        pane: pane.to_owned(),
    }
}

#[test]
fn a_draft_belongs_to_its_chat_and_blank_text_removes_it() {
    let mut drafts = Drafts::default();
    assert!(drafts.set(&herdr("p1"), "chat-a", "half a thought"));
    assert_eq!(drafts.text(&herdr("p1"), "chat-a"), Some("half a thought"));
    // Another chat later in the same pane does not get it.
    assert_eq!(drafts.text(&herdr("p1"), "chat-b"), None);
    assert!(drafts.marks.contains(&herdr("p1")));
    // The same text again changes nothing.
    assert!(!drafts.set(&herdr("p1"), "chat-a", "half a thought"));
    // Sending clears the composer, which removes the draft and its pencil.
    assert!(drafts.set(&herdr("p1"), "chat-a", "  \n"));
    assert_eq!(drafts.text(&herdr("p1"), "chat-a"), None);
    assert!(drafts.marks.is_empty());
    assert!(!drafts.set(&herdr("p1"), "chat-a", ""));
}

#[test]
fn another_chat_in_the_pane_replaces_the_old_draft() {
    let mut drafts = Drafts::default();
    drafts.set(&herdr("p1"), "chat-a", "old");
    assert!(drafts.set(&herdr("p1"), "chat-b", ""));
    assert!(drafts.drafts.is_empty());
}

#[test]
fn the_marks_are_shared_until_a_pencil_comes_or_goes() {
    let mut drafts = Drafts::default();
    drafts.set(&herdr("p1"), "chat-a", "h");
    let marks = drafts.marks.clone();
    drafts.set(&herdr("p1"), "chat-a", "he");
    drafts.set(&herdr("p1"), "chat-a", "hey");
    assert!(Arc::ptr_eq(&marks, &drafts.marks));
    drafts.set(
        &Place::Core {
            key: "core:t1".to_owned(),
        },
        "chat-c",
        "x",
    );
    assert!(!Arc::ptr_eq(&marks, &drafts.marks));
    assert_eq!(drafts.marks.len(), 2);
}

#[test]
fn drafts_are_bounded() {
    let mut drafts = Drafts::default();
    let long = "é".repeat(MAX_TEXT_BYTES);
    drafts.set(&herdr("p1"), "chat-a", &long);
    let kept = drafts.text(&herdr("p1"), "chat-a").unwrap();
    assert!(kept.len() <= MAX_TEXT_BYTES && long.starts_with(kept));
    for index in 0..MAX_DRAFTS * 2 {
        drafts.set(&herdr(&format!("pane-{index}")), "chat", "text");
    }
    assert_eq!(drafts.drafts.len(), MAX_DRAFTS);
    // The least recently edited went first.
    assert_eq!(drafts.text(&herdr("p1"), "chat-a"), None);
    let field = "x".repeat(MAX_FIELD_BYTES + 1);
    assert!(!drafts.set(&herdr(&field), "chat", "text"));
    assert!(!drafts.set(&herdr("p2"), &field, "text"));
    assert!(!drafts.set(&herdr("p2"), "", "text"));
}

#[test]
fn drafts_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE);
    let mut drafts = Drafts::at(Some(path.clone()));
    drafts.set(&herdr("p1"), "chat-a", "line one\nline two");
    drafts.set(
        &Place::Core {
            key: "core:t1".to_owned(),
        },
        "chat-c",
        "core",
    );
    drafts.finish();
    let restored = Drafts::at(Some(path));
    assert_eq!(
        restored.text(&herdr("p1"), "chat-a"),
        Some("line one\nline two")
    );
    assert_eq!(
        restored.text(
            &Place::Core {
                key: "core:t1".to_owned()
            },
            "chat-c"
        ),
        Some("core")
    );
    assert_eq!(restored.marks.len(), 2);
}

#[test]
fn damaged_or_invalid_files_keep_nothing_wrong() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE);
    std::fs::write(&path, b"{not json").unwrap();
    assert!(Drafts::at(Some(path.clone())).drafts.is_empty());
    // A blank draft is dropped, and of two in one place the later wins.
    std::fs::write(
        &path,
        br#"{"drafts":[
            {"place":{"kind":"herdr","endpoint":"local","pane":"p1"},"chat":"a","text":"old"},
            {"place":{"kind":"core","key":"core:t1"},"chat":"c","text":"  "},
            {"place":{"kind":"herdr","endpoint":"local","pane":"p1"},"chat":"b","text":"new"}
        ]}"#,
    )
    .unwrap();
    let drafts = Drafts::at(Some(path));
    assert_eq!(drafts.drafts.len(), 1);
    assert_eq!(drafts.text(&herdr("p1"), "b"), Some("new"));
    assert_eq!(drafts.marks.len(), 1);
}
