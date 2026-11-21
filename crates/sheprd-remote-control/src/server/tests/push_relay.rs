//! A computer without an APNs key pushes through the Sheprd push relay
//! unless `--no-push-relay` says not to; `--push-relay`, which service
//! definitions written before carry, still parses.

use super::{options::USAGE, *};

#[test]
fn the_push_relay_is_on_unless_turned_off() {
    assert!(serve(&["serve", "--state-dir", "/s"]).push_relay);
    assert!(serve(&["serve", "--state-dir", "/s", "--push-relay"]).push_relay);
    assert!(!serve(&["serve", "--state-dir", "/s", "--no-push-relay"]).push_relay);
    assert!(USAGE.contains("--no-push-relay"));
}
