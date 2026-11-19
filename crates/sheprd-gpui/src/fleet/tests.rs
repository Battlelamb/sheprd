use super::*;
use std::time::{Duration, Instant};

mod scrub;

fn link(standing: Standing, ago: Duration, synced: &[&str]) -> (Link, Instant) {
    let now = Instant::now() + Duration::from_secs(7200);
    let link = Link {
        heads: Vec::new(),
        synced: synced.iter().map(|head| (*head).to_owned()).collect(),
        last: Some(now - ago),
        running: false,
        standing,
    };
    (link, now)
}

#[test]
fn a_member_syncs_when_this_computer_moved_or_now_and_then() {
    let mine = vec!["h2".to_owned()];
    let (fresh, now) = link(Standing::Member, Duration::from_secs(5), &["h2"]);
    assert!(!fresh.due(&mine, now));
    let (moved, now) = link(Standing::Member, Duration::from_secs(5), &["h1"]);
    assert!(moved.due(&mine, now));
    let (stale, now) = link(Standing::Member, REFRESH, &["h2"]);
    assert!(stale.due(&mine, now));
}

#[test]
fn machines_outside_the_fleet_are_asked_rarely() {
    let mine = vec!["h1".to_owned()];
    let (outside, now) = link(Standing::Outside, Duration::from_secs(600), &[]);
    assert!(!outside.due(&mine, now));
    let (outside, now) = link(Standing::Outside, OUTSIDE, &[]);
    assert!(outside.due(&mine, now));
    let (failed, now) = link(Standing::Failed, RETRY, &[]);
    assert!(failed.due(&mine, now));
    let (mut running, now) = link(Standing::Unknown, OUTSIDE, &[]);
    running.running = true;
    assert!(!running.due(&mine, now));
    assert!(Link::default().due(&mine, now));
}

#[test]
fn machines_show_their_target() {
    let ssh = ConnectTarget::Ssh {
        target: "me@build-01".into(),
        session: "default".into(),
    };
    assert_eq!(label("id", &ssh), "me@build-01");
    assert_eq!(label("id", &ConnectTarget::Local), "id");
}

#[test]
fn a_fleet_host_keeps_one_saved_device_id() {
    let host = "0123456789abcdef0123456789ABCDEF0123456789abcdef0123456789abcdef";
    assert_eq!(
        saved::device_id(host).as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(saved::device_id("short"), None);
}

#[test]
fn an_answer_reads_or_says_why_not() {
    let applied = remote::parse_applied(br#"{"fleet":"f","heads":["h"],"ops":[]}"#).unwrap();
    assert_eq!(applied.fleet, "f");
    assert!(matches!(
        remote::parse_applied(b"nope"),
        Err(Error::Answer(_))
    ));
}
