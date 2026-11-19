//! What plain hosts' agents are told: only removals that removed every
//! member they name, and members' keys versioned by their own rotations.

use super::*;
use crate::control::fleet::access::{self as host_access, Id};

fn access_body(host: HostId, member: &Device, allow: bool, rev: u64) -> Body {
    Body::Access(Access {
        host,
        member: member.id(),
        allow,
        rev,
    })
}

fn removing(targets: &[&Device]) -> Body {
    Body::Remove(Remove {
        member: targets[0].id(),
        also_members: targets[1..].iter().map(|target| target.id()).collect(),
        also_hosts: Vec::new(),
        reason: Reason::Compromised,
    })
}

#[test]
fn a_voided_removal_naming_a_member_another_removal_took_is_not_passed_to_agents() {
    let (a, b, c, d) = (computer(1), computer(2), computer(3), computer(4));
    let mut log = found(&a);
    for member in [&b, &c, &d] {
        admit(&mut log, &a, member);
    }
    // c removes a and b while a removes c: c's removal is void.
    let before = log.heads().to_vec();
    let seq = log.state().members[&c.id()].seq + 1;
    let voided = sign_on(&log, &c, &before, seq, removing(&[&a, &b]));
    act(&mut log, &a, removing(&[&c]));
    apply(&mut log, voided.clone());
    // d, having seen it all, removes a: a is gone, b stays.
    let taken = act(&mut log, &d, removing(&[&a]));
    let state = log.state();
    assert!(state.tombstones.contains(&a.id()));
    assert!(state.members.contains_key(&b.id()));
    assert!(!host_access::for_agents(state, &voided));
    assert!(host_access::for_agents(state, &taken));
}

#[test]
fn a_members_key_version_follows_its_rotations_not_its_other_operations() {
    let (a, b) = (computer(1), computer(2));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    let intro = plain_host(40);
    let host = intro.id().unwrap();
    act(&mut log, &a, Body::HostAdd(intro));
    let version = |log: &Log| {
        host_access::compute(log, &host, [], |_| false)
            .unwrap()
            .members[&Id(b.id().0)]
            .keys
    };
    assert_eq!(version(&log), 0, "keys from its admission");
    let mut rotated = b.bundle.clone();
    rotated.tunnel = [0x62; 32];
    let rotation = act(&mut log, &b, Body::Rotate(rotated));
    assert_eq!(version(&log), rotation.unsigned().seq);
    act(&mut log, &b, access_body(host, &b, true, 1));
    act(&mut log, &b, access_body(host, &b, true, 2));
    assert_eq!(version(&log), rotation.unsigned().seq);
}

#[test]
fn a_rotation_on_a_shorter_branch_still_outranks_the_keys_it_replaced() {
    let (a, b) = (computer(1), computer(2));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    let intro = plain_host(40);
    let host = intro.id().unwrap();
    act(&mut log, &a, Body::HostAdd(intro));
    // b's key in two places: one branch signs on without rotating, the
    // other rotates once.
    let before = log.heads().to_vec();
    let seq = log.state().members[&b.id()].seq + 1;
    let mut rotated = b.bundle.clone();
    rotated.tunnel = [0x62; 32];
    let rotation = sign_on(&log, &b, &before, seq, Body::Rotate(rotated));
    for rev in 1..=5 {
        act(&mut log, &b, access_body(host, &b, true, rev));
    }
    let long = host_access::compute(&log, &host, [], |_| false).unwrap();
    apply(&mut log, rotation);
    let merged = host_access::compute(&log, &host, [], |_| false).unwrap();
    let (old, new) = (&long.members[&Id(b.id().0)], &merged.members[&Id(b.id().0)]);
    assert_eq!(new.tunnel, [0x62; 32]);
    assert!(new.keys > old.keys, "{} > {}", new.keys, old.keys);
    // The agent that held the long branch's view takes the rotation.
    let held = host_access::Access::merge(None, long).unwrap();
    let after = host_access::Access::merge(Some(&held), merged).unwrap();
    assert_eq!(after.members[&Id(b.id().0)].tunnel, [0x62; 32]);
}

#[test]
fn a_removal_suspending_several_members_counts_once_against_the_settled_bound() {
    let (a, b) = (computer(1), computer(2));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    let intro = plain_host(40);
    let host = intro.id().unwrap();
    act(&mut log, &a, Body::HostAdd(intro));
    let stranger = computer(77);
    // Eight removals, each reported for eight members, then one more:
    // signed by no member, so every one is void.
    let asked = |removal: u8| host_access::Asked {
        removal: OpId([removal; 32]),
        signer: stranger.id(),
        seq: 1,
        target: b.id(),
    };
    let mut reported: Vec<_> = (0..8).flat_map(|_| (0x10..0x18).map(asked)).collect();
    reported.push(asked(0xf0));
    let file = host_access::compute(&log, &host, reported, |_| false).unwrap();
    assert_eq!(file.settled.len(), 9);
    assert!(file.settled.contains(&Id([0xf0; 32])));
}

#[test]
fn a_session_proof_is_the_fleet_key_signing_the_logs_domain_and_the_hash() {
    use p256::ecdsa::signature::Verifier as _;
    let (log, b) = {
        let (a, b) = (computer(1), computer(2));
        let mut log = found(&a);
        admit(&mut log, &a, &b);
        (log, b)
    };
    let hash = [9; 32];
    let proof = crate::control::fleet::removal::prove(&b.key, &hash);
    // The agent's wire signs the same bytes the log's domain names.
    let message = [DOMAIN, b"session".as_slice(), &hash].concat();
    let signature = p256::ecdsa::Signature::from_slice(&proof).unwrap();
    assert!(b.key.verifying_key().verify(&message, &signature).is_ok());
    assert!(crate::control::fleet::removal::proves(
        log.state(),
        &b.id(),
        &hash,
        &proof
    ));
    assert!(!crate::control::fleet::removal::proves(
        log.state(),
        &b.id(),
        &[8; 32],
        &proof
    ));
}

#[test]
fn an_agents_access_file_names_each_members_fleet_key() {
    let (a, b) = (computer(1), computer(2));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    let intro = plain_host(40);
    let host = intro.id().unwrap();
    act(&mut log, &a, Body::HostAdd(intro));
    let file = host_access::compute(&log, &host, [], |_| false).unwrap();
    assert_eq!(file.members[&Id(b.id().0)].fleet_key, b.bundle.key);
}
