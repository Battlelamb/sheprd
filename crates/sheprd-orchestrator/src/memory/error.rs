//! Agent memory failures.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Memory database failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("A memory entry needs some text")]
    Empty,
    #[error("Unknown memory kind {0:?}")]
    Kind(String),
    #[error(
        "This repository's memory is schema {found}; this sheprd understands up to {known}. Update sheprd to use it."
    )]
    NewerSchema { found: i64, known: i64 },
    #[error("No state directory is available for agent memory")]
    NoStateDirectory,
    #[error(transparent)]
    Private(#[from] crate::private_file::Error),
    #[error("A memory entry is at most {max} bytes; keep it to a fact or two")]
    TooLong { max: usize },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
