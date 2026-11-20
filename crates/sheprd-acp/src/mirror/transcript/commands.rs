//! What Claude Code records when the person runs one of its own commands in
//! the terminal: a slash command, its output, or a `!` shell line. Claude
//! writes each as a user message in its own markup, which a chat would show
//! as raw tags under the person's name. Only a message that is wholly one
//! of these envelopes counts; words around a tag stay the person's.

use sheprd_chat_protocol::EntryKind;

use super::notices;

/// What a command record becomes in the chat.
#[derive(Debug, PartialEq)]
pub(super) enum Command {
    /// A notice in place of the record.
    Show(EntryKind),
    /// Nothing: output that says nothing, such as `(no content)`.
    Hide,
}

/// The command `content` records, or `None` for anything else.
pub(super) fn command(content: &str) -> Option<Command> {
    let content = content.trim();
    if !content.ends_with('>') {
        return None;
    }
    if content.starts_with("<command-name>") {
        let name = inner(content, "command-name")?.trim();
        let args = inner(content, "command-args").unwrap_or_default().trim();
        let line = if args.is_empty() {
            name.to_owned()
        } else {
            format!("{name} {args}")
        };
        return Some(Command::Show(notices::source(
            format!("Ran {}", notices::compact(&line)),
            content,
        )));
    }
    if content.starts_with("<local-command-stdout>")
        || content.starts_with("<local-command-stderr>")
    {
        let output = inner(content, "local-command-stdout")
            .or_else(|| inner(content, "local-command-stderr"))?;
        return Some(output_notice("Command output", output));
    }
    if content.starts_with("<bash-input>") {
        let line = inner(content, "bash-input")?;
        return Some(Command::Show(notices::source(
            format!("Ran ! {}", notices::compact(line)),
            line,
        )));
    }
    if content.starts_with("<bash-stdout>") || content.starts_with("<bash-stderr>") {
        let stdout = inner(content, "bash-stdout").unwrap_or_default();
        let stderr = inner(content, "bash-stderr").unwrap_or_default();
        let output = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
            (_, true) => stdout.to_owned(),
            (true, false) => stderr.to_owned(),
            (false, false) => format!("{stdout}\n{stderr}"),
        };
        return Some(output_notice("Shell output", &output));
    }
    None
}

fn output_notice(label: &str, output: &str) -> Command {
    let output = output.trim();
    if output.is_empty() || output == "(no content)" {
        return Command::Hide;
    }
    let first = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    Command::Show(notices::source(
        format!("{label} · {}", notices::compact(first)),
        output,
    ))
}

/// The text between `<tag>` and the `</tag>` after it.
fn inner<'a>(content: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = content.find(&open)? + open.len();
    let end = start + content[start..].find(&close)?;
    Some(&content[start..end])
}
