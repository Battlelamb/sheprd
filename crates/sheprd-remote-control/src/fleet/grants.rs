//! What the log makes of this computer, rewritten after every change:
//!
//! - as a host: one `authorized_keys` line per computer member allowed on
//!   it (`ssh_access`), their dial keys on its relays, and their tunnel
//!   keys admitted;
//! - as a member: a relay record and known hosts file for each fleet host
//!   it may reach, which `relay-connect` dials, and none for one it may
//!   not.
//!
//! A suspended member, cut off by a removal whose history has not arrived,
//! is left out of all of it at once.

use super::{Fleet, Result, me};
use crate::control::fleet::{Body, HostId, Kind, MemberId, State as Log};
use crate::desktop::{files, record::Record};
use crate::ssh_access::SshKey;
use crate::web::{Shared, State};
use sheprd_relay::wire::Key;
use std::{collections::BTreeSet, path::Path, time::Duration};

/// How often this computer checks whether its own record moved.
const RECORD_CHECK: Duration = Duration::from_secs(10 * 60);
/// The ids of the relay records the fleet wrote, so it removes only its
/// own.
const RECORDS: &str = "fleet/records.json";

/// A fleet member that dialed in: who, and whether it may reach SSH here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Peer {
    pub(crate) member: MemberId,
    pub(crate) ssh: bool,
}

/// The live computer member whose tunnel key `remote` is, if any. Every
/// live computer member may pass a removal; only those allowed here reach
/// SSH.
pub(crate) fn peer(fleet: &Fleet, remote: &[u8; 32]) -> Option<Peer> {
    let suspended = fleet.suspended();
    let inner = fleet.inner();
    let state = inner.log.as_ref()?.state();
    let mut holders = state.members.iter().filter(|(id, member)| {
        **id != fleet.me()
            && member.bundle.kind == Kind::Computer
            && member.bundle.tunnel == *remote
    });
    let (member, _) = holders.next()?;
    // Two members naming one key (admitted concurrently, which the log
    // cannot refuse) say nothing about who dialed: neither is let in.
    if holders.next().is_some() || suspended.contains(member) {
        return None;
    }
    let ssh = own_host(state, &fleet.me()).is_some_and(|host| state.allowed(&host, member));
    Some(Peer {
        member: *member,
        ssh,
    })
}

/// Whether `member` may still reach SSH here: what a live session checks
/// after every change.
pub(crate) fn still_allowed(fleet: &Fleet, member: &MemberId) -> bool {
    if fleet.suspended().contains(member) {
        return false;
    }
    let inner = fleet.inner();
    let Some(state) = inner.log.as_ref().map(|log| log.state()) else {
        return false;
    };
    own_host(state, &fleet.me()).is_some_and(|host| state.allowed(&host, member))
}

/// The dial keys of every live computer member but this one, for the
/// relays to let through.
pub(crate) fn dial_keys(fleet: &Fleet) -> Vec<Key> {
    let suspended = fleet.suspended();
    let inner = fleet.inner();
    let Some(log) = &inner.log else {
        return Vec::new();
    };
    log.state()
        .members
        .iter()
        .filter(|(id, member)| {
            **id != fleet.me() && member.bundle.kind == Kind::Computer && !suspended.contains(id)
        })
        .map(|(_, member)| member.bundle.dial)
        .collect()
}

/// This computer's own host in `state`.
pub(crate) fn own_host(state: &Log, me: &MemberId) -> Option<HostId> {
    state.members.get(me)?.bundle.host.as_ref()?.id().ok()
}

/// Rewrites the SSH lines and relay records from the log. Blocking.
pub(crate) fn apply(state: &State, fleet: &Fleet) -> Result<()> {
    let suspended = fleet.suspended();
    let (lines, records) = {
        let inner = fleet.inner();
        match inner.log.as_ref() {
            Some(log) => (
                lines(log.state(), &log.fleet().short(), &fleet.me(), &suspended),
                records(log.state(), &fleet.me(), &suspended),
            ),
            None => (Vec::new(), Vec::new()),
        }
    };
    if let Ok(access) = state.auth().ssh_access() {
        access.keys.set_fleet(&lines)?;
    }
    revoke_removed(state, fleet)?;
    write_records(fleet.state_dir(), &records)
}

/// Revokes every desktop paired here one by one with a tunnel key a member
/// proved here and that is no longer a member: removing it from the fleet
/// removes every way in it had. Blocking.
fn revoke_removed(state: &State, fleet: &Fleet) -> Result<()> {
    let gone: Vec<(MemberId, Vec<[u8; 32]>)> = {
        let inner = fleet.inner();
        let Some(log) = &inner.log else {
            return Ok(());
        };
        inner
            .proven
            .iter()
            .filter(|(member, _)| !log.is_member(member))
            .map(|(member, keys)| (*member, keys.iter().copied().collect()))
            .collect()
    };
    for (member, keys) in &gone {
        for tunnel in keys {
            supersede(state, tunnel)?;
        }
        // Saved, so the file and what is held agree.
        fleet.forget_proven(member)?;
    }
    Ok(())
}

/// Revokes the desktop paired here one by one, over Bluetooth, whose
/// tunnel key is `tunnel`, once that device has proved it is a fleet member
/// allowed here: it just finished the tunnel's handshake with that key and
/// asked for SSH. The fleet lets it in now, and a pairing left beside it
/// would outlive its removal from the fleet. Matching on a key the device
/// proved, not on what a bundle claims, keeps a member from revoking a
/// desktop by naming its public keys. Blocking.
pub(crate) fn supersede(state: &State, tunnel: &[u8; 32]) -> Result<()> {
    let mut auth = state.auth();
    let paired: Vec<crate::control::DeviceId> = auth
        .devices()
        .records()
        .iter()
        .filter(|record| {
            record.ssh.is_some()
                && record
                    .relay
                    .as_ref()
                    .and_then(|relay| relay.tunnel_key())
                    .is_some_and(|key| key == *tunnel)
        })
        .map(|record| record.id.clone())
        .collect();
    for device in paired {
        tracing::info!("the fleet now lets in a desktop paired here; its own pairing goes");
        auth.revoke(&device)?;
    }
    Ok(())
}

/// One line per computer member allowed on this computer, tagged with the
/// fleet's and the member's short ids.
fn lines(state: &Log, fleet: &str, me: &MemberId, suspended: &[MemberId]) -> Vec<(String, SshKey)> {
    let Some(host) = own_host(state, me) else {
        return Vec::new();
    };
    state
        .grants(&host)
        .into_iter()
        .filter(|member| !suspended.contains(member))
        .filter_map(|member| {
            let ssh = state.members.get(&member)?.bundle.ssh.as_deref()?;
            let key = SshKey::parse(ssh).ok()?;
            Some((format!("{fleet}:{}", member.short()), key))
        })
        .collect()
}

/// The fleet hosts this computer may reach, as relay records with their
/// SSH host keys.
fn records(state: &Log, me: &MemberId, suspended: &[MemberId]) -> Vec<(Record, Vec<String>)> {
    if suspended.contains(me) {
        return Vec::new();
    }
    let own = own_host(state, me);
    state
        .hosts
        .iter()
        .filter(|(id, _)| Some(**id) != own && state.allowed(id, me))
        .map(|(_, host)| {
            (
                Record {
                    computer: host.intro.name.clone(),
                    identity: host.record.identity.clone(),
                    relays: host.record.relays.clone(),
                    route: host.record.route.clone(),
                    fleet: true,
                },
                host.record.ssh_host_keys.clone(),
            )
        })
        .collect()
}

/// Writes `records`, and removes those the fleet wrote before and no
/// longer grants. A record pairing wrote under the same id is left alone.
pub(super) fn write_records(state_dir: &Path, records: &[(Record, Vec<String>)]) -> Result<()> {
    let file = state_dir.join(RECORDS);
    let before: BTreeSet<String> = files::read_bounded(&file, 64 * 1024)?
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut written = BTreeSet::new();
    for (record, host_keys) in records {
        // A machine paired here over Bluetooth keeps the record pairing
        // wrote; the fleet never takes it over.
        if let Ok(id) = record.id()
            && Record::load(state_dir, &id).is_ok_and(|kept| !kept.fleet)
        {
            continue;
        }
        match record.save(state_dir, host_keys) {
            Ok(id) => {
                written.insert(id);
            }
            Err(error) => tracing::warn!(error = %error, "could not keep a fleet host's record"),
        }
    }
    for gone in before.difference(&written) {
        // Paired here since the fleet wrote it: pairing owns it now.
        if Record::load(state_dir, gone).is_ok_and(|kept| !kept.fleet) {
            continue;
        }
        Record::remove(state_dir, gone)?;
    }
    let json = serde_json::to_vec(&written).map_err(|_| crate::desktop::Error::RecordFormat)?;
    files::write_private(&file, &json)?;
    Ok(())
}

/// Follows the fleet for the life of the daemon: rewrites what it grants
/// after every change, names the members' dial keys to the relays again,
/// and publishes this computer's record when its relays, route, tunnel
/// identity, or SSH host keys move.
pub(crate) async fn follow(state: Shared) {
    let Some(fleet) = state.fleet.clone() else {
        return;
    };
    let mut changed = fleet.watch();
    let mut check = tokio::time::interval(RECORD_CHECK);
    // What the log grants is written at start too: it may have changed on
    // disk while the daemon was stopped (`fleet apply` with no daemon).
    changed.mark_changed();
    loop {
        tokio::select! {
            result = changed.changed() => {
                if result.is_err() {
                    return;
                }
            }
            _ = check.tick() => {
                let (state, fleet) = (state.clone(), fleet.clone());
                let _ = tokio::task::spawn_blocking(move || publish_record(&state, &fleet)).await;
                continue;
            }
        }
        let (blocking, fleet_ref) = (state.clone(), fleet.clone());
        let applied = tokio::task::spawn_blocking(move || apply(&blocking, &fleet_ref)).await;
        if let Ok(Err(error)) = applied {
            tracing::warn!(error = %error, "could not apply the fleet's grants");
        }
        if let Some(relay) = &state.relay {
            relay.devices_changed();
        }
    }
}

/// Signs a `HostSeen` with this computer's record when it moved. Blocking.
fn publish_record(state: &State, fleet: &Fleet) {
    let now = crate::web::now_ms();
    let held = {
        let inner = fleet.inner();
        let Some(log) = &inner.log else {
            return;
        };
        let Some(host) = own_host(log.state(), &fleet.me()) else {
            return;
        };
        log.state().hosts.get(&host).map(|host| host.record.clone())
    };
    let Ok((record, signature)) = me::record(state, now) else {
        return;
    };
    if held.is_some_and(|held| me::moved(&held, &record)) {
        let seen = crate::control::fleet::HostSeen {
            record,
            signature: Some(signature),
        };
        if let Err(error) = fleet.sign(Body::HostSeen(seen), now) {
            tracing::warn!(error = %error, "could not publish this computer's record to the fleet");
        }
    }
}
