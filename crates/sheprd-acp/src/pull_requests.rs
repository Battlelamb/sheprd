//! The pull requests a finished tool call worked on, read from the command
//! it ran and what that printed: `gh pr` commands that change a pull
//! request, `gh api` writes to one, and `git push`. Commands that only read,
//! such as `gh pr view` or `gh pr list`, name none.
//!
//! The command is split the way a shell would only as far as quotes and
//! command separators go; anything subtler is left unread, as a missed pull
//! request costs less than a wrong one. Only GitHub addresses are kept,
//! through [`PullRequest::new`].

use sheprd_chat_protocol::{
    PullRequest, PullRequestAction as Action, Tool, ToolContent, ToolKind, ToolStatus,
};

/// Pull requests one call may name; the rest are left out.
pub(crate) const MAX_FOUND: usize = 8;
/// The bytes of a command read, and of each text a call printed.
const MAX_COMMAND: usize = 16 * 1024;
const MAX_OUTPUT: usize = 64 * 1024;
const HOST: &str = "https://github.com/";

/// What one command of a call did, and the pull requests its arguments name.
#[derive(Debug, Default)]
struct Work {
    actions: Vec<Action>,
    named: Vec<PullRequest>,
    /// A `gh pr` command, whose messages name what it changed.
    gh_pr: bool,
}

/// The pull requests `tool` worked on, each once with what was done to it.
pub(crate) fn found(tool: &Tool) -> Vec<PullRequest> {
    if tool.status != ToolStatus::Completed
        || !matches!(tool.kind, ToolKind::Execute | ToolKind::Other)
    {
        return Vec::new();
    }
    let mut work = Work::default();
    for command in commands(&words(command_text(&tool.title))) {
        let Some(found) = gh_pr(&command)
            .or_else(|| gh_api(&command))
            .or_else(|| git_push(&command))
        else {
            continue;
        };
        for action in found.actions {
            if !work.actions.contains(&action) {
                work.actions.push(action);
            }
        }
        work.named.extend(found.named);
        work.gh_pr |= found.gh_pr;
    }
    if work.actions.is_empty() {
        return Vec::new();
    }
    let mut pulls: Vec<PullRequest> = Vec::new();
    let printed = tool.content.iter().filter_map(|content| match content {
        ToolContent::Text { text } => Some(head(text, MAX_OUTPUT)),
        _ => None,
    });
    let mut seen = work.named;
    for text in printed {
        seen.extend(addresses(text));
        if work.gh_pr {
            seen.extend(messages(text));
        }
    }
    for mut pull in seen {
        if pulls.len() == MAX_FOUND {
            break;
        }
        if pulls.iter().any(|known| known.is(&pull)) {
            continue;
        }
        pull.actions.clone_from(&work.actions);
        pulls.push(pull);
    }
    pulls
}

/// The command a call's title shows: agents such as Claude Code put it in
/// backticks, escaping those inside.
fn command_text(title: &str) -> String {
    let title = head(title.trim(), MAX_COMMAND);
    let inner = title
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'))
        .unwrap_or(title);
    inner.replace("\\`", "`")
}

/// At most `limit` bytes of `text`, cut at a character.
fn head(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// A word of a command, or a separator between two commands.
#[derive(Debug, PartialEq, Eq)]
enum Word {
    Text(String),
    Break,
}

/// `command` in words, with quotes taken off and separators (`;`, `&`, `|`,
/// a newline, or a parenthesis) between commands.
fn words(command: String) -> Vec<Word> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut chars = command.chars();
    let end_word = |words: &mut Vec<Word>, word: &mut String, started: &mut bool| {
        if *started {
            words.push(Word::Text(std::mem::take(word)));
            *started = false;
        }
    };
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                started = true;
                word.extend(chars.by_ref().take_while(|&ch| ch != '\''));
            }
            '"' => {
                started = true;
                while let Some(ch) = chars.next() {
                    match ch {
                        '"' => break,
                        '\\' => word.extend(chars.next()),
                        ch => word.push(ch),
                    }
                }
            }
            '\\' => {
                started = true;
                word.extend(chars.next().filter(|&ch| ch != '\n'));
            }
            ';' | '&' | '|' | '\n' | '(' | ')' => {
                end_word(&mut words, &mut word, &mut started);
                if words.last().is_some_and(|last| *last != Word::Break) {
                    words.push(Word::Break);
                }
            }
            ch if ch.is_whitespace() => end_word(&mut words, &mut word, &mut started),
            ch => {
                started = true;
                word.push(ch);
            }
        }
    }
    end_word(&mut words, &mut word, &mut started);
    words
}

/// Each command's words, between separators.
fn commands(words: &[Word]) -> impl Iterator<Item = Vec<&str>> {
    words.split(|word| *word == Word::Break).map(|command| {
        command
            .iter()
            .filter_map(|word| match word {
                Word::Text(text) => Some(text.as_str()),
                Word::Break => None,
            })
            .collect()
    })
}

/// Whether `word` runs `program`, by any path.
fn runs(word: &str, program: &str) -> bool {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    name.strip_suffix(".exe").unwrap_or(name) == program
}

/// The words after `gh SUBCOMMAND`, wherever it starts in `command`.
fn gh<'a>(command: &'a [&'a str], subcommand: &str) -> Option<&'a [&'a str]> {
    let at = command
        .windows(2)
        .position(|pair| runs(pair[0], "gh") && pair[1] == subcommand)?;
    Some(&command[at + 2..])
}

/// A `gh pr` command that changes a pull request.
fn gh_pr(command: &[&str]) -> Option<Work> {
    let (verb, args) = gh(command, "pr")?.split_first()?;
    let flag = |name: &str| args.contains(&name);
    let action = match *verb {
        "create" | "new" => Action::Created,
        "edit" => Action::Edited,
        "merge" if flag("--auto") || flag("--disable-auto") => Action::Edited,
        "merge" => Action::Merged,
        "comment" => Action::Commented,
        "review" => Action::Reviewed,
        "ready" if flag("--undo") => Action::Edited,
        "ready" => Action::Ready,
        "close" => Action::Closed,
        "reopen" => Action::Reopened,
        _ => return None,
    };
    let mut repo = None;
    let mut named = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "-R" | "--repo" => repo = args.next().copied(),
            arg if arg.starts_with("--repo=") => repo = arg.strip_prefix("--repo="),
            arg if named.is_none() && !arg.starts_with('-') => named = Some(arg),
            _ => {}
        }
    }
    // The pull request a command names: its address, or its number with
    // `--repo`. Without either, its own messages name it.
    let named = named.and_then(|arg| {
        PullRequest::parse(arg).or_else(|| {
            let number = arg.strip_prefix('#').unwrap_or(arg).parse().ok()?;
            PullRequest::new(repo?, number, Vec::new())
        })
    });
    Some(Work {
        actions: vec![action],
        named: named.into_iter().collect(),
        gh_pr: true,
    })
}

/// `gh api` options that take a value as the next word.
const API_VALUES: &[&str] = &[
    "--cache",
    "--field",
    "--header",
    "--hostname",
    "--input",
    "--jq",
    "--method",
    "--preview",
    "--raw-field",
    "--template",
    "-F",
    "-H",
    "-X",
    "-f",
    "-p",
    "-q",
    "-t",
];

/// A `gh api` request that writes to a pull request:
/// `repos/OWNER/REPO/pulls/NUMBER[/PART]` with a method other than `GET`,
/// or with fields, which `gh` sends as a `POST`.
fn gh_api(command: &[&str]) -> Option<Work> {
    let args = gh(command, "api")?;
    let mut path = None;
    let mut method = None;
    let mut fields = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (*arg, None),
        };
        if !API_VALUES.contains(&name) {
            if !arg.starts_with('-') && path.is_none() {
                path = Some(*arg);
            }
            continue;
        }
        let value = inline.or_else(|| args.next().copied());
        match name {
            "-X" | "--method" => method = value,
            "-f" | "-F" | "--field" | "--raw-field" => fields.extend(value),
            "--input" => fields.push(""),
            _ => {}
        }
    }
    let writes = method.map_or(!fields.is_empty(), |method| {
        !method.eq_ignore_ascii_case("GET")
    });
    if !writes {
        return None;
    }
    let mut parts = path?.trim_start_matches('/').split('/');
    let (repos, owner, repo, pulls, number) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    if repos != "repos" || pulls != "pulls" {
        return None;
    }
    let pull = PullRequest::new(&format!("{owner}/{repo}"), number.parse().ok()?, Vec::new())?;
    let state = |value: &str| fields.contains(&format!("state={value}").as_str());
    let action = match parts.next() {
        Some("merge") => Action::Merged,
        Some("comments") => Action::Commented,
        Some("reviews") => Action::Reviewed,
        Some(_) => Action::Edited,
        None if state("closed") => Action::Closed,
        None if state("open") => Action::Reopened,
        None => Action::Edited,
    };
    Some(Work {
        actions: vec![action],
        named: vec![pull],
        gh_pr: false,
    })
}

/// `git push`, past `git`'s own options; what it printed may name the pull
/// request the branch belongs to.
fn git_push(command: &[&str]) -> Option<Work> {
    let at = command.iter().position(|word| runs(word, "git"))?;
    let mut args = command[at + 1..].iter();
    while let Some(arg) = args.next() {
        match *arg {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                args.next();
            }
            arg if arg.starts_with('-') => {}
            "push" => {
                return Some(Work {
                    actions: vec![Action::Pushed],
                    ..Work::default()
                });
            }
            _ => return None,
        }
    }
    None
}

/// Every pull request address in `text`.
fn addresses(text: &str) -> impl Iterator<Item = PullRequest> + '_ {
    text.match_indices(HOST)
        .filter_map(|(at, _)| PullRequest::scan(&text[at..]).map(|(pull, _)| pull))
}

/// The pull requests `gh pr` names in its messages, such as
/// `✓ Merged pull request OWNER/REPO#7 (Title)`.
fn messages(text: &str) -> impl Iterator<Item = PullRequest> + '_ {
    const PHRASE: &str = "pull request ";
    // ASCII lowering keeps every byte where it was.
    let lowered = text.to_ascii_lowercase();
    let starts: Vec<usize> = lowered
        .match_indices(PHRASE)
        .map(|(at, _)| at + PHRASE.len())
        .collect();
    starts.into_iter().filter_map(move |start| {
        let rest = &text[start..];
        let reference = rest.split(char::is_whitespace).next()?;
        let reference = reference.trim_end_matches(|ch: char| !ch.is_ascii_digit());
        let (repo, number) = reference.split_once('#')?;
        PullRequest::new(repo, number.parse().ok()?, Vec::new())
    })
}

#[cfg(test)]
mod tests;
