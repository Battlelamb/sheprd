//! Guests: people or agents outside the chat whose words an external tool,
//! such as a session-sharing tool, relays into it.
//!
//! A guest prompt is never the person's: it waits as a permission request
//! until the person allows it, and the agent is told who wrote it, through
//! which tool, and that it is untrusted. See `docs/chat-guests.md`.

use serde::{Deserialize, Serialize};

/// The longest guest name, in bytes.
pub const MAX_GUEST_NAME: usize = 64;
/// The longest tool name, in bytes.
pub const MAX_GUEST_TOOL: usize = 32;
/// The longest prompt a guest may send, in bytes: it is shown in full in the
/// permission request every client draws.
pub const MAX_GUEST_PROMPT_SIZE: usize = 16 * 1024;
/// The most guest prompts waiting for the person at once.
pub const MAX_GUESTS_WAITING: usize = 8;

/// Who wrote a prompt the person did not type, as the relaying tool names
/// them. Neither name is verified by Sheprd: they are labels the tool
/// supplies, shown to the person and the agent as such.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Guest {
    /// The guest's display name, such as `Bob`.
    pub name: String,
    /// The tool that relayed the prompt, such as `tma`.
    pub tool: String,
}

impl Guest {
    pub fn new(name: impl Into<String>, tool: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tool: tool.into(),
        }
    }

    /// Whether both names are something a client may show and a note may
    /// quote: not blank, within their limits, and free of control
    /// characters and of the characters that end a quoted attribute.
    pub fn is_valid(&self) -> bool {
        label_ok(&self.name, MAX_GUEST_NAME) && label_ok(&self.tool, MAX_GUEST_TOOL)
    }

    /// How clients name the guest, such as `Bob via tma`.
    pub fn label(&self) -> String {
        format!("{} via {}", self.name, self.tool)
    }
}

fn label_ok(text: &str, max: usize) -> bool {
    !text.trim().is_empty()
        && text.len() <= max
        && !text
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '<' | '>' | '&' | '`'))
}

/// A guest's prompt waiting for the person, carried by its permission
/// request so a client can show it in full.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GuestPrompt {
    pub guest: Guest,
    pub text: String,
}

/// The option ids of a guest prompt's permission request.
pub const GUEST_ALLOW: &str = "guest_allow";
pub const GUEST_DENY: &str = "guest_deny";
