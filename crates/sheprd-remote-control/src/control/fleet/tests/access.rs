//! A plain host's access file, as the log computes it, and the agent's
//! reading of operations and frames checked against the log's own.

use super::*;
use crate::control::fleet::access::{self as host_access, Id};
use sheprd_relay_agent::removal;

fn access_body(host: HostId, member: &Device, allow: bool, rev: u64) -> Body {
    Body::Access(Access {
        host,
        member: member.id(),
        allow,
        rev,
    })
}

/// a, b, and c, computers, a phone, and a plain host a added.
fn fleet() -> (Log, [Device; 3], HostId) {
    let (a, b, c) = (computer(1), computer(2), computer(3));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    admit(&mut log, &a, &c);
    admit(&mut log, &a, &phone(9));
    let intro = plain_host(40);
    let host = intro.id().unwrap();
    act(&mut log, &a, Body::HostAdd(intro));
    (log, [a, b, c], host)
}

fn id(member: &Device) -> Id {
    Id(member.id().0)
}

#[test]
fn every_computer_is_named_with_its_keys_and_access() {
    let (mut log, [a, b, c], host) = fleet();
    act(&mut log, &a, access_body(host, &c, false, 1));
    let file = host_access::compute(&log, &host, []).unwrap();
    assert_eq!(file.fleet, Id(log.fleet().0));
    assert_eq!(file.members.len(), 3, "phones are not named");
    let first = &file.members[&id(&a)];
    assert!(first.allow);
    assert_eq!(first.rev, 0);
    assert_eq!(first.dial, a.bundle.dial);
    assert_eq!(first.tunnel, a.bundle.tunnel);
    assert_eq!(Some(first.ssh.as_str()), a.bundle.ssh.as_deref());
    assert!(file.members[&id(&b)].allow);
    let excluded = &file.members[&id(&c)];
    assert!(!excluded.allow);
    assert_eq!(excluded.rev, 1);
    assert!(file.removed.is_empty());
}

#[test]
fn removed_members_are_tombstones_and_named_no_more() {
    let (mut log, [a, b, _], host) = fleet();
    act(&mut log, &a, remove_body(&b));
    let file = host_access::compute(&log, &host, []).unwrap();
    assert!(file.removed.contains(&id(&b)));
    assert!(!file.members.contains_key(&id(&b)));
}

#[test]
fn a_current_log_settles_every_removal_asked_about() {
    let (mut log, [a, b, _], host) = fleet();
    let removal = act(&mut log, &a, remove_body(&b));
    let access = act(&mut log, &a, access_body(host, &a, true, 1));
    // An id no removal has, as a member could make up to keep another
    // suspended, and one that is no removal: neither stands.
    let unknown = OpId([0x42; 32]);
    let file = host_access::compute(&log, &host, [removal.id(), access.id(), unknown]).unwrap();
    let mut wanted = vec![Id(removal.id().0), Id(access.id().0), Id(unknown.0)];
    wanted.sort();
    assert_eq!(file.settled.into_iter().collect::<Vec<_>>(), wanted);
}

#[test]
fn a_log_waiting_for_parents_settles_only_the_removals_it_holds() {
    let (mut log, [a, b, c], host) = fleet();
    let removal = act(&mut log, &a, remove_body(&c));
    // An operation whose parent has not arrived: the log may lack a
    // removal it is about to get.
    let mut other = copy(&log);
    let admitted = admit(&mut other, &b, &computer(5));
    let child = act(&mut other, &b, Body::HostAdd(plain_host(41)));
    assert_eq!(log.insert(child, NOW), Ok(Inserted::Pending));
    let unknown = OpId([0x42; 32]);
    let file = host_access::compute(&log, &host, [removal.id(), unknown]).unwrap();
    assert_eq!(
        file.settled.into_iter().collect::<Vec<_>>(),
        [Id(removal.id().0)]
    );
    drop(admitted);
}

#[test]
fn only_a_known_plain_host_has_an_access_file() {
    let (mut log, [a, ..], _) = fleet();
    assert_eq!(
        host_access::compute(&log, &HostId([7; 32]), []),
        Err(Error::Unknown)
    );
    let (intro, _) = ca_host(41, HostKind::Sheprd);
    let sheprd = intro.id().unwrap();
    act(&mut log, &a, Body::HostAdd(intro));
    assert_eq!(host_access::compute(&log, &sheprd, []), Err(Error::Invalid));
}

#[test]
fn a_host_two_members_added_with_different_keys_has_none() {
    let (a, b) = (computer(1), computer(2));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    let intro = plain_host(40);
    let host = intro.id().unwrap();
    let mut other = intro.clone();
    other.record.ssh_host_keys.push(ssh_line(251));
    let before = log.heads().to_vec();
    let next = |member: &Device| log.state().members[&member.id()].seq + 1;
    let ops = [
        sign_on(&log, &a, &before, next(&a), Body::HostAdd(intro)),
        sign_on(&log, &b, &before, next(&b), Body::HostAdd(other)),
    ];
    for op in ops {
        apply(&mut log, op);
    }
    assert_eq!(host_access::compute(&log, &host, []), Err(Error::HostHeld));
}

#[test]
fn a_stale_members_file_cannot_undo_a_removal_on_the_agent() {
    let (mut log, [a, b, _], host) = fleet();
    let stale = copy(&log);
    act(&mut log, &a, remove_body(&b));
    let fresh = host_access::compute(&log, &host, []).unwrap();
    let held = host_access::Access::merge(None, fresh).unwrap();
    let old = host_access::compute(&stale, &host, []).unwrap();
    let merged = host_access::Access::merge(Some(&held), old).unwrap();
    assert!(!merged.members.contains_key(&id(&b)));
    assert_eq!(merged.digest().unwrap(), held.digest().unwrap());
}

#[test]
fn a_file_the_agent_merged_has_the_digest_the_log_computes() {
    let (log, _, host) = fleet();
    let file = host_access::compute(&log, &host, []).unwrap();
    let held = host_access::Access::merge(None, file.clone()).unwrap();
    assert_eq!(held.digest().unwrap(), file.digest().unwrap());
}

#[test]
fn the_agent_reads_a_removal_as_the_log_encodes_it() {
    let (mut log, [a, b, c], _) = fleet();
    let body = Body::Remove(Remove {
        member: b.id(),
        also_members: vec![c.id()],
        also_hosts: vec![HostId([5; 32])],
        reason: Reason::Lost,
    });
    let op = act(&mut log, &a, body);
    let read = removal::peek(&op.encode()).unwrap().unwrap();
    assert_eq!(read.id, op.id().0);
    assert_eq!(read.fleet, log.fleet().0);
    assert_eq!(read.signer, a.id().0);
    assert_eq!(read.targets, [b.id().0, c.id().0]);
}

#[test]
fn the_agent_reads_every_other_operation_as_no_removal() {
    let (log, ..) = fleet();
    for op in log.ops() {
        assert_eq!(removal::peek(&op.encode()), Ok(None), "{:?}", op.body());
    }
}

#[test]
fn a_frame_of_operation_ids_reads_back() {
    let message = removal::Message::Held(vec![OpId([3; 32]), OpId([4; 32])]);
    let bytes = message.encode().unwrap();
    assert_eq!(
        crate::control::fleet::removal::Message::decode(&bytes).unwrap(),
        message
    );
    assert_eq!(
        crate::control::fleet::removal::Message::decode(&bytes[..3]),
        Err(removal::Error::Encoding)
    );
}
