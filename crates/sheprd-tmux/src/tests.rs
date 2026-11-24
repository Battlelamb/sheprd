#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::{io::Cursor, sync::mpsc};

#[test]
fn a_native_child_pane_takes_precedence_over_its_tmux_parent() {
    let tmux = Tmux::named_id("parent").unwrap();
    for (nested, expected) in [
        ("", Some(tmux.clone())),
        ("HERDR_PANE_ID", None),
        ("HC_TERMINAL_ID", None),
    ] {
        let caller = Tmux::caller_in(|key| match key {
            "SHEPRD_TERMINAL_ID" => Some(tmux.to_string()),
            key if key == nested => Some("child".into()),
            _ => None,
        });
        assert_eq!(caller, expected);
    }
}

#[test]
fn a_saved_id_cannot_target_other_sessions() {
    let id = Tmux::named_id("shell").unwrap();
    assert!(id.is_tmux());
    for bad in ["shell", "t1", "%0", "tmux-*", "tmux-1:0", "tmux-$(x)"] {
        assert!(Tmux::target(&TerminalId::new(bad).unwrap()).is_err());
    }
}

#[test]
fn capture_text_is_never_treated_as_a_notification() {
    let (tx, rx) = mpsc::sync_channel(16);
    let stream = [
        "%begin 1 1 0",
        "%end 1 1 0",
        "%begin 1 2 1",
        "0 2 80 0",
        "%end 1 2 1",
        "%begin 1 3 1",
        "",
        "%end 1 3 1",
        "%begin 1 4 1",
        "",
        "%end 1 4 1",
        "%begin 2 2 1",
        "%output %1 hello",
        "%end 2 2 1",
        "%end 2 2 1",
        "%begin 3 3 1",
        "3 2 1",
        "%end 3 3 1",
        "%output %2 wrong",
        r"%output %1 \033\377x",
        "%exit",
        "",
    ]
    .join("\n");
    control::read(Cursor::new(stream.as_bytes()), "%1", 100, &tx, None).unwrap();
    drop(tx);
    let bytes: Vec<_> = rx
        .into_iter()
        .filter_map(|m| {
            if let control::Message::Bytes(b) = m {
                Some(b)
            } else {
                None
            }
        })
        .flatten()
        .collect();
    assert_eq!(
        bytes,
        b"%output %1 hello\r\n%end 2 2 1\x1b[0m\x1b[3;4H\x1b[?1h\x1b\xffx"
    );
}

#[cfg(unix)]
#[test]
fn remote_shell_round_trips_every_argument() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("tmux");
    std::fs::write(&executable, "#!/bin/sh\nprintf '%s\\0' \"$@\"\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let args = [
        "new-session",
        "a b",
        "it's here",
        "$HOME",
        ";",
        "two\nlines",
        "",
    ];
    let mut command = sheprd_client::new_command("tmux");
    command.args(args);
    let output = sheprd_client::new_command("/bin/sh")
        .env_clear()
        .env("PATH", dir.path())
        .arg("-c")
        .arg(host::remote_script(&command).unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut expected = args.join("\0").into_bytes();
    expected.push(0);
    assert_eq!(output.stdout, expected);
}

#[test]
fn chat_discovery_has_the_reporting_process_identity() {
    let id = Tmux::named_id("chat").unwrap();
    let report = AgentReport {
        pid: 42,
        display: "Codex".into(),
        status: AgentStatus::Idle,
        title: None,
        endpoint: Some("0123456789abcdef0123456789abcdef".into()),
    };
    let metadata = URL_SAFE_NO_PAD.encode(b"[null,{}]");
    let report = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&report).unwrap());
    let row = format!("{id}|0||{metadata}|{report}|sheprd-acp|/tmp/a|b");
    let info = record(&row).unwrap();
    assert_eq!(info.cwd, Some("/tmp/a|b".into()));
    assert_eq!(info.foreground.unwrap().pid, 42);
    assert_eq!(
        info.labels[sheprd_chat_protocol::TOKEN],
        "0123456789abcdef0123456789abcdef"
    );
    assert!(
        record(&row.replace("|sheprd-acp|", "|sh|"))
            .unwrap()
            .foreground
            .is_none()
    );
}

#[cfg(all(unix, feature = "integration-test"))]
#[allow(dead_code, clippy::disallowed_methods)]
#[path = "../../test-support/sandbox.rs"]
mod sandbox;

#[cfg(all(unix, feature = "integration-test"))]
mod lifecycle;
