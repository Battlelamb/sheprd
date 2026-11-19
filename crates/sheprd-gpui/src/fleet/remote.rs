//! The fleet's halves on another machine, run over SSH as
//! `relay_sharing` offers relays: `sheprd-remote-control fleet apply`,
//! `fleet bundle`, and `fleet intro`. Everything here blocks: run it on the
//! background executor.

use super::Error;
use crate::teleport::Host;
use sheprd_client::{ConnectTarget, ScriptLimits};
use sheprd_remote_control::control::fleet::sync::{Answer, Applied, Apply, MAX_REQUEST_BYTES};
use std::{sync::atomic::AtomicBool, time::Duration};

/// What `fleet apply` may print: an answer carries at most
/// `MAX_ANSWER_BYTES` of operations, base64url, and its heads.
const OUTPUT: u64 = MAX_REQUEST_BYTES as u64;
const IDLE: Duration = Duration::from_secs(30);

/// Finds the program without installing it, as `relay_sharing` does. A
/// host without one, or with a build older than the fleet, prints
/// nothing. `ARGS` is replaced with the subcommand's fixed arguments.
const SCRIPT: &str = "\
program=$(command -v sheprd-remote-control || command -v herdr-remote-control || true)
[ -n \"$program\" ] || program=\"$HOME/.local/bin/sheprd-remote-control\"
if [ -x \"$program\" ]; then \"$program\" fleet ARGS 2>/dev/null || true; fi
cat >/dev/null
";

fn run(
    target: &ConnectTarget,
    args: &str,
    input: &[u8],
    cancelled: &AtomicBool,
) -> Result<Vec<u8>, Error> {
    let host = Host::new(target).map_err(|_| Error::Unsupported)?;
    let mut output = Vec::new();
    host.stream(
        &SCRIPT.replace("ARGS", args),
        input,
        &mut output,
        ScriptLimits {
            output: OUTPUT,
            idle: IDLE,
        },
        cancelled,
    )
    .map_err(Error::Run)?;
    if output.iter().all(u8::is_ascii_whitespace) {
        return Err(Error::NoProgram);
    }
    Ok(output)
}

/// Hands `apply` to the host's fleet; what it holds that this computer
/// lacks, and its heads.
pub(super) fn apply(
    target: &ConnectTarget,
    apply: &Apply,
    cancelled: &AtomicBool,
) -> Result<Applied, Error> {
    let input = serde_json::to_vec(apply).map_err(Error::Answer)?;
    let output = run(target, "apply", &input, cancelled)?;
    parse_applied(&output)
}

/// The host's bundle, signed for `fleet`: what a member admits it with.
pub(super) fn bundle(
    target: &ConnectTarget,
    fleet: &str,
    cancelled: &AtomicBool,
) -> Result<(String, String), Error> {
    // The id is checked as hex before it reaches the shell.
    if fleet.len() != 64 || !fleet.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Unsupported);
    }
    let output = run(target, &format!("bundle --fleet {fleet}"), b"", cancelled)?;
    match serde_json::from_slice(&output).map_err(Error::Answer)? {
        Answer::Bundle { bundle, signature } => Ok((bundle, signature)),
        _ => Err(Error::NoProgram),
    }
}

/// The host as a Sheprd host, signed by its certificate authority.
pub(super) fn intro(target: &ConnectTarget, cancelled: &AtomicBool) -> Result<String, Error> {
    let output = run(target, "intro", b"", cancelled)?;
    match serde_json::from_slice(&output).map_err(Error::Answer)? {
        Answer::Intro { intro } => Ok(intro),
        _ => Err(Error::NoProgram),
    }
}

/// An `apply` answer; anything else, such as a host that refused because
/// it is in no fleet, is an error that says so.
pub(crate) fn parse_applied(output: &[u8]) -> Result<Applied, Error> {
    serde_json::from_slice(output).map_err(Error::Answer)
}

/// Removes every way a fleet reaches the host's account, without signing
/// anything: the daemon's `fleet scrub` and the relay agent's, each when
/// installed. Prints whether either ran.
const SCRUB: &str = "\
cleared=false
program=$(command -v sheprd-remote-control || command -v herdr-remote-control || true)
[ -n \"$program\" ] || program=\"$HOME/.local/bin/sheprd-remote-control\"
if [ -x \"$program\" ] && \"$program\" fleet scrub >/dev/null 2>&1; then cleared=true; fi
agent=$(command -v sheprd-relay-agent || true)
[ -n \"$agent\" ] || agent=\"$HOME/.local/bin/sheprd-relay-agent\"
if [ -x \"$agent\" ] && \"$agent\" fleet scrub >/dev/null 2>&1; then cleared=true; fi
printf '{\"scrubbed\":%s}\\n' \"$cleared\"
cat >/dev/null
";

/// `fleet scrub` on the host, the last resort for a host no member can
/// reach or a fleet that is gone: whether anything there held a fleet.
pub(super) fn scrub(target: &ConnectTarget, cancelled: &AtomicBool) -> Result<bool, Error> {
    #[derive(serde::Deserialize)]
    struct Scrubbed {
        scrubbed: bool,
    }
    let host = Host::new(target).map_err(|_| Error::Unsupported)?;
    let mut output = Vec::new();
    host.stream(
        SCRUB,
        &b""[..],
        &mut output,
        ScriptLimits {
            output: 1024,
            idle: IDLE,
        },
        cancelled,
    )
    .map_err(Error::Run)?;
    let answer: Scrubbed = serde_json::from_slice(&output).map_err(Error::Answer)?;
    Ok(answer.scrubbed)
}
