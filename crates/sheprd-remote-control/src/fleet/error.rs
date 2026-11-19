#[derive(Debug, thiserror::Error)]
pub enum Error {
    // keep-sorted start
    #[error(transparent)]
    Auth(#[from] crate::auth::Error),
    #[error(transparent)]
    Control(#[from] crate::control::Error),
    #[error("the remote control daemon refused: {0}")]
    Daemon(String),
    #[error(transparent)]
    Desktop(#[from] crate::desktop::Error),
    #[error("this computer is in a fleet already; leave it first")]
    InFleet,
    #[error(transparent)]
    Joins(#[from] super::joins::Error),
    #[error(transparent)]
    Links(#[from] super::links::Error),
    #[error(transparent)]
    Log(#[from] crate::control::fleet::Error),
    #[error("this computer cannot sign with its certificate authority")]
    NoAuthority,
    #[error("this computer is not in a fleet")]
    NoFleet,
    #[error("this computer listens on no relay; turn one on first")]
    NoRelays,
    #[error("the operations handed over do not admit this computer to a fleet")]
    NotAdmitted,
    #[error("the fleet did not take the operation this computer signed")]
    NotApplied,
    #[error("that bundle is not this computer's")]
    NotMine,
    #[error("the fleet's peers file does not read")]
    Peers,
    #[error(transparent)]
    SshAccess(#[from] crate::ssh_access::Error),
    #[error(transparent)]
    Tunnel(#[from] sheprd_tunnel::Error),
    // keep-sorted end
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
