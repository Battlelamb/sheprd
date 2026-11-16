//! The texts a note is made of: the built-in templates in this crate's
//! `prompts` folder, each replaced by a copy the user edits in
//! `chat-prompts` under Herdr's configuration directory, and the user's own
//! instructions. Read once when the chat starts, on the machine the agent
//! runs on.

use std::{
    borrow::Cow,
    io,
    path::{Path, PathBuf},
};

use sheprd_chat_protocol::{Device, Origin};

use super::template;

/// The longest file read, in bytes; a longer one is not used.
pub(crate) const MAX_FILE: u64 = 16 * 1024;

/// The folder the user's copies live in, under the configuration root.
const DIR: &str = "herdr/chat-prompts";
/// Where instructions lived before the folder; read when it has none.
const LEGACY_INSTRUCTIONS: &str = "herdr/chat-system-prompt.md";
const INSTRUCTIONS: &str = "instructions.md";

/// Pictures read per message and their size, as [`crate::pictures`] and
/// [`crate::files`] bound them, for the templates to quote.
const MAX_PICTURES: usize = crate::pictures::MAX_PICTURES;
const MAX_IMAGE_MB: u64 = crate::files::MAX_IMAGE / (1024 * 1024);

/// Declares each template once: its kind, its file, and its built-in text.
macro_rules! kinds {
    ($($kind:ident => $file:literal,)*) => {
        /// Which template a part of the note comes from.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub(crate) enum Kind {
            $($kind,)*
        }

        impl Kind {
            const ALL: &[Self] = &[$(Self::$kind,)*];

            fn file(self) -> &'static str {
                match self {
                    $(Self::$kind => $file,)*
                }
            }

            fn builtin(self) -> &'static str {
                match self {
                    $(Self::$kind => include_str!(concat!("../../prompts/", $file)),)*
                }
            }
        }
    };
}

kinds! {
    // keep-sorted start
    Desktop => "desktop.md",
    Intro => "intro.md",
    Phone => "phone.md",
    Remote => "remote.md",
    Terminal => "terminal.md",
    // keep-sorted end
}

impl Kind {
    /// The template for a person typing at `origin`.
    pub(crate) fn of(origin: Origin) -> Self {
        match origin.device {
            Device::PhoneApp | Device::PhoneWeb => Self::Phone,
            Device::Desktop if origin.opens_paths() => Self::Desktop,
            Device::Terminal if origin.opens_paths() => Self::Terminal,
            Device::Desktop | Device::Terminal | Device::Other => Self::Remote,
        }
    }
}

/// Why a file of the user's was not used, or not all of it.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Problem {
    #[error("{} is not used: it is larger than 16 KiB", path.display())]
    TooLarge { path: PathBuf },
    #[error("{} is not used: it is not UTF-8 text", path.display())]
    NotText { path: PathBuf },
    #[error("{} is not used: it cannot be read", path.display())]
    Unreadable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} uses {{{{ {name} }}}}, which Sheprd does not know; it is sent as written", path.display())]
    UnknownName { name: String, path: PathBuf },
}

/// The templates and instructions a chat uses.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Prompts {
    /// The user's instructions for every chat, already filled in.
    pub(crate) instructions: Option<String>,
    /// The user's copies, by kind; a kind without one uses its built-in.
    templates: Vec<(Kind, String)>,
}

/// What every template may name besides where the person is.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Facts {
    pub(crate) agent: String,
    pub(crate) cwd: String,
}

/// Names that depend on where the person is, which instructions may not
/// use: instructions go once, wherever the person is then.
const PLACE_NAMES: &[&str] = &["connection", "device", "where"];
const FACT_NAMES: &[&str] = &["agent", "cwd", "max_image_mb", "max_pictures", "os"];

impl Prompts {
    /// Built-in templates with `instructions`.
    #[cfg(test)]
    pub(crate) fn with_instructions(instructions: &str) -> Self {
        Self {
            instructions: Some(instructions.to_owned()),
            templates: Vec::new(),
        }
    }

    /// The user's folder on this machine.
    pub(crate) fn dir() -> Option<PathBuf> {
        let root = sheprd_client::local_path::config_root().ok()?;
        Some(root.join(DIR))
    }

    /// Reads the user's files from `dir` (and the older instructions file
    /// beside it), and what was wrong with any of them.
    pub(crate) fn load(dir: &Path, facts: &Facts) -> (Self, Vec<Problem>) {
        let mut problems = Vec::new();
        let mut templates = Vec::new();
        for &kind in Kind::ALL {
            let path = dir.join(kind.file());
            if let Some(text) = read(&path, &mut problems) {
                check(&path, &text, &[PLACE_NAMES, FACT_NAMES], &mut problems);
                templates.push((kind, text));
            }
        }
        let path = dir.join(INSTRUCTIONS);
        let legacy = dir
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join(LEGACY_INSTRUCTIONS));
        let instructions = read(&path, &mut problems)
            .map(|text| (path, text))
            .or_else(|| {
                let legacy = legacy?;
                read(&legacy, &mut problems).map(|text| (legacy, text))
            })
            .map(|(path, text)| {
                check(&path, &text, &[FACT_NAMES], &mut problems);
                fill(&text, facts, None)
            });
        let prompts = Self {
            instructions,
            templates,
        };
        (prompts, problems)
    }

    /// The template of `kind` filled in for a person at `origin`.
    pub(crate) fn render(&self, kind: Kind, facts: &Facts, origin: Option<Origin>) -> String {
        let template = self
            .templates
            .iter()
            .find(|(known, _)| *known == kind)
            .map_or(kind.builtin(), |(_, text)| text.as_str());
        fill(template, facts, origin)
    }
}

fn fill(template: &str, facts: &Facts, origin: Option<Origin>) -> String {
    let rendered = template::render(template, |name| {
        Some(match name {
            "agent" => Cow::Borrowed(facts.agent.as_str()),
            "connection" => Cow::Borrowed(super::connection_name(origin?.via)),
            "cwd" => Cow::Borrowed(facts.cwd.as_str()),
            "device" => Cow::Borrowed(super::device_name(origin?.device)),
            "max_image_mb" => Cow::Owned(MAX_IMAGE_MB.to_string()),
            "max_pictures" => Cow::Owned(MAX_PICTURES.to_string()),
            "os" => Cow::Borrowed(std::env::consts::OS),
            "where" => Cow::Owned(super::place_phrase(origin?)),
            _ => return None,
        })
    });
    rendered.trim().to_owned()
}

/// The text of the file at `path`, when it exists and says something; a
/// file that cannot be used is a problem rather than a silent default.
fn read(path: &Path, problems: &mut Vec<Problem>) -> Option<String> {
    let path_buf = || path.to_owned();
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(source) => {
            problems.push(Problem::Unreadable {
                path: path_buf(),
                source,
            });
            return None;
        }
    };
    if metadata.len() > MAX_FILE {
        problems.push(Problem::TooLarge { path: path_buf() });
        return None;
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) => {
            problems.push(Problem::Unreadable {
                path: path_buf(),
                source,
            });
            return None;
        }
    };
    if bytes.len() as u64 > MAX_FILE {
        problems.push(Problem::TooLarge { path: path_buf() });
        return None;
    }
    let Ok(text) = String::from_utf8(bytes) else {
        problems.push(Problem::NotText { path: path_buf() });
        return None;
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Warns once about each name in `text` none of `known` holds.
fn check(path: &Path, text: &str, known: &[&[&str]], problems: &mut Vec<Problem>) {
    let mut warned: Vec<&str> = Vec::new();
    for name in template::names(text) {
        if known.iter().any(|names| names.contains(&name)) || warned.contains(&name) {
            continue;
        }
        warned.push(name);
        problems.push(Problem::UnknownName {
            name: name.to_owned(),
            path: path.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests;
