//! Leaving a fleet: the log stays until the removal was passed on, then
//! this computer may found or join another.

use super::*;
use crate::control::fleet::{Reason, sync::Ask};

#[tokio::test(flavor = "multi_thread")]
async fn a_computer_that_left_may_found_another_fleet_once_its_removal_is_passed_on() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    join(&a, &b);
    let old = a.view().fleet;
    assert_eq!(
        b.ask(Ask::Remove {
            member: b.me(),
            also_members: Vec::new(),
            also_hosts: Vec::new(),
            reason: Reason::Left,
        }),
        Answer::Done
    );
    // Until a holds the removal, b keeps the log to pass it on.
    assert!(b.fleet.in_fleet().unwrap());
    assert!(matches!(
        ask::answer(&b.state, &b.fleet, Ask::Found, now_ms()),
        Err(Error::InFleet)
    ));
    let removal: Vec<OpId> = b
        .ops()
        .iter()
        .filter(|op| matches!(op.body(), Body::Remove(_)))
        .map(Op::id)
        .collect();
    b.fleet.held(Holder::Member(a.fleet.me()), removal).unwrap();
    // Then the fleet is gone here, on disk too, and b founds another.
    assert!(ask::view(&b.fleet).is_none());
    assert!(
        Fleet::load(b.dir.path(), now_ms())
            .unwrap()
            .inner()
            .log
            .is_none()
    );
    assert_eq!(b.ask(Ask::Found), Answer::Done);
    assert_ne!(b.view().fleet, old);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_still_in_the_fleet_is_never_released() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    join(&a, &b);
    b.fleet
        .held(Holder::Member(a.fleet.me()), a.ops().iter().map(Op::id))
        .unwrap();
    assert!(b.fleet.in_fleet().unwrap());
    assert!(ask::view(&b.fleet).is_some());
}
