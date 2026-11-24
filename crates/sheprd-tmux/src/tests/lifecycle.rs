use super::*;

#[test]
#[ignore = "requires an explicitly selected tmux executable"]
fn local_and_hosted_lifecycle() {
    let program = sandbox::daemon_binary();
    if std::env::var_os("SHEPRD_TMUX_TEST_CHILD").is_none() {
        let isolated = sandbox::Sandbox::new();
        let status = isolated
            .command(std::env::current_exe().unwrap(), "test.log")
            .args([
                "--ignored",
                "--exact",
                "tests::lifecycle::local_and_hosted_lifecycle",
                "--nocapture",
            ])
            .env("HERDR_TEST_BINARY", program)
            .env("TMUX_TMPDIR", &isolated.dir)
            .env("SHEPRD_TMUX_TEST_CHILD", "1")
            .status()
            .unwrap();
        assert!(
            status.success(),
            "{}",
            std::fs::read_to_string(isolated.dir.join("test.log")).unwrap()
        );
        return;
    }
    exercise(Tmux {
        program: program.clone(),
        ..Tmux::new(100)
    });
    // Exercise the remote command contract through a POSIX shell. Resolve
    // only the sandbox's explicitly selected tmux for this run.
    let bin = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(program, bin.path().join("tmux")).unwrap();
    exercise(Tmux::new(100).on_host(ShellHost(bin)));
}

struct ShellHost(tempfile::TempDir);

impl CommandHost for ShellHost {
    fn with_command(&self, script: &str, _persistent: bool, run: CommandTask<'_>) -> Result<()> {
        let mut command = sheprd_client::new_command("/bin/sh");
        command.env("PATH", format!("{}:/usr/bin:/bin", self.0.path().display()));
        command.args(["-c", script]);
        run(command)
    }
}

fn exercise(tmux: Tmux) {
    use sheprd_backend::{ClientPaneInputEvent, History, HistoryAnswer, Terminal};
    use std::time::Instant;
    let spawn = Spawn {
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            r#"row=0
while [ "$row" -lt 40 ]; do
    printf 'history-row-%s\n' "$row"
    row=$((row + 1))
done
printf 'ready\n'
while read -r line; do
    case "$line" in
        enter) printf '\033[?1049h\033[Heditor-active';;
        leave) printf '\033[?1049l';;
        *) printf 'reply=%s\n' "$line";;
    esac
done"#
                .into(),
        ],
        cwd: std::env::current_dir().ok(),
        name: Some("session".into()),
        labels: BTreeMap::from([("test".into(), "value".into())]),
        ..Default::default()
    };
    let info = tmux.spawn(&spawn).unwrap();
    struct Cleanup<'a>(&'a Tmux, TerminalId);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.kill(&self.1);
        }
    }
    let cleanup = Cleanup(&tmux, info.id.clone());
    assert_eq!(tmux.terminals().unwrap(), vec![info.clone()]);
    assert!(matches!(tmux.spawn(&spawn), Err(Error::NameTaken)));
    assert!(tmux.root_pid(&info.id).unwrap() > 0);
    tmux.report(
        &info.id,
        Some(&AgentReport {
            pid: std::process::id(),
            status: AgentStatus::Blocked,
            display: "Claude".into(),
            title: Some("a task".into()),
            endpoint: Some("chat".into()),
        }),
    )
    .unwrap();
    let reported = tmux.terminals().unwrap().pop().unwrap();
    assert_eq!(reported.agent, AgentStatus::Blocked);
    assert_eq!(reported.title.as_deref(), Some("a task"));
    assert_eq!(reported.labels[sheprd_chat_protocol::TOKEN], "chat");
    tmux.report(&info.id, None).unwrap();
    assert_eq!(tmux.terminals().unwrap(), vec![info.clone()]);
    let size = Size::new(80, 24).unwrap();
    let wait = |view: &mut TmuxTerminal, needle: &str| {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            view.next(Duration::from_millis(20)).unwrap();
            let HistoryAnswer::Text(text) = view.history(&History::Text).unwrap() else {
                panic!("expected text");
            };
            if text.contains(needle) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "screen missing {needle:?}: {text:?}"
            );
        }
    };
    let mut active = tmux.attach(&info.id, size, Role::Active).unwrap();
    wait(&mut active, "ready");
    active
        .input(&ClientPaneInputEvent::TextCommit("hello\r".into()))
        .unwrap();
    wait(&mut active, "reply=hello");
    drop(active);
    let mut observer = tmux.attach(&info.id, size, Role::Observer).unwrap();
    wait(&mut observer, "reply=hello");
    wait(&mut observer, "history-row-0");
    observer
        .input(&ClientPaneInputEvent::TextCommit("wrong\r".into()))
        .unwrap();
    observer.resize(Size::new(100, 40).unwrap()).unwrap();
    let mut active = tmux.attach(&info.id, size, Role::Active).unwrap();
    active
        .input(&ClientPaneInputEvent::TextCommit("again λ\r".into()))
        .unwrap();
    wait(&mut observer, "reply=again λ");
    assert!(
        matches!(observer.history(&History::Text).unwrap(),HistoryAnswer::Text(t) if !t.contains("wrong"))
    );
    active
        .input(&ClientPaneInputEvent::TextCommit("enter\r".into()))
        .unwrap();
    wait(&mut active, "editor-active");
    drop(observer);
    drop(active);
    let mut restored = tmux.attach(&info.id, size, Role::Active).unwrap();
    wait(&mut restored, "editor-active");
    restored
        .input(&ClientPaneInputEvent::TextCommit("leave\r".into()))
        .unwrap();
    wait(&mut restored, "reply=hello");
    wait(&mut restored, "history-row-0");
    let quick = tmux
        .spawn(&Spawn {
            argv: vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf script-done; exit 7".into(),
            ],
            ..Spawn::default()
        })
        .unwrap();
    let short = Cleanup(&tmux, quick.id.clone());
    let mut view = tmux.attach(&quick.id, size, Role::Active).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(code) = view
            .next(Duration::from_millis(50))
            .unwrap()
            .into_iter()
            .find_map(|update| match update {
                sheprd_backend::SurfaceUpdate::Ended(exit) => Some(exit.code),
                _ => None,
            })
        {
            assert_eq!(code, Some(7));
            break;
        }
        assert!(Instant::now() < deadline, "script exit was never reported");
    }
    assert!(
        matches!(view.history(&History::Text).unwrap(),HistoryAnswer::Text(text) if text.contains("script-done"))
    );
    drop(short);
    drop(cleanup);
    assert!(tmux.terminals().unwrap().is_empty());
}
