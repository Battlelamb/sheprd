//! Carrying a phone's relay keys to another of its computers
//! (`sheprd_remote_control::control::host_relay`). The phone signed them
//! with its own key there; this app only finds the host. It asks the host
//! the phone's pairing names first, then every other machine it connects
//! to, running `herdr-remote-control relay-enroll` there until one says it
//! is the computer the request names and signs its answer with that
//! computer's certificate authority. Nothing is installed: a host without
//! the program holds no phone. Everything here blocks.

use crate::teleport::Host;
use sheprd_client::{ConnectTarget, ScriptLimits};
use sheprd_remote_control::control::host_relay::{self, Outcome, Request};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// What `relay-enroll` may print: one outcome, at most a relay list.
const OUTPUT: u64 = 64 * 1024;
const IDLE: Duration = Duration::from_secs(20);

/// Finds the program as `device_tags::sync` does, without installing it.
/// An older build refuses `relay-enroll` and prints nothing. Standard input
/// is drained when no program reads it.
pub(crate) const SCRIPT: &str = "\
program=$(command -v sheprd-remote-control || command -v herdr-remote-control || true)
[ -n \"$program\" ] || program=\"$HOME/.local/bin/sheprd-remote-control\"
[ -x \"$program\" ] || program=\"$HOME/.local/bin/herdr-remote-control\"
if [ -x \"$program\" ]; then \"$program\" relay-enroll 2>/dev/null || true; fi
cat >/dev/null
";

/// How long one request's search may run, so its answer reaches the near
/// daemon while it still waits for it.
pub(crate) const BUDGET: Duration = Duration::from_millis(host_relay::CARRY_MS);

/// Tries each of `targets` in order; what carrying `request` came to.
/// Stopped by `cancelled` (the app quit, or `BUDGET` ran out), it failed:
/// the phone tries again later, where `Elsewhere` would leave the computer
/// alone for a day.
pub(crate) fn carry<'a>(
    request: &Request,
    targets: impl IntoIterator<Item = &'a ConnectTarget>,
    cancelled: &AtomicBool,
) -> Outcome {
    let mut failed = None;
    for target in targets {
        if cancelled.load(Ordering::Relaxed) {
            return failed.unwrap_or_else(timed_out);
        }
        match ask(request, target, cancelled) {
            // Only the computer the request names can sign for it; any
            // other host claiming to be it is passed over.
            Some(outcome @ (Outcome::Relayed { .. } | Outcome::Kept { .. })) => {
                if outcome.proven(request) {
                    return outcome;
                }
                tracing::warn!("A host answered for another computer's relay keys; passed over");
            }
            // The first failure is kept, in case it was that computer's.
            Some(outcome @ Outcome::Failed { .. }) => {
                failed.get_or_insert(outcome);
            }
            Some(Outcome::Elsewhere) | None => {}
        }
    }
    if cancelled.load(Ordering::Relaxed) {
        return failed.unwrap_or_else(timed_out);
    }
    failed.unwrap_or(Outcome::Elsewhere)
}

fn timed_out() -> Outcome {
    Outcome::Failed {
        reason: "no host answered in time".into(), // i18n: data
    }
}

/// The host's answer; `None` when it has no program that answers.
fn ask(request: &Request, target: &ConnectTarget, cancelled: &AtomicBool) -> Option<Outcome> {
    let host = Host::new(target).ok()?;
    let input = serde_json::to_vec(request).ok()?;
    let mut output = Vec::new();
    if let Err(error) = host.stream(
        SCRIPT,
        input.as_slice(),
        &mut output,
        ScriptLimits {
            output: OUTPUT,
            idle: IDLE,
        },
        cancelled,
    ) {
        tracing::info!(%error, "Could not ask a host to keep a phone's relay keys");
        return None;
    }
    parse(&output)
}

/// The host's answer, refused unless it is in bounds; `None` when it
/// printed nothing.
pub(crate) fn parse(output: &[u8]) -> Option<Outcome> {
    if output.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    serde_json::from_slice::<Outcome>(output)
        .ok()
        .filter(Outcome::valid)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
