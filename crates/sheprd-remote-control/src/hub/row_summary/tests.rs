use super::*;
use sheprd_chat_protocol::{Entry, EntryId};

fn entry(id: u64, kind: EntryKind) -> Entry {
    Entry {
        id: EntryId(id),
        kind,
    }
}

fn user(id: u64, text: &str) -> Entry {
    entry(
        id,
        EntryKind::User {
            text: text.into(),
            files: 0,
        },
    )
}

fn agent(id: u64, text: &str) -> Entry {
    entry(id, EntryKind::Agent { text: text.into() })
}

#[test]
fn the_chat_s_own_agent_goes_first_then_claude() {
    assert_eq!(
        Program::candidates("Codex"),
        [Program::Codex, Program::Claude]
    );
    assert_eq!(
        Program::candidates("gemini"),
        [Program::Gemini, Program::Claude]
    );
    assert_eq!(Program::candidates("Claude"), [Program::Claude]);
    assert_eq!(Program::candidates("OpenCode"), [Program::Claude]);
}

#[test]
fn no_candidate_runs_with_tools_or_saves_a_session() {
    let claude = Program::Claude.args();
    assert!(
        claude
            .windows(2)
            .any(|pair| pair[0] == "--tools" && pair[1].is_empty())
    );
    assert!(claude.contains(&"--no-session-persistence".into()));
    let codex = Program::Codex.args();
    assert!(
        codex
            .windows(2)
            .any(|pair| pair[0] == "--sandbox" && pair[1] == "read-only")
    );
    assert!(codex.contains(&"--ephemeral".into()));
    let gemini = Program::Gemini.args();
    assert!(gemini.contains(&"--skip-trust".into()));
    assert!(
        gemini
            .windows(2)
            .any(|pair| pair[0] == "--approval-mode" && pair[1] == "plan")
    );
}

#[test]
fn the_excerpt_keeps_what_was_said_oldest_first() {
    let mut transcript = Transcript::new("Claude");
    assert!(excerpt_of(&transcript).is_empty());
    transcript.entries = vec![
        user(1, "Fix the\nlogin"),
        entry(
            2,
            EntryKind::Thought {
                text: "hidden".into(),
            },
        ),
        agent(3, "Fixed it."),
    ];
    assert_eq!(
        excerpt_of(&transcript),
        "Person: Fix the login\n\nAgent: Fixed it."
    );
}

#[test]
fn the_excerpt_keeps_the_end_of_a_long_chat() {
    let mut transcript = Transcript::new("Claude");
    transcript.entries = (1..=40)
        .map(|id| agent(id, &format!("{id} {}", "x".repeat(1_000))))
        .collect();
    let input = excerpt_of(&transcript);
    assert!(input.chars().count() <= MAX_INPUT_CHARS);
    assert!(input.ends_with(&"x".repeat(100)));
    assert!(input.contains("Agent: 40 "));
    assert!(!input.contains("Agent: 1 "));
}

#[test]
fn running_summaries_are_bounded() {
    let slots: Vec<Slot> = (0..MAX_RUNNING).map(|_| Slot::take().unwrap()).collect();
    assert!(matches!(Slot::take(), Err(Error::Busy)));
    drop(slots);
    assert!(Slot::take().is_ok());
}

#[cfg(unix)]
fn sh(script: &str) -> (OsString, Vec<OsString>) {
    ("sh".into(), vec!["-c".into(), script.into()])
}

#[cfg(unix)]
#[test]
fn the_answer_is_one_bounded_line() {
    let text = run_first(
        [sh("cat > /dev/null; printf '  Fixed the\\n  login.  '")],
        "Person: hi",
        WAIT,
    )
    .unwrap();
    assert_eq!(text, "Fixed the login.");
    let long = run_first([sh("cat > /dev/null; yes word | head -c 20000")], "x", WAIT).unwrap();
    assert_eq!(long.chars().count(), MAX_SUMMARY_CHARS);
}

#[cfg(unix)]
#[test]
fn a_missing_agent_falls_to_the_next() {
    let missing = ("sheprd-no-such-agent".into(), Vec::new());
    assert_eq!(
        run_first([missing, sh("echo Done.")], "x", WAIT).unwrap(),
        "Done."
    );
    let only_missing = ("sheprd-no-such-agent".into(), Vec::new());
    assert!(matches!(
        run_first([only_missing], "x", WAIT),
        Err(Error::NoAgent)
    ));
}

#[cfg(unix)]
#[test]
fn a_failing_agent_falls_to_the_next_but_a_slow_one_does_not() {
    assert_eq!(
        run_first([sh("exit 55"), sh("echo Done.")], "x", WAIT).unwrap(),
        "Done."
    );
    assert_eq!(
        run_first([sh("true"), sh("echo Done.")], "x", WAIT).unwrap(),
        "Done."
    );
    let slow = run_first(
        [sh("sleep 5"), sh("echo Done.")],
        "x",
        Duration::from_millis(200),
    );
    assert!(matches!(slow, Err(Error::TimedOut)));
    // The last failure is the one told.
    let missing = ("sheprd-no-such-agent".into(), Vec::new());
    assert!(matches!(
        run_first([missing, sh("exit 1")], "x", WAIT),
        Err(Error::Failed)
    ));
}

#[cfg(unix)]
#[test]
fn failures_are_told_apart() {
    assert!(matches!(
        run_first([sh("exit 3")], "x", WAIT),
        Err(Error::Failed)
    ));
    assert!(matches!(
        run_first([sh("printf '  '")], "x", WAIT),
        Err(Error::Empty)
    ));
    let started = Instant::now();
    let slow = run_first([sh("sleep 5")], "x", Duration::from_millis(200));
    assert!(matches!(slow, Err(Error::TimedOut)));
    assert!(started.elapsed() < Duration::from_secs(4));
}
