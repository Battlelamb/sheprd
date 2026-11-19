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
        nonce: None,
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
        nonce: None,
    })) else {
        panic!("no answer");
    };
    assert_eq!(applied.ops.len(), 1);
    b.fleet
        .receive(sync::decode_ops(&applied.ops).unwrap(), now_ms())
        .unwrap();
    // The same hosts and grants; each computer's own `target` differs.
    let shared = |node: &Node| {
        node.view()
            .hosts
            .into_iter()
            .map(|host| (host.id, host.grants))
            .collect::<Vec<_>>()
    };
    assert_eq!(shared(&b), shared(&a));
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
async fn apply_naming_another_fleet_learns_nothing() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    assert_eq!(b.ask(Ask::Found), Answer::Done);
    let apply = Apply {
        fleet: Some(b.view().fleet),
        heads: Vec::new(),
        ops: Vec::new(),
        nonce: None,
    };
    assert!(matches!(
        ask::answer(&a.state, &a.fleet, Ask::Apply(apply), now_ms()),
        Err(Error::OtherFleet)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_proves_which_host_gave_it() {
    let (http, _, _) = relay().await;
    let a = Node::new(http, 1).await;
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    let nonce = "n0nce";
    let Answer::Applied(applied) = a.ask(Ask::Apply(Apply {
        fleet: None,
        heads: Vec::new(),
        ops: Vec::new(),
        nonce: Some(nonce.into()),
    })) else {
        panic!("no answer");
    };
    let hosts: Vec<String> = a.view().hosts.into_iter().map(|host| host.id).collect();
    assert!(sync::proven(&applied, nonce, &hosts));
    // Another nonce, other heads, or a host the fleet does not hold: no.
    assert!(!sync::proven(&applied, "other", &hosts));
    let mut moved = applied.clone();
    moved.heads.push("00".repeat(32));
    assert!(!sync::proven(&moved, nonce, &hosts));
    assert!(!sync::proven(&applied, nonce, &["11".repeat(32)]));
    // A machine that only echoes the fleet id, with no proof.
    let echoed = sync::Applied {
        proof: None,
        ..applied
    };
    assert!(!sync::proven(&echoed, nonce, &hosts));
}

#[tokio::test(flavor = "multi_thread")]
async fn scrubbing_forgets_the_fleet_and_what_it_wrote() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    join(&a, &b);
    grants::apply(&b.state, &b.fleet).unwrap();
    assert_eq!(b.lines().len(), 1);
    let records = b.dir.path().join("desktop/hosts");
    assert_eq!(records.read_dir().unwrap().count(), 1);
    assert_eq!(b.ask(Ask::Scrub), Answer::Done);
    grants::apply(&b.state, &b.fleet).unwrap();
    assert!(b.lines().is_empty());
    assert_eq!(records.read_dir().unwrap().count(), 0);
    assert!(ask::view(&b.fleet).is_none());
    // The same key may found or join another fleet after.
    assert!(!b.dir.path().join("fleet/log.bin").exists());
    assert_eq!(
        Fleet::load(b.dir.path(), now_ms()).unwrap().me(),
        b.fleet.me()
    );
}
