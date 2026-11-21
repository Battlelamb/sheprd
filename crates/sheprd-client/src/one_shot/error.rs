//! Why Claude, run once on its own, gave no answer.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // keep-sorted start
    #[error("the agent answered nothing")]
    Empty,
    #[error("the agent stopped with an error")]
    Failed,
    #[error("no agent that can answer is installed on this computer")]
    NoAgent,
    #[error("the agent could not start")]
    Start(#[source] std::io::Error),
    #[error("the agent took too long to answer")]
    TimedOut,
    // keep-sorted end
}
