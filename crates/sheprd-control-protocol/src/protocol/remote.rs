//! Remote control: the daemon's status and devices, how its iPhone pushes
//! leave it (the push relay), the pairing dialog, the revoke confirmation,
//! restarting the daemon, and keeping a private relay's listen token. The daemon's own types are reused as they are;
//! no pairing token ever crosses this socket, and a listen token crosses
//! it only from the caller, once, to be kept: no answer carries it back.
use serde::{Deserialize, Serialize};
use sheprd_remote_control::control::{Device, Status, push::PushStatus, relays::ListenToken};

/// Asking the daemon, and for `revoke` the device list first, takes up to
/// its own answer timeout a request.
const DAEMON_WAIT_SECONDS: u64 = 12;
/// `push-status` also asks every linked host over SSH, at once, each
/// within its own idle limit.
const HOSTS_WAIT_SECONDS: u64 = 45;
/// Linked hosts a `push-status` reply names at most.
pub const MAX_PUSH_HOSTS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RemoteOp {
    Status,
    /// Opens the pairing dialog in the window.
    Pair {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<String>,
    },
    Devices,
    /// How iPhone pushes leave the daemon (off, own APNs key, or push
    /// relay) and how the last of each kind went. Read-only: the push
    /// relay is turned on in Settings.
    PushStatus,
    /// Opens the confirmation for revoking a device, by ID or name.
    Revoke {
        device: String,
    },
    /// Restarts the daemon from the binary beside the app.
    Restart,
    /// Keeps this computer's listen token for the relay at `relay`, or
    /// without one forgets it. `Debug` never shows the token.
    RelayToken {
        relay: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<ListenToken>,
    },
}

impl RemoteOp {
    pub fn wait_seconds(&self) -> u64 {
        match self {
            Self::PushStatus => HOSTS_WAIT_SECONDS,
            Self::Status | Self::Devices | Self::Revoke { .. } | Self::RelayToken { .. } => {
                DAEMON_WAIT_SECONDS
            }
            Self::Pair { .. } | Self::Restart => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reply {
    Status {
        /// Whether remote control is on in Settings.
        enabled: bool,
        /// What the daemon says, when it answers.
        #[serde(default)]
        status: Option<Box<Status>>,
        /// Why it did not, when it is on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Devices {
        devices: Vec<Device>,
    },
    PushStatus {
        /// Whether remote control is on in Settings.
        enabled: bool,
        /// Whether `[remote] push_relay` allows the push relay. An own
        /// APNs key wins over it, so `status` may still say `own_key`.
        push_relay: bool,
        /// What the daemon says, when it answers; boxed, as `Status`'s is,
        /// to keep every response small.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<Box<PushStatus>>,
        /// Why it did not, when remote control is on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// Each linked host's daemon, asked over SSH, at most
        /// [`MAX_PUSH_HOSTS`]; left out when there are none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hosts: Vec<HostPush>,
    },
    /// A listen token was kept or forgotten for `relay`, as the app
    /// normalized its address; `token` is whether one is now kept.
    RelayToken {
        relay: String,
        token: bool,
    },
}

/// How a linked host's daemon sends iPhone pushes, as it answered over SSH.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPush {
    /// The host's name, as Sheprd lists it.
    pub name: String,
    /// What its daemon says; absent when none runs there, or it is too old
    /// to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<PushStatus>,
    /// Why it could not be asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests;
