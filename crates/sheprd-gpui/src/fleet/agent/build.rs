//! Which `sheprd-relay-agent` a plain host gets (`docs/relay-plan.md`,
//! Fleet, Spreading a host, step 3), from its `uname -sm`:
//!
//! - a release copies only the release's own build for the host's
//!   platform: static musl on Linux, Developer ID signed on each Mac
//!   architecture. It downloads the release's agent archive once per
//!   version into this app's state and keeps the build only after checking
//!   it against the Ed25519-signed manifest packed first in the archive
//!   (`sheprd_updater::download_host_agent`), by the key this build
//!   embeds. Nothing else reaches the host: not a program on `PATH`, not a
//!   file beside the app. The host then checks the copy's SHA-256 before
//!   it puts it in place (`host_build::copy_script`), and only then runs it.
//! - a development build has no signed release to read: it copies its own
//!   agent to a host of its kind, else `relay-agent/TRIPLE/` beside the
//!   executable.
//!
//! A host on any other platform (Windows, the BSDs, 32-bit Linux) is
//! refused before anything is copied: the agent's service and installer
//! are POSIX shell with systemd or launchd.

mod error;

pub use error::Error;

use super::PROGRAM;
use crate::host_build::{self, Platform};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

/// Where builds for other platforms ship, beside a development build's
/// executable.
pub(super) const BUILDS: &str = "relay-agent";

/// One download at a time: sharing two hosts of the same kind starts two
/// jobs for the same build, and the second then finds it in place.
static DOWNLOADS: Mutex<()> = Mutex::new(());

/// The platform `uname -sm` names, when the release ships an agent for it.
pub(crate) fn platform(uname: &str) -> Result<Platform, Error> {
    let platform = Platform::parse(uname).map_err(|error| match error {
        host_build::Error::Platform(platform) => Error::Unsupported { platform },
        other => Error::from(other),
    })?;
    target(&platform).ok_or_else(|| Error::Unsupported {
        platform: platform.to_string(),
    })?;
    Ok(platform)
}

/// The release target of the agent built for `platform`.
pub(crate) fn target(platform: &Platform) -> Option<&'static str> {
    // `triple` names every system but macOS as Linux.
    if !matches!(platform.os, "linux" | "macos") {
        return None;
    }
    let triple = platform.triple();
    sheprd_updater::AGENT_TARGETS
        .into_iter()
        .find(|target| *target == triple)
}

/// Where the build for a host comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The release's build for this target, checked against its signed
    /// manifest.
    Release(&'static str),
    /// A development build's own agent, or one shipped beside it.
    Local(PathBuf),
}

/// Which build fits a host on `platform`: a release's own, when this is a
/// release (`release`), else this development build's (`bundled` for a
/// host of its kind, else `relay-agent/TRIPLE/` in `beside`).
pub(crate) fn choose(
    platform: &Platform,
    release: bool,
    bundled: Option<PathBuf>,
    beside: &Path,
) -> Result<Source, Error> {
    let unsupported = || Error::Unsupported {
        platform: platform.to_string(),
    };
    let target = target(platform).ok_or_else(unsupported)?;
    if release {
        return Ok(Source::Release(target));
    }
    build_for(platform, beside, bundled)
        .map(Source::Local)
        .ok_or_else(|| Error::NoBuild {
            platform: platform.to_string(),
        })
}

/// A development build's agent for `platform`: `bundled`, its own, when the
/// host is this kind of computer, else `relay-agent/TRIPLE/` in `beside`.
pub(crate) fn build_for(
    platform: &Platform,
    beside: &Path,
    bundled: Option<PathBuf>,
) -> Option<PathBuf> {
    host_build::build_for(platform, beside, BUILDS, PROGRAM, bundled)
}

/// The build of the agent for a host on `platform`, on this computer.
/// Blocks: a release downloads it the first time.
pub(super) fn locate(platform: &Platform, cancelled: &AtomicBool) -> Result<PathBuf, Error> {
    let bundled = (!crate::RELEASE_BUILD)
        .then(|| crate::bundled::locate(PROGRAM))
        .flatten();
    match choose(
        platform,
        crate::RELEASE_BUILD,
        bundled,
        &host_build::beside_app(),
    )? {
        Source::Release(target) => download(target, cancelled),
        Source::Local(path) => Ok(path),
    }
}

/// Where the release's build for `target` is kept once checked.
pub(crate) fn cached(dir: &Path, target: &str) -> PathBuf {
    dir.join(format!("{PROGRAM}-{target}"))
}

/// The release's build for `target`, downloaded and checked once per
/// version into this app's private state. Blocks.
fn download(target: &'static str, cancelled: &AtomicBool) -> Result<PathBuf, Error> {
    let dir = crate::remote::state_dir()
        .map_err(|_| Error::NoStateDir)?
        .join("host-builds")
        .join(crate::APP_VERSION);
    let path = cached(&dir, target);
    let _one = DOWNLOADS.lock().unwrap_or_else(PoisonError::into_inner);
    if path.is_file() {
        return Ok(path);
    }
    let failed = |source| Error::Cache {
        path: dir.clone(),
        source,
    };
    std::fs::create_dir_all(&dir).map_err(failed)?;
    let part = dir.join(format!(".{}.part", unique(target)));
    // Only a build that matched the signed manifest is renamed into place.
    if let Err(source) = sheprd_updater::download_host_agent(target, &part, cancelled) {
        let _ = std::fs::remove_file(&part);
        return Err(Error::Download {
            target: target.to_owned(),
            source,
        });
    }
    std::fs::rename(&part, &path).map_err(failed)?;
    Ok(path)
}

/// A name no other job in this process uses, for its temporary file.
fn unique(stem: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "{PROGRAM}-{stem}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
