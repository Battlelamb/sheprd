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
