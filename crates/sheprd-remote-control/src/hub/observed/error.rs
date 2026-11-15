/// Why a terminal agent's conversation cannot be shown as a chat. The phone
/// keeps showing the terminal instead.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    // keep-sorted start
    #[error("Herdr's answer about the pane did not have the expected shape")]
    Answer(#[source] serde_json::Error),
    #[error("the agent's own process could not be told apart in its pane")]
    Process,
    #[error("Herdr names no session this agent's conversation can be read from")]
    Session,
    #[error("the observer could not start")]
    Start(#[source] std::io::Error),
    #[error("only Claude Code in a Herdr 1 pane on macOS or Linux shows as a chat")]
    Unsupported,
    // keep-sorted end
}
