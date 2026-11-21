//! `--push-relay` lets a computer without an APNs key push through the
//! Sheprd relay; it is off unless given.

use super::{options::USAGE, *};

#[test]
fn the_push_relay_is_off_unless_asked_for() {
    assert!(!serve(&["serve", "--state-dir", "/s"]).push_relay);
    assert!(serve(&["serve", "--state-dir", "/s", "--push-relay"]).push_relay);
    assert!(USAGE.contains("--push-relay"));
}
