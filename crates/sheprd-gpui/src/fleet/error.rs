//! Why the fleet could not do what was asked.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // keep-sorted start
    #[error("The relay agent's answer is not one this app can read")]
    Agent(#[from] sheprd_relay_agent::Error),
    #[error("The host's answer is not one this app can read")]
    Answer(#[source] serde_json::Error),
    #[error(transparent)]
    Build(#[from] crate::host_build::Error),
    #[error("The remote control daemon refused: {0}")]
    Daemon(String),
    #[error(
        "The host's SSH key is not one this computer trusts for it; open it once over SSH to check it"
    )]
    HostKey,
    #[error("The host is not one the fleet can share")]
    Intro(#[source] sheprd_remote_control::control::fleet::Error),
    #[error("This app ships no sheprd-relay-agent for {platform}")]
    NoBuild { platform: String },
    #[error("Remote control is off on this computer")]
    NoDaemon,
    #[error("The host has no Sheprd remote control to ask")]
    NoProgram,
    #[error("This computer listens on no relay; turn one on in Remote Control first")]
    NoRelays,
    #[error("Could not read the account's name on the host")]
    NoUser,
    #[error("The relay agent's service did not start on the host")]
    NotRunning,
    #[error("Could not reach the remote control daemon")]
    Request(#[source] sheprd_remote_control::control::Error),
    #[error("Could not run sheprd-remote-control on the host")]
    Run(#[source] sheprd_client::Error),
    #[error("Could not check the host's SSH key against this computer's known hosts")]
    Trust(#[source] sheprd_client::Error),
    #[error(
        "This host's SSH key is not checked; the fleet shares only hosts whose key is checked strictly"
    )]
    Unchecked,
    #[error("The fleet cannot reach this kind of device")]
    Unsupported,
    // keep-sorted end
}
