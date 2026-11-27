#![allow(clippy::unwrap_used)]

use super::*;
use sheprd_chat_protocol::EntryId;
use sheprd_control_protocol::protocol::modal::{CueKind, ModalScope};
use std::time::{Duration, Instant};

#[test]
fn parses_all_interaction_directives_and_cues() {
    let (dir, title) = parse::opening(":::urgent Tap your YubiKey").unwrap();
    assert_eq!(dir, BlockDirective::Urgent);
    assert_eq!(title, "Tap your YubiKey");

    let urgent = parse::parse_block(
        dir,
        &title,
        "preset: yubikey\nWaiting on git push origin main",
    );
    assert_eq!(urgent.scope, ModalScope::All);
    assert_eq!(urgent.cue, Some(CueKind::Yubikey));
    assert_eq!(urgent.body, "Waiting on git push origin main");
    assert_eq!(parse::cue_line("::cue yubikey"), Some(CueKind::Yubikey));
    assert_eq!(parse::cue_line(":::cue yubikey"), None);

    let modal = parse::parse_block(
        BlockDirective::Modal,
        "Confirm push",
        "choices: Push | Open diff | Cancel\nreply: true\nReady to push.",
    );
    assert_eq!(modal.scope, ModalScope::Window);
    match modal.payload {
        Payload::Modal {
            choices,
            quick_reply,
            ..
        } => {
            assert_eq!(choices, vec!["Push", "Open diff", "Cancel"]);
            assert!(quick_reply);
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    let secret = parse::parse_block(
        BlockDirective::Secret,
        "Provide key",
        "key: STRIPE_TEST_KEY\ndest: env-file:.env.test.local\nNeeded for webhook tests.",
    );
    assert_eq!(secret.cue, Some(CueKind::Secret));
    match secret.payload {
        Payload::Secret { key, dest, .. } => {
            assert_eq!(key, "STRIPE_TEST_KEY");
            assert_eq!(dest, "env-file:.env.test.local");
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    let runbook = parse::parse_block(
        BlockDirective::Runbook,
        "Ship v0.18",
        "- [x] agent: Run just ci\n- [ ] you: Tap YubiKey\n",
    );
    match runbook.payload {
        Payload::Runbook { steps } => {
            assert_eq!(steps.len(), 2);
            assert!(steps[0].done && !steps[0].human);
            assert!(!steps[1].done && steps[1].human);
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    let progress = parse::parse_block(
        BlockDirective::Progress,
        "Running tests",
        "percent: 68%\nstep: chat::effects",
    );
    assert_eq!(progress.scope, ModalScope::All);
    match progress.payload {
        Payload::Progress {
            percent,
            step,
            done,
        } => {
            assert_eq!(percent, Some(68));
            assert_eq!(step, "chat::effects");
            assert!(!done);
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[test]
fn escalation_ladder_routes_by_presence_and_idle_time() {
    assert_eq!(
        escalate(ModalScope::All, Presence::InChat, 0, true),
        EscalationLevel::InlineOnly
    );
    assert_eq!(
        escalate(ModalScope::Window, Presence::SameWindowOtherTab, 0, true),
        EscalationLevel::WindowModal
    );
    assert_eq!(
        escalate(ModalScope::All, Presence::OtherApp, 0, true),
        EscalationLevel::GlobalHud
    );
    assert_eq!(
        escalate(ModalScope::Window, Presence::OtherApp, 5, true),
        EscalationLevel::WindowModal
    );
    assert_eq!(
        escalate(ModalScope::Window, Presence::OtherApp, 12, true),
        EscalationLevel::GlobalHud
    );
    assert_eq!(
        escalate(ModalScope::All, Presence::Away { idle_secs: 25 }, 25, true),
        EscalationLevel::PhoneEscalation
    );
    assert_eq!(
        escalate(ModalScope::All, Presence::OtherApp, 0, false),
        EscalationLevel::WindowModal
    );
}

#[test]
fn store_deduplicates_per_pane_and_auto_clears_on_newer_entry() {
    let mut store = InteractionStore::default();
    let now = Instant::now();
    let win = WindowId::from(1u64);
    let urgent = parse::parse_block(BlockDirective::Urgent, "Tap YubiKey", "Blocked on push");

    // History load never opens modals.
    assert!(!store.sync_chat_entry(
        win,
        "p1",
        "Claude",
        "repo",
        EntryId(1),
        1,
        true,
        std::slice::from_ref(&urgent),
        &[],
        now,
        0,
        true,
        true,
        true,
    ));
    assert!(store.summaries(now).is_empty());

    // Live entry opens modal.
    assert!(store.sync_chat_entry(
        win,
        "p1",
        "Claude",
        "repo",
        EntryId(2),
        1,
        false,
        &[urgent],
        &[],
        now,
        0,
        true,
        true,
        true,
    ));
    assert_eq!(store.summaries(now).len(), 1);
    assert_eq!(store.active_edge_cue(now), Some(CueKind::Yubikey));

    // Newer entry from the same chat auto-clears the earlier YubiKey modal.
    assert!(store.sync_chat_entry(
        win,
        "p1",
        "Claude",
        "repo",
        EntryId(3),
        1,
        false,
        &[],
        &[],
        now + Duration::from_secs(2),
        0,
        true,
        true,
        true,
    ));
    assert!(store.summaries(now + Duration::from_secs(2)).is_empty());
}

#[test]
fn vault_injects_0600_env_file_and_redacts_secret_debug() {
    let secret = vault::SecretValue::new("sk_test_super_secret_999".to_owned()).unwrap();
    assert!(!format!("{secret:?}").contains("sk_test_super_secret_999"));
    assert!(vault::parse_destination("env-file:../escape.env").is_err());
    assert!(vault::parse_destination("env-file:/etc/passwd").is_err());
    assert!(vault::validate_key("BAD KEY").is_err());

    let dir = tempfile::tempdir().unwrap();
    let dest = vault::parse_destination("env-file:.env.test.local").unwrap();
    let receipt = vault::inject(dir.path(), "STRIPE_TEST_KEY", &dest, &secret).unwrap();
    assert!(!receipt.summary.contains("sk_test_super_secret_999"));

    let written = std::fs::read_to_string(dir.path().join(".env.test.local")).unwrap();
    assert_eq!(written, "STRIPE_TEST_KEY=sk_test_super_secret_999\n");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dir.path().join(".env.test.local"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        // Refuse symlink target.
        let link = dir.path().join(".env.link");
        std::os::unix::fs::symlink(dir.path().join(".env.test.local"), &link).unwrap();
        let link_dest = vault::parse_destination("env-file:.env.link").unwrap();
        assert!(matches!(
            vault::inject(dir.path(), "STRIPE_TEST_KEY", &link_dest, &secret),
            Err(error::Error::SymlinkRefused { .. })
        ));
    }
}

#[test]
fn audio_cue_synthesizer_produces_bounded_pcm_for_every_cue() {
    for &cue in CueKind::ALL {
        let samples = audio::synthesize_cue(cue);
        assert!(!samples.is_empty(), "{cue:?}");
        assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }
}

#[test]
fn cli_modal_commands_parse_cleanly() {
    use crate::cli::control::{ControlCommand, ModalCommand, parse};
    use sheprd_control_protocol::protocol::modal::ModalOp;
    use std::ffi::OsString;

    let run = |args: &[&str]| {
        parse(
            OsString::from(args[0]),
            args[1..].iter().map(OsString::from),
        )
        .unwrap()
        .command
    };

    assert_eq!(
        run(&["modal"]),
        ControlCommand::Modal(ModalCommand::Op(ModalOp::List))
    );
    assert_eq!(
        run(&["modal", "urgent", "--title", "Tap YubiKey", "git", "push"]),
        ControlCommand::Modal(ModalCommand::Op(ModalOp::Show {
            scope: ModalScope::All,
            title: "Tap YubiKey".to_owned(),
            body: "git push".to_owned(),
            preset: None,
            choices: Vec::new(),
            quick_reply: false,
            cue: None,
        }))
    );
    assert_eq!(
        run(&[
            "modal",
            "secret",
            "--key",
            "STRIPE_TEST_KEY",
            "--dest",
            "env-file:.env.test.local",
            "--save-keychain",
        ]),
        ControlCommand::Modal(ModalCommand::Op(ModalOp::Secret {
            title: "Provide STRIPE_TEST_KEY".to_owned(),
            key: "STRIPE_TEST_KEY".to_owned(),
            dest: "env-file:.env.test.local".to_owned(),
            reason: String::new(),
            save_keychain: true,
        }))
    );
    assert_eq!(
        run(&[
            "modal",
            "progress",
            "--title",
            "Tests",
            "--percent",
            "68",
            "--step",
            "effects"
        ]),
        ControlCommand::Modal(ModalCommand::Op(ModalOp::Progress {
            title: "Tests".to_owned(),
            percent: Some(68),
            step: "effects".to_owned(),
            done: false,
        }))
    );
    assert_eq!(
        run(&["modal", "cue", "yubikey"]),
        ControlCommand::Modal(ModalCommand::Op(ModalOp::Cue {
            cue: CueKind::Yubikey,
        }))
    );
    assert_eq!(
        run(&["modal", "clear", "--all"]),
        ControlCommand::Modal(ModalCommand::Op(ModalOp::Clear {
            all: true,
            id: None,
        }))
    );
}
