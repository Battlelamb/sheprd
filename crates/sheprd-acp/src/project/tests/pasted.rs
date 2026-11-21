//! Prompts show without the envelope agents wrap pasted text in.

use super::*;

const WRAPPED: &str = "<pasted_content id=\"0e15\">\nTask: ship it\n</pasted_content>";

#[test]
fn a_prompt_shows_without_its_paste_envelope() {
    let mut projector = Projector::default();
    let Change::Push { entry } = projector.prompt(WRAPPED, 0) else {
        panic!("a prompt is pushed");
    };
    assert_eq!(entry.kind.text(), Some("Task: ship it"));
}

#[test]
fn a_prompt_with_files_keeps_its_text_as_written() {
    let mut projector = Projector::default();
    let text = format!("{WRAPPED}\n{}a.png", sheprd_chat_protocol::ATTACHED);
    let Change::Push { entry } = projector.prompt(&text, 1) else {
        panic!("a prompt is pushed");
    };
    assert_eq!(entry.kind.text(), Some(text.as_str()));
}

#[test]
fn a_replayed_prompt_shows_without_its_paste_envelope() {
    let mut projector = Projector::default();
    let transcript = Transcript::new("Claude");
    let change = projector
        .text(&transcript, Stream::User, None, WRAPPED.to_owned())
        .unwrap();
    let Change::Push { entry } = change else {
        panic!("a prompt is pushed");
    };
    assert_eq!(entry.kind.text(), Some("Task: ship it"));
}
