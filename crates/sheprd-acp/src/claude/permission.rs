//! Claude Code's `can_use_tool` questions as ACP permission requests, and
//! the answers back.

use acp::v1 as wire;
use serde_json::{Map, Value, json};

use sheprd_chat_protocol::{Question, QuestionOption};

use super::tools;
use crate::harness::out;

/// What a chosen option tells Claude.
#[derive(Clone, Debug, PartialEq)]
enum Choice {
    /// Run the tool, with `input` in place of what Claude asked for when
    /// set, and the permission `updates` Claude suggested when set.
    Allow {
        input: Option<Value>,
        updates: Option<Value>,
    },
    Deny {
        message: &'static str,
    },
}

/// A question waiting for its answer.
#[derive(Debug)]
pub(super) struct Ask {
    choices: Vec<(String, Choice)>,
    /// The control request it answers.
    control: String,
    input: Value,
    /// The one question of an `AskUserQuestion` call, which a client may
    /// answer in its own words or with several choices.
    question: Option<Question>,
}

/// The options to offer for one `can_use_tool` request, and the question
/// that keeps them.
pub(super) fn ask(control: String, request: &Value) -> (Vec<wire::PermissionOption>, Ask) {
    let tool = request
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let input = request.get("input").cloned().unwrap_or(Value::Null);
    let suggestions = request
        .get("permission_suggestions")
        .and_then(Value::as_array)
        .filter(|suggestions| !suggestions.is_empty());
    let options = match tool {
        tools::EXIT_PLAN => exit_plan(),
        tools::QUESTION => question(&input),
        _ => {
            let mut options = vec![allow_once("allow", "Yes")];
            if let Some(suggestions) = suggestions {
                options.push((
                    wire::PermissionOption::new(
                        "always",
                        always_name(suggestions),
                        wire::PermissionOptionKind::AllowAlways,
                    ),
                    Choice::Allow {
                        input: None,
                        updates: Some(Value::Array(suggestions.clone())),
                    },
                ));
            }
            options.push(reject("reject", "No", "The user refused this."));
            options
        }
    };
    let (offered, choices) = options
        .into_iter()
        .map(|(option, choice)| {
            let id = option.option_id.0.to_string();
            (option, (id, choice))
        })
        .unzip();
    (
        offered,
        Ask {
            choices,
            control,
            question: (tool == tools::QUESTION)
                .then(|| shown_question(&input))
                .flatten(),
            input,
        },
    )
}

impl Ask {
    /// The permission request's `_meta`: the question in full, when it is
    /// one a client may answer itself.
    pub(super) fn meta(&self) -> Option<wire::Meta> {
        let question = serde_json::to_value(self.question.as_ref()?).ok()?;
        let mut meta = wire::Meta::new();
        meta.insert(out::QUESTION.to_owned(), question);
        Some(meta)
    }

    /// Claude's input for answers a client gave to the question, joined
    /// as Claude Code joins several choices.
    fn answered(&self, answers: &[&str]) -> Option<Choice> {
        let question = self.question.as_ref()?;
        if answers.is_empty() {
            return None;
        }
        let mut input = self.input.as_object().cloned().unwrap_or_else(Map::new);
        input.insert(
            "answers".to_owned(),
            json!({ question.text.as_str(): answers.join(", ") }),
        );
        Some(Choice::Allow {
            input: Some(Value::Object(input)),
            updates: None,
        })
    }

    /// The control response for the session's answer: the chosen option,
    /// or a refusal that also stops the turn when the question was dropped.
    pub(super) fn answer(self, result: &Result<Value, wire::Error>) -> Value {
        let chosen = result
            .as_ref()
            .ok()
            .and_then(|value| {
                serde_json::from_value::<wire::RequestPermissionResponse>(value.clone()).ok()
            })
            .and_then(|response| match response.outcome {
                wire::RequestPermissionOutcome::Selected(selected) => Some(selected),
                _ => None,
            })
            .and_then(|selected| {
                let id = selected.option_id.0.as_ref();
                if id == out::ANSWERS {
                    let answers = selected.meta.as_ref()?.get(out::ANSWERS)?.as_array()?;
                    let answers: Vec<&str> = answers.iter().filter_map(Value::as_str).collect();
                    return self.answered(&answers);
                }
                self.choices
                    .iter()
                    .find(|(known, _)| known == id)
                    .map(|(_, choice)| choice.clone())
            });
        let decision = match chosen {
            Some(Choice::Allow { input, updates }) => {
                let mut decision = json!({
                    "behavior": "allow",
                    "updatedInput": input.unwrap_or(self.input),
                });
                if let Some(updates) = updates {
                    decision["updatedPermissions"] = updates;
                }
                decision
            }
            Some(Choice::Deny { message }) => json!({"behavior": "deny", "message": message}),
            None => json!({
                "behavior": "deny",
                "message": "The user cancelled.",
                "interrupt": true,
            }),
        };
        super::control_success(&self.control, decision)
    }
}

type Offered = (wire::PermissionOption, Choice);

fn allow_once(id: &str, name: &str) -> Offered {
    (
        wire::PermissionOption::new(id.to_owned(), name, wire::PermissionOptionKind::AllowOnce),
        Choice::Allow {
            input: None,
            updates: None,
        },
    )
}

fn reject(id: &str, name: &str, message: &'static str) -> Offered {
    (
        wire::PermissionOption::new(id.to_owned(), name, wire::PermissionOptionKind::RejectOnce),
        Choice::Deny { message },
    )
}

/// Leaving plan mode also picks the mode to code in.
fn exit_plan() -> Vec<Offered> {
    let switch =
        |mode: &str| Some(json!([{"type": "setMode", "mode": mode, "destination": "session"}]));
    vec![
        (
            wire::PermissionOption::new(
                "plan-accept-edits",
                "Yes, and accept edits",
                wire::PermissionOptionKind::AllowAlways,
            ),
            Choice::Allow {
                input: None,
                updates: switch("acceptEdits"),
            },
        ),
        (
            wire::PermissionOption::new(
                "plan-default",
                "Yes, and ask before edits",
                wire::PermissionOptionKind::AllowOnce,
            ),
            Choice::Allow {
                input: None,
                updates: switch("default"),
            },
        ),
        reject(
            "reject",
            "No, keep planning",
            "The user wants to keep planning.",
        ),
    ]
}

/// One question's choices become the options; the answer goes back as the
/// tool's input, as Claude Code expects. Several questions at once are
/// left to the chat.
fn question(input: &Value) -> Vec<Offered> {
    let Some(asked) = tools::question(input) else {
        return vec![reject(
            "reject",
            "Answer in the chat",
            "Ask these questions in your reply instead; the user will answer there.",
        )];
    };
    let labels: Vec<&str> = input
        .get("questions")
        .and_then(|questions| questions.get(0))
        .filter(|question| question.get("multiSelect") != Some(&Value::Bool(true)))
        .and_then(|question| question.get("options"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| option.get("label").and_then(Value::as_str))
        .collect();
    let mut options: Vec<Offered> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let mut answered = input.as_object().cloned().unwrap_or_else(Map::new);
            answered.insert("answers".to_owned(), json!({ asked: label }));
            (
                out::manual(wire::PermissionOption::new(
                    format!("answer-{index}"),
                    *label,
                    wire::PermissionOptionKind::AllowOnce,
                )),
                Choice::Allow {
                    input: Some(Value::Object(answered)),
                    updates: None,
                },
            )
        })
        .collect();
    let refusal = if options.is_empty() {
        "Ask this question in your reply instead; the user will answer there."
    } else {
        "The user chose not to answer."
    };
    options.push(reject("reject", "Answer in the chat", refusal));
    options
}

/// The one question of an `AskUserQuestion` call as clients show it.
fn shown_question(input: &Value) -> Option<Question> {
    let text = tools::question(input)?;
    let asked = input.get("questions")?.get(0)?;
    let field =
        |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(Question {
        header: field(asked, "header"),
        multiple: asked.get("multiSelect") == Some(&Value::Bool(true)),
        options: asked
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| {
                Some(QuestionOption {
                    description: field(option, "description"),
                    label: field(option, "label")?,
                })
            })
            .collect(),
        text: text.to_owned(),
    })
}

/// What "always" means for the updates Claude suggests.
fn always_name(suggestions: &[Value]) -> String {
    let first = &suggestions[0];
    match first.get("type").and_then(Value::as_str) {
        Some("setMode") => match first.get("mode").and_then(Value::as_str) {
            Some("acceptEdits") => "Yes, and accept edits this session".to_owned(),
            Some("bypassPermissions") => "Yes, and stop asking this session".to_owned(),
            _ => "Yes, and switch mode".to_owned(),
        },
        Some("addRules") => {
            let rules: Vec<String> = first
                .get("rules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|rule| {
                    let tool = rule.get("toolName")?.as_str()?;
                    Some(match rule.get("ruleContent").and_then(Value::as_str) {
                        Some(content) => format!("{tool}({content})"),
                        None => tool.to_owned(),
                    })
                })
                .collect();
            if rules.is_empty() {
                "Yes, and don't ask again".to_owned()
            } else {
                format!("Yes, and don't ask again for {}", rules.join(", "))
            }
        }
        _ => "Yes, and don't ask again".to_owned(),
    }
}
