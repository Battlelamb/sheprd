//! Small local attachment previews, generated off the UI thread by macOS
//! Quick Look. Other platforms keep the file card's generic icon. Results
//! (including failures) share the chat's bounded visual cache and worker slots.

use crate::{
    chat::visual::{self, Drawn},
    window::FileLink,
};
use std::path::Path;

mod error;
pub(crate) use error::Error;

const EXTENSIONS: &[&str] = &[
    // keep-sorted start
    "avi", "m4v", "mkv", "mov", "mp4", "pdf", "webm",
    // keep-sorted end
];

/// Only passive document/media formats enter Quick Look. Check both the
/// printed name and the resolved target, so a renamed symlink cannot select
/// another file's thumbnail provider.
pub(crate) fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
}

pub(super) fn draw(file: &FileLink) -> visual::Result<Drawn> {
    #[cfg(target_os = "macos")]
    {
        let path = file
            .thumbnail_path()
            .filter(|path| supported(path))
            .ok_or(Error::Unavailable)?;
        let bytes = generate(&path)?;
        visual::decode_bytes(bytes, image::ImageFormat::Png)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = file;
        Err(Error::Unavailable.into())
    }
}

#[cfg(target_os = "macos")]
fn generate(path: &Path) -> Result<Vec<u8>, Error> {
    use std::{process::Stdio, time::Duration};
    let directory = tempfile::Builder::new()
        .prefix("sheprd-thumbnail-")
        .tempdir()?;
    // Absolute executable and separate arguments: attachment names never
    // become shell code. -o suppresses Quick Look's UI entirely.
    let mut child = sheprd_client::new_command("/usr/bin/qlmanage")
        .args(["-t", "-s", "256", "-o"])
        .arg(directory.path())
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    wait(&mut child, Duration::from_secs(8))?;
    let mut name = path.file_name().ok_or(Error::Unavailable)?.to_os_string();
    name.push(".png");
    read(&directory.path().join(name))
}

#[cfg(target_os = "macos")]
fn wait(child: &mut std::process::Child, timeout: std::time::Duration) -> Result<(), Error> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + timeout;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(Error::Unavailable)
                };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Ok(None) => break Err(Error::Limit),
            Err(error) => break Err(Error::Io(error)),
        }
    };
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(target_os = "macos")]
fn read(path: &Path) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    const MAX_BYTES: u64 = 2 * 1024 * 1024;
    let output = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    output.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Error::Limit);
    }
    let (width, height) =
        image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Png)
            .into_dimensions()?;
    if width == 0 || height == 0 || width > 256 || height > 256 {
        return Err(Error::Limit);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
