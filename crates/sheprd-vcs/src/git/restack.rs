//! Restacking: once the layer below a branch merged into the stack's base
//! (squashed or rebased into new commits), the branch still carries that
//! layer's old commits. Restacking replays only the branch's own commits, the
//! ones after the merged layer's head, onto the base as its remote has it now.
//!
//! Local only: it never pushes. A dirty checkout is refused rather than
//! stashed, and a conflict leaves the rebase stopped for the user or an agent
//! to resolve, never aborted or retried here.

mod error;

pub use error::Error;

use super::{Outcome, command, git};
use sheprd_github::pull_request::{base_repository, clean, run};
use std::time::Instant;

/// What a restack did, for the chrome to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Restacked {
    /// `branch` now sits on `remote`/`base` with only its own commits.
    Onto {
        branch: String,
        remote: String,
        base: String,
    },
    /// `branch` no longer carried the merged commits; nothing changed.
    Unchanged { branch: String },
}

impl Restacked {
    /// The English the worker reports, for logs and tests.
    fn message(&self) -> String {
        match self {
            Self::Onto {
                branch,
                remote,
                base,
            } => format!(
                "Restacked {branch} onto {remote}/{base}; push it with --force-with-lease when ready"
            ),
            Self::Unchanged { branch } => {
                format!("Nothing to restack: {branch} no longer carries the merged commits")
            }
        }
    }
}

impl From<Restacked> for Outcome {
    fn from(restacked: Restacked) -> Self {
        Outcome {
            message: restacked.message(),
            url: None,
            restacked: Some(restacked),
        }
    }
}

pub(super) fn restack(
    checkout: &str,
    branch: &str,
    below: &str,
    base: &str,
    deadline: Instant,
    cancelled: &impl Fn() -> bool,
) -> crate::Result<Outcome> {
    if !commit_id(below) || !branch_name(base) {
        return Err(super::Error::from(Error::Target).into());
    }
    if super::checkout_dirty(checkout, deadline, cancelled)? {
        return Err(super::Error::from(Error::Dirty).into());
    }
    // The branch and commit confirmed; a terminal or agent may move them
    // while the fetch runs, so they are read again before rewriting.
    let head = checked_out(checkout, branch, deadline, cancelled)?;
    if !carries(checkout, below, deadline, cancelled)? {
        return Ok(Restacked::Unchanged {
            branch: branch.to_owned(),
        }
        .into());
    }
    let remote = base_repository(checkout, deadline, cancelled)?
        .remote
        .name();
    let refspec = format!("refs/heads/{base}");
    git(
        checkout,
        &["fetch", "--no-tags", remote, &refspec],
        "fetch the stack's base",
        deadline,
        cancelled,
    )?;
    if checked_out(checkout, branch, deadline, cancelled)? != head {
        return Err(super::Error::from(Error::Moved).into());
    }
    // A rebase that is not interactive asks for no message, but a hook or a
    // setting could still start an editor nobody would see. `updateRefs`
    // would also move other local branches on the replayed commits; this
    // changes the checkout's own branch only. (A setting, not
    // `--no-update-refs`, so a Git before 2.38 ignores it.)
    let mut rebase = command(
        checkout,
        &[
            "-c",
            "rebase.updateRefs=false",
            "rebase",
            "--onto",
            "FETCH_HEAD",
            below,
        ],
    );
    rebase.env("GIT_EDITOR", "true");
    let (ok, output) = run(&mut rebase, deadline, cancelled)?;
    if !ok {
        let stopped = Error::Stopped {
            details: clean(output.trim()),
        };
        return Err(super::Error::from(stopped).into());
    }
    Ok(Restacked::Onto {
        branch: branch.to_owned(),
        remote: remote.to_owned(),
        base: base.to_owned(),
    }
    .into())
}

/// HEAD's commit, once `branch` is checked out; else the checkout moved.
fn checked_out(
    checkout: &str,
    branch: &str,
    deadline: Instant,
    cancelled: &impl Fn() -> bool,
) -> crate::Result<String> {
    let (on_branch, current) = run(
        &mut command(checkout, &["symbolic-ref", "--quiet", "--short", "HEAD"]),
        deadline,
        cancelled,
    )?;
    if !on_branch || current.trim() != branch {
        return Err(super::Error::from(Error::Moved).into());
    }
    git(
        checkout,
        &["rev-parse", "HEAD"],
        "read HEAD",
        deadline,
        cancelled,
    )
}

/// Whether HEAD still has `below` in its history. Git exits 1 with nothing
/// to say when it does not, and otherwise fails with a reason.
fn carries(
    checkout: &str,
    below: &str,
    deadline: Instant,
    cancelled: &impl Fn() -> bool,
) -> crate::Result<bool> {
    let (ok, output) = run(
        &mut command(checkout, &["merge-base", "--is-ancestor", below, "HEAD"]),
        deadline,
        cancelled,
    )?;
    if ok {
        return Ok(true);
    }
    if output.trim().is_empty() {
        return Ok(false);
    }
    Err(super::Error::Failed {
        operation: "read the branch's history",
        details: clean(output.trim()),
    }
    .into())
}

/// A full commit ID in lowercase hex, as GitHub reports one.
pub(super) fn commit_id(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64)
        && oid
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// A branch name safe to name in a refspec: no option, range, revision
/// syntax, or control character.
pub(super) fn branch_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with(['-', '/'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
        && !name.contains("@{")
        && !name.ends_with(".lock")
        && !name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
}

#[cfg(test)]
mod tests;
