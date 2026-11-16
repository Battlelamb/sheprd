//! What the agent is told besides the person's words: the user's chat
//! instructions, once per conversation, and where the person is typing
//! whenever they switch devices, so the agent answers in a form they can
//! see. A phone cannot open a path on this computer, so there the agent is
//! told to put pictures in the chat instead.
//!
//! ACP has no system prompt every agent takes, so both go as a text block
//! ahead of the prompt's own. The transcript shows only the person's words.
//! The texts are templates the user may replace ([`prompts`]).

use sheprd_chat_protocol::{Device, Origin, Via};

pub(crate) use note::{Place, Told, hide, split};
pub(crate) use prompts::{Facts, Kind, Prompts};

mod note;
mod prompts;
mod template;

/// What the agent has been told in this conversation.
#[derive(Debug, Default)]
pub(crate) struct Context {
    facts: Facts,
    /// Where the person last said they are, for the intro of a note sent
    /// for a client that does not say.
    last: Option<Origin>,
    prompts: Prompts,
    /// What the agent last heard, from notes sent, restored, or replayed.
    /// Compared only when a prompt goes, so a replay may run in any order.
    told: Told,
}

impl Context {
    /// A conversation in which earlier notes told the agent `told`.
    pub(crate) fn new(prompts: Prompts, facts: Facts, told: Told) -> Self {
        Self {
            facts,
            last: None,
            prompts,
            told,
        }
    }

    /// Takes in a note the agent replayed from its saved conversation.
    pub(crate) fn heard(&mut self, note: &str) {
        self.told.heard(note);
    }

    /// The chat switched to another agent, which the texts name from now.
    pub(crate) fn switched(&mut self, agent: &str) {
        agent.clone_into(&mut self.facts.agent);
    }

    /// The note to send ahead of a prompt typed at `origin`: the
    /// instructions unless the agent last heard these very ones, and where
    /// the person is when that changes what they can see. None when there
    /// is nothing new.
    pub(crate) fn before(&mut self, origin: Option<Origin>) -> Option<String> {
        if origin.is_some() {
            self.last = origin;
        }
        let instructions = self
            .prompts
            .instructions(&self.facts)
            .map(|text| note::escape(&text))
            .filter(|text| self.told.instructions.as_ref() != Some(text));
        let moved = origin.filter(|origin| self.told.place != Some(Place::from(*origin)));
        if instructions.is_none() && moved.is_none() {
            return None;
        }
        let mut body = self.part(Kind::Intro, self.last);
        if let Some(instructions) = instructions {
            body.push_str("\nThe person's instructions for every chat:\n");
            body.push_str(&note::instructions(&instructions));
            self.told.instructions = Some(instructions);
        }
        let place = moved.map(Place::from);
        if let Some(origin) = moved {
            self.told.place = place;
            body.push('\n');
            body.push_str(&self.part(Kind::of(origin), Some(origin)));
        }
        Some(note::wrap(place, &body))
    }

    /// One template filled in and escaped, as a paragraph of the note.
    fn part(&self, kind: Kind, origin: Option<Origin>) -> String {
        let mut text = note::escape(&self.prompts.render(kind, &self.facts, origin));
        text.push('\n');
        text
    }
}

/// What `{{ device }}` says.
fn device_name(device: Device) -> &'static str {
    match device {
        Device::Desktop => "Sheprd's desktop app",
        Device::PhoneApp => "Sheprd's iPhone and iPad app",
        Device::PhoneWeb => "Sheprd's web app on a phone",
        Device::Terminal => "the chat view of this terminal pane",
        Device::Other => "a Sheprd client",
    }
}

/// What `{{ connection }}` says.
fn connection_name(via: Via) -> &'static str {
    match via {
        Via::Lan => "the local network",
        Via::Local => "this computer",
        Via::Relay => "Sheprd's encrypted relay",
        Via::Remote => "SSH",
        Via::Tailscale => "Tailscale",
        Via::Other => "a remote connection",
    }
}

/// What `{{ where }}` says: the device and how it reaches this computer,
/// to follow "The person is typing".
fn place_phrase(origin: Origin) -> String {
    let device = match origin.device {
        Device::Desktop => "in Sheprd's desktop app",
        Device::PhoneApp => "on their phone or tablet, in Sheprd's iPhone and iPad app",
        Device::PhoneWeb => "on their phone, in Sheprd's web app",
        Device::Terminal => "in the chat view of this terminal pane",
        Device::Other => "in a Sheprd client",
    };
    let via = match origin.via {
        Via::Lan => ", connected to this computer over the local network",
        Via::Local => " on this computer",
        Via::Relay => ", connected to this computer through Sheprd's encrypted relay",
        Via::Remote => " on another computer, connected to this one over SSH",
        Via::Tailscale => ", connected to this computer over Tailscale",
        Via::Other => ", connected to this computer remotely",
    };
    format!("{device}{via}")
}

#[cfg(test)]
mod tests;
