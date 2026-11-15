//! What Sheprd tells the agent besides the person's words: the system
//! prompt and where each prompt was typed.

use sheprd_chat_protocol::{Device, Origin, Via};

use super::*;

const PHONE: Origin = Origin::new(Device::PhoneApp, Via::Tailscale);
const DESKTOP: Origin = Origin::new(Device::Desktop, Via::Local);

fn from(origin: Option<Origin>, text: &str) -> Command {
    Command::Prompt {
        origin,
        text: text.to_owned(),
        attachments: Vec::new(),
    }
}

/// The last `session/prompt`: its id and its text blocks.
fn sent_prompt(harness: &Harness) -> (i64, Vec<String>) {
    let request = harness.last_request("session/prompt");
    let blocks = request["params"]["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| block["text"].as_str().unwrap().to_owned())
        .collect();
    (request["id"].as_i64().unwrap(), blocks)
}

fn finish(harness: &mut Harness, id: i64) {
    harness.respond(id, json!({"stopReason": "end_turn"}));
}

#[test]
fn a_phone_prompt_tells_the_agent_first_and_the_transcript_shows_only_the_words() {
    let mut harness = ready();
    harness.command(0, from(Some(PHONE), "show me the mockups"));
    let (id, sent) = sent_prompt(&harness);
    assert_eq!(sent.len(), 2);
    assert!(sent[0].starts_with("<sheprd>"));
    assert!(sent[0].contains("Tailscale"));
    assert!(sent[0].contains("Markdown image"));
    assert_eq!(sent[1], "show me the mockups");
    let shown: Vec<_> = harness
        .transcript()
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::User { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(shown, ["show me the mockups"]);

    // The same place again says nothing new; another place does.
    finish(&mut harness, id);
    harness.command(0, from(Some(PHONE), "again"));
    let (id, sent) = sent_prompt(&harness);
    assert_eq!(sent, ["again"]);
    finish(&mut harness, id);
    harness.command(0, from(Some(DESKTOP), "back at my desk"));
    let (_, sent) = sent_prompt(&harness);
    assert!(sent[0].contains("desktop app on this computer"));
    assert_eq!(sent[1], "back at my desk");
}

#[test]
fn a_prompt_without_an_origin_sends_only_its_words() {
    let mut harness = ready();
    harness.command(0, from(None, "hi"));
    assert_eq!(sent_prompt(&harness).1, ["hi"]);
}

#[test]
fn a_queued_prompt_keeps_where_it_was_typed() {
    let mut harness = ready();
    harness.command(0, from(Some(DESKTOP), "one"));
    let (id, _) = sent_prompt(&harness);
    harness.command(0, from(Some(PHONE), "two"));
    finish(&mut harness, id);
    let (_, sent) = sent_prompt(&harness);
    assert!(sent[0].contains("phone"), "{sent:?}");
    assert_eq!(sent[1], "two");
}

#[test]
fn the_system_prompt_goes_with_the_first_prompt_only() {
    let mut harness = harness_with(Config {
        system_prompt: Some("Answer in French.".to_owned()),
        ..config()
    });
    harness.session.start();
    let id = harness.last_request("initialize")["id"].as_i64().unwrap();
    harness.respond(id, json!({"protocolVersion": 1, "agentCapabilities": {}}));
    let id = harness.last_request("session/new")["id"].as_i64().unwrap();
    harness.respond(id, json!({"sessionId": "s1"}));
    harness.command(0, from(None, "bonjour"));
    let (id, sent) = sent_prompt(&harness);
    assert!(sent[0].contains("Answer in French."));
    assert_eq!(sent[1], "bonjour");
    finish(&mut harness, id);
    harness.command(0, from(None, "encore"));
    assert_eq!(sent_prompt(&harness).1, ["encore"]);
}
