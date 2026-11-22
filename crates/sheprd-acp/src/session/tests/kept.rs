//! Keeping the chat's thread on disk: its record once a prompt reached an
//! agent, a segment per agent, and every entry in its transcript file.

use sheprd_chat_protocol::{
    EntryId,
    thread::{self, Record, Segment, Thread, ThreadId, file::Contents},
};

use super::threads::{handed_to_codex, respond_as, sent_prompt};
use super::*;

/// A chat that may switch between Claude and Codex, keeping thread `id` in
/// `dir`, whose agent opened session `s1`; `load` continues a session.
fn kept(dir: &std::path::Path, id: ThreadId, load: Option<&str>) -> Harness {
    let mut harness = harness_with(Config {
        choices: load.is_none().then(|| vec![Agent::Claude, Agent::Codex]),
        load: load.map(str::to_owned),
        thread: Some(ThreadStart {
            dir: dir.to_owned(),
            id,
        }),
        ..config()
    });
    harness.session.start();
    let initialize = harness.last_request("initialize")["id"].as_i64().unwrap();
    harness.respond(
        initialize,
        json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}),
    );
    match load {
        Some(_) => {
            let open = harness.last_request("session/load")["id"].as_i64().unwrap();
            // The session replays what the thread's file already holds.
            harness.agent_says("replayed");
            harness.respond(open, json!({}));
        }
        None => {
            let open = harness.last_request("session/new")["id"].as_i64().unwrap();
            harness.respond(open, json!({"sessionId": "s1"}));
        }
    }
    harness
}

fn record(dir: &std::path::Path, id: &ThreadId) -> Option<Record> {
    Record::read(&thread::record_path(dir, id)).ok()
}

fn contents(dir: &std::path::Path, id: &ThreadId) -> Contents {
    Contents::read(&thread::transcript_path(dir, id)).unwrap()
}

#[test]
fn a_thread_is_kept_once_a_prompt_reaches_its_agent() {
    let dir = tempfile::tempdir().unwrap();
    let id = ThreadId::random();
    let mut harness = kept(dir.path(), id, None);
    // Nothing is kept for a chat no one wrote in.
    assert!(record(dir.path(), &id).is_none());
    assert_eq!(
        harness.transcript().thread.as_ref().unwrap().segments[0]
            .session
            .as_deref(),
        Some("s1")
    );

    let turn = prompt(&mut harness, 1, "Fix the build\nplease");
    let kept = record(dir.path(), &id).expect("kept from the first prompt");
    assert_eq!(kept.title, "Fix the build");
    assert_eq!(kept.cwd, PathBuf::from("/work"));
    assert_eq!(
        kept.thread.segments,
        [Segment {
            after: None,
            agent: Agent::Claude,
            model: None,
            session: Some("s1".to_owned()),
        }]
    );
    harness.agent_says("Fixed");
    harness.respond(turn, json!({"stopReason": "end_turn"}));
    // The turn's end writes its entries, each at its latest.
    let written = contents(dir.path(), &id);
    assert!(written.entries.iter().any(|entry| matches!(
        &entry.kind,
        EntryKind::User { text, .. } if text.starts_with("Fix the build")
    )));
    assert!(written.entries.iter().any(|entry| matches!(
        &entry.kind,
        EntryKind::Agent { text } if text == "Fixed"
    )));
}

#[test]
fn a_switch_adds_a_segment_and_its_handoff_to_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let id = ThreadId::random();
    let mut harness = kept(dir.path(), id, None);
    let codex = handed_to_codex(&mut harness);
    let thread = harness.transcript().thread.clone().unwrap();
    assert_eq!(record(dir.path(), &id).unwrap().thread, thread);
    let [claude, taking_over] = thread.segments.as_slice() else {
        panic!("{thread:?}");
    };
    assert_eq!(
        (claude.agent, claude.session.as_deref()),
        (Agent::Claude, Some("s1"))
    );
    assert_eq!(
        (taking_over.agent, taking_over.session.as_deref()),
        (Agent::Codex, Some("c1"))
    );
    // Codex began after the last entry Claude's turn left.
    let reply = harness
        .transcript()
        .entries
        .iter()
        .find(|entry| matches!(&entry.kind, EntryKind::Turn(_)))
        .unwrap()
        .id;
    assert_eq!(taking_over.after, Some(reply));

    let (turn, _) = sent_prompt(&mut harness, &codex, "next");
    respond_as(&mut harness, 1, turn, json!({"stopReason": "end_turn"}));
    let written = contents(dir.path(), &id);
    assert!(written.entries.iter().any(
        |entry| matches!(&entry.kind, EntryKind::Handoff(handoff) if handoff.to == Agent::Codex)
    ));
}

#[test]
fn ending_the_chat_writes_what_waits() {
    let dir = tempfile::tempdir().unwrap();
    let id = ThreadId::random();
    let mut harness = kept(dir.path(), id, None);
    prompt(&mut harness, 1, "hello");
    harness.agent_says("still streaming");
    assert_eq!(harness.input(Input::Shutdown), Flow::Exit);
    assert!(
        contents(dir.path(), &id)
            .entries
            .iter()
            .any(|entry| matches!(
                &entry.kind,
                EntryKind::Agent { text } if text == "still streaming"
            ))
    );
}

#[test]
fn a_continued_thread_goes_on_in_its_segment_after_its_entries() {
    let dir = tempfile::tempdir().unwrap();
    let id = ThreadId::random();
    let mut thread = Thread::new(id, Agent::Claude);
    thread.opened("s1");
    Record {
        created_ms: 1,
        cwd: PathBuf::from("/work"),
        thread: thread.clone(),
        title: "Earlier".to_owned(),
        updated_ms: 1,
    }
    .write(dir.path())
    .unwrap();
    let mut writer = thread::file::Writer::new(dir.path(), &id);
    let earlier = sheprd_chat_protocol::Entry {
        id: EntryId(40),
        kind: EntryKind::User {
            text: "Earlier".to_owned(),
            files: 0,
        },
    };
    writer.append(&[&earlier]).unwrap();

    let mut harness = kept(dir.path(), id, Some("s1"));
    assert_eq!(harness.transcript().thread.as_ref(), Some(&thread));
    prompt(&mut harness, 1, "again");
    assert_eq!(harness.input(Input::Shutdown), Flow::Exit);
    let written = contents(dir.path(), &id);
    // The replay is not written twice, and new entries follow the file's.
    assert!(!written.entries.iter().any(|entry| matches!(
        &entry.kind,
        EntryKind::Agent { text } if text == "replayed"
    )));
    let again = written
        .entries
        .iter()
        .find(|entry| matches!(&entry.kind, EntryKind::User { text, .. } if text == "again"))
        .unwrap();
    assert!(again.id > EntryId(40), "{again:?}");
    assert_eq!(record(dir.path(), &id).unwrap().title, "Earlier");
}

#[test]
fn a_continued_thread_with_another_session_starts_a_segment() {
    let dir = tempfile::tempdir().unwrap();
    let id = ThreadId::random();
    let mut thread = Thread::new(id, Agent::Codex);
    thread.opened("c1");
    Record {
        created_ms: 1,
        cwd: PathBuf::from("/work"),
        thread,
        title: "Earlier".to_owned(),
        updated_ms: 1,
    }
    .write(dir.path())
    .unwrap();
    let harness = kept(dir.path(), id, Some("s1"));
    let segments = &harness.transcript().thread.as_ref().unwrap().segments;
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].agent, Agent::Claude);
}

#[test]
fn a_thread_that_cannot_be_written_says_so_once_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    // A file where the threads folder should be.
    let blocked = dir.path().join("threads");
    std::fs::write(&blocked, b"").unwrap();
    let mut harness = kept(&blocked, ThreadId::random(), None);
    let turn = prompt(&mut harness, 1, "hello");
    harness.respond(turn, json!({"stopReason": "end_turn"}));
    prompt(&mut harness, 1, "again");
    let warnings: Vec<_> = harness
        .notices()
        .into_iter()
        .filter(|(level, text)| {
            *level == NoticeLevel::Warning && text.contains("can no longer be kept")
        })
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[test]
fn a_chat_without_a_thread_sends_none() {
    let harness = ready();
    assert_eq!(harness.transcript().thread, None);
}
