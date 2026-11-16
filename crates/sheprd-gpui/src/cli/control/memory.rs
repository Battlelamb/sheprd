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

pub(super) fn memory(args: &mut Args) -> Result<MemoryCommand, CliError> {
    // Read before the verb, so `memory --all` lists as `memory list --all`
    // does rather than taking the flag for a verb.
    let all = args.flag("--all");
    let given = args.number("--limit", 1..=MAX_LIMIT)?;
    let limit = given.unwrap_or(DEFAULT_LIMIT);
    // `add` and `forget` refuse them rather than ignore them.
    let listing = || {
        if all || given.is_some() {
            return Err(CliError::InvalidValue(
                "--all and --limit go with memory list and memory search",
            ));
        }
        Ok(())
    };
    let Some(verb) = args.verb() else {
        return Ok(MemoryCommand::List { all, limit });
    };
    Ok(match verb.to_str() {
        Some("list") => MemoryCommand::List { all, limit },
        Some("search") => MemoryCommand::Search {
            query: args.words()?.ok_or(CliError::MissingArgument {
                command: "memory search",
                what: "WORDS",
            })?,
            all,
            limit,
        },
        Some("add") => {
            listing()?;
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
        Some("forget") => {
            listing()?;
            MemoryCommand::Forget {
                id: args
                    .required_text("ID")?
                    .parse()
                    .map_err(|_| CliError::InvalidValue("ID"))?,
            }
        }
        _ => return Err(unknown("memory", verb)),
    })
}
