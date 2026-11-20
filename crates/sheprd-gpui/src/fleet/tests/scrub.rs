//! What `fleet scrub` on a host printed: success only when it says its
//! access is gone.

use super::*;

#[test]
fn a_scrub_succeeds_only_when_access_is_known_gone() {
    assert!(remote::scrub_outcome(br#"{"scrubbed":true,"failed":false}"#).is_ok());
    assert!(matches!(
        remote::scrub_outcome(br#"{"scrubbed":true,"failed":true}"#),
        Err(Error::Unscrubbed)
    ));
    assert!(matches!(
        remote::scrub_outcome(br#"{"scrubbed":false,"failed":true}"#),
        Err(Error::Unscrubbed)
    ));
    assert!(matches!(
        remote::scrub_outcome(br#"{"scrubbed":false,"failed":false}"#),
        Err(Error::NoProgram)
    ));
    // An answer that does not say whether anything failed is not trusted.
    assert!(matches!(
        remote::scrub_outcome(br#"{"scrubbed":true}"#),
        Err(Error::Answer(_))
    ));
}

#[test]
fn the_relay_agent_is_scrubbed_before_the_daemon() {
    // Forgetting the daemon's fleet may end the session the script runs
    // in: the agent's cleanup must already have run.
    let agent = remote::SCRUB.find("\"$agent\" fleet scrub").unwrap();
    let daemon = remote::SCRUB.find("\"$program\" fleet scrub").unwrap();
    assert!(agent < daemon);
}

/// Runs the scrub script under `sh` with `home` as `HOME` and `path` as
/// `PATH`, and returns what it printed.
#[cfg(unix)]
fn run_scrub(home: &std::path::Path, path: &std::ffi::OsStr) -> String {
    use std::io::Write as _;
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(remote::SCRUB)
        .env_clear()
        .env("HOME", home)
        .env("PATH", path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"").unwrap();
    let output = child.wait_with_output().unwrap();
    String::from_utf8(output.stdout).unwrap()
}

/// An executable `sheprd-relay-agent` in `dir` that knows `fleet scrub`
/// and leaves `marker` when it runs it.
#[cfg(unix)]
fn agent(dir: &std::path::Path, marker: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("sheprd-relay-agent");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\ncase \"$1\" in --help) echo 'usage: fleet scrub';; fleet) : > '{}';; esac\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn the_scrub_runs_the_installed_agent_never_another_on_path() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let decoy = root.path().join("decoy");
    let (installed_ran, decoy_ran) = (root.path().join("installed"), root.path().join("decoy-ran"));
    agent(&decoy, &decoy_ran);
    // The decoy comes first on PATH; the system's tools follow.
    let path =
        std::env::join_paths([decoy.as_path(), "/usr/bin".as_ref(), "/bin".as_ref()]).unwrap();
    // With no agent installed, the step is skipped, decoy or not.
    std::fs::create_dir_all(&home).unwrap();
    let printed = run_scrub(&home, &path);
    assert!(!decoy_ran.exists(), "a copy on PATH must never run");
    assert!(remote::scrub_outcome(printed.as_bytes()).is_err());
    // Installed where Sheprd puts it, that copy is the one run.
    agent(&home.join(".local/bin"), &installed_ran);
    let printed = run_scrub(&home, &path);
    assert!(installed_ran.exists());
    assert!(!decoy_ran.exists(), "a copy on PATH must never run");
    assert!(remote::scrub_outcome(printed.as_bytes()).is_ok());
}
