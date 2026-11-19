//! A shared Sheprd host is no member and cannot sign its own record: a
//! member syncing it carries its newer, CA-signed record into the log.

use super::*;
use crate::control::fleet::sync::Apply;

/// `a`'s fleet shares `host`, which takes the log.
fn share(a: &Node, host: &Node) {
    let Answer::Intro { intro } = host.ask(Ask::Intro) else {
        panic!("no intro");
    };
    a.ask(Ask::AddHost {
        intro,
        deny: Vec::new(),
    });
    host.ask(Ask::Apply(Apply {
        ops: sync::encode_ops(&a.ops()),
        ..Apply::default()
    }));
}

fn record_keys(a: &Node, host: &Node) -> Vec<String> {
    let id = host.fleet.host().unwrap();
    a.fleet.inner().log.as_ref().unwrap().state().hosts[&id]
        .record
        .ssh_host_keys
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_carries_a_shared_hosts_moved_record() {
    let (http, _, _) = relay().await;
    let (a, host) = (Node::new(http, 1).await, Node::new(http, 5).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    share(&a, &host);
    let before = record_keys(&a, &host);
    // Unchanged: nothing is signed.
    let heads = a.view().heads;
    let Answer::Intro { intro } = host.ask(Ask::Intro) else {
        panic!("no intro");
    };
    assert_eq!(a.ask(Ask::Seen { intro }), Answer::Done);
    assert_eq!(a.view().heads, heads);
    // Its SSH host key moved.
    std::fs::write(
        host.dir.path().join("etc-ssh/ssh_host_ed25519_key.pub"),
        format!(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI{} root@node\n",
            "Z".repeat(43)
        ),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    let Answer::Intro { intro } = host.ask(Ask::Intro) else {
        panic!("no intro");
    };
    assert_eq!(a.ask(Ask::Seen { intro }), Answer::Done);
    let after = record_keys(&a, &host);
    assert_ne!(after, before);
    assert!(after[0].contains(&"Z".repeat(43)), "{after:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_members_record_is_never_carried_for_it() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    join(&a, &b);
    std::fs::write(
        b.dir.path().join("etc-ssh/ssh_host_ed25519_key.pub"),
        format!(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI{} root@node\n",
            "Y".repeat(43)
        ),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    let heads = a.view().heads;
    let Answer::Intro { intro } = b.ask(Ask::Intro) else {
        panic!("no intro");
    };
    // b signs its own record; a carries nothing for it.
    assert_eq!(a.ask(Ask::Seen { intro }), Answer::Done);
    assert_eq!(a.view().heads, heads);
}
