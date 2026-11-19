//! The fleet from a phone (`docs/relay-plan.md`, Fleet, phase 5). Every
//! call needs a paired device's session.
//!
//! - `GET /api/v2/fleet?member=ID`: the fleet as this computer sees it,
//!   and what the phone needs to sign its next operation when `ID` is a
//!   live member: the fleet, its sequence number, and the parents. The
//!   phone keeps no log in this version; a removal it signs counts once
//!   any member holds it, whatever parents it names.
//! - `POST /api/v2/fleet/sync`: operations the phone signed or carries,
//!   taken like any member's (`Fleet::receive`); the answer is what the
//!   phone lacks (`sync::answer`). Removals flood the fleet as soon as they
//!   land, as from any member.
//! - `POST /api/v2/fleet/join`: a phone's bundle and its join signature,
//!   kept for a person to admit in Sheprd (`fleet::joins`); never admitted
//!   here.
//!
//! Operations are signed by their authors and checked by the log, so
//! nothing a phone posts can grant it what no live member signed.

use super::{ApiError, Device, Shared, render::now_ms};
use crate::control::fleet::{
    Body, HostId, HostKind, MAX_OP, MemberId, View,
    sync::{self, Applied},
};
use crate::fleet::{Fleet, joins::Standing};
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Operations one sync may carry.
pub(super) const MAX_SYNC_OPS: usize = 64;
/// Their bytes, decoded.
pub(super) const MAX_SYNC_BYTES: usize = sync::MAX_ANSWER_BYTES;
/// Heads one sync may name.
pub(super) const MAX_HEADS: usize = 64;
/// The largest sync body: its operations in base64url, and JSON around
/// them.
pub(super) const MAX_SYNC_BODY: usize = MAX_SYNC_BYTES * 4 / 3 + 32 * 1024;
/// The largest join body: one bundle and a signature.
pub(super) const MAX_JOIN_BODY: usize = 2 * MAX_OP;

#[derive(Deserialize)]
pub(super) struct StatusQuery {
    /// The phone's member id, hex, once it has one.
    member: Option<String>,
}

/// What `GET /api/v2/fleet` answers.
#[derive(Debug, Default, Serialize)]
pub(super) struct Status {
    /// None while this computer is in no fleet.
    #[serde(skip_serializing_if = "Option::is_none")]
    view: Option<View>,
    heads: Vec<String>,
    /// What the member signs next on, when it is live here.
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<Next>,
    /// Where the member's join stands, when it asked here.
    #[serde(skip_serializing_if = "Option::is_none")]
    join: Option<Standing>,
    /// The hosts the member linked a device key for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    links: Vec<String>,
    /// Where each Sheprd host and computer member says it takes direct
    /// connections, by host: what a phone tries before its relays.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reach: Vec<Reach>,
}

/// What a member signs its next operation on.
#[derive(Debug, Serialize)]
pub(super) struct Next {
    fleet: String,
    seq: u64,
    parents: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct Reach {
    host: String,
    direct: Vec<String>,
}

pub(super) async fn status(
    State(state): State<Shared>,
    _device: Device,
    Query(query): Query<StatusQuery>,
) -> Result<Json<Status>, ApiError> {
    let fleet = fleet(&state)?;
    let member = query
        .member
        .as_deref()
        .map(sync::member)
        .transpose()
        .map_err(crate::fleet::Error::from)?;
    blocking(move || Ok(read(&fleet, member.as_ref(), now_ms()))).await
}

fn read(fleet: &Fleet, member: Option<&MemberId>, now_ms: u64) -> Status {
    let view = crate::fleet::ask::view(fleet);
    let join = member.and_then(|member| {
        fleet
            .joins()
            .pending(member, now_ms)
            .then_some(Standing::Pending)
    });
    let inner = fleet.inner();
    let Some(log) = inner.log.as_ref() else {
        return Status {
            join,
            ..Status::default()
        };
    };
    let state = log.state();
    let live = member.filter(|member| log.is_member(member));
    let next = live.map(|member| {
        // The body is not signed here: only its sequence number and
        // parents are read.
        let draft = log.draft(*member, now_ms, Body::HostRemove(HostId([0; 32])));
        Next {
            fleet: draft.fleet.to_string(),
            seq: draft.seq,
            parents: draft.parents.iter().map(ToString::to_string).collect(),
        }
    });
    let links = live
        .map(|member| {
            state
                .links
                .keys()
                .filter(|(phone, _)| phone == member)
                .map(|(_, host)| host.to_string())
                .collect()
        })
        .unwrap_or_default();
    let reach = state
        .hosts
        .iter()
        .filter(|(_, host)| host.intro.kind != HostKind::Agent && !host.record.direct.is_empty())
        .map(|(id, host)| Reach {
            host: id.to_string(),
            direct: host.record.direct.clone(),
        })
        .collect();
    Status {
        view,
        heads: log.heads().iter().map(ToString::to_string).collect(),
        next,
        join: live.map_or(join, |_| Some(Standing::Member)),
        links,
        reach,
    }
}

pub(super) async fn sync(
    State(state): State<Shared>,
    _device: Device,
    Json(apply): Json<sync::Apply>,
) -> Result<Json<Applied>, ApiError> {
    let fleet = fleet(&state)?;
    if apply.ops.len() > MAX_SYNC_OPS || apply.heads.len() > MAX_HEADS {
        return Err(ApiError::Invalid("too many operations in one sync"));
    }
    let heads = apply.heads().map_err(crate::fleet::Error::from)?;
    let ops = apply.ops().map_err(crate::fleet::Error::from)?;
    if ops.iter().map(|op| op.encode().len()).sum::<usize>() > MAX_SYNC_BYTES {
        return Err(ApiError::Invalid("too many operations in one sync"));
    }
    let named = apply.fleet.clone();
    blocking(move || {
        let held = fleet
            .inner()
            .log
            .as_ref()
            .map(|log| log.fleet().to_string());
        // A phone never makes this computer join a fleet: that takes a
        // member's admission and Sheprd's dialog.
        let held = held.ok_or(crate::fleet::Error::NoFleet)?;
        if named.is_some_and(|named| !named.eq_ignore_ascii_case(&held)) {
            return Err(crate::fleet::Error::Log(crate::control::fleet::Error::Fleet).into());
        }
        let now = now_ms();
        let refused = fleet.receive(ops, now)?;
        if refused > 0 {
            tracing::info!(refused, "a phone's fleet operations were refused");
        }
        let inner = fleet.inner();
        let log = inner.log.as_ref().ok_or(crate::fleet::Error::NoFleet)?;
        Ok(sync::answer(log, &heads, refused))
    })
    .await
}

/// `POST /api/v2/fleet/join`: a phone's bundle, base64url, and its
/// signature over the fleet id and the bundle.
#[derive(Deserialize)]
pub(super) struct JoinRequest {
    bundle: String,
    signature: String,
}

#[derive(Debug, Serialize)]
pub(super) struct Joined {
    member: String,
    state: Standing,
}

pub(super) async fn join(
    State(state): State<Shared>,
    Device(device): Device,
    Json(request): Json<JoinRequest>,
) -> Result<Json<Joined>, ApiError> {
    let fleet = fleet(&state)?;
    let bundle = sync::bytes(&request.bundle)
        .and_then(|bytes| crate::control::fleet::Bundle::from_bytes(&bytes))
        .map_err(crate::fleet::Error::from)?;
    let signature: [u8; 64] = sync::bytes(&request.signature)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(crate::fleet::Error::Joins(
            crate::fleet::joins::Error::Signature,
        ))?;
    let name = state
        .auth()
        .devices()
        .get(&device)
        .map(|record| record.name.clone())
        .ok_or(ApiError::Unauthorized)?;
    blocking(move || {
        let member = bundle.id();
        let inner = fleet.inner();
        let log = inner.log.as_ref().ok_or(crate::fleet::Error::NoFleet)?;
        let standing = fleet
            .joins()
            .ask(log, bundle, signature, (device, name), now_ms())
            .map_err(crate::fleet::Error::from)?;
        Ok(Joined {
            member: member.to_string(),
            state: standing,
        })
    })
    .await
}

fn fleet(state: &Shared) -> Result<Arc<Fleet>, ApiError> {
    state.fleet.clone().ok_or(ApiError::NotFound)
}

async fn blocking<T: Send + 'static>(
    action: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<Json<T>, ApiError> {
    tokio::task::spawn_blocking(action)
        .await
        .map_err(|_| ApiError::Invalid("the request was interrupted"))?
        .map(Json)
}
