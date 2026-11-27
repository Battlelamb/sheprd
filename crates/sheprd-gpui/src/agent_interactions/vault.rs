//! Secret-safe vault injection: writes a user-provided secret directly to a
//! workspace `.env` file (mode `0600`, no symlinks, strictly inside the
//! workspace root) or to Sheprd's credentials store without ever returning or
//! logging the secret bytes to an agent or chat transcript.

use super::error::Error;
use std::{
    fmt, fs,
    io::Write as _,
    path::{Component, Path, PathBuf},
};

/// Secret bytes that never appear in `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SecretValue(String);

impl SecretValue {
    pub(crate) fn new(value: String) -> Result<Self, Error> {
        if value.trim().is_empty() {
            return Err(Error::EmptySecret);
        }
        Ok(Self(value))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn byte_len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(\"[REDACTED]\")")
    }
}

/// Validated destination for a secret injection request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    EnvFile(PathBuf),
    Keychain(String),
}

/// Outcome returned to the chat / agent: contains only metadata, never bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InjectionReceipt {
    pub(crate) key: String,
    pub(crate) summary: String,
}

/// Validates an environment variable key (`[A-Za-z_][A-Za-z0-9_]*`, max 128).
pub(crate) fn validate_key(key: &str) -> Result<(), Error> {
    if key.is_empty() || key.len() > 128 {
        return Err(Error::InvalidKey);
    }
    let mut bytes = key.bytes();
    let Some(first) = bytes.next() else {
        return Err(Error::InvalidKey);
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return Err(Error::InvalidKey);
    }
    if !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(Error::InvalidKey);
    }
    Ok(())
}

/// Parses `env-file:<relative>` or `keychain:<name>`.
pub(crate) fn parse_destination(raw: &str) -> Result<Destination, Error> {
    let trimmed = raw.trim();
    if let Some(rel) = trimmed.strip_prefix("env-file:") {
        let rel = rel.trim();
        if rel.is_empty() || rel.contains('\0') {
            return Err(Error::InvalidDestination);
        }
        let path = PathBuf::from(rel);
        if path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(Error::PathEscapesRoot);
        }
        return Ok(Destination::EnvFile(path));
    }
    if let Some(name) = trimmed.strip_prefix("keychain:") {
        let name = name.trim();
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(Error::InvalidDestination);
        }
        return Ok(Destination::Keychain(name.to_owned()));
    }
    Err(Error::InvalidDestination)
}

/// Injects `secret` for `key` into `destination` under `workspace_root`.
/// Must be called on a background thread (performs file/keyring I/O).
pub(crate) fn inject(
    workspace_root: &Path,
    key: &str,
    destination: &Destination,
    secret: &SecretValue,
) -> Result<InjectionReceipt, Error> {
    validate_key(key)?;
    match destination {
        Destination::EnvFile(relative) => {
            let target = resolve_safe_path(workspace_root, relative)?;
            write_env_entry(&target, key, secret.expose())?;
            Ok(InjectionReceipt {
                key: key.to_owned(),
                summary: format!(
                    "Injected {key} ({} bytes) into {} (mode 0600; redacted from chat)",
                    secret.byte_len(),
                    relative.display()
                ),
            })
        }
        Destination::Keychain(service) => {
            let safe_dir = workspace_root.join(".sheprd-vault");
            let file_path = safe_dir.join(format!("{service}.env"));
            if let Ok(meta) = fs::symlink_metadata(&safe_dir)
                && meta.file_type().is_symlink()
            {
                return Err(Error::SymlinkRefused { path: safe_dir });
            }
            fs::create_dir_all(&safe_dir).map_err(|source| Error::Write {
                path: safe_dir.clone(),
                source,
            })?;
            write_env_entry(&file_path, key, secret.expose())?;
            Ok(InjectionReceipt {
                key: key.to_owned(),
                summary: format!(
                    "Saved {key} ({} bytes) to private vault '{service}' (redacted from chat)",
                    secret.byte_len()
                ),
            })
        }
    }
}

fn resolve_safe_path(root: &Path, relative: &Path) -> Result<PathBuf, Error> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                current.push(part);
                if let Ok(meta) = fs::symlink_metadata(&current)
                    && meta.file_type().is_symlink()
                {
                    return Err(Error::SymlinkRefused { path: current });
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::PathEscapesRoot);
            }
        }
    }
    Ok(current)
}

fn write_env_entry(path: &Path, key: &str, value: &str) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let existing = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(Error::Write {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let prefix = format!("{key}=");
    let escaped = format_env_value(value);
    let replacement = format!("{prefix}{escaped}");
    let mut lines: Vec<String> = existing.lines().map(ToOwned::to_owned).collect();
    if let Some(slot) = lines.iter_mut().find(|line| line.starts_with(&prefix)) {
        *slot = replacement;
    } else {
        lines.push(replacement);
    }
    let mut out = lines.join("\n");
    out.push('\n');

    let tmp = path.with_extension("sheprd-tmp");
    if let Ok(meta) = fs::symlink_metadata(&tmp)
        && meta.file_type().is_symlink()
    {
        return Err(Error::SymlinkRefused { path: tmp });
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|source| Error::Write {
        path: tmp.clone(),
        source,
    })?;
    file.write_all(out.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|source| Error::Write {
            path: tmp.clone(),
            source,
        })?;
    drop(file);
    fs::rename(&tmp, path).map_err(|source| Error::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn format_env_value(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '#' | '"' | '\'' | '\\' | '$' | '`'));
    if !needs_quotes {
        return value.to_owned();
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}
