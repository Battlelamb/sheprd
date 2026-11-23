//! The model and thinking picked last for each agent, kept across chats.

use std::path::Path;

use sheprd_chat_protocol::{OptionValue, Setting, Settings};

use super::harnesses::{launch, requests, set_agent};
use super::*;

fn options(model: &str, effort: &str) -> Value {
    json!([
        {"id": "model", "name": "Model", "category": "model", "type": "select",
            "currentValue": model, "options": [
                {"value": "opus", "name": "Opus"}, {"value": "sonnet", "name": "Sonnet"}]},
        {"id": "effort", "name": "Thinking", "category": "thought_level", "type": "select",
            "currentValue": effort, "options": [
                {"value": "low", "name": "Low"}, {"value": "high", "name": "High"}]}
    ])
}

fn choice(id: &str, value: &str) -> Setting {
    Setting {
        id: id.into(),
        value: OptionValue::Choice { id: value.into() },
    }
}

fn picked(agent: &str, model: &str, effort: &str) -> Settings {
    Settings {
        agent: agent.into(),
        options: vec![choice("model", model), choice("effort", effort)],
        ..Settings::default()
    }
}

/// A chat keeping picks in `dir` whose agent opened session `s1`, offering
/// Opus with low thinking and two modes.
fn opened(dir: &Path, config: Config) -> Harness {
    let load = config.load.is_some();
    let mut harness = harness_with(config);
    harness.session.keep_picks(dir.to_owned());
    harness.session.start();
    let id = harness.last_request("initialize")["id"].as_i64().unwrap();
    harness.respond(
        id,
        json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}),
    );
    let method = if load { "session/load" } else { "session/new" };
    let id = harness.last_request(method)["id"].as_i64().unwrap();
    harness.respond(
        id,
        json!({"sessionId": "s1", "configOptions": options("opus", "low"),
            "modes": {"currentModeId": "default", "availableModes": [
                {"id": "default", "name": "Default"}, {"id": "plan", "name": "Plan"}]}}),
    );
    harness
}

fn sent_options(harness: &Harness) -> Vec<(String, String)> {
    harness
        .sent()
        .iter()
        .filter(|message| message["method"] == "session/set_config_option")
        .map(|message| {
            (
                message["params"]["configId"].as_str().unwrap().to_owned(),
                message["params"]["value"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_pick_is_kept_without_the_mode_or_auto_accepting() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = opened(root.path(), config());
    harness.command(0, Command::SetAutoAccept { on: true });
    harness.command(
        0,
        Command::SetOption {
            id: "effort".into(),
            value: OptionValue::Choice { id: "high".into() },
        },
    );
    let request = harness.last_request("session/set_config_option");
    harness.respond(
        request["id"].as_i64().unwrap(),
        json!({"configOptions": options("opus", "high")}),
    );
    assert_eq!(
        crate::known::picked(root.path(), Agent::Claude),
        Some(picked("claude", "opus", "high"))
    );
    assert_eq!(crate::known::picked(root.path(), Agent::Codex), None);
}

#[test]
fn a_new_chat_sets_the_agents_picks_once_its_session_opens() {
    let root = tempfile::tempdir().unwrap();
    crate::known::save_picked(
        root.path(),
        Agent::Claude,
        &picked("claude", "opus", "high"),
    )
    .unwrap();
    let mut harness = opened(root.path(), config());
    // Opus is already set, so only the thinking goes.
    let request = harness.last_request("session/set_config_option");
    assert_eq!(request["params"]["configId"], "effort");
    assert_eq!(request["params"]["value"], "high");
    harness.respond(
        request["id"].as_i64().unwrap(),
        json!({"configOptions": options("opus", "high")}),
    );
    assert!(sent_options(&harness).is_empty());
    assert!(harness.notices().is_empty(), "{:?}", harness.notices());
}

#[test]
fn a_continued_chat_or_one_given_settings_keeps_them_over_the_picks() {
    let root = tempfile::tempdir().unwrap();
    crate::known::save_picked(
        root.path(),
        Agent::Claude,
        &picked("claude", "opus", "high"),
    )
    .unwrap();
    let continued = opened(
        root.path(),
        Config {
            load: Some("old".into()),
            ..config()
        },
    );
    assert!(sent_options(&continued).is_empty());
    let given = opened(
        root.path(),
        Config {
            settings: Some(picked("claude", "sonnet", "low")),
            ..config()
        },
    );
    assert_eq!(sent_options(&given), [("model".into(), "sonnet".into())]);
}

#[test]
fn switching_agents_brings_back_the_new_agents_picks() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = opened(
        root.path(),
        Config {
            choices: Some(vec![Agent::Claude, Agent::Codex]),
            ..config()
        },
    );
    // Picked in another chat after this one started.
    crate::known::save_picked(
        root.path(),
        Agent::Codex,
        &picked("codex", "sonnet", "high"),
    )
    .unwrap();
    set_agent(&mut harness, Agent::Codex);
    let agent = launch(&mut harness);
    let id = requests(&agent)[0]["id"].as_i64().unwrap();
    let answer = |harness: &mut Harness, id: i64, result: Value| {
        harness.input(Input::Agent(
            HarnessId(1),
            Inbound::Response {
                id: wire::RequestId::Number(id),
                result: Ok(result),
            },
        ));
    };
    answer(
        &mut harness,
        id,
        json!({"protocolVersion": 1, "agentCapabilities": {}}),
    );
    let id = requests(&agent)[0]["id"].as_i64().unwrap();
    answer(
        &mut harness,
        id,
        json!({"sessionId": "c1", "configOptions": options("opus", "low")}),
    );
    let sent = requests(&agent);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["method"], "session/set_config_option");
    assert_eq!(sent[0]["params"]["configId"], "model");
    assert_eq!(sent[0]["params"]["value"], "sonnet");
}
