//! Why the agent gave no summary; shown on the chat's row as it says.

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    // keep-sorted start
    #[error("too many summaries are running; try again in a moment")]
    Busy,
    #[error("the agent answered nothing")]
    Empty,
    #[error("the agent stopped with an error")]
    Failed,
    #[error("summaries use Claude Code, which is not installed on this computer")]
    NoAgent,
    #[error("the agent could not start")]
    Start(#[source] std::io::Error),
    #[error("the agent took too long to answer")]
    TimedOut,
    // keep-sorted end
}
