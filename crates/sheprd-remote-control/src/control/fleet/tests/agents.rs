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
