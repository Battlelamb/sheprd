//! The note as the agent reads it and its saved conversation keeps it,
//! ahead of the person's words:
//!
//! ```text
//! <sheprd device="phone_app">
//! …
//! <instructions>
//! the user's chat system prompt
//! </instructions>
//! …
//! </sheprd>
//! ```
//!
//! A restored or mirrored conversation hides these notes, as the live chat
//! does, and learns from them what the agent was already told, so a chat
//! that continues does not tell it again.

use sheprd_chat_protocol::{Device, EntryKind, Transcript};

const OPEN: &str = "<sheprd";
const CLOSE: &str = "</sheprd>";
const INSTRUCTIONS: &str = "<instructions>\n";
const INSTRUCTIONS_END: &str = "\n</instructions>";

/// What earlier notes told the agent; a later note overrides an earlier one.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Told {
    pub(crate) device: Option<Device>,
    /// The instructions as the note wrote them, see [`escape`].
    pub(crate) instructions: Option<String>,
}

impl Told {
    pub(crate) fn heard(&mut self, note: &str) {
        if let Some(device) = device_of(note) {
            self.device = Some(device);
        }
        if let Some(instructions) = instructions_of(note) {
            self.instructions = Some(instructions.to_owned());
        }
    }
}

/// The opening tag of a note, naming the device when it says where the
/// person is.
pub(super) fn open(device: Option<Device>) -> String {
    let name = device
        .and_then(|device| serde_json::to_value(device).ok())
        .and_then(|value| value.as_str().map(str::to_owned));
    match name {
        Some(name) => format!("{OPEN} device=\"{name}\">\n"),
        None => format!("{OPEN}>\n"),
    }
}

pub(super) fn close() -> &'static str {
    CLOSE
}

/// The instructions block for `instructions`, already [`escape`]d.
pub(super) fn instructions(instructions: &str) -> String {
    format!("{INSTRUCTIONS}{instructions}{INSTRUCTIONS_END}\n")
}

/// `text` with nothing in it that would end the note or its instructions
/// early.
pub(super) fn escape(text: &str) -> String {
    text.replace(CLOSE, "</ sheprd>")
        .replace("</instructions>", "</ instructions>")
}

/// The note `text` starts with, and the person's words after it.
pub(crate) fn split(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    let rest = text.strip_prefix(OPEN)?;
    if !rest.starts_with(['>', ' ']) {
        return None;
    }
    let end = text.find(CLOSE)? + CLOSE.len();
    Some((&text[..end], text[end..].trim_start()))
}

/// Takes the notes out of `transcript`'s messages from the person, and a
/// message that held nothing else, and says what they told the agent.
pub(crate) fn hide(transcript: &mut Transcript) -> Told {
    let mut told = Told::default();
    transcript.entries.retain_mut(|entry| {
        let EntryKind::User { text, files } = &mut entry.kind else {
            return true;
        };
        let Some((note, rest)) = split(text) else {
            return true;
        };
        told.heard(note);
        *text = rest.to_owned();
        !text.is_empty() || *files > 0
    });
    told
}

fn device_of(note: &str) -> Option<Device> {
    let tag = &note[..note.find('>')?];
    let name = tag.split_once("device=\"")?.1.split_once('"')?.0;
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

fn instructions_of(note: &str) -> Option<&str> {
    let (_, rest) = note.split_once(INSTRUCTIONS)?;
    Some(rest.split_once(INSTRUCTIONS_END)?.0)
}
