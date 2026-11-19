//! A relay route has one owner: a host's record naming another host's
//! route does not count, and flags it.

use super::*;

/// A record for `intro`'s host at `rev` on `route`, signed by its CA.
fn seen(intro: &HostIntro, ca: &SigningKey, rev: u64, route: &str, seed: u8) -> Body {
    let mut moved = record(intro.id().unwrap(), seed, rev);
    moved.route = route.into();
    let signature = Some(raw_signature(ca, &moved.message().unwrap()));
    Body::HostSeen(HostSeen {
        record: moved,
        signature,
    })
}

#[test]
fn a_host_claiming_another_hosts_route_does_not_count() {
    let (a, b) = (computer(1), computer(2));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    let (first, _) = ca_host(40, HostKind::Sheprd);
    let (second, second_ca) = ca_host(41, HostKind::Sheprd);
    act(&mut log, &a, Body::HostAdd(first.clone()));
    act(&mut log, &a, Body::HostAdd(second.clone()));
    let taken = first.record.route.clone();
    act(&mut log, &b, seen(&second, &second_ca, 2, &taken, 41));
    let state = log.state();
    let claimant = &state.hosts[&second.id().unwrap()];
    assert!(claimant.flagged);
    assert_eq!(claimant.record, second.record);
    let owner = &state.hosts[&first.id().unwrap()];
    assert!(!owner.flagged);
    assert_eq!(owner.record, first.record);
}

#[test]
fn a_host_keeps_its_own_route_as_its_record_changes() {
    let a = computer(1);
    let mut log = found(&a);
    let (intro, ca) = ca_host(40, HostKind::Sheprd);
    act(&mut log, &a, Body::HostAdd(intro.clone()));
    let own = intro.record.route.clone();
    // Its identity and addresses rotate; its route stays its own.
    act(&mut log, &a, seen(&intro, &ca, 2, &own, 60));
    let host = &log.state().hosts[&intro.id().unwrap()];
    assert!(!host.flagged);
    assert_eq!(host.record.rev, 2);
    assert_eq!(host.record.identity, "identity60");
}

#[test]
fn a_record_that_does_not_verify_flags_nothing() {
    let a = computer(1);
    let mut log = found(&a);
    let (first, _) = ca_host(40, HostKind::Sheprd);
    let (second, _) = ca_host(41, HostKind::Sheprd);
    act(&mut log, &a, Body::HostAdd(first.clone()));
    act(&mut log, &a, Body::HostAdd(second.clone()));
    // Signed by a key that is not the second host's CA.
    let forged = seen(&second, &signing_key(99), 2, &first.record.route, 41);
    act(&mut log, &a, forged);
    for intro in [&first, &second] {
        let host = &log.state().hosts[&intro.id().unwrap()];
        assert!(!host.flagged);
        assert!(!host.held);
        assert_eq!(host.record, intro.record);
    }
}

/// A Sheprd host whose intro names `route`.
fn on_route(seed: u8, route: &str) -> HostIntro {
    let (mut intro, ca) = ca_host(seed, HostKind::Sheprd);
    intro.record.route = route.into();
    intro.record_signature = Some(raw_signature(&ca, &intro.record.message().unwrap()));
    intro
}

#[test]
fn a_later_host_with_a_lower_id_cannot_take_a_route() {
    let a = computer(1);
    let mut log = found(&a);
    let (first, first_ca) = ca_host(40, HostKind::Sheprd);
    let route = first.record.route.clone();
    // A seed whose host sorts before the first.
    let later = (41..=255)
        .map(|seed| on_route(seed, &route))
        .find(|intro| intro.id().unwrap() < first.id().unwrap())
        .unwrap();
    act(&mut log, &a, Body::HostAdd(first.clone()));
    act(&mut log, &a, Body::HostAdd(later.clone()));
    let state = log.state();
    assert!(state.hosts[&later.id().unwrap()].held);
    assert!(!state.hosts[&first.id().unwrap()].held);
    // The first keeps its route as its record moves on.
    act(&mut log, &a, seen(&first, &first_ca, 2, &route, 60));
    let host = &log.state().hosts[&first.id().unwrap()];
    assert_eq!(host.record.rev, 2);
    assert!(!host.flagged);
    // And pruning gives it to the same host.
    let before = log.state().clone();
    let hash = before.hash().unwrap();
    let checkpoint = act(&mut log, &a, Body::Checkpoint(hash));
    log.prune(&checkpoint.id()).unwrap();
    act(&mut log, &a, Body::HostRemove(HostId([9; 32])));
    assert!(log.state().hosts[&later.id().unwrap()].held);
    assert!(!log.state().hosts[&first.id().unwrap()].held);
}

#[test]
fn a_route_first_named_in_a_record_cannot_be_taken_by_a_later_addition() {
    let a = computer(1);
    let mut log = found(&a);
    let (first, first_ca) = ca_host(40, HostKind::Sheprd);
    act(&mut log, &a, Body::HostAdd(first.clone()));
    act(&mut log, &a, seen(&first, &first_ca, 2, "fresh", 40));
    let later = on_route(41, "fresh");
    act(&mut log, &a, Body::HostAdd(later.clone()));
    let state = log.state();
    assert!(state.hosts[&later.id().unwrap()].held);
    assert!(!state.hosts[&first.id().unwrap()].held);
    assert_eq!(state.routes["fresh"], first.id().unwrap());
}

#[test]
fn pruned_and_whole_logs_agree_on_route_owners() {
    let (a, b) = (computer(1), computer(2));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    let (first, first_ca) = ca_host(40, HostKind::Sheprd);
    let route = first.record.route.clone();
    // A host added after the first, sorting before it.
    let (later, later_ca) = (41..=255)
        .map(|seed| ca_host(seed, HostKind::Sheprd))
        .find(|(intro, _)| intro.id().unwrap() < first.id().unwrap())
        .unwrap();
    act(&mut log, &a, Body::HostAdd(first.clone()));
    act(&mut log, &a, Body::HostAdd(later.clone()));
    // The first moves to a new route; the later one tries its first one.
    act(&mut log, &a, seen(&first, &first_ca, 2, "moved", 40));
    act(&mut log, &b, seen(&later, &later_ca, 2, &route, 41));
    let hash = log.state().hash().unwrap();
    let checkpoint = act(&mut log, &a, Body::Checkpoint(hash));
    let mut whole = copy(&log);
    log.prune(&checkpoint.id()).unwrap();
    assert_eq!(log.state(), whole.state());
    let after: [(&Device, Body); 3] = [
        (&b, seen(&later, &later_ca, 3, "moved", 41)),
        (&a, seen(&first, &first_ca, 3, &route, 40)),
        (&a, Body::HostAdd(on_route(60, "moved"))),
    ];
    for (by, body) in after {
        let op = act(&mut log, by, body);
        apply(&mut whole, op);
        assert_eq!(log.state(), whole.state());
    }
    // The first went back to its own route, which freed the one it had
    // moved to; the later host never owned either.
    let state = log.state();
    let newest = on_route(60, "moved").id().unwrap();
    assert_eq!(state.routes[&route], first.id().unwrap());
    assert_eq!(state.routes["moved"], newest);
    assert!(!state.hosts[&newest].held);
    assert!(state.hosts[&later.id().unwrap()].flagged);
}
