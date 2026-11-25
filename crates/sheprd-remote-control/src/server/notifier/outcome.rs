//! What a terminal agent's finished run left in its checkout, read with Git
//! off the notifier's loop: the finish waits at most [`WAIT`] for it, then
//! goes out as it was decided.

use crate::{
    native::changes::local::git,
    push::{Notification, RunOutcome},
};
use std::{path::Path, time::Duration};
use tokio::sync::mpsc;

/// How long a finish waits for Git before it goes out without an outcome.
const WAIT: Duration = Duration::from_secs(2);

/// Sends `finished` on `told` once Git said what the run since `since_ms`
/// left in `cwd`, or as it is when Git fails, finds nothing, or is slow.
pub(super) fn tell(
    finished: Notification,
    cwd: String,
    since_ms: u64,
    told: mpsc::UnboundedSender<Notification>,
) {
    tokio::spawn(async move {
        let reading = tokio::task::spawn_blocking(move || read(Path::new(&cwd), since_ms));
        let finished = match tokio::time::timeout(WAIT, reading).await {
            Ok(Ok(Some(outcome))) => outcome.told(finished),
            _ => finished,
        };
        let _ = told.send(finished);
    });
}

/// What `cwd`'s checkout holds of a run that began at `since_ms`: `None`
/// outside a checkout or when Git fails.
pub(super) fn read(cwd: &Path, since_ms: u64) -> Option<RunOutcome> {
    let limit = format!("--max-count={}", crate::push::MAX_OUTCOME_COUNT + 1);
    // Git reads `@<seconds>` as that moment, whatever the time zone.
    let since = format!("--since=@{}", since_ms / 1000);
    let commits = count(&git(cwd, &["rev-list", "--count", &limit, &since, "HEAD"]).ok()?)?;
    let pushed = commits > 0
        && git(cwd, &["rev-list", "--count", "@{upstream}..HEAD"])
            .ok()
            .and_then(|ahead| count(&ahead))
            == Some(0);
    let status = git(
        cwd,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )
    .ok()?;
    Some(RunOutcome {
        commits,
        pushed,
        uncommitted: status.lines().filter(|line| !line.is_empty()).count(),
    })
}

fn count(text: &str) -> Option<usize> {
    text.trim().parse().ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
