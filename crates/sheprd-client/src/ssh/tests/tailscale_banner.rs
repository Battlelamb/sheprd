//! The Tailscale banner a run has to notice. A host in check mode holds the
//! connection and `ssh` never prompts, so a run that does not read stderr can
//! only report its timeout; these tests pin that the refusal comes from the
//! banner, at once, and carries the address a surface can offer.

use super::*;

/// `sh` printing Tailscale's banner to stderr and then holding the connection,
/// as a host in check mode does. Nothing here lets go of it on its own, so a
/// run that waited would report the timeout instead.
fn host_waiting_for_a_check() -> Command {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(
            r#"printf '%s\n' \
'Tailscale SSH requires an additional check to connect to alice@node1:' \
'    https://login.tailscale.com/a/abc123' >&2
while :; do sleep 1; done"#,
        )
        .stderr(Stdio::piped());
    command
}

/// The banner ends the run with the check it named, well before the budget the
/// caller gave: the address belongs to this connection, so holding it open for
/// a check nobody may answer is time the window spends showing nothing.
#[cfg(unix)]
#[test]
fn a_banner_is_reported_at_once_instead_of_timing_out() {
    let started = Instant::now();
    let error = match run_reading_banner(&mut host_waiting_for_a_check(), PROBE_TIMEOUT, || false) {
        Ok(_) => panic!("a host holding a check never finishes the run"),
        Err(error) => error,
    };
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the check was read from the banner, not waited out: {:?}",
        started.elapsed()
    );
    match error {
        crate::Error::Ssh(Error::Refused(SshFailure::Tailscale(check))) => {
            assert_eq!(check.url(), Some("https://login.tailscale.com/a/abc123"));
        }
        other => panic!("expected the Tailscale check, got {other:?}"),
    }
}

/// A banner whose address Sheprd cannot read still says why the host stopped,
/// so the run reports the check rather than a bare timeout.
#[cfg(unix)]
#[test]
fn a_banner_without_a_readable_address_is_still_a_check() {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("printf '%s\\n' 'Tailscale SSH requires an additional check' >&2; sleep 30")
        .stderr(Stdio::piped());
    match run_reading_banner(&mut command, Duration::from_secs(5), || false) {
        Err(crate::Error::Ssh(Error::Refused(SshFailure::Tailscale(check)))) => {
            assert_eq!(check.url(), None, "no address was readable");
        }
        other => panic!("expected the Tailscale check, got {other:?}"),
    }
}
