//! `memory`: what agents learned about a repository, for the next agent.
use super::{Args, CliError, unknown};
use sheprd_orchestrator::memory::{Kind, MAX_LIMIT, MAX_TEXT};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryCommand {
    /// The repository's entries and the current branch's, or every entry.
    List {
        all: bool,
        limit: usize,
    },
    /// Entries matching any word of `query`, best first.
    Search {
        query: String,
        all: bool,
        limit: usize,
    },
    /// Keep `text` for the repository, or for the current branch only.
    Add {
        branch: bool,
        kind: Kind,
        text: String,
    },
    Forget {
        id: i64,
    },
}

const DEFAULT_LIMIT: usize = 20;

fn limit(args: &mut Args) -> Result<usize, CliError> {
    Ok(args
        .number("--limit", 1..=MAX_LIMIT)?
        .unwrap_or(DEFAULT_LIMIT))
}

pub(super) fn memory(args: &mut Args) -> Result<MemoryCommand, CliError> {
    let Some(verb) = args.verb() else {
        return Ok(MemoryCommand::List {
            all: args.flag("--all"),
            limit: limit(args)?,
        });
    };
    Ok(match verb.to_str() {
        Some("list") => MemoryCommand::List {
            all: args.flag("--all"),
            limit: limit(args)?,
        },
        Some("search") => {
            let all = args.flag("--all");
            let limit = limit(args)?;
            MemoryCommand::Search {
                query: args.words()?.ok_or(CliError::MissingArgument {
                    command: "memory search",
                    what: "WORDS",
                })?,
                all,
                limit,
            }
        }
        Some("add") => {
            let branch = args.flag("--branch");
            let kind = match args.text_option("--kind")? {
                Some(kind) => kind.parse().map_err(|_| CliError::InvalidValue("--kind"))?,
                None => Kind::default(),
            };
            let text = args.words()?.ok_or(CliError::MissingArgument {
                command: "memory add",
                what: "TEXT",
            })?;
            if text.trim().len() > MAX_TEXT {
                return Err(CliError::InvalidValue("TEXT over 2000 bytes"));
            }
            MemoryCommand::Add { branch, kind, text }
        }
        Some("forget") => MemoryCommand::Forget {
            id: args
                .required_text("ID")?
                .parse()
                .map_err(|_| CliError::InvalidValue("ID"))?,
        },
        _ => return Err(unknown("memory", verb)),
    })
}
