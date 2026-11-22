use crate::chat::tests::file_links::document_link;
use crate::chat::tests::visual::message;
use crate::chat::tests::{Program, agent, cursor, overlay, snapshot, wait_until};
use gpui::{Modifiers, TestAppContext};
use sheprd_client::ConnectTarget;

#[test]
fn standalone_files_become_cards_without_reinterpreting_prose_or_code() {
    for name in [
        "clip.mp4",
        "sheet.xlsx",
        "report.pdf",
        "archive.zip",
        "data.unknown",
    ] {
        let (_, shape) = message(&format!("Here it is:\n\n[{name}](/tmp/{name})\n\nDone."));
        assert_eq!(
            shape,
            [
                "text:Here it is:".to_owned(),
                format!("file:/tmp/{name}"),
                "text:Done.".to_owned()
            ]
        );
    }
    for text in [
        "See [clip](/tmp/clip.mp4) here.",
        "[source](/tmp/main.rs:12)",
        "[website](https://example.com)",
        "[unsafe](javascript:alert)",
        "[network](file://other-host/share/clip.mp4)",
        "`[clip](/tmp/clip.mp4)`",
        "```md\n[clip](/tmp/clip.mp4)\n```",
        "    [clip](/tmp/clip.mp4)",
        "[unfinished](/tmp/clip.mp4",
        "[one](/tmp/one) [two](/tmp/two)",
    ] {
        let (_, shape) = message(text);
        assert!(
            !shape.iter().any(|part| part.starts_with("file:")),
            "{text}"
        );
    }
    let (_, shape) = message("[Home — **clip**](</tmp/Home (Window).mp4>)");
    assert_eq!(shape, ["file:/tmp/Home (Window).mp4"]);
}

#[test]
fn file_cards_are_bounded_and_excess_links_stay_visible() {
    let limit = sheprd_chat_protocol::MAX_ATTACHMENTS;
    let source = (0..limit + 1)
        .map(|i| format!("[file-{i}](/tmp/file-{i})\n"))
        .collect::<String>();
    let (_, shape) = message(&source);
    assert_eq!(
        shape
            .iter()
            .filter(|part| part.starts_with("file:"))
            .count(),
        limit
    );
    assert_eq!(shape.last().unwrap(), &format!("text:file-{limit}"));
}

#[gpui::test]
fn video_card_opens_on_click_in_the_system_viewer_even_with_editor_preference(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Home (Window).mp4");
    std::fs::write(&path, b"\0\0\0\x18ftypmp42").unwrap();
    let source = document_link("Home video", &path);
    let (view, _peer, cx) = overlay::open(cx, ConnectTarget::Local);
    view.update(cx, |view, _| {
        view.config.open_files_in = crate::config::FileTarget::Editor;
    });
    let program = Program::new();
    let _stream = overlay::attach_with(
        &view,
        &program,
        snapshot(cursor(1, 0), vec![agent(1, &source)]),
        cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("chat-file-card").is_some());
    assert!(cx.opened_url().is_none(), "rendering never opens the file");
    let button = cx.debug_bounds("chat-file-open").unwrap();
    cx.simulate_click(button.center(), Modifiers::default());
    wait_until(|| {
        cx.run_until_parked();
        cx.opened_url().is_some()
    });
    let opened = url::Url::parse(&cx.opened_url().unwrap())
        .unwrap()
        .to_file_path()
        .unwrap();
    assert_eq!(
        std::fs::canonicalize(opened).unwrap(),
        std::fs::canonicalize(path).unwrap()
    );
}
