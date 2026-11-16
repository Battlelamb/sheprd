//! `memory`, run in this process for the repository of the current
//! directory, so Sheprd need not run: the memory file is keyed by the Git
//! common directory, which every worktree of the repository shares, and
//! lives on the host the repository is on.
use super::super::{Context, Output};
use crate::{
    cli::control::MemoryCommand,
    control::{Error as ControlError, ErrorCode},
};
use sheprd_orchestrator::{
    Repository,
    memory::{self, Added, Entry, Memory, Scope},
};
use std::fmt::Write as _;

/// The repository and branch of the current directory, with its memory open.
struct Here {
    memory: Memory,
    branch: Option<String>,
}

fn rejected(code: ErrorCode, message: impl Into<String>) -> crate::Error {
    ControlError::Rejected {
        code,
        message: message.into(),
    }
    .into()
}

fn failed(error: memory::Error) -> crate::Error {
    let code = match error {
        memory::Error::Empty | memory::Error::Kind(_) | memory::Error::TooLong { .. } => {
            ErrorCode::InvalidRequest
        }
        _ => ErrorCode::Failed,
    };
    rejected(code, error.to_string())
}

fn here() -> crate::Result<Here> {
    let dir = std::env::current_dir().map_err(crate::Error::LocalGit)?;
    let (common, branch) = super::here::repository(&dir).map_err(|error| match error {
        crate::Error::NotInCheckout => rejected(
            ErrorCode::InvalidRequest,
            "Memory is kept per Git repository; run this inside a checkout",
        ),
        error => error,
    })?;
    let path = memory::path(&Repository::Local(common))
        .ok_or_else(|| failed(memory::Error::NoStateDirectory))?;
    Ok(Here {
        memory: Memory::open(&path).map_err(failed)?,
        branch,
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// One line per entry: its ID, kind, branch when it has one, and text.
pub(super) fn describe(entries: &[Entry]) -> String {
    let mut text = String::new();
    for entry in entries {
        let _ = write!(text, "#{} [{}]", entry.id, entry.kind);
        if let Some(branch) = &entry.branch {
            let _ = write!(text, " ({branch})");
        }
        // An entry is one paragraph; its own line breaks would read as
        // separate entries.
        let flat: Vec<&str> = entry.text.split_whitespace().collect();
        let _ = writeln!(text, " {}", flat.join(" "));
    }
    text.trim_end().to_owned()
}

fn entries(entries: Vec<Entry>, empty: &str) -> crate::Result<Output> {
    let json = serde_json::json!({ "entries": entries });
    if entries.is_empty() {
        return Ok(Output::data(empty, &json)?.nothing());
    }
    Output::data(describe(&entries), &json)
}

fn scope(all: bool, branch: Option<&str>) -> Scope<'_> {
    if all {
        Scope::All
    } else {
        Scope::Here { branch }
    }
}

pub(in crate::control::client) fn memory(
    command: MemoryCommand,
    _: &Context,
) -> crate::Result<Output> {
    let mut here = here()?;
    let branch = here.branch.as_deref();
    match command {
        MemoryCommand::List { all, limit } => entries(
            here.memory
                .list(scope(all, branch), limit)
                .map_err(failed)?,
            "This repository has no memory yet",
        ),
        MemoryCommand::Search { query, all, limit } => entries(
            here.memory
                .search(scope(all, branch), &query, limit, now())
                .map_err(failed)?,
            "Nothing in memory matches",
        ),
        MemoryCommand::Add {
            branch: on_branch,
            kind,
            text,
        } => {
            let target = match (on_branch, branch) {
                (false, _) => None,
                (true, Some(branch)) => Some(branch),
                (true, None) => {
                    return Err(rejected(
                        ErrorCode::InvalidRequest,
                        "--branch needs a checked-out branch, and HEAD is detached",
                    ));
                }
            };
            let added = here
                .memory
                .add(target, kind, &text, now())
                .map_err(failed)?;
            let place = target.map_or_else(|| "the repository".to_owned(), str::to_owned);
            let message = match added {
                Added::New(id) => format!("Remembered #{id} for {place}"),
                Added::Again(id) => format!("Already remembered as #{id} for {place}"),
            };
            Output::data(
                message,
                &serde_json::json!({ "id": added.id(), "new": matches!(added, Added::New(_)) }),
            )
        }
        MemoryCommand::Forget { id } => {
            if here.memory.forget(id).map_err(failed)? {
                Ok(Output::message(format!("Forgot #{id}")))
            } else {
                Err(rejected(
                    ErrorCode::NotFound,
                    format!("This repository's memory has no #{id}"),
                ))
            }
        }
    }
}
