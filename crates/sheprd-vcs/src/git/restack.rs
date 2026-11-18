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
    if !carries(checkout, below, deadline, cancelled)? {
        return Ok(Outcome {
            message: format!("Nothing to restack: {branch} no longer carries the merged commits"),
            url: None,
        });
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
    // A rebase that is not interactive asks for no message, but a hook or a
    // setting could still start an editor nobody would see.
    let mut rebase = command(checkout, &["rebase", "--onto", "FETCH_HEAD", below]);
    rebase.env("GIT_EDITOR", "true");
    let (ok, output) = run(&mut rebase, deadline, cancelled)?;
    if !ok {
        let stopped = Error::Stopped {
            details: clean(output.trim()),
        };
        return Err(super::Error::from(stopped).into());
    }
    Ok(Outcome {
        message: format!(
            "Restacked {branch} onto {remote}/{base}; push it with --force-with-lease when ready"
        ),
        url: None,
    })
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
