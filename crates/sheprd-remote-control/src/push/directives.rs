//! The effect directives an agent's reply may hold (`docs/rich-chat.md`,
//! Text size and effects), as far as a push cares: `::wizz` asks for the
//! person's attention, and no directive belongs in a notification's text.

/// Whether `line` is a directive: `::name`, `:::name`, or the `:::` that
/// ends a bubble, indented by three spaces at most, as a line indented
/// further is code.
fn directive(line: &str) -> Option<&str> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 || line.starts_with('\t') {
        return None;
    }
    let name = line.trim().strip_prefix("::")?;
    let name = name.strip_prefix(':').unwrap_or(name);
    name.chars().all(|c| c.is_ascii_lowercase()).then_some(name)
}

/// The lines of `text` outside code fences, where a directive counts.
fn prose(text: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut fence: Option<&str> = None;
    text.lines().map(move |line| {
        let trimmed = line.trim_start();
        let marker = ["```", "~~~"]
            .into_iter()
            .find(|marker| trimmed.starts_with(marker));
        match (fence, marker) {
            (Some(open), Some(marker)) if open == marker => {
                fence = None;
                (line, false)
            }
            (Some(_), _) => (line, false),
            (None, Some(marker)) => {
                fence = Some(marker);
                (line, false)
            }
            (None, None) => (line, true),
        }
    })
}

/// Whether the agent's reply asks for the person's attention: a `::wizz`
/// line outside code.
pub(super) fn wizzes(text: &str) -> bool {
    // `:::wizz` would open a bubble, not ask.
    prose(text).any(|(line, prose)| prose && line.trim() == "::wizz" && directive(line).is_some())
}

/// `text` without its directive lines, which only mean something drawn.
pub(super) fn without_directives(text: &str) -> String {
    prose(text)
        .filter(|(line, prose)| !(*prose && directive(line).is_some()))
        .map(|(line, _)| line)
        .collect::<Vec<_>>()
        .join("\n")
}
