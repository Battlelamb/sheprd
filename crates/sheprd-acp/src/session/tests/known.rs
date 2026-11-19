use sheprd_chat_protocol::{
    Choice, ConfigOption, Mode, Modes, OptionCategory, OptionKind, OptionValue,
};

use super::harnesses::{launch, rejection, requests, set_agent};
use super::*;
use crate::known::Settings;

fn model(current: &str) -> ConfigOption {
    let choice = |id: &str, name: &str| Choice {
        description: None,
        group: None,
        id: id.to_owned(),
        name: name.to_owned(),
    };
    ConfigOption {
        category: OptionCategory::Model,
        description: None,
        id: "model".to_owned(),
        kind: OptionKind::Select {
            choices: vec![choice("opus", "Opus"), choice("sonnet", "Sonnet")],
            current: current.to_owned(),
        },
        name: "Model".to_owned(),
    }
}

fn modes() -> Modes {
    let mode = |id: &str| Mode {
        description: None,
        id: id.to_owned(),
        name: id.to_owned(),
    };
    Modes {
        available: vec![mode("default"), mode("plan")],
        current: "default".to_owned(),
    }
}

fn known(agent: Agent) -> crate::known::Known {
    [(
        agent,
        Settings {
            modes: Some(modes()),
            options: vec![model("opus")],
        },
    )]
    .into()
}

fn options_json(current: &str) -> Value {
    json!([{"id": "model", "name": "Model", "category": "model", "type": "select",
        "currentValue": current, "options": [
            {"value": "opus", "name": "Opus"}, {"value": "sonnet", "name": "Sonnet"}]}])
}

/// Answers the handshake and opens session `s1` with `options`.
fn open(harness: &mut Harness, options: Value) {
    let id = harness.last_request("initialize")["id"].as_i64().unwrap();
    harness.respond(id, json!({"protocolVersion": 1, "agentCapabilities": {}}));
    let id = harness.last_request("session/new")["id"].as_i64().unwrap();
    harness.respond(id, json!({"sessionId": "s1", "configOptions": options}));
}

#[test]
fn a_new_session_keeps_what_it_offered_once_it_changed() {
    let mut harness = harness_with(config());
    harness.session.start();
    open(&mut harness, options_json("opus"));
    assert_eq!(
        *harness.settings.borrow(),
        [(
            Agent::Claude,
            Settings {
                modes: None,
                options: vec![model("opus")],
            }
        )]
    );

    let mut harness = harness_with(Config {
        known: [(
            Agent::Claude,
            Settings {
                modes: None,
                options: vec![model("opus")],
            },
        )]
        .into(),
        ..config()
    });
    harness.session.start();
    open(&mut harness, options_json("opus"));
    assert!(harness.settings.borrow().is_empty());
}

#[test]
fn a_starting_agent_shows_its_known_settings_and_takes_picks() {
    let mut harness = harness_with(Config {
        known: known(Agent::Claude),
        ..config()
    });
    harness.session.start();
    assert_eq!(harness.transcript().status, Status::Starting);
    assert_eq!(harness.transcript().options, [model("opus")]);
    assert_eq!(harness.transcript().modes, Some(modes()));

    let client = harness.subscribe(1, false, None);
    drain(&client);
    let picked = harness.command(
        1,
        Command::SetOption {
            id: "model".to_owned(),
            value: OptionValue::Choice {
                id: "sonnet".to_owned(),
            },
        },
    );
    let mode = harness.command(
        1,
        Command::SetMode {
            id: "plan".to_owned(),
        },
    );
    let messages = drain(&client);
    assert_eq!(rejection(&messages, picked), None);
    assert_eq!(rejection(&messages, mode), None);
    // Shown at once; nothing reaches the agent before its session.
    assert_eq!(harness.transcript().options, [model("sonnet")]);
    assert_eq!(harness.transcript().modes.as_ref().unwrap().current, "plan");
    let sent = harness.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["method"], "initialize");

    let id = sent[0]["id"].as_i64().unwrap();
    harness.respond(id, json!({"protocolVersion": 1, "agentCapabilities": {}}));
    let id = harness.last_request("session/new")["id"].as_i64().unwrap();
    harness.respond(
        id,
        json!({"sessionId": "s1", "configOptions": options_json("opus"),
            "modes": {"currentModeId": "default", "availableModes": [
                {"id": "default", "name": "default"}, {"id": "plan", "name": "plan"}]}}),
    );
    let sent = harness.sent();
    let option = sent
        .iter()
        .find(|message| message["method"] == "session/set_config_option")
        .expect("the pick is sent once the session opens");
    assert_eq!(option["params"]["configId"], "model");
    assert_eq!(option["params"]["value"], "sonnet");
    let mode = sent
        .iter()
        .find(|message| message["method"] == "session/set_mode")
        .expect("the mode is sent once the session opens");
    assert_eq!(mode["params"]["modeId"], "plan");
    // What is kept is what the agent opened with, not the pick.
    assert!(harness.settings.borrow().is_empty());
}

#[test]
fn a_pick_with_nothing_known_waits_for_the_agent() {
    let mut harness = harness_with(config());
    harness.session.start();
    let client = harness.subscribe(1, false, None);
    let picked = harness.command(
        1,
        Command::SetOption {
            id: "model".to_owned(),
            value: OptionValue::Choice {
                id: "sonnet".to_owned(),
            },
        },
    );
    assert_eq!(
        rejection(&drain(&client), picked),
        Some(Rejection::NotReady)
    );
}

#[test]
fn switching_shows_the_new_agents_settings_and_back_the_old_ones() {
    let mut harness = harness_with(Config {
        choices: Some(vec![Agent::Claude, Agent::Codex]),
        known: known(Agent::Codex),
        ..config()
    });
    harness.session.start();
    open(&mut harness, options_json("sonnet"));
    let client = harness.subscribe(1, false, None);
    drain(&client);

    set_agent(&mut harness, Agent::Codex);
    let messages = drain(&client);
    let Some(ServerMessage::Snapshot { transcript, .. }) = messages.first() else {
        panic!("the client starts over from a snapshot: {messages:?}");
    };
    assert_eq!(transcript.status, Status::Starting);
    assert_eq!(transcript.options, [model("opus")]);
    assert_eq!(transcript.modes, Some(modes()));

    let agent = launch(&mut harness);
    assert_eq!(requests(&agent).len(), 1);
    set_agent(&mut harness, Agent::Claude);
    // Claude left on Sonnet, and its session continues where it was.
    assert_eq!(harness.transcript().options, [model("sonnet")]);
}

#[test]
fn a_session_offering_nothing_clears_the_known_settings() {
    let mut harness = harness_with(Config {
        known: known(Agent::Claude),
        ..config()
    });
    harness.session.start();
    // A pick the session will not offer is not sent.
    harness.command(
        1,
        Command::SetOption {
            id: "model".to_owned(),
            value: OptionValue::Choice {
                id: "sonnet".to_owned(),
            },
        },
    );
    open(&mut harness, json!([]));
    assert_eq!(harness.transcript().status, Status::Idle);
    assert!(harness.transcript().options.is_empty());
    assert_eq!(harness.transcript().modes, None);
    assert!(
        harness
            .sent()
            .iter()
            .all(|message| message["method"] != "session/set_config_option")
    );
    // Forgotten for the next start too.
    assert_eq!(
        *harness.settings.borrow(),
        [(Agent::Claude, Settings::default())]
    );
}

#[test]
fn a_pick_the_agent_cannot_be_sent_leaves_the_session_failed() {
    let (sender, receiver) = bounded(64);
    let mut harness = harness_with(Config {
        known: known(Agent::Claude),
        ..config()
    });
    harness.session.outbox = Outbox(sender);
    harness.session.start();
    let initialize = serde_json::from_slice::<Value>(&receiver.recv().unwrap()).unwrap();
    harness.command(
        1,
        Command::SetMode {
            id: "plan".to_owned(),
        },
    );
    harness.respond(
        initialize["id"].as_i64().unwrap(),
        json!({"protocolVersion": 1, "agentCapabilities": {}}),
    );
    let new = serde_json::from_slice::<Value>(&receiver.recv().unwrap()).unwrap();
    // The agent stops reading its input before the pick is sent.
    drop(receiver);
    harness.respond(
        new["id"].as_i64().unwrap(),
        json!({"sessionId": "s1", "configOptions": options_json("opus"),
            "modes": {"currentModeId": "default", "availableModes": [
                {"id": "default", "name": "default"}, {"id": "plan", "name": "plan"}]}}),
    );
    assert!(matches!(harness.transcript().status, Status::Failed { .. }));
}
