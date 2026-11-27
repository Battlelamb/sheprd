//! The effect directives an agent's reply may hold (`docs/rich-chat.md`,
//! Text size and effects), as far as a push cares: `::wizz` asks for the
//! person's attention, and no directive belongs in a notification's text.

/// Block interaction directives that carry an optional title on their opening
/// line (`:::urgent Tap your YubiKey` … `:::`).
const INTERACTION_BLOCKS: &[&str] = &[
    "countdown",
    "diff",
    "drop",
    "modal",
    "oauth",
    "otp",
    "progress",
    "runbook",
    "secret",
    "spotlight",
    "urgent",
];

/// Block directives that ask for urgent/time-sensitive attention on a phone.
const URGENT_BLOCKS: &[&str] = &["countdown", "oauth", "otp", "secret", "urgent"];

/// Whether `line` is a directive: `::name`, `::cue name`, `:::name [title]`,
/// or the `:::` that ends a block, indented by three spaces at most, as a line
/// indented further is code.
fn directive(line: &str) -> Option<&str> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 || line.starts_with('\t') {
        return None;
    }
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("::")?;
    if let Some(block) = rest.strip_prefix(':') {
        if block.is_empty() {
            return Some("");
        }
        let name = block.split_whitespace().next().unwrap_or("");
        if block == name && name.chars().all(|c| c.is_ascii_lowercase()) {
            return Some(name);
        }
        if INTERACTION_BLOCKS.contains(&name) {
            return Some(name);
        }
        return None;
    }
    if rest.chars().all(|c| c.is_ascii_lowercase()) {
        return Some(rest);
    }
    if let Some(cue) = rest.strip_prefix("cue ")
        && !cue.trim().is_empty()
        && cue.trim().chars().all(|c| c.is_ascii_lowercase())
    {
        return Some("cue");
    }
    None
}

/// Optional title carried on an interaction directive's opening line.
fn interaction_title(line: &str) -> Option<&str> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 || line.starts_with('\t') {
        return None;
    }
    let block = line.trim().strip_prefix(":::")?;
    let mut parts = block.splitn(2, char::is_whitespace);
    let name = parts.next()?;
    if !INTERACTION_BLOCKS.contains(&name) {
        return None;
    }
    parts
        .next()
        .map(str::trim)
        .filter(|title| !title.is_empty())
}

/// A code fence: its marker, and how long a run of it opened the fence.
#[derive(Clone, Copy)]
struct Fence {
    marker: char,
    length: usize,
}

impl Fence {
    /// The fence `line` opens: three or more backticks or tildes, indented
    /// by three spaces at most.
    fn open(line: &str) -> Option<Self> {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() > 3 {
            return None;
        }
        let marker = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let length = trimmed.chars().take_while(|c| *c == marker).count();
        (length >= 3).then_some(Self { marker, length })
    }

    /// Whether `line` closes the fence: a run of its marker at least as
    /// long, and nothing after it.
    fn closes(self, line: &str) -> bool {
        let trimmed = line.trim_start_matches(' ');
        let run = trimmed.chars().take_while(|c| *c == self.marker).count();
        run >= self.length && trimmed[run * self.marker.len_utf8()..].trim().is_empty()
    }
}

/// The lines of `text`, each with whether it is prose: outside a code
/// fence, where a directive counts.
fn prose(text: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut fence: Option<Fence> = None;
    text.lines().map(move |line| match fence {
        Some(open) => {
            if open.closes(line) {
                fence = None;
            }
            (line, false)
        }
        None => {
            fence = Fence::open(line);
            (line, fence.is_none())
        }
    })
}

/// Whether the agent's reply asks for the person's urgent attention: a
/// `::wizz` line or an urgent interaction block (`:::urgent`, `:::otp`,
/// `:::oauth`, `:::secret`, `:::countdown`) outside code.
pub(super) fn wizzes(text: &str) -> bool {
    prose(text).any(|(line, prose)| {
        if !prose {
            return false;
        }
        let Some(name) = directive(line) else {
            return false;
        };
        let trimmed = line.trim();
        (trimmed == "::wizz" && name == "wizz")
            || (trimmed.starts_with(":::") && URGENT_BLOCKS.contains(&name))
    })
}

/// `text` without its directive lines, which only mean something drawn; an
/// interaction block's opening title stays as readable text for push excerpts.
pub(super) fn without_directives(text: &str) -> String {
    prose(text)
        .filter_map(|(line, prose)| {
            if prose && directive(line).is_some() {
                interaction_title(line)
            } else {
                Some(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
