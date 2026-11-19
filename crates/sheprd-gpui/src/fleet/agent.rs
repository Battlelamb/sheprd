//! A plain host's relay agent, over the app's SSH (`docs/relay-plan.md`,
//! Fleet, Spreading a host, step 3): putting `sheprd-relay-agent` on the
//! host as `host_build` puts this app's other helpers there, keeping it
//! running as a user service, reading its `status --json`, and handing it
//! the access file (`fleet apply`). Everything here blocks: run it on the
//! background executor.
//!
//! The build comes from what this app ships: its own `sheprd-relay-agent`
//! when the host is this kind of computer, else `relay-agent/TRIPLE/`
//! beside the executable. It lands in
//! `~/.local/share/sheprd-relay-agent/bin/VERSION/`, linked from
//! `~/.local/bin`, and is copied only when the host holds other bytes.

use super::Error;
use crate::{
    host_build::{self, Platform},
    teleport::Host,
};
use sheprd_client::{ScriptLimits, shell_quote};
use sheprd_relay_agent::status::Report;
use sheprd_remote_control::control::fleet::{HostId, HostIntro, HostKind, SelfRecord, Verified};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

/// The agent's program name on the host.
pub(crate) const PROGRAM: &str = "sheprd-relay-agent";
/// Where builds for other platforms ship, beside the app's executable.
const BUILDS: &str = "relay-agent";
/// The service's name: a systemd user unit, or a launchd agent's label.
const UNIT: &str = "sheprd-relay-agent.service";
const LABEL: &str = "org.sheprd.relay-agent";
/// A slow link can take a while to carry a few megabytes.
const COPY_IDLE: Duration = Duration::from_secs(300);
const OUTPUT: u64 = 64 * 1024;
/// Starting the service and waiting for it to listen.
const SERVICE_IDLE: Duration = Duration::from_secs(60);
/// The most relays the service is started with; a record holds 16.
const MAX_RELAYS: usize = 16;
/// The most SSH host keys a record carries.
const MAX_HOST_KEYS: usize = 4;

/// The build of the agent for `platform` this app ships.
pub(crate) fn build_for(
    platform: &Platform,
    beside: &Path,
    bundled: Option<PathBuf>,
) -> Option<PathBuf> {
    host_build::build_for(platform, beside, BUILDS, PROGRAM, bundled)
}

/// A relay endpoint safe to write into a unit, a property list, and a
/// command line as it is: an `http(s)://` URL of plain characters.
pub(crate) fn plain_relay(relay: &str) -> bool {
    (relay.starts_with("https://") || relay.starts_with("http://"))
        && relay.len() <= 512
        && relay
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b":/.-_[]".contains(&byte))
}

/// The host script that keeps the agent running as a user service on
/// `relays`: a systemd user unit on Linux, a launchd agent on macOS, both
/// restarted so an update takes effect. It then waits up to ten seconds for
/// the agent to listen, and prints its `status --json`.
pub(crate) fn service_script(relays: &[String]) -> String {
    let args: String = relays
        .iter()
        .map(|relay| format!(" --relay {relay}"))
        .collect();
    let plist_args: String = relays
        .iter()
        .map(|relay| format!("<string>--relay</string><string>{relay}</string>"))
        .collect();
    format!(
        // i18n: data
        r#"agent="$HOME/.local/bin/{PROGRAM}"
case "$HOME" in *'&'*|*'<'*|*'>'*|*'"'*|*'%'*) echo "the home directory's name cannot go in a service" >&2; exit 5;; esac
case "$(uname -s)" in
Darwin)
  plist="$HOME/Library/LaunchAgents/{LABEL}.plist"
  mkdir -p "$HOME/Library/LaunchAgents"
  printf '%s\n' '<?xml version="1.0" encoding="UTF-8"?>' \
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
    '<plist version="1.0"><dict>' \
    '<key>Label</key><string>{LABEL}</string>' \
    "<key>ProgramArguments</key><array><string>$agent</string><string>run</string>{plist_args}</array>" \
    '<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>' \
    '</dict></plist>' > "$plist.part"
  job="gui/$(id -u)/{LABEL}"
  if cmp -s "$plist.part" "$plist" && launchctl print "$job" >/dev/null 2>&1; then
    # The same job is loaded: restart it in place, so the new build runs
    # and the agent is never unloaded.
    rm -f "$plist.part"
    launchctl kickstart -k "$job" >&2
  else
    mv -f "$plist.part" "$plist"
    launchctl bootout "$job" >/dev/null 2>&1 || true
    # `bootout` can return before the old job is gone, and a `bootstrap`
    # sent until then fails: try again for a while rather than leave no
    # service.
    tries=0
    until launchctl bootstrap "gui/$(id -u)" "$plist" 2>/dev/null; do
      tries=$((tries + 1))
      if [ "$tries" -ge 10 ]; then launchctl bootstrap "gui/$(id -u)" "$plist" >&2; break; fi
      sleep 1
    done
  fi
  ;;
*)
  dir="${{XDG_CONFIG_HOME:-$HOME/.config}}/systemd/user"
  mkdir -p "$dir"
  printf '%s\n' '[Unit]' 'Description=sheprd-relay-agent: Sheprd reaches this SSH server through relays' \
    '' '[Service]' "ExecStart=\"$agent\" run{args}" 'Restart=on-failure' 'RestartSec=10' \
    '' '[Install]' 'WantedBy=default.target' > "$dir/{UNIT}.part"
  mv -f "$dir/{UNIT}.part" "$dir/{UNIT}"
  # Without lingering a user unit stops at logout; allowed for oneself on
  # most systems, and harmless where it is not.
  loginctl enable-linger "$(id -un)" >/dev/null 2>&1 || true
  systemctl --user daemon-reload >&2
  systemctl --user enable {UNIT} >&2
  systemctl --user restart {UNIT} >&2
  ;;
esac
i=0
while [ $i -lt 10 ]; do
  case "$("$agent" status --json 2>/dev/null)" in *'"running": true'*) break;; esac
  sleep 1
  i=$((i + 1))
done
"$agent" status --json
"#
    )
}

/// Finds the agent without installing it; runs it with `ARGS`, or prints
/// nothing when there is none.
const RUN: &str = "\
agent=$(command -v sheprd-relay-agent || true)
[ -n \"$agent\" ] || agent=\"$HOME/.local/bin/sheprd-relay-agent\"
if [ -x \"$agent\" ]; then \"$agent\" ARGS; fi
";

fn run(host: &Host, args: &str, input: &[u8], cancelled: &AtomicBool) -> Result<Vec<u8>, Error> {
    let mut output = Vec::new();
    host.stream(
        &RUN.replace("ARGS", args),
        input,
        &mut output,
        ScriptLimits {
            output: OUTPUT,
            idle: Host::QUERY_IDLE,
        },
        cancelled,
    )
    .map_err(Error::Run)?;
    Ok(output)
}

/// The agent's `status --json`, or `None` when the host has none.
pub(super) fn status(host: &Host, cancelled: &AtomicBool) -> Result<Option<Report>, Error> {
    let output = run(host, "status --json", b"", cancelled)?;
    parse_status(&output)
}

/// `status --json` as the agent printed it; nothing printed, no agent.
pub(crate) fn parse_status(output: &[u8]) -> Result<Option<Report>, Error> {
    if output.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    Ok(Some(Report::parse(output)?))
}

/// Hands the agent `access`, the file the daemon computed; what it holds
/// after.
pub(super) fn apply(host: &Host, access: &str, cancelled: &AtomicBool) -> Result<Report, Error> {
    let output = run(host, "fleet apply", access.as_bytes(), cancelled)?;
    parse_status(&output)?.ok_or(Error::NoProgram)
}

/// Puts this app's build of the agent on `host` unless the same bytes are
/// there, keeps it running on `relays`, and reads its status. Blocks.
pub(super) fn install(
    host: &Host,
    relays: &[String],
    cancelled: &AtomicBool,
) -> Result<Report, Error> {
    let uname = host.capture("uname -sm\n", cancelled).map_err(Error::Run)?;
    let platform = Platform::parse(&String::from_utf8_lossy(&uname))?;
    let source = build_for(
        &platform,
        &host_build::beside_app(),
        crate::bundled::locate(PROGRAM),
    )
    .ok_or_else(|| Error::NoBuild {
        platform: platform.to_string(),
    })?;
    let (_, sha256) = host_build::digest(PROGRAM, &source)?;
    let installed = host
        .capture(&probe_script(crate::APP_VERSION), cancelled)
        .map_err(Error::Run)?;
    if String::from_utf8_lossy(&installed).trim() != sha256 {
        let file =
            std::fs::File::open(&source).map_err(|source_error| host_build::Error::Binary {
                program: PROGRAM,
                path: source.clone(),
                source: source_error,
            })?;
        host.stream(
            &host_build::copy_script(PROGRAM, crate::APP_VERSION, &sha256),
            file,
            Vec::new(),
            ScriptLimits {
                output: OUTPUT,
                idle: COPY_IDLE,
            },
            cancelled,
        )
        .map_err(|error| match &error {
            sheprd_client::Error::ScriptExit { status, .. }
                if status.code() == Some(host_build::CHECKSUM_EXIT) =>
            {
                Error::Build(host_build::Error::Checksum)
            }
            _ => Error::Run(error),
        })?;
    }
    let mut output = Vec::new();
    host.stream(
        &service_script(relays),
        &b""[..],
        &mut output,
        ScriptLimits {
            output: OUTPUT,
            idle: SERVICE_IDLE,
        },
        cancelled,
    )
    .map_err(Error::Run)?;
    parse_status(&output)?.ok_or(Error::NoProgram)
}

/// Prints the SHA-256 of the agent of `version` the host holds, or nothing.
fn probe_script(version: &str) -> String {
    format!(
        // i18n: data
        "file=\"$HOME/.local/share/{PROGRAM}/bin/\"{}\"/{PROGRAM}\"\n\
         if [ -f \"$file\" ]; then\n\
         if command -v sha256sum >/dev/null 2>&1; then sum=$(sha256sum \"$file\"); else sum=$(shasum -a 256 \"$file\"); fi\n\
         echo \"${{sum%% *}}\"\n\
         fi\n",
        shell_quote(version)
    )
}

/// The account the host's SSH session logs in as.
pub(super) fn user(host: &Host, cancelled: &AtomicBool) -> Result<String, Error> {
    let output = host.capture("id -un\n", cancelled).map_err(Error::Run)?;
    let user = String::from_utf8_lossy(&output).trim().to_owned();
    let valid = !user.is_empty()
        && user.len() <= 64
        && !user.starts_with('-')
        && user
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte));
    valid.then_some(user).ok_or(Error::NoUser)
}

/// The host keys the agent reported that this computer's known hosts list
/// for the host, in the agent's order: a strictly checked session would
/// accept each. The first names the host in the fleet.
pub(crate) fn trusted_keys(reported: &[String], known: &[String]) -> Vec<String> {
    reported
        .iter()
        .filter(|key| known.contains(key))
        .take(MAX_HOST_KEYS)
        .cloned()
        .collect()
}

/// The plain host the fleet names a machine whose agent reported
/// `reported`, when this computer trusts its first key.
pub(crate) fn host_id(reported: &[String], known: &[String]) -> Option<HostId> {
    let first = reported.first()?;
    known.contains(first).then(|| HostId::plain(first))
}

/// The intro a member shares a plain host with: what its agent reported,
/// held to this computer's known hosts, logged in as `user`, on `relays`,
/// vouched for at `now_ms`.
pub(crate) fn intro(
    report: &Report,
    known: &[String],
    name: &str,
    user: &str,
    relays: &[String],
    now_ms: u64,
) -> Result<HostIntro, Error> {
    // The first key names the host: it must be one the session trusted.
    if host_id(&report.ssh_host_keys, known).is_none() {
        return Err(Error::HostKey);
    }
    let ssh_host_keys = trusted_keys(&report.ssh_host_keys, known);
    let name: String = name
        .chars()
        .filter(|char| !char.is_control())
        .take(64)
        .collect();
    let mut intro = HostIntro {
        kind: HostKind::Agent,
        name: if name.trim().is_empty() {
            "host".to_owned() // i18n: data
        } else {
            name
        },
        user: user.to_owned(),
        ca: None,
        record: SelfRecord {
            host: HostId([0; 32]),
            rev: now_ms,
            route: report.route.clone(),
            relays: relays.iter().take(MAX_RELAYS).cloned().collect(),
            identity: report.identity.clone(),
            direct: Vec::new(),
            ssh_host_keys,
            signed_ms: now_ms,
        },
        record_signature: None,
        verified: Verified::KnownHosts,
    };
    intro.record.host = intro.id().map_err(Error::Intro)?;
    // Checks every field as the daemon will.
    intro.to_bytes().map_err(Error::Intro)?;
    Ok(intro)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
