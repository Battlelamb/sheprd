//! Unread rows: a chat's reply stays unread until someone reads it, on any
//! device, and the next reply brings it back.

use super::attention::{chat, finished_at, layout, panes};
use crate::{
    control::read::Change,
    web::attention::{ChatFacts, Opened},
};
use sheprd_client::protocol::AgentStatus;

fn change(pane: &str, read: bool) -> Change {
    Change {
        panes: vec![pane.to_owned()],
        read,
    }
}

#[test]
fn an_idle_chat_with_an_unread_reply_is_unread_and_counts() {
    let opened = Opened::default();
    // Herdr turned done to idle when the computer focused it: the reply
    // is still unread until someone says they read it.
    let idle = layout(vec![chat("p", AgentStatus::Idle)], Vec::new());
    let seen = opened.seen(&idle, finished_at(5));
    assert_eq!(seen.unread, panes(["p"]));
    assert_eq!(seen.needing, panes(["p"]));
}

#[test]
fn reading_clears_until_the_next_reply() {
    let opened = Opened::default();
    let idle = layout(vec![chat("p", AgentStatus::Idle)], Vec::new());
    let unread = opened.change(&idle, &change("p", true), finished_at(5));
    assert_eq!(unread.panes, panes([]));
    assert_eq!(opened.seen(&idle, finished_at(5)).unread, panes([]));
    assert_eq!(opened.seen(&idle, finished_at(6)).unread, panes(["p"]));
}

#[test]
fn marking_unread_by_hand_shows_until_read() {
    let opened = Opened::default();
    let idle = layout(vec![chat("p", AgentStatus::Idle)], Vec::new());
    opened.change(&idle, &change("p", true), finished_at(5));
    let unread = opened.change(&idle, &change("p", false), finished_at(5));
    assert_eq!(unread.panes, panes(["p"]));
    opened.open(&idle, "p", finished_at(5));
    assert_eq!(opened.seen(&idle, finished_at(5)).unread, panes([]));
}

#[test]
fn a_reply_while_a_view_is_open_is_read() {
    let opened = Opened::default();
    let working = layout(vec![chat("p", AgentStatus::Working)], Vec::new());
    let idle = layout(vec![chat("p", AgentStatus::Idle)], Vec::new());
    opened.begin(&working, "p", finished_at(5));
    assert_eq!(opened.seen(&idle, finished_at(6)).unread, panes([]));
}

#[test]
fn a_working_chat_keeps_its_last_reply_read() {
    let opened = Opened::default();
    let idle = layout(vec![chat("p", AgentStatus::Idle)], Vec::new());
    let working = layout(vec![chat("p", AgentStatus::Working)], Vec::new());
    opened.open(&idle, "p", finished_at(5));
    assert_eq!(opened.seen(&working, finished_at(5)).unread, panes([]));
}

#[test]
fn panes_this_computer_does_not_have_are_not_marked() {
    let opened = Opened::default();
    let idle = layout(vec![chat("p", AgentStatus::Idle)], Vec::new());
    let changed = opened.changed();
    opened.change(&idle, &change("elsewhere", false), finished_at(5));
    assert!(!changed.has_changed().unwrap());
    let quiet = |_: &str| ChatFacts::default();
    assert_eq!(opened.seen(&idle, quiet).unread, panes([]));
}

#[test]
fn reading_tells_views_once() {
    let opened = Opened::default();
    let idle = layout(vec![chat("p", AgentStatus::Idle)], Vec::new());
    let mut changed = opened.changed();
    opened.change(&idle, &change("p", true), finished_at(5));
    assert!(changed.has_changed().unwrap());
    changed.mark_unchanged();
    opened.change(&idle, &change("p", true), finished_at(5));
    assert!(!changed.has_changed().unwrap());
}
