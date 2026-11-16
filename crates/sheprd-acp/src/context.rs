//! What the agent is told besides the person's words: the user's chat
//! system prompt, once per conversation, and where the person is typing
//! whenever they switch devices, so the agent answers in a form they can
//! see. A phone cannot open a path on this computer, so there the agent is
//! told to put pictures in the chat instead.
//!
//! With the first prompt it also tells the agent how to use Sheprd's
//! repository memory, shared by every worktree, tab, and chat of the
//! repository on this host, when a `sheprd` command is here to use it.
//!
//! ACP has no system prompt every agent takes, so all of it goes as a text
//! block ahead of the prompt's own. The transcript shows only the person's
//! words.

use std::path::{Path, PathBuf};

use sheprd_chat_protocol::{Device, Origin, Via};

pub(crate) use note::{Place, Told, hide, split};

mod note;

/// The longest chat system prompt read, in bytes; a longer file is skipped.
pub(crate) const MAX_SYSTEM_PROMPT: u64 = 16 * 1024;

/// The file the user writes their chat system prompt in, under Herdr's
/// configuration directory.
const SYSTEM_PROMPT_FILE: &str = "herdr/chat-system-prompt.md";

/// Where the user's chat system prompt lives on this machine.
pub(crate) fn system_prompt_path() -> Option<PathBuf> {
    sheprd_client::local_path::config_root()
        .ok()
        .map(|root| root.join(SYSTEM_PROMPT_FILE))
}

/// The chat system prompt in `path`: its text when the file exists, is at
/// most [`MAX_SYSTEM_PROMPT`] bytes of UTF-8, and says something.
pub(crate) fn read_system_prompt(path: &Path) -> Option<String> {
    let bytes = crate::files::read(path, MAX_SYSTEM_PROMPT)?;
    let text = String::from_utf8(bytes).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The `sheprd` command beside this program, as releases install them, or
/// else on `PATH`: the one the memory guide names.
pub(crate) fn sheprd_executable() -> Option<PathBuf> {
    let name = format!("sheprd{}", std::env::consts::EXE_SUFFIX);
    let beside = std::env::current_exe()
        .ok()
        .and_then(|program| Some(program.parent()?.join(&name)));
    let on_path = || {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(&name))
            .find(|path| path.is_file())
    };
    beside.filter(|path| path.is_file()).or_else(on_path)
}

/// How the agent keeps and searches the repository's memory with `sheprd`.
pub(crate) fn memory_guide(sheprd: &Path) -> String {
    let text = sheprd.display().to_string();
    let sheprd = if text.contains(char::is_whitespace) {
        format!("\"{text}\"")
    } else {
        text
    };
    format!(
        "Sheprd keeps a memory for this Git repository, shared by every worktree, tab, and \
         chat of it on this computer, so what one agent learns the next one can find:\n\
         - Before you start a task, search it for what earlier agents learned about that \
         area: `{sheprd} memory search WORDS`. `{sheprd} memory` lists the repository's \
         entries and this branch's.\n\
         - When you learn something the next agent would otherwise have to find out again \
         (a build or test command that works, a trap and what to do instead, a decision \
         and its reason, how the person wants things done), add it in a sentence or two: \
         `{sheprd} memory add --kind fact|gotcha|command|decision|preference TEXT`.\n\
         - For this branch's work only, such as where you stopped, add `--branch --kind \
         handoff`. Remove a wrong entry with `{sheprd} memory forget ID`.\n\
         Keep entries short and true, and add what the code and its docs do not already \
         say. Never add secrets, credentials, tokens, or private output.\n"
    )
}

/// What the agent has been told in this conversation.
#[derive(Debug, Default)]
pub(crate) struct Context {
    /// The user's system prompt now, escaped for the note.
    system_prompt: Option<String>,
    /// What the agent last heard, from notes sent, restored, or replayed.
    /// Compared only when a prompt goes, so a replay may run in any order.
    told: Told,
    /// How to use the repository's memory, escaped for the note; `None`
    /// without a `sheprd` command to use it with.
    memory: Option<String>,
}

impl Context {
    /// A conversation in which earlier notes told the agent `told`.
    pub(crate) fn new(system_prompt: Option<String>, told: Told) -> Self {
        Self {
            system_prompt: system_prompt.map(|text| note::escape(&text)),
            told,
            memory: None,
        }
    }

    /// Tells the agent, with the first prompt, how to use the memory
    /// `sheprd` keeps, unless an earlier note of this conversation did.
    pub(crate) fn with_memory(mut self, sheprd: Option<&Path>) -> Self {
        self.memory = sheprd.map(|sheprd| note::escape(&memory_guide(sheprd)));
        self
    }

    /// Takes in a note the agent replayed from its saved conversation.
    pub(crate) fn heard(&mut self, note: &str) {
        self.told.heard(note);
    }

    /// The note to send ahead of a prompt typed at `origin`: the system
    /// prompt unless the agent last heard these very instructions, and
    /// where the person is when that changes what they can see. None when
    /// there is nothing new.
    pub(crate) fn before(&mut self, origin: Option<Origin>) -> Option<String> {
        let system_prompt = self
            .system_prompt
            .clone()
            .filter(|text| self.told.instructions.as_ref() != Some(text));
        let memory = self.memory.clone().filter(|_| !self.told.memory);
        let origin = origin.filter(|origin| self.told.place != Some(Place::from(*origin)));
        if system_prompt.is_none() && memory.is_none() && origin.is_none() {
            return None;
        }
        let mut body = String::from(
            "This note comes from Sheprd, the app the person chats with you in, \
             not from the person. It holds until another note replaces it.\n",
        );
        if let Some(system_prompt) = system_prompt {
            body.push_str("\nThe person's instructions for every chat:\n");
            body.push_str(&note::instructions(&system_prompt));
            self.told.instructions = Some(system_prompt);
        }
        if let Some(memory) = memory {
            body.push('\n');
            body.push_str(&note::memory(&memory));
            self.told.memory = true;
        }
        let place = origin.map(Place::from);
        if let Some(origin) = origin {
            self.told.place = place;
            body.push('\n');
            body.push_str(&describe(origin));
        }
        Some(note::wrap(place, &body))
    }
}

/// Where the person is and what they can see there.
pub(crate) fn describe(origin: Origin) -> String {
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
    let mut text = format!("The person is typing {device}{via}.\n");
    if origin.opens_paths() {
        text.push_str(
            "They can open files and folders on this computer, so paths are fine. \
             A picture linked as a Markdown image, ![what it shows](/absolute/path.png), \
             also shows in the chat.\n",
        );
        if origin.device == Device::Desktop {
            text.push_str(
                "For HTML pages and mockups, `sheprd browser open PATH` opens a tab \
                 beside the chat.\n",
            );
        }
        return text;
    }
    text.push_str(
        "They cannot open files or folders on this computer: a path shows them nothing, \
         and neither does a link to a local file. Put what they should see in the chat:\n\
         - Pictures, such as screenshots, mockups, and diagrams: save each as PNG, JPEG, \
         GIF, or WebP, under 3 MB, and link it in your reply as a Markdown image, \
         ![what it shows](/absolute/path.png). Sheprd attaches the file to the chat; \
         at most 4 pictures per message.\n\
         - HTML mockups and pages: render each variant to a PNG screenshot and link \
         those pictures, never only the .html file or its folder.\n\
         - Code and text: quote the part that matters in your reply.\n",
    );
    if matches!(origin.device, Device::PhoneApp | Device::PhoneWeb) {
        text.push_str("Their screen is small: keep replies short and tables narrow.\n");
    }
    text
}

#[cfg(test)]
mod tests;
