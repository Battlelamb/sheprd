//! The fleet on disk: kept across restarts, and taken over SSH when no
//! daemon runs.

use super::*;
use crate::control::fleet::sync::Apply;

#[tokio::test(flavor = "multi_thread")]
async fn the_key_and_the_log_survive_a_restart() {
    let (http, _, _) = relay().await;
    let a = Node::new(http, 1).await;
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    let heads = a.fleet.inner().log.as_ref().unwrap().heads().to_vec();
    let again = Fleet::load(a.dir.path(), now_ms()).unwrap();
    assert_eq!(again.me(), a.fleet.me());
    assert_eq!(
        again.inner().log.as_ref().unwrap().heads(),
        heads.as_slice()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fleet_apply_with_no_daemon_running_keeps_the_log() {
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
    // No daemon answers on b's control socket: the command writes the log
    // itself, which b's daemon reads when it starts.
    let apply = Apply {
        fleet: Some(fleet.clone()),
        heads: Vec::new(),
        ops: sync::encode_ops(&a.ops()),
    };
    let applied = run_apply(b.dir.path(), apply).unwrap();
    assert_eq!(applied.fleet, fleet);
    assert_eq!(applied.refused, 0);
    let reread = Fleet::load(b.dir.path(), now_ms()).unwrap();
    assert_eq!(
        reread.inner().log.as_ref().unwrap().fleet().to_string(),
        fleet
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_carries_what_the_caller_lacks() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    join(&a, &b);
    let caller = b.fleet.inner().log.as_ref().unwrap().heads().to_vec();
    // a moves on; b asks with its heads and gets only what is new.
    a.ask(Ask::Access {
        host: b
            .view()
            .hosts
            .iter()
            .find(|host| host.added_by == b.me())
            .unwrap()
            .id
            .clone(),
        member: a.me(),
        allow: false,
    });
    let Answer::Applied(applied) = a.ask(Ask::Apply(Apply {
        fleet: Some(a.view().fleet),
        heads: caller.iter().map(ToString::to_string).collect(),
        ops: Vec::new(),
    })) else {
        panic!("no answer");
    };
    assert_eq!(applied.ops.len(), 1);
    b.fleet
        .receive(sync::decode_ops(&applied.ops).unwrap(), now_ms())
        .unwrap();
    assert_eq!(b.view().hosts, a.view().hosts);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peers_file_that_does_not_read_is_an_error() {
    let (http, _, _) = relay().await;
    let a = Node::new(http, 1).await;
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    std::fs::write(
        a.dir.path().join("fleet/peers.json"),
        b"{\"unconfirmed\": [1]}",
    )
    .unwrap();
    assert!(matches!(
        Fleet::load(a.dir.path(), now_ms()),
        Err(Error::Peers)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peers_file_with_one_proven_key_per_member_still_reads() {
    let (http, _, _) = relay().await;
    let a = Node::new(http, 1).await;
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    let member = "11".repeat(32);
    let key = "5a".repeat(32);
    std::fs::write(
        a.dir.path().join("fleet/peers.json"),
        format!("{{\"proven\": {{\"{member}\": \"{key}\"}}}}"),
    )
    .unwrap();
    let again = Fleet::load(a.dir.path(), now_ms()).unwrap();
    assert_eq!(again.inner().proven[&MemberId([0x11; 32])].len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_past_the_bound_is_not_kept() {
    let (http, _, _) = relay().await;
    let a = Node::new(http, 1).await;
    let member = MemberId([1; 32]);
    for seed in 0..u8::try_from(MAX_PROVEN).unwrap() {
        assert!(a.fleet.proved(member, [seed; 32]).unwrap());
    }
    assert!(!a.fleet.proved(member, [0xee; 32]).unwrap());
    assert!(a.fleet.proved(member, [0; 32]).unwrap());
}
