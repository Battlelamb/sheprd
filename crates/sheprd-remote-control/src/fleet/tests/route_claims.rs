//! A host that moved to a route keeps it, and the keys saved for it, when
//! another host introduces itself on that route afterwards, or when a
//! claim the log orders first arrives after the keys were saved.

use super::*;
use crate::control::fleet::{HostKind, HostSeen};
use crate::desktop::record::Record;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

/// The record `node` keeps for `route`.
fn kept(node: &Node, route: &str) -> Record {
    let probe = Record {
        computer: String::new(),
        identity: String::new(),
        relays: Vec::new(),
        route: route.to_owned(),
        fleet: true,
        host: None,
    };
    Record::load(node.dir.path(), &probe.id().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_introduction_cannot_take_a_route_a_host_moved_to() {
    let (a, b, c) = three().await;
    let route = URL_SAFE_NO_PAD.encode([0x5a; 32]);
    // a moves to `route`, its record signed by its own authority.
    let (mut moved, _) = me::record(&a.state, now_ms() + 1).unwrap();
    moved.route = route.clone();
    let authority = a.state.authority.as_ref().unwrap();
    let signature = authority.sign(&moved.message().unwrap()).unwrap();
    a.fleet
        .sign(
            Body::HostSeen(HostSeen {
                record: moved,
                signature: Some(signature),
            }),
            now_ms(),
        )
        .unwrap();
    b.fleet.receive(a.ops(), now_ms()).unwrap();
    grants::apply(&b.state, &b.fleet).unwrap();
    let before = kept(&b, &route);
    let a_host =
        grants::own_host(b.fleet.inner().log.as_ref().unwrap().state(), &a.fleet.me()).unwrap();
    assert_eq!(before.host, Some(a_host.to_string()));
    // c then adds a host of its own whose introduction names that route.
    c.fleet.receive(a.ops(), now_ms()).unwrap();
    let other = Node::new("127.0.0.1:9".parse().unwrap(), 9).await;
    let mut intro = me::intro(&other.state, now_ms()).unwrap();
    intro.kind = HostKind::Sheprd;
    intro.record.route = route.clone();
    let authority = other.state.authority.as_ref().unwrap();
    intro.record_signature = Some(authority.sign(&intro.record.message().unwrap()).unwrap());
    c.fleet.sign(Body::HostAdd(intro), now_ms()).unwrap();
    b.fleet.receive(c.ops(), now_ms()).unwrap();
    grants::apply(&b.state, &b.fleet).unwrap();
    assert_eq!(kept(&b, &route), before);
}

#[test]
fn keys_saved_for_a_host_that_still_names_the_route_stay() {
    let dir = tempfile::tempdir().unwrap();
    let route = URL_SAFE_NO_PAD.encode([7; 32]);
    let record = |host: &str, identity: &str| Record {
        computer: "Studio".into(),
        identity: identity.into(),
        relays: vec!["https://relay.example.org".into()],
        route: route.clone(),
        fleet: true,
        host: Some(host.into()),
    };
    let keys = vec!["ssh-ed25519 AAAA".to_owned()];
    let id = record("a", "identity-a").id().unwrap();
    let routes = grants::Routes {
        named: BTreeSet::from([("a".to_owned(), route.clone())]),
        ..Default::default()
    };
    grants::write_records(
        dir.path(),
        &[(record("a", "identity-a"), keys.clone())],
        &routes,
    )
    .unwrap();
    // The log gives the route to c, while a still names it: a's keys stay.
    grants::write_records(
        dir.path(),
        &[(record("c", "identity-c"), keys.clone())],
        &routes,
    )
    .unwrap();
    assert_eq!(
        Record::load(dir.path(), &id).unwrap(),
        record("a", "identity-a")
    );
    // Once a names it no longer, the owner's record replaces it.
    grants::write_records(
        dir.path(),
        &[(record("c", "identity-c"), keys)],
        &grants::Routes::default(),
    )
    .unwrap();
    assert_eq!(
        Record::load(dir.path(), &id).unwrap(),
        record("c", "identity-c")
    );
}
