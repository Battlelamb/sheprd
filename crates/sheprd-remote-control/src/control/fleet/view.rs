//! The fleet as Settings, `sheprd fleet`, and `sheprd remote status`
//! show it: plain fields, ids in hex, nothing secret.

use super::{HostKind, Kind, Log, MemberId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub fleet: String,
    /// This device's member id.
    pub me: String,
    pub members: Vec<MemberView>,
    pub hosts: Vec<HostView>,
    /// Members removed, ever.
    pub removed: usize,
    /// Members flagged for signing in two places.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equivocating: Vec<String>,
    /// Members cut off by a removal whose history has not arrived.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suspended: Vec<String>,
    /// Operations waiting for their parents.
    pub pending: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admitted_by: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostView {
    pub id: String,
    pub name: String,
    pub kind: HostKind,
    pub user: String,
    pub added_by: String,
    /// The computer members allowed on it.
    pub grants: Vec<String>,
    pub relays: Vec<String>,
    pub rev: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flagged: bool,
}

impl View {
    /// `log` as member `me` sees it, with `suspended` members cut off.
    #[must_use]
    pub fn of(log: &Log, me: &MemberId, suspended: &[MemberId]) -> Self {
        let state = log.state();
        Self {
            fleet: log.fleet().to_string(),
            me: me.to_string(),
            members: state
                .members
                .iter()
                .map(|(id, member)| MemberView {
                    id: id.to_string(),
                    name: member.bundle.name.clone(),
                    kind: member.bundle.kind,
                    admitted_by: member.admitted_by.map(|by| by.to_string()),
                })
                .collect(),
            hosts: state
                .hosts
                .iter()
                .map(|(id, host)| HostView {
                    id: id.to_string(),
                    name: host.intro.name.clone(),
                    kind: host.intro.kind,
                    user: host.intro.user.clone(),
                    added_by: host.added_by.to_string(),
                    grants: state
                        .grants(id)
                        .iter()
                        .filter(|member| !suspended.contains(member))
                        .map(ToString::to_string)
                        .collect(),
                    relays: host.record.relays.clone(),
                    rev: host.record.rev,
                    held: host.held,
                    flagged: host.flagged,
                })
                .collect(),
            removed: state.tombstones.len(),
            equivocating: state.equivocating.iter().map(ToString::to_string).collect(),
            suspended: suspended.iter().map(ToString::to_string).collect(),
            pending: log.pending(),
        }
    }
}
