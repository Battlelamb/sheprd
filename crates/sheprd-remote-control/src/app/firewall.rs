//! Whether macOS's application firewall will keep letting phones reach the
//! daemon. The firewall remembers "Allow incoming connections" for a binary
//! signed with a developer ID by its signer, so the approval survives
//! updates. For an ad-hoc or unsigned binary, such as a local build, it
//! remembers that exact build: once the file changes, the next start of the
//! daemon (a restart of the Mac, of the service, or launchd's own) is
//! blocked until someone clicks Allow at the computer. Loopback still works,
//! so nothing on the desktop shows it. Everything here blocks.

use super::{Error, Result};
use std::path::Path;

/// How long an approval of the daemon lasts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Approval {
    /// No firewall to approve it, or a signer the firewall remembers.
    #[default]
    Lasting,
    /// The firewall is on and remembers only this build of the daemon.
    ThisBuild,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SOCKETFILTERFW: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const CODESIGN: &str = "/usr/bin/codesign";

/// How long the firewall's approval of `program` lasts. Runs
/// `socketfilterfw` and `codesign`, neither of which needs privileges; off
/// macOS there is no such firewall.
pub fn approval(program: &Path) -> Result<Approval> {
    #[cfg(target_os = "macos")]
    {
        if !firewall_on(&output(SOCKETFILTERFW, ["--getglobalstate".as_ref()])?) {
            return Ok(Approval::Lasting);
        }
        let signature = output(CODESIGN, ["-dv".as_ref(), program.as_os_str()])?;
        Ok(if has_team(&signature) {
            Approval::Lasting
        } else {
            Approval::ThisBuild
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = program;
        Ok(Approval::Lasting)
    }
}

/// Both streams of `program`, whatever its exit status: `codesign -dv`
/// writes to stderr, and fails on an unsigned file, which has no team.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn output<'a>(
    program: &'static str,
    args: impl IntoIterator<Item = &'a std::ffi::OsStr>,
) -> Result<String> {
    let output = sheprd_client::new_command(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|source| Error::Spawn { program, source })?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

/// Whether `socketfilterfw --getglobalstate` says the firewall is on:
/// "Firewall is enabled. (State = 1)", or 2 when it blocks all but
/// essential services. Anything else, including words it never printed,
/// is taken as off, so a change in its wording never raises a warning.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn firewall_on(text: &str) -> bool {
    text.split_once("(State = ")
        .and_then(|(_, rest)| rest.split_once(')'))
        .and_then(|(state, _)| state.trim().parse::<u8>().ok())
        .is_some_and(|state| state > 0)
}

/// Whether `codesign -dv` names a team: "TeamIdentifier=ABCDE12345" rather
/// than "TeamIdentifier=not set", or no line at all for an unsigned file.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn has_team(text: &str) -> bool {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("TeamIdentifier="))
        .any(|team| !team.is_empty() && team != "not set")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
