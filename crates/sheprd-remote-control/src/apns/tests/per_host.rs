//! Alerts from several computers to one phone: the same pane on two of
//! them neither replaces nor groups with the other's, while a newer alert
//! for one chat replaces its older one.

use super::alert::{COMPUTER, device, fixed_seal, json};
use crate::apns::{
    alert::{Sealed, clear, collapse_id, push, thread_id},
    seal,
};
use crate::push::{Kind, Notification, Urgency};
use sheprd_apns::SealedPush;

const OTHER: &str = "0D7C1E5A9B3F2C4D6E8F0A1B2C3D4E5F60718293A4B5C6D7E8F90A1B2C3D4E5F";

fn asking(pane: &str) -> Notification {
    Notification {
        body: "Run tests?".to_owned(),
        clear: false,
        kind: Kind::Question,
        pane: pane.to_owned(),
        permission: None,
        tag: format!("chat-{pane}"),
        title: "Claude needs an answer".to_owned(),
        urgency: Urgency::High,
        wizz: false,
    }
}

fn sent(computer: &str, notification: &Notification) -> (Option<String>, String) {
    let sealed = Sealed::new(notification, computer.to_owned(), 1);
    let push = push(sealed, &notification.tag, fixed_seal).unwrap();
    let thread = json(&push.body)["aps"]["thread-id"]
        .as_str()
        .unwrap()
        .to_owned();
    (push.collapse_id, thread)
}

#[test]
fn the_same_pane_on_two_computers_never_collapses_or_groups() {
    let question = asking("w1:p2");
    let (here, here_thread) = sent(COMPUTER, &question);
    let (there, there_thread) = sent(OTHER, &question);
    assert_ne!(here, there);
    assert_ne!(here_thread, there_thread);
    // The app's own thread, `AlertKeys.thread(computer:pane:)`.
    assert_eq!(here_thread, format!("{COMPUTER}/w1:p2"));
}

#[test]
fn a_newer_alert_for_one_chat_still_replaces_the_older() {
    let first = asking("w1:p2");
    let finished = Notification {
        kind: Kind::TurnEnded,
        title: "Claude finished".to_owned(),
        urgency: Urgency::Normal,
        ..first.clone()
    };
    assert_eq!(sent(COMPUTER, &first), sent(COMPUTER, &finished));
    assert_ne!(sent(COMPUTER, &first).0, sent(COMPUTER, &asking("w1:p3")).0);
}

#[test]
fn every_collapse_id_fits_apns() {
    for tag in ["chat-w1:p2", &format!("chat-{}", "x".repeat(200))] {
        assert!(collapse_id(COMPUTER, tag).len() <= 64);
    }
}

#[test]
fn the_push_relay_accepts_the_computer_in_the_clear_thread() {
    let question = asking("w1:p2");
    let sealed = Sealed::new(&question, COMPUTER.to_owned(), 1);
    let allowed = clear(COMPUTER, "w1:p2", &question.tag);
    assert!(allowed.contains(&thread_id(COMPUTER, "w1:p2")));
    assert!(allowed.contains(&collapse_id(COMPUTER, &question.tag)));
    SealedPush::build(&device().public_key(), seal::INFO, &allowed, |sealer| {
        push(sealed, &question.tag, |plaintext| {
            Ok(sealer.seal(plaintext)?)
        })
    })
    .unwrap();
}
