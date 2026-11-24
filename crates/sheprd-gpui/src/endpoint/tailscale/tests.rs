//! The check an endpoint keeps across the retries that empty its live error.

#![allow(clippy::unwrap_used)]

use super::{LiveState, PendingCheck};
use sheprd_client::{SshFailure, TailscaleCheck};

const URL: &str = "https://login.tailscale.com/a/o84652-335960037";

/// The sentence a `PermissionDenied` refusal leaves: no typed failure at all.
fn refusal_sentence() -> String {
    format!(
        "Tailscale SSH wants you to verify your identity: open {URL} to authenticate, then try again"
    )
}

fn named(url: &str) -> SshFailure {
    SshFailure::Tailscale(TailscaleCheck::from_address(url).unwrap())
}

fn state(failure: Option<SshFailure>, error: Option<String>) -> LiveState {
    let mut live = LiveState::default();
    live.ssh_failure = failure;
    live.error = error;
    live
}

#[test]
fn a_typed_failure_is_pending_at_once() {
    let mut check = PendingCheck::default();
    let live = state(Some(named(URL)), None);
    check.observe(&live, None);
    assert_eq!(
        check.pending(&live).and_then(TailscaleCheck::url),
        Some(URL)
    );
}

#[test]
fn a_sentence_is_read_when_the_transport_reports_no_failure() {
    // The transport reports `PermissionDenied`, so the address reaches the
    // window as text: a surface reading only the typed failure would never
    // offer the one thing that finishes the check.
    let mut check = PendingCheck::default();
    let live = state(None, Some(refusal_sentence()));
    check.observe(&live, None);
    assert_eq!(
        check.pending(&live).and_then(TailscaleCheck::url),
        Some(URL)
    );
}

#[test]
fn the_check_outlives_a_retry_that_empties_the_live_error() {
    let mut check = PendingCheck::default();
    check.observe(&state(None, Some(refusal_sentence())), None);
    // The retry replaced `live` with nothing worth reading, and the outage it
    // recorded never named the address either.
    let retrying = state(None, Some("connecting".to_owned()));
    check.observe(&retrying, Some("the host stopped answering"));
    assert_eq!(
        check.pending(&retrying).and_then(TailscaleCheck::url),
        Some(URL),
        "the address is remembered, not re-read from the last attempt"
    );
}

#[test]
fn an_address_that_is_not_tailscales_is_never_remembered() {
    let mut check = PendingCheck::default();
    let live = state(
        None,
        Some("open https://example.com/a/abc to authenticate".to_owned()),
    );
    check.observe(&live, None);
    assert_eq!(check.pending(&live), None, "host text is not a link");
}

#[test]
fn a_connected_host_forgets_the_check() {
    let mut check = PendingCheck::default();
    check.observe(&state(None, Some(refusal_sentence())), None);
    check.clear();
    assert_eq!(check.pending(&LiveState::default()), None);
}
