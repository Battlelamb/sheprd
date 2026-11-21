#![allow(clippy::unwrap_used)]
use super::*;
use core::prelude::v1::test;

#[test]
fn requests_are_numbered_apart() {
    let first = next_request();
    let second = next_request();
    assert!(second > first);
}

#[test]
fn dropping_a_request_stops_its_work() {
    let cancelled = Arc::new(AtomicBool::new(false));
    drop(Running {
        way: Way::Claude,
        request: 1,
        cancelled: cancelled.clone(),
    });
    assert!(cancelled.load(Ordering::Acquire));
}

#[test]
fn claude_reads_the_end_of_each_source_and_no_more_than_fits() {
    let long = format!("start {} end", "x".repeat(10_000));
    let kept = work::tail(&long);
    assert!(kept.ends_with("x end"));
    assert!(!kept.contains("start"));
    assert_eq!(work::tail("short"), "short");
    let input = work::bounded(vec!["a".repeat(7_000), "b".repeat(7_000), "c".into()]);
    assert!(input.chars().count() <= sheprd_chat_protocol::excerpt::MAX_INPUT_CHARS);
    assert!(input.starts_with('a'));
    assert!(
        !input.contains('b'),
        "a part that does not fit ends the input"
    );
    assert_eq!(work::bounded(vec!["a".into(), "b".into()]), "a\n\nb");
}

#[test]
fn a_known_terminal_agent_is_called_as_its_chats_are() {
    assert_eq!(sources::display_name("claude"), "Claude");
    assert_eq!(sources::display_name("opencode"), "OpenCode");
    assert_eq!(sources::display_name("Claude"), "Claude");
    assert_eq!(sources::display_name("droid"), "droid");
}
