//! Handing the log to a computer with no fleet: in parts, as a member's
//! `fleet apply` does when the log is larger than one answer, and to a
//! Sheprd host the fleet shares, which is no member.

use super::*;
use crate::control::fleet::sync::{Applied, Apply};

fn apply(node: &Node, ops: &[Op], more: bool) -> Answer {
    node.ask(Ask::Apply(Apply {
        ops: sync::encode_ops(ops),
        more,
        ..Apply::default()
    }))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_log_handed_over_in_parts_joins_with_the_last() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    let fleet = a.view().fleet;
    let Answer::Bundle { bundle, signature } = b.ask(Ask::Bundle {
        fleet: fleet.clone(),
    }) else {
        panic!("no bundle");
    };
    a.ask(Ask::Admit {
        bundle,
        signature,
        method: crate::control::fleet::Method::Ssh,
    });
    let ops = a.ops();
    // The first part does not admit b: it is kept, not refused.
    let (first, rest) = ops.split_at(1);
    assert_eq!(apply(&b, first, true), Answer::Applied(Applied::default()));
    assert!(b.fleet.inner().log.is_none());
    let Answer::Applied(applied) = apply(&b, rest, false) else {
        panic!("not applied");
    };
    assert_eq!(applied.fleet, fleet);
    assert!(b.view().members.iter().any(|member| member.id == b.me()));
}

#[tokio::test(flavor = "multi_thread")]
async fn parts_that_do_not_admit_this_computer_never_join() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    // a's log, which does not admit b, as a part, then an empty last
    // part: b stays in no fleet.
    apply(&b, &a.ops(), true);
    assert!(b.fleet.inner().log.is_none());
    assert!(matches!(
        ask::answer(&b.state, &b.fleet, Ask::Apply(Apply::default()), now_ms()),
        Err(Error::NotAdmitted)
    ));
    assert!(b.fleet.inner().log.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_sheprd_host_takes_the_log_and_lets_allowed_members_in() {
    let (http, _, _) = relay().await;
    let (a, host) = (Node::new(http, 1).await, Node::new(http, 5).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    // Before it is shared, the host refuses the log.
    assert!(matches!(
        host.fleet.receive(a.ops(), now_ms()),
        Err(Error::NotAdmitted)
    ));
    let Answer::Intro { intro } = host.ask(Ask::Intro) else {
        panic!("no intro");
    };
    assert_eq!(
        a.ask(Ask::AddHost {
            intro,
            deny: Vec::new(),
        }),
        Answer::Done
    );
    let Answer::Applied(applied) = apply(&host, &a.ops(), false) else {
        panic!("not applied");
    };
    assert_eq!(applied.fleet, a.view().fleet);
    // It holds the log as a host, not as a member, and lets a in.
    let view = host.view();
    assert!(!view.members.iter().any(|member| member.id == host.me()));
    assert!(
        view.hosts
            .iter()
            .any(|shared| Some(shared.id.clone()) == host.fleet.host().map(|id| id.to_string()))
    );
    assert!(grants::still_allowed(&host.fleet, &a.fleet.me()));
    grants::apply(&host.state, &host.fleet).unwrap();
    assert_eq!(host.lines().len(), 1, "{:?}", host.lines());
}
