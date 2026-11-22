use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // keep-sorted start
    #[error("not a thread id")]
    Id,
    #[error("could not read or write a thread")]
    Io(#[from] io::Error),
    #[error("a thread record is not valid JSON")]
    Json(#[from] serde_json::Error),
    #[error("a thread segment's model name exceeds {} bytes", super::MAX_MODEL)]
    Model,
    #[error("a thread record's folder is not an absolute path")]
    Path,
    #[error("a thread has no segments or more than {}", super::MAX_SEGMENTS)]
    Segments,
    #[error("a thread segment's session id is not plain enough to type into a shell")]
    Session,
    #[error("a thread record exceeds {} bytes", super::MAX_RECORD_SIZE)]
    Size,
    #[error("a thread record's title exceeds {} bytes", super::MAX_TITLE)]
    Title,
    #[error("a thread's transcript file exceeds {} bytes", super::file::MAX_FILE)]
    TranscriptSize,
    #[error("a thread record has version {0}")]
    Version(u32),
    // keep-sorted end
}
