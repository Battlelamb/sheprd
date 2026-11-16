use super::*;
use sheprd_chat_protocol::EntryKind;

/// The frames recorded while a second message ran on a T3 Code 0.1.7 thread.
fn recorded() -> Vec<ServerFrame> {
    let frames: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../sheprd-t3/fixtures/live-turn.json")).unwrap();
    frames
        .into_iter()
        .map(|frame| serde_json::from_value(frame).unwrap())
        .collect()
}

#[test]
fn a_live_turn_reaches_the_chat_as_it_happens_read_only() {
    // Request 2 was the thread's stream in the recording.
    let mut follower = Follower::new("2".to_owned());
    assert!(!follower.ready());
    let mut seen = Vec::new();
    for frame in recorded() {
        match follower.frame(frame) {
            Step::Changed(transcript) => seen.push(*transcript),
            Step::Same => {}
            Step::Ended(reason) => panic!("the stream ended: {reason}"),
        }
    }
    assert!(follower.ready());
    // The snapshot, then the turn's changes, each its own transcript.
    assert!(seen.len() > 2, "{} transcripts", seen.len());
    let first = &seen[0];
    let last = seen.last().unwrap();
    assert!(first.read_only && last.read_only);
    assert_eq!(last.agent, "fakeacp in T3 Code");
    assert!(last.entries.len() > first.entries.len());
    assert!(last.entries.iter().any(|entry| matches!(
        &entry.kind,
        EntryKind::Agent { text } if text.starts_with("Echo: Second turn from Sheprd")
    )));
    assert_eq!(last.status, Status::Idle);
    // Somewhere in between, the agent was working.
    assert!(
        seen.iter()
            .any(|transcript| transcript.status == Status::Working)
    );
}

#[test]
fn other_requests_frames_change_nothing_and_an_exit_ends_the_stream() {
    let mut follower = Follower::new("2".to_owned());
    let other = ServerFrame::Chunk {
        request_id: "1".to_owned(),
        values: vec![serde_json::json!({"kind": "synchronized"})],
    };
    assert_eq!(follower.frame(other), Step::Same);
    assert_eq!(follower.frame(ServerFrame::Pong), Step::Same);
    let refused: ServerFrame = serde_json::from_value(serde_json::json!({"_tag": "Exit",
        "requestId": "2", "exit": {"_tag": "Failure",
            "cause": [{"_tag": "Fail", "error": "Thread not found"}]}}))
    .unwrap();
    assert_eq!(
        follower.frame(refused),
        Step::Ended("Thread not found".to_owned())
    );
}
