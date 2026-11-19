//! A host's history starts at the addition that made it live: removed and
//! added again, it starts over, and a log pruned between the two decides
//! as the whole log does.

use super::*;

fn three() -> (Log, Device, Device, Device) {
    let (a, b, c) = (computer(1), computer(2), computer(3));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    admit(&mut log, &a, &c);
    (log, a, b, c)
}

/// Checkpoints `log` as `by`, and returns a pruned copy and the whole one.
fn split(log: &mut Log, by: &Device) -> (Log, Log) {
    let hash = log.state().hash().unwrap();
    let checkpoint = act(log, by, Body::Checkpoint(hash));
    let whole = copy(log);
    let mut pruned = copy(log);
    pruned.prune(&checkpoint.id()).unwrap();
    assert_eq!(pruned.state(), whole.state());
    (pruned, whole)
}

/// Signs `body` as `by` on `pruned` and applies it to both.
fn both(pruned: &mut Log, whole: &mut Log, by: &Device, body: Body) {
    let op = act(pruned, by, body);
    assert_eq!(whole.insert(op, NOW), Ok(Inserted::Applied));
    assert_eq!(pruned.state(), whole.state());
}

fn access(host: HostId, member: &Device, allow: bool) -> Body {
    Body::Access(Access {
        host,
        member: member.id(),
        allow,
        rev: 0,
    })
}

#[test]
fn a_host_added_again_starts_over_pruned_or_not() {
    let (mut log, a, b, c) = three();
    let host = plain_host(40).id().unwrap();
    act(&mut log, &a, Body::HostAdd(plain_host(40)));
    act(&mut log, &a, access(host, &c, false));
    assert!(!log.state().allowed(&host, &c.id()));
    act(&mut log, &b, Body::HostRemove(host));
    let (mut pruned, mut whole) = split(&mut log, &a);
    both(&mut pruned, &mut whole, &a, Body::HostAdd(plain_host(40)));
    assert!(pruned.state().allowed(&host, &c.id()));
    // Both take the same next rev, whichever drafted it.
    both(&mut pruned, &mut whole, &b, access(host, &c, false));
    let op = act(&mut whole, &a, access(host, &c, true));
    assert_eq!(pruned.insert(op, NOW), Ok(Inserted::Applied));
    assert_eq!(pruned.state(), whole.state());
    assert!(pruned.state().allowed(&host, &c.id()));
}

#[test]
fn removing_a_computers_own_host_stops_sharing_it() {
    let (mut log, a, b, c) = three();
    let host = c.bundle.host.as_ref().unwrap().id().unwrap();
    assert!(log.state().allowed(&host, &a.id()));
    act(&mut log, &b, Body::HostRemove(host));
    assert!(!log.state().hosts.contains_key(&host));
    assert!(!log.state().allowed(&host, &a.id()));
    assert!(log.state().members.contains_key(&c.id()));
    let (mut pruned, mut whole) = split(&mut log, &a);
    both(
        &mut pruned,
        &mut whole,
        &b,
        Body::HostRemove(HostId([9; 32])),
    );
    assert!(!pruned.state().hosts.contains_key(&host));
    // The computer naming it again, having seen the removal, brings it back.
    let mut rotated = c.bundle.clone();
    rotated.name = "back".into();
    both(&mut pruned, &mut whole, &c, Body::Rotate(rotated));
    assert!(pruned.state().allowed(&host, &a.id()));
}

#[test]
fn a_rotation_after_pruning_claims_its_new_route() {
    let (mut log, a, b, c) = three();
    let (intro, ca) = ca_host(3, HostKind::Member);
    assert_eq!(Some(&intro), c.bundle.host.as_ref());
    let host = intro.id().unwrap();
    let (mut pruned, mut whole) = split(&mut log, &a);
    let mut moved = intro.clone();
    moved.record = record(host, 3, 2);
    moved.record.route = "new-route".into();
    moved.record_signature = Some(raw_signature(&ca, &moved.record.message().unwrap()));
    let mut rotated = c.bundle.clone();
    rotated.host = Some(moved);
    both(&mut pruned, &mut whole, &c, Body::Rotate(rotated));
    assert_eq!(pruned.state().routes["new-route"], host);
    // Another host naming it is held on both.
    let (mut other, other_ca) = ca_host(41, HostKind::Sheprd);
    other.record.route = "new-route".into();
    other.record_signature = Some(raw_signature(&other_ca, &other.record.message().unwrap()));
    both(&mut pruned, &mut whole, &b, Body::HostAdd(other.clone()));
    assert!(pruned.state().hosts[&other.id().unwrap()].held);
}

#[test]
fn a_deny_on_either_concurrent_addition_counts() {
    let (log, a, b, c) = three();
    let host = plain_host(40).id().unwrap();
    let before = log.heads().to_vec();
    let next = |member: &Device| log.state().members[&member.id()].seq + 1;
    let by_a = sign_on(&log, &a, &before, next(&a), Body::HostAdd(plain_host(40)));
    let by_b = sign_on(&log, &b, &before, next(&b), Body::HostAdd(plain_host(40)));
    for (on, by) in [(&by_a, &a), (&by_b, &b)] {
        let mut log = copy(&log);
        apply(&mut log, by_a.clone());
        apply(&mut log, by_b.clone());
        // Signed having seen only one of the two additions.
        let rev = log.state().access_rev + 1;
        let deny = sign_on(
            &log,
            by,
            &[on.id()],
            next(by) + 1,
            Body::Access(Access {
                host,
                member: c.id(),
                allow: false,
                rev,
            }),
        );
        apply(&mut log, deny);
        assert!(!log.state().allowed(&host, &c.id()));
    }
}

#[test]
fn a_rotation_keeps_the_route_decisions_before_it() {
    let (mut log, a, b, c) = three();
    let own = c.bundle.host.as_ref().unwrap();
    // Another host on c's route, added after c named it: held.
    let (mut other, other_ca) = ca_host(41, HostKind::Sheprd);
    other.record.route = own.record.route.clone();
    other.record_signature = Some(raw_signature(&other_ca, &other.record.message().unwrap()));
    act(&mut log, &b, Body::HostAdd(other.clone()));
    let other_id = other.id().unwrap();
    let own_id = own.id().unwrap();
    assert!(log.state().hosts[&other_id].held);
    assert!(!log.state().hosts[&own_id].held);
    let (mut pruned, mut whole) = split(&mut log, &a);
    let mut rotated = c.bundle.clone();
    rotated.name = "renamed".into();
    both(&mut pruned, &mut whole, &c, Body::Rotate(rotated));
    let state = pruned.state();
    assert!(state.hosts[&other_id].held);
    assert!(!state.hosts[&own_id].held);
    assert_eq!(state.routes[&own.record.route], own_id);
}

#[test]
fn an_equivocation_after_a_checkpoint_holds_the_same_in_pruned_and_whole_logs() {
    let (mut log, a, _, c) = three();
    act(&mut log, &c, Body::HostAdd(plain_host(40)));
    let (mut pruned, mut whole) = split(&mut log, &a);
    both(&mut pruned, &mut whole, &c, Body::HostAdd(plain_host(41)));
    // c's key signs in two places.
    let heads = pruned.heads().to_vec();
    let seq = pruned.state().members[&c.id()].seq + 1;
    for n in [1, 2] {
        let op = sign_on(&pruned, &c, &heads, seq, Body::HostRemove(HostId([n; 32])));
        assert_eq!(pruned.insert(op.clone(), NOW), Ok(Inserted::Applied));
        assert_eq!(whole.insert(op, NOW), Ok(Inserted::Applied));
    }
    assert_eq!(pruned.state(), whole.state());
    let state = pruned.state();
    assert!(state.equivocating.contains(&c.id()));
    // What the checkpoint saw stands; what it had not is held.
    assert!(state.hosts.contains_key(&plain_host(40).id().unwrap()));
    assert!(!state.hosts.contains_key(&plain_host(41).id().unwrap()));
}

#[test]
fn an_equivocators_own_checkpoint_settles_nothing() {
    let (mut log, _, _, c) = three();
    act(&mut log, &c, Body::HostAdd(plain_host(40)));
    let hash = log.state().hash().unwrap();
    act(&mut log, &c, Body::Checkpoint(hash));
    let heads = log.heads().to_vec();
    let seq = log.state().members[&c.id()].seq + 1;
    for n in [1, 2] {
        let op = sign_on(&log, &c, &heads, seq, Body::HostRemove(HostId([n; 32])));
        apply(&mut log, op);
    }
    assert!(log.state().equivocating.contains(&c.id()));
    assert!(
        !log.state()
            .hosts
            .contains_key(&plain_host(40).id().unwrap())
    );
}
