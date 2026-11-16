//! Agent memory: what agents learned about a repository, kept for the next
//! agent in any of its worktrees, tabs, or chats on the same host.
//!
//! It lives in `repositories/<uuid v5>/memory.sqlite3`, beside the
//! repository's database (`location`) and keyed the same way, by the Git
//! common directory every worktree shares. It is a file of its own rather
//! than a table of `state.sqlite3`, whose schema agent-launcher shares: a
//! new table there would raise that schema's version and leave older
//! agent-launcher builds read-only.
//!
//! An entry is about the whole repository, or only one branch's work. Search
//! is SQLite's FTS5, ranked by relevance; reading an entry through a search
//! marks it used, and the least recently used entries go first once a
//! repository holds [`MAX_ENTRIES`]. Every call blocks on disk I/O, so the
//! app runs it off the UI thread; the `sheprd memory` command runs it in its
//! own process.

mod error;
#[cfg(test)]
mod tests;

pub use error::{Error, Result};

use crate::location::Repository;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::Serialize;
use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

/// The newest schema this build reads and writes.
pub const SCHEMA_VERSION: i64 = 1;
/// The longest entry kept, in bytes: a fact or two, not a document.
pub const MAX_TEXT: usize = 2000;
/// The most entries one repository keeps; the least recently used go first.
pub const MAX_ENTRIES: u32 = 2000;
/// The most entries one listing or search returns.
pub const MAX_LIMIT: usize = 200;

const FILE: &str = "memory.sqlite3";
/// How long a write waits for another process's transaction.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY NOT NULL,
    branch TEXT,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    used_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS memories_scope_text ON memories(ifnull(branch, ''), text);
CREATE INDEX IF NOT EXISTS memories_used ON memories(used_at);
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts
    USING fts5(text, content = 'memories', content_rowid = 'id');
CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE OF text ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text);
END;
";

const COLUMNS: &str = "id, branch, kind, text, created_at, used_at";

/// The memory file of `repository`, or `None` without a state directory.
pub fn path(repository: &Repository) -> Option<PathBuf> {
    Some(path_in(
        &sheprd_client::local_path::state_dir()?,
        repository,
    ))
}

/// The memory file of `repository` under the state directory `state`.
pub fn path_in(state: &Path, repository: &Repository) -> PathBuf {
    crate::location::database_path_in(state, repository).with_file_name(FILE)
}

/// What an entry is, so a reader can tell a rule from a passing remark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// How to build, test, or run something here.
    Command,
    /// A choice that was made, and why.
    Decision,
    /// Something true about the code or its setup.
    #[default]
    Fact,
    /// A trap: what failed, and what works instead.
    Gotcha,
    /// Where work stands, for whoever picks it up next.
    Handoff,
    /// How the person wants things done.
    Preference,
}

/// Every kind, in the order help text names them.
pub const KINDS: &[Kind] = &[
    Kind::Fact,
    Kind::Gotcha,
    Kind::Command,
    Kind::Decision,
    Kind::Preference,
    Kind::Handoff,
];

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Decision => "decision",
            Self::Fact => "fact",
            Self::Gotcha => "gotcha",
            Self::Handoff => "handoff",
            Self::Preference => "preference",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Kind {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        KINDS
            .iter()
            .copied()
            .find(|kind| kind.as_str() == text)
            .ok_or_else(|| Error::Kind(text.to_owned()))
    }
}

/// Which entries a call sees: the repository's, and one branch's when named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope<'a> {
    /// Every entry, of the repository and of each branch.
    All,
    /// The repository's entries, and those of `branch` when there is one.
    Here { branch: Option<&'a str> },
}

/// One remembered entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub id: i64,
    /// The branch it is about; `None` for the whole repository.
    pub branch: Option<String>,
    pub kind: Kind,
    pub text: String,
    /// Unix seconds.
    pub created: i64,
    /// Unix seconds it was last added again or found by a search.
    pub used: i64,
}

/// What [`Memory::add`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    New(i64),
    /// The same text was already kept in that scope; it is marked used.
    Again(i64),
}

impl Added {
    pub fn id(self) -> i64 {
        match self {
            Self::New(id) | Self::Again(id) => id,
        }
    }
}

/// One repository's memory, open.
pub struct Memory {
    connection: Connection,
}

impl Memory {
    /// Opens `path`, creating the private directory and file when missing,
    /// and migrates it to [`SCHEMA_VERSION`].
    pub fn open(path: &Path) -> Result<Self> {
        crate::private_file::prepare(path)?;
        let connection = Connection::open(path)?;
        Self::with(connection)
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self> {
        Self::with(Connection::open_in_memory()?)
    }

    fn with(mut connection: Connection) -> Result<Self> {
        connection.busy_timeout(BUSY_TIMEOUT)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found: i64 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if found > SCHEMA_VERSION {
            return Err(Error::NewerSchema {
                found,
                known: SCHEMA_VERSION,
            });
        }
        if found < 1 {
            transaction.execute_batch(SCHEMA_V1)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        transaction.commit()?;
        Ok(Self { connection })
    }

    /// Keeps `text` for the repository, or for `branch` only, at `now` (Unix
    /// seconds). The same text already kept there is marked used instead.
    pub fn add(&mut self, branch: Option<&str>, kind: Kind, text: &str, now: i64) -> Result<Added> {
        let text = clean(text)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT id FROM memories WHERE ifnull(branch, '') = ifnull(?1, '') AND text = ?2",
                params![branch, text],
                |row| row.get(0),
            )
            .optional()?;
        let added = match existing {
            Some(id) => {
                transaction.execute(
                    "UPDATE memories SET used_at = ?2, kind = ?3 WHERE id = ?1",
                    params![id, now, kind.as_str()],
                )?;
                Added::Again(id)
            }
            None => {
                transaction.execute(
                    "INSERT INTO memories (branch, kind, text, created_at, used_at) \
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![branch, kind.as_str(), text, now],
                )?;
                Added::New(transaction.last_insert_rowid())
            }
        };
        transaction.execute(
            "DELETE FROM memories WHERE id IN (SELECT id FROM memories \
             ORDER BY used_at DESC, id DESC LIMIT -1 OFFSET ?1)",
            [MAX_ENTRIES],
        )?;
        transaction.commit()?;
        Ok(added)
    }

    /// The entries `scope` sees, most recently used first, at most `limit`.
    pub fn list(&self, scope: Scope<'_>, limit: usize) -> Result<Vec<Entry>> {
        let (all, branch) = scope_params(scope);
        let mut statement = self.connection.prepare(&format!(
            "SELECT {COLUMNS} FROM memories \
             WHERE ?1 OR branch IS NULL OR branch = ?2 \
             ORDER BY used_at DESC, id DESC LIMIT ?3"
        ))?;
        let rows = statement.query_map(params![all, branch, bounded(limit)], entry)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The entries `scope` sees that match any word of `query`, best first,
    /// at most `limit`; each found is marked used at `now`. Words match as
    /// prefixes, and FTS5's own query syntax is not taken from the caller.
    pub fn search(
        &mut self,
        scope: Scope<'_>,
        query: &str,
        limit: usize,
        now: i64,
    ) -> Result<Vec<Entry>> {
        let Some(query) = match_query(query) else {
            return Ok(Vec::new());
        };
        let (all, branch) = scope_params(scope);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found = {
            let mut statement = transaction.prepare(
                "SELECT m.id, m.branch, m.kind, m.text, m.created_at, m.used_at \
                 FROM memories_fts JOIN memories m ON m.id = memories_fts.rowid \
                 WHERE memories_fts MATCH ?1 AND (?2 OR m.branch IS NULL OR m.branch = ?3) \
                 ORDER BY bm25(memories_fts), m.used_at DESC LIMIT ?4",
            )?;
            let rows = statement.query_map(params![query, all, branch, bounded(limit)], entry)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for found in &found {
            transaction.execute(
                "UPDATE memories SET used_at = ?2 WHERE id = ?1",
                params![found.id, now],
            )?;
        }
        transaction.commit()?;
        Ok(found)
    }

    /// Forgets entry `id`; false when there was none.
    pub fn forget(&mut self, id: i64) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM memories WHERE id = ?1", [id])?
            > 0)
    }
}

/// `text` as kept: trimmed, not empty, at most [`MAX_TEXT`] bytes.
fn clean(text: &str) -> Result<&str> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::Empty);
    }
    if text.len() > MAX_TEXT {
        return Err(Error::TooLong { max: MAX_TEXT });
    }
    Ok(text)
}

fn bounded(limit: usize) -> i64 {
    i64::try_from(limit.clamp(1, MAX_LIMIT)).unwrap_or(1)
}

fn scope_params(scope: Scope<'_>) -> (bool, Option<&str>) {
    match scope {
        Scope::All => (true, None),
        Scope::Here { branch } => (false, branch),
    }
}

/// An FTS5 query matching any word of `query` as a prefix: each word is
/// quoted, so operators and punctuation in it are only text.
fn match_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .take(32)
        .map(|word| format!("\"{word}\"*"))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

fn entry(row: &Row<'_>) -> rusqlite::Result<Entry> {
    let kind: String = row.get(2)?;
    Ok(Entry {
        id: row.get(0)?,
        branch: row.get(1)?,
        // A kind a newer build wrote reads as a fact rather than failing.
        kind: kind.parse().unwrap_or_default(),
        text: row.get(3)?,
        created: row.get(4)?,
        used: row.get(5)?,
    })
}
