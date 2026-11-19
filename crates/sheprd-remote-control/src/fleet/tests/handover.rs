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
    // It stays a host after a restart: how it joined is kept.
    let again = Fleet::load(host.dir.path(), now_ms()).unwrap();
    assert_eq!(again.host(), host.fleet.host());
    assert!(grants::still_allowed(&again, &a.fleet.me()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removed_member_never_acts_as_a_shared_host() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    join(&a, &b);
    // b joined as a member: it has a certificate authority, but is no
    // shared host.
    assert!(b.fleet.host.is_some());
    assert_eq!(b.fleet.host(), None);
    a.ask(Ask::Remove {
        member: b.me(),
        also_members: Vec::new(),
        also_hosts: Vec::new(),
        reason: crate::control::fleet::Reason::Lost,
    });
    b.fleet.receive(a.ops(), now_ms()).unwrap();
    let a_host = {
        let inner = a.fleet.inner();
        grants::own_host(inner.log.as_ref().unwrap().state(), &a.fleet.me(), None).unwrap()
    };
    let inner = b.fleet.inner();
    let state = inner.log.as_ref().unwrap().state();
    assert!(state.tombstones.contains(&b.fleet.me()));
    // Even handed a host the fleet holds, a removed member gets none.
    assert_eq!(grants::own_host(state, &b.fleet.me(), Some(a_host)), None);
    drop(inner);
    assert!(!grants::still_allowed(&b.fleet, &a.fleet.me()));
    grants::apply(&b.state, &b.fleet).unwrap();
    assert!(b.lines().is_empty(), "{:?}", b.lines());
    let again = Fleet::load(b.dir.path(), now_ms()).unwrap();
    assert_eq!(again.host(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_fleets_handing_over_at_once_keep_their_own_parts() {
    let (http, _, _) = relay().await;
    let (a, c, b) = (
        Node::new(http, 1).await,
        Node::new(http, 3).await,
        Node::new(http, 2).await,
    );
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    assert_eq!(c.ask(Ask::Found), Answer::Done);
    let (first, second) = (a.view().fleet, c.view().fleet);
    for (by, fleet) in [(&a, &first), (&c, &second)] {
        let Answer::Bundle { bundle, signature } = b.ask(Ask::Bundle {
            fleet: fleet.clone(),
        }) else {
            panic!("no bundle");
        };
        by.ask(Ask::Admit {
            bundle,
            signature,
            method: crate::control::fleet::Method::Ssh,
        });
    }
    let named = |fleet: &str, ops: &[Op], more| {
        b.ask(Ask::Apply(Apply {
            fleet: Some(fleet.to_owned()),
            ops: sync::encode_ops(ops),
            more,
            ..Apply::default()
        }))
    };
    let (a_ops, c_ops) = (a.ops(), c.ops());
    // Both first parts arrive, then a's last: c's parts never mix in.
    named(&first, &a_ops[..1], true);
    named(&second, &c_ops[..1], true);
    let Answer::Applied(applied) = named(&first, &a_ops[1..], false) else {
        panic!("not applied");
    };
    assert_eq!(applied.fleet, first);
    assert_eq!(applied.refused, 0);
}

#[test]
fn parts_of_one_fleet_kept_at_once_are_all_kept() {
    let key = SigningKey::from_slice(&[3; 32]).unwrap();
    let ops: Vec<Op> = (1..=32)
        .map(|seq| {
            Unsigned {
                fleet: crate::control::fleet::FleetId([1; 32]),
                signer: MemberId([2; 32]),
                seq,
                issued_ms: 1,
                parents: vec![OpId([5; 32])],
                body: Body::HostRemove(HostId([4; 32])),
            }
            .sign(&key)
            .unwrap()
        })
        .collect();
    let handovers = super::super::joining::Handovers::default();
    std::thread::scope(|scope| {
        for chunk in ops.chunks(4) {
            let handovers = &handovers;
            scope.spawn(move || {
                for op in chunk {
                    handovers.keep("f", std::slice::from_ref(op), 1).unwrap();
                    // The same part twice adds nothing.
                    handovers.keep("f", std::slice::from_ref(op), 1).unwrap();
                }
            });
        }
    });
    assert_eq!(handovers.take("f", 1).len(), ops.len());
    assert!(handovers.take("f", 1).is_empty());
}
