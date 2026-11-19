//! What Sheprd asks its daemon about the fleet (`Request::Fleet`), and the
//! exchange `sheprd-remote-control fleet apply` runs over SSH on another
//! member or host. Operations travel as their canonical bytes in
//! base64url, so nothing here can change what was signed.

use super::{
    Body, Error, FleetId, HostId, Kind, Log, MemberId, Method, Op, OpId, Reason, Result, State,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

/// The most operation bytes one answer carries before saying `more`.
pub const MAX_ANSWER_BYTES: usize = 256 * 1024;
/// The largest `fleet apply` request read.
pub const MAX_REQUEST_BYTES: usize = 2 * MAX_ANSWER_BYTES + 64 * 1024;

/// One `fleet apply`: the caller's heads, so the answer carries what it
/// lacks, and the operations it believes the receiver lacks.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Apply {
    /// The fleet, hex; none from a caller asking a host with no fleet yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet: Option<String>,
    #[serde(default)]
    pub heads: Vec<String>,
    #[serde(default)]
    pub ops: Vec<String>,
}

/// What a `fleet apply` came to.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Applied {
    pub fleet: String,
    pub heads: Vec<String>,
    /// What the caller lacks, in graph order.
    pub ops: Vec<String>,
    /// How many of the caller's operations were refused.
    #[serde(default)]
    pub refused: usize,
    /// The answer stopped at [`MAX_ANSWER_BYTES`]: ask again.
    #[serde(default)]
    pub more: bool,
}

impl Apply {
    /// The heads, read; one that is not an id fails the request.
    pub fn heads(&self) -> Result<Vec<OpId>> {
        self.heads
            .iter()
            .map(|head| OpId::from_hex(head).ok_or(Error::Encoding))
            .collect()
    }

    /// The operations, read.
    pub fn ops(&self) -> Result<Vec<Op>> {
        decode_ops(&self.ops)
    }
}

/// The answer to `apply` from `log`: its heads and what the caller lacks.
#[must_use]
pub fn answer(log: &Log, caller_heads: &[OpId], refused: usize) -> Applied {
    let mut ops = Vec::new();
    let mut bytes = 0;
    let mut more = false;
    for op in log.missing_for(caller_heads) {
        let encoded = op.encode();
        if bytes + encoded.len() > MAX_ANSWER_BYTES {
            more = true;
            break;
        }
        bytes += encoded.len();
        ops.push(URL_SAFE_NO_PAD.encode(encoded));
    }
    Applied {
        fleet: log.fleet().to_string(),
        heads: log.heads().iter().map(ToString::to_string).collect(),
        ops,
        refused,
        more,
    }
}

/// Operations from base64url text.
pub fn decode_ops(ops: &[String]) -> Result<Vec<Op>> {
    ops.iter()
        .map(|text| {
            let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| Error::Encoding)?;
            Op::decode(&bytes)
        })
        .collect()
}

/// Operations as base64url text.
#[must_use]
pub fn encode_ops<'a>(ops: impl IntoIterator<Item = &'a Op>) -> Vec<String> {
    ops.into_iter()
        .map(|op| URL_SAFE_NO_PAD.encode(op.encode()))
        .collect()
}

/// What Sheprd asks its daemon.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "ask")]
pub enum Ask {
    /// The fleet as this daemon sees it.
    Status,
    /// Founds a fleet with this computer as its only member.
    Found,
    /// This computer's bundle, signed for `fleet`, for a member that is
    /// about to admit it.
    Bundle { fleet: String },
    /// Admits a member, after a person's approval here.
    Admit {
        bundle: String,
        signature: String,
        method: Method,
    },
    /// Removes a member, and what goes with it.
    Remove {
        member: String,
        #[serde(default)]
        also_members: Vec<String>,
        #[serde(default)]
        also_hosts: Vec<String>,
        reason: Reason,
    },
    /// Shares a host, with members left out.
    AddHost {
        intro: String,
        #[serde(default)]
        deny: Vec<String>,
    },
    /// Allows a computer member on a host, or not.
    Access {
        host: String,
        member: String,
        allow: bool,
    },
    /// Removes a host everywhere.
    RemoveHost { host: String },
    /// Takes operations another member carried, and says what it lacks.
    Apply(Apply),
    /// The phones waiting to join through this computer, for a person to
    /// admit one with [`Ask::Admit`]; none is ever admitted otherwise.
    Joins,
}

/// What the daemon answers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "answer")]
pub enum Answer {
    Status {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view: Option<Box<super::View>>,
    },
    Bundle {
        bundle: String,
        signature: String,
    },
    Applied(Applied),
    Done,
    Joins {
        joins: Vec<Join>,
    },
}

/// A phone that asked to join through this computer
/// (`POST /api/v2/fleet/join`), kept in memory for a while.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Join {
    /// The joiner's member id, hex.
    pub member: String,
    /// The name in its bundle.
    pub name: String,
    pub kind: Kind,
    /// Its bundle and its signature over [`super::Bundle::join_message`],
    /// base64url, as [`Ask::Admit`] takes them.
    pub bundle: String,
    pub signature: String,
    /// The paired device that asked, as this computer names it.
    pub device: String,
    pub received_ms: u64,
}

/// Reads a member id from hex.
pub fn member(text: &str) -> Result<MemberId> {
    MemberId::from_hex(text).ok_or(Error::Encoding)
}

/// Reads a host id from hex.
pub fn host(text: &str) -> Result<HostId> {
    HostId::from_hex(text).ok_or(Error::Encoding)
}

/// Reads a fleet id from hex.
pub fn fleet(text: &str) -> Result<FleetId> {
    FleetId::from_hex(text).ok_or(Error::Encoding)
}

/// Bytes from base64url.
pub fn bytes(text: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text).map_err(|_| Error::Encoding)
}

/// Text from bytes, base64url.
#[must_use]
pub fn text(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The next `rev` for an `Access` on `host` and `member`: one above what
/// `state` holds.
#[must_use]
pub fn next_rev(state: &State, host: HostId, member: MemberId) -> u64 {
    state
        .access
        .get(&(host, member))
        .map_or(0, |register| register.rev)
        .saturating_add(1)
}

/// Whether `log` admits `member`: what a daemon with no fleet checks
/// before taking a log handed to it, so it joins only a fleet that a
/// member admitted it to.
#[must_use]
pub fn admits(log: &Log, member: &MemberId) -> bool {
    log.is_member(member)
        && log
            .ops()
            .any(|op| matches!(op.body(), Body::Admit(admit) if admit.bundle.id() == *member))
}
