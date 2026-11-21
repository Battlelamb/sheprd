//! A prompt's entry keeps what was sent to the agent ahead of the person's
//! words, for a client to show when asked, apart from those words.

use super::*;
use crate::context::{Facts, Prompts};

fn started(config: Config) -> Harness {
    let mut harness = harness_with(config);
    harness.session.start();
    harness
}

fn prompt(text: &str) -> Command {
    Command::Prompt {
        origin: None,
        text: text.to_owned(),
        attachments: Vec::new(),
    }
}

/// The text and note of each prompt in the transcript.
fn prompts(harness: &Harness) -> Vec<(String, Option<String>)> {
    harness
        .transcript()
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::User { text, note, .. } => Some((text.clone(), note.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_prompt_keeps_the_note_sent_ahead_of_it() {
    let mut harness = started(Config {
        prompts: Prompts::with_instructions("Answer in French."),
        ..config()
    });
    let id = harness.last_request("initialize")["id"].as_i64().unwrap();
    harness.respond(id, json!({"protocolVersion": 1, "agentCapabilities": {}}));
    let id = harness.last_request("session/new")["id"].as_i64().unwrap();
    harness.respond(id, json!({"sessionId": "s1"}));
    harness.command(0, prompt("bonjour"));
    let request = harness.last_request("session/prompt");
    let note = request["params"]["prompt"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(note.contains("Answer in French."), "{note}");
    harness.respond(
        request["id"].as_i64().unwrap(),
        json!({"stopReason": "end_turn"}),
    );
    // Nothing new to tell: the next prompt went alone.
    harness.command(0, prompt("encore"));
    assert_eq!(
        prompts(&harness),
        [
            ("bonjour".to_owned(), Some(note)),
            ("encore".to_owned(), None),
        ]
    );
}

#[test]
fn a_replayed_note_goes_with_the_words_after_it_only() {
    let mut harness = started(Config {
        load: Some("old".to_owned()),
        ..config()
    });
    let id = harness.last_request("initialize")["id"].as_i64().unwrap();
    harness.respond(
        id,
        json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}),
    );
    let load = harness.last_request("session/load");
    let note = Context::new(
        Prompts::with_instructions("Be brief."),
        Facts::default(),
        Default::default(),
    )
    .before(None)
    .unwrap();
    let updates = [
        json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": note}}),
        json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "first"}}),
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "ok"}}),
        json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "second"}}),
    ];
    for update in updates {
        harness.input(Input::Agent(
            HarnessId::default(),
            Inbound::Notification {
                method: "session/update".to_owned(),
                params: json!({"sessionId": "old", "update": update}),
            },
        ));
    }
    harness.respond(load["id"].as_i64().unwrap(), json!({}));
    assert_eq!(
        prompts(&harness),
        [
            ("first".to_owned(), Some(note)),
            ("second".to_owned(), None),
        ]
    );
}
