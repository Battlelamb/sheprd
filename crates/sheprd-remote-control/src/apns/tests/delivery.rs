//! Which mode sends, what each reaches, and that a relay outage drops
//! pushes at once instead of holding them.

use super::*;
use crate::apns::recipient::Handle;
use p256::pkcs8::{EncodePrivateKey as _, LineEnding};
use sheprd_apns::RelayHost;
use std::time::Duration;

fn handle() -> Recipient {
    Handle::parse(&format!("1a2b3c4d.{}", "A".repeat(160)), RelayHost::Sandbox)
        .unwrap()
        .into()
}

fn relay() -> Delivery {
    Delivery::Relay(Box::new(Relays::new().unwrap()))
}

fn write_private(path: &std::path::Path, contents: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn the_relay_is_off_unless_allowed_and_an_own_key_wins() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Delivery::load(dir.path(), false).unwrap().is_off());
    assert!(matches!(
        Delivery::load(dir.path(), true).unwrap(),
        Delivery::Relay(_)
    ));

    let apns = sheprd_apns::dir(dir.path());
    let pem = p256::SecretKey::from_slice(&[7; 32])
        .unwrap()
        .to_pkcs8_pem(LineEnding::LF)
        .unwrap();
    write_private(&apns.join("AuthKey.p8"), pem.as_bytes());
    write_private(
        &apns.join("config.json"),
        br#"{"team_id":"ABCDE12345","key_id":"XYZ9876543","topic":"so.pen.herdr.remote"}"#,
    );
    for relay in [false, true] {
        let delivery = Delivery::load(dir.path(), relay).unwrap();
        assert_eq!(delivery.name(), "own key", "relay {relay}");
    }
}

#[test]
fn a_key_reaches_tokens_and_the_relay_handles() {
    let own_key = Delivery::OwnKey(Box::new(sender_for_tests()));
    assert!(own_key.reaches(&token(1)));
    assert!(!own_key.reaches(&handle()));
    assert!(relay().reaches(&handle()));
    assert!(!relay().reaches(&token(1)));
    assert!(!Delivery::Off.reaches(&token(1)));
    assert!(!Delivery::Off.reaches(&handle()));
}

#[test]
fn each_reply_says_what_came_of_the_push() {
    use sheprd_apns::RelayOutcome;
    let refused = |status| Sent::Refused {
        status,
        reason: None,
    };
    for (outcome, sent) in [
        (RelayOutcome::Delivered, Sent::Delivered),
        (RelayOutcome::Gone, Sent::Gone),
        (RelayOutcome::Limited, Sent::Limited),
        (RelayOutcome::Invalid, refused(400)),
        (RelayOutcome::Stale, refused(403)),
        (RelayOutcome::Unavailable, Sent::Unreachable),
    ] {
        assert_eq!(Sent::from(outcome), sent, "{outcome:?}");
    }
    assert_eq!(Sent::from(Ok(Outcome::Gone)), Sent::Gone);
    assert_eq!(Sent::from(Ok(Outcome::RateLimited)), Sent::Limited);
    assert_eq!(
        Sent::from(Err(sheprd_apns::Error::Timeout)),
        Sent::Unreachable
    );
}

#[test]
fn an_outage_lasts_its_window() {
    let relays = Relays::new().unwrap();
    let host = RelayHost::Production;
    assert!(!relays.is_down(host, 1_000));
    relays.mark_down(host, 1_000);
    assert!(relays.is_down(host, 1_000));
    assert!(relays.is_down(host, 1_000 + crate::apns::delivery::OUTAGE_MS - 1));
    assert!(!relays.is_down(host, 1_000 + crate::apns::delivery::OUTAGE_MS));
    // A later failure extends it; an earlier one does not shorten it.
    relays.mark_down(host, 500);
    assert!(relays.is_down(host, 1_000 + crate::apns::delivery::OUTAGE_MS - 1));
}

#[test]
fn one_relay_down_leaves_the_other_up() {
    let relays = Relays::new().unwrap();
    relays.mark_down(RelayHost::Sandbox, 1_000);
    assert!(relays.is_down(RelayHost::Sandbox, 1_000));
    assert!(!relays.is_down(RelayHost::Production, 1_000));
}

#[tokio::test]
async fn during_an_outage_pushes_are_dropped_without_a_call() {
    let relays = Relays::new().unwrap();
    let now = 1_800_000_000_000;
    relays.mark_down(RelayHost::Sandbox, now);
    let delivery = Delivery::Relay(Box::new(relays));
    let key = p256::SecretKey::from_slice(&[5; 32]).unwrap().public_key();
    let live = Live {
        event: Event::Update { alert: None },
        pane: "w1-p3".to_owned(),
        state: ContentState {
            options: Vec::new(),
            permission: None,
            phase: Phase::Working,
            plan_done: None,
            plan_total: None,
            previous: Vec::new(),
            question: None,
            started_at: 0,
            step: "Working".to_owned(),
            summary: None,
            waiting: 0,
            working: 1,
        },
    };
    // Reaching the relay would take a DNS lookup and a TLS handshake at
    // least; a dropped push takes neither.
    let sent = tokio::time::timeout(
        Duration::from_millis(200),
        delivery.live(&handle(), Some(&key), &live, now),
    )
    .await
    .unwrap();
    assert_eq!(sent, Sent::Unreachable);
}

#[tokio::test]
async fn a_relayed_activity_needs_the_phones_key() {
    let live_without_key = relay()
        .live(
            &handle(),
            None,
            &Live {
                event: Event::End { dismissal_s: 0 },
                pane: "w1-p3".to_owned(),
                state: ContentState {
                    options: Vec::new(),
                    permission: None,
                    phase: Phase::Finished,
                    plan_done: None,
                    plan_total: None,
                    previous: Vec::new(),
                    question: None,
                    started_at: 0,
                    step: String::new(),
                    summary: None,
                    waiting: 0,
                    working: 0,
                },
            },
            0,
        )
        .await;
    assert_eq!(live_without_key, Sent::Unsent);
}
