//! What is typed in a chat's composer and not sent is kept, comes back in a
//! new composer for the same chat, as after a restart, and goes once sent.

use super::{
    overlay::{attach_with, open, poll},
    *,
};
use crate::SheprdWindow;
use crate::chat::drafts::{Drafts, Place};
use gpui::{Entity, TestAppContext, VisualTestContext};
use sheprd_chat_protocol::Command;
use sheprd_client::ConnectTarget;

fn idle() -> ServerMessage {
    let mut transcript = Transcript::new("Claude");
    transcript.status = Status::Idle;
    transcript.entries = vec![agent(1, "Hi.")];
    ServerMessage::Snapshot {
        cursor: cursor(1, 0),
        transcript: Box::new(transcript),
    }
}

fn composer_text(view: &Entity<SheprdWindow>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        let view = view.read(cx);
        let pane = view.chats.panes().next()?;
        Some(pane.composer.as_ref()?.read(cx).text().to_owned())
    })
}

#[gpui::test]
fn an_unsent_draft_comes_back_and_goes_once_sent(cx: &mut TestAppContext) {
    let (view, _peer, cx) = open(cx, ConnectTarget::Local);
    let program = Program::new();
    let mut stream = attach_with(&view, &program, idle(), cx);
    cx.simulate_input("half a thought");
    poll(&view, &program, cx);
    let place = cx.update(|_, cx| {
        let view = view.read(cx);
        Place::Herdr {
            endpoint: view.endpoints[view.selected_endpoint].id.clone(),
            pane: view.chats.panes().next().unwrap().target.pane_id.clone(),
        }
    });
    let marked = cx.update(|_, cx| Drafts::current(cx));
    assert!(marked.contains(&place), "the draft is kept and marked");
    view.read_with(cx, |view, _| {
        assert!(view.chat_drafts.contains(&place), "the window marks it");
    });

    // A new composer for the same chat, as after a restart, starts with it.
    view.update(cx, |view, _| {
        for pane in view.chats.panes_mut() {
            pane.composer = None;
            pane.composer_typed = None;
        }
    });
    poll(&view, &program, cx);
    assert_eq!(composer_text(&view, cx).as_deref(), Some("half a thought"));

    // Sent, it goes, and so does its pencil.
    cx.update(|window, cx| {
        let focus = view
            .read(cx)
            .chats
            .panes()
            .next()
            .unwrap()
            .input_focus(cx)
            .unwrap();
        window.focus(&focus, cx);
    });
    cx.simulate_keystrokes("enter");
    match read(&mut stream) {
        Some(ClientMessage::Command {
            command: Command::Prompt { text, .. },
            ..
        }) => assert_eq!(text, "half a thought"),
        other => panic!("a prompt, got {other:?}"),
    }
    poll(&view, &program, cx);
    let marked = cx.update(|_, cx| Drafts::current(cx));
    assert!(!marked.contains(&place), "a sent draft is not kept");
}
