#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;

#[test]
fn answers_are_taken_one_at_a_time() {
    let mut buffer = b"cores 2\r\n@@herdr-answer-end\r\ncores 3\n@@herdr-answer".to_vec();
    assert_eq!(take_answer(&mut buffer).as_deref(), Some("cores 2\r\n"));
    assert_eq!(take_answer(&mut buffer), None, "the end line is incomplete");
    buffer.extend_from_slice(b"-end\n");
    assert_eq!(take_answer(&mut buffer).as_deref(), Some("cores 3\n"));
    assert!(buffer.is_empty());
    // The marker counts only as a whole line.
    let mut buffer = b"x @@herdr-answer-end\n".to_vec();
    assert_eq!(take_answer(&mut buffer), None);
}

#[test]
fn only_the_newest_answer_is_taken() {
    let mut buffer =
        b"cores 1\n@@herdr-answer-end\ncores 2\r\n@@herdr-answer-end\r\ncores 3\n".to_vec();
    assert_eq!(take_latest(&mut buffer).as_deref(), Some("cores 2\r\n"));
    assert_eq!(buffer, b"cores 3\n", "an answer still arriving is kept");
    assert_eq!(take_latest(&mut buffer), None);
}

#[test]
fn a_script_wraps_its_lines_in_the_loop() {
    let text = script(
        "$herdrSetup = 1",
        "    $herdrBody = 2",
        Duration::from_secs(5),
    );
    let setup = text.find("$herdrSetup = 1").unwrap();
    let body = text.find("$herdrBody = 2").unwrap();
    let end = text.find("AppendLine('@@herdr-answer-end')").unwrap();
    assert!(
        text.find("'sshd*'").unwrap() < setup,
        "the watch comes first"
    );
    assert!(setup < body && body < end);
    assert!(text.contains("Start-Sleep -Seconds 5\n"));
    assert!(!text.contains("__"), "every placeholder is filled");
    // A period under a second still sleeps, rather than spinning.
    assert!(script("", "", Duration::ZERO).contains("Start-Sleep -Seconds 1\n"));
}

/// A process that exits before answering is closed, not waited on until the
/// timeout.
#[test]
fn an_exited_process_is_closed_at_once() {
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("cmd.exe");
        command.args(["/c", "exit 3"]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "exit 3"]);
        command
    };
    let mut session = Session::start(&mut command).unwrap();
    let started = Instant::now();
    assert!(matches!(
        session.sample(Duration::from_secs(30)),
        Err(crate::Error::WindowsHost(Error::Closed))
    ));
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// Answers are read as they come, also through a Windows client's spool.
#[test]
fn answers_arrive_while_the_process_runs() {
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("powershell.exe");
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(script(
                "",
                "    [void]$herdrOut.AppendLine('tick')",
                Duration::from_secs(1),
            ));
        command.stderr(Stdio::null());
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = std::process::Command::new("/bin/sh");
        command.args([
            "-c",
            "while :; do printf 'tick\\n@@herdr-answer-end\\n'; sleep 1; done",
        ]);
        command
    };
    let mut session = Session::start(&mut command).unwrap();
    let limit = Duration::from_secs(60);
    for _ in 0..2 {
        assert_eq!(session.sample(limit).unwrap().trim(), "tick");
    }
}

/// The command line sshd hands the login shell for `script`, which
/// `cmd.exe` refuses beyond [`COMMAND_LIMIT`].
pub(crate) fn remote_command_length(script: &str) -> usize {
    let bootstrap = compressed(script).unwrap();
    // `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand `,
    // then the bootstrap as UTF-16 in base64.
    66 + (bootstrap.encode_utf16().count() * 2).div_ceil(3) * 4
}

#[test]
fn a_small_script_fits_the_command_line() {
    assert!(remote_command_length(&script("", "", Duration::from_secs(2))) < COMMAND_LIMIT);
}

/// The compressed form, run by this machine's own PowerShell, runs the
/// script it carries.
#[cfg(windows)]
#[test]
fn a_compressed_script_runs_as_itself() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let text = script(
        "$herdrWord = 'it''s here'",
        "    [void]$herdrOut.AppendLine($herdrWord)",
        Duration::from_secs(1),
    );
    let bootstrap = compressed(&text).unwrap();
    let utf16: Vec<u8> = bootstrap
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut command = std::process::Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
        ])
        .arg(STANDARD.encode(utf16))
        .stderr(Stdio::null());
    let mut session = Session::start(&mut command).unwrap();
    let limit = Duration::from_secs(60);
    for _ in 0..2 {
        assert_eq!(session.sample(limit).unwrap().trim(), "it's here");
    }
}

/// The first answer may take longer than later ones (PowerShell loading
/// modules, a script compiling C#): it is waited for past `timeout`, so a
/// slow machine is not restarted forever.
#[test]
fn a_slow_first_answer_is_waited_for() {
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("powershell.exe");
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(script(
                "Start-Sleep -Seconds 3",
                "    [void]$herdrOut.AppendLine('late')",
                Duration::from_secs(1),
            ));
        command.stderr(Stdio::null());
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = std::process::Command::new("/bin/sh");
        command.args([
            "-c",
            "sleep 3; while :; do printf 'late\n@@herdr-answer-end\n'; sleep 1; done",
        ]);
        command
    };
    let mut session = Session::start(&mut command).unwrap();
    let step = Duration::from_millis(500);
    assert_eq!(session.sample(step).unwrap().trim(), "late");
    // Later answers keep to the step, a period apart.
    assert_eq!(
        session.sample(Duration::from_secs(30)).unwrap().trim(),
        "late"
    );
}
