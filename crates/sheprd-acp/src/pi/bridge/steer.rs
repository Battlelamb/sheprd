//! Prompts steered into pi's running turn. pi queues a `steer` and hands it
//! to the model after the step that runs, and its `queue_update` events list
//! what still waits: a steered prompt that leaves that list was taken. A
//! prompt that waits when a run ends is taken by the run pi then continues
//! with, which the session's prompt waits for; one pi kept without running
//! is withdrawn and given back to the session.

use acp::v1 as wire;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Bridge, Pending};
use crate::harness::{Out, out, steer};

/// A prompt sent to pi's steering queue.
#[derive(Debug)]
pub(super) struct Steering {
    id: Uuid,
    /// pi answered, so its queue lists the prompt until the model takes it.
    queued: bool,
    text: String,
}

impl Bridge {
    /// Sends the prompt to pi's steering queue while a turn runs.
    pub(super) fn steer(&mut self, acp: wire::RequestId, params: Value) -> Vec<Out> {
        let Ok(request) = serde_json::from_value::<steer::Steer>(params) else {
            return vec![out::reply(acp, Err(wire::Error::invalid_params()))];
        };
        if self.prompt.is_none() {
            return vec![out::reply(acp, Err(out::failure("No turn is running.")))];
        }
        let (text, images) = super::message(&request.prompt);
        let mut command = json!({"type": "steer", "message": text});
        if !images.is_empty() {
            command["images"] = Value::Array(images);
        }
        self.steers.push(Steering {
            id: request.id,
            queued: false,
            text,
        });
        vec![self.command(
            command,
            Pending::Steer {
                acp,
                prompt: request.id,
            },
        )]
    }

    /// pi's answer to a steered prompt.
    pub(super) fn steer_answered(
        &mut self,
        acp: wire::RequestId,
        prompt: Uuid,
        failed: Option<String>,
        data: &Value,
    ) -> Vec<Out> {
        let Some(index) = self.steers.iter().position(|steer| steer.id == prompt) else {
            return vec![out::reply(acp, Ok(json!({})))];
        };
        if let Some(error) = failed {
            self.steers.remove(index);
            return vec![out::reply(acp, Err(out::failure(error)))];
        }
        // The turn ended before pi queued it: it would wait for a prompt
        // that may never come, so it goes as the next prompt instead.
        if self.prompt.is_none() {
            self.steers.remove(index);
            return vec![
                self.command(json!({"type": "clear_queue"}), Pending::ClearQueue),
                out::reply(acp, Err(out::failure("The turn ended."))),
            ];
        }
        let mut outs = vec![out::reply(acp, Ok(json!({})))];
        // An input handler of pi's took it, so no queue lists it.
        if data.get("disposition").and_then(Value::as_str) == Some("handled") {
            self.steers.remove(index);
            outs.push(steer::steered(&self.session, prompt));
            return outs;
        }
        self.steers[index].queued = true;
        outs.extend(self.taken());
        outs
    }

    /// pi's steering queue changed.
    pub(super) fn queue_changed(&mut self, message: &Value) -> Vec<Out> {
        self.waiting = message
            .get("steering")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        self.taken()
    }

    /// The queued prompts pi's queue no longer lists, which the model took.
    fn taken(&mut self) -> Vec<Out> {
        let mut waiting = self.waiting.clone();
        let mut outs = Vec::new();
        self.steers.retain(|steer| {
            if !steer.queued {
                return true;
            }
            if let Some(at) = waiting.iter().position(|text| *text == steer.text) {
                waiting.remove(at);
                return true;
            }
            outs.push(steer::steered(&self.session, steer.id));
            false
        });
        outs
    }

    /// A run ended with `messages`. Prompts pi still queues are taken by
    /// the run it continues with, so the run's messages are held until that
    /// one ends. A cancelled run does not continue, and a pi too old to say
    /// when it settles is not waited for: what it queues is given back.
    pub(super) fn give_back_unless_held(
        &mut self,
        messages: Option<&Value>,
        aborted: bool,
    ) -> Vec<Out> {
        let mut outs = self.taken();
        let waiting = self.steers.iter().any(|steer| steer.queued);
        if waiting && !aborted && self.settles {
            self.held = Some(messages.cloned().unwrap_or(Value::Null));
            return outs;
        }
        self.held = None;
        outs.extend(self.give_back());
        outs
    }

    /// Withdraws the prompts pi still queues and gives them back to the
    /// session, which sends them as the next prompt.
    fn give_back(&mut self) -> Vec<Out> {
        let mut outs = self.taken();
        let (back, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.steers)
            .into_iter()
            .partition(|steer| steer.queued);
        self.steers = kept;
        if back.is_empty() {
            return outs;
        }
        outs.push(self.command(json!({"type": "clear_queue"}), Pending::ClearQueue));
        outs.extend(
            back.into_iter()
                .map(|steer| steer::unsteered(&self.session, steer.id)),
        );
        outs
    }

    /// pi has no more work of its own: a prompt it still queues will not
    /// run, and the run held for it ends.
    pub(super) fn settled(&mut self) -> Vec<Out> {
        self.settles = true;
        let Some(messages) = self.held.take() else {
            return Vec::new();
        };
        let mut outs = self.give_back();
        outs.extend(self.finished(Some(&messages)));
        outs
    }
}
