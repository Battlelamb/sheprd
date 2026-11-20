//! The bounds on computers and hosts leave room for what they should: a
//! seat a removal freed, and a route only a host within the bound holds.

use super::*;

#[test]
fn a_replacement_takes_the_seat_a_removal_freed_pruned_or_not() {
    let a = computer(1);
    let mut log = found(&a);
    for seed in 2..=u8::try_from(MAX_COMPUTERS).unwrap() {
        admit(&mut log, &a, &computer(seed));
    }
    let hash = log.state().hash().unwrap();
    let checkpoint = act(&mut log, &a, Body::Checkpoint(hash));
    let mut whole = copy(&log);
    let mut pruned = copy(&log);
    pruned.prune(&checkpoint.id()).unwrap();
    let replacement = computer(200);
    for log in [&mut pruned, &mut whole] {
        act(log, &a, remove_body(&computer(2)));
        admit(log, &a, &replacement);
        assert!(log.state().members.contains_key(&replacement.id()));
        // And it signs.
        act(log, &replacement, Body::HostRemove(HostId([9; 32])));
    }
    assert_eq!(pruned.state(), whole.state());
}

#[test]
fn a_host_the_bound_left_out_holds_no_route() {
    let (a, c) = (computer(1), computer(3));
    let mut log = found(&a);
    admit(&mut log, &a, &c);
    // Fill the hosts in one batch.
    let mut parents = log.heads().to_vec();
    let first = log.state().members[&a.id()].seq + 1;
    let mut ops = Vec::new();
    for (seq, seed) in (first..).zip(0..=u8::MAX) {
        let op = Unsigned {
            fleet: log.fleet(),
            signer: a.id(),
            seq,
            issued_ms: NOW,
            parents: parents.clone(),
            body: Body::HostAdd(plain_host(seed)),
        }
        .sign(&a.key)
        .unwrap();
        parents = vec![op.id()];
        ops.push(op);
    }
    for (_, result) in log.insert_all(ops, NOW) {
        assert_eq!(result, Ok(Inserted::Applied));
    }
    assert_eq!(log.state().hosts.len(), MAX_HOSTS + 2);
    // One more host, on a free route: left out by the bound.
    let (mut extra, extra_ca) = ca_host(60, HostKind::Sheprd);
    extra.record.route = "free-route".into();
    extra.record_signature = Some(raw_signature(&extra_ca, &extra.record.message().unwrap()));
    act(&mut log, &a, Body::HostAdd(extra.clone()));
    assert!(!log.state().hosts.contains_key(&extra.id().unwrap()));
    // c moves its own host onto that route: it is c's.
    let (mut own, ca) = ca_host(3, HostKind::Member);
    let host = own.id().unwrap();
    own.record = record(host, 3, 2);
    own.record.route = "free-route".into();
    own.record_signature = Some(raw_signature(&ca, &own.record.message().unwrap()));
    let mut moved = c.bundle.clone();
    moved.host = Some(own);
    act(&mut log, &c, Body::Rotate(moved));
    let state = log.state();
    assert!(!state.hosts[&host].held);
    assert_eq!(state.routes["free-route"], host);
    assert!(state.allowed(&host, &a.id()));
}
