//! Joining: both directions open, and a log that does not admit this
//! computer is refused.

use super::*;

fn host_files(node: &Node) -> usize {
    std::fs::read_dir(node.dir.path().join("desktop/hosts"))
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn every_member_reaches_every_other() {
    let (a, b, c) = three().await;
    for node in [&a, &b, &c] {
        until(10, || node.lines().len() == 2 && host_files(node) == 2).await;
        let view = node.view();
        assert_eq!(view.members.len(), 3);
        assert_eq!(view.hosts.len(), 3);
        // Each line is restricted to loopback, with the fleet's tag.
        for line in node.lines() {
            assert!(
                line.starts_with(
                    "from=\"127.0.0.1,::1\",no-agent-forwarding,no-X11-forwarding ssh-ed25519 "
                ),
                "{line}"
            );
            assert!(
                line.contains(&format!("sheprd-fleet:{}:", &view.fleet[..16])),
                "{line}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_log_that_does_not_admit_this_computer_is_refused() {
    let (http, _, _) = relay().await;
    let (a, stranger) = (Node::new(http, 1).await, Node::new(http, 4).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    assert!(matches!(
        stranger.fleet.receive(a.ops(), now_ms()),
        Err(Error::NotAdmitted)
    ));
    assert!(stranger.fleet.inner().log.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_is_in_one_fleet() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    assert_eq!(b.ask(Ask::Found), Answer::Done);
    let fleet = a.view().fleet;
    assert!(matches!(
        ask::answer(&b.state, &b.fleet, Ask::Bundle { fleet }, now_ms()),
        Err(Error::InFleet)
    ));
    assert!(matches!(
        ask::answer(&a.state, &a.fleet, Ask::Found, now_ms()),
        Err(Error::InFleet)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn joining_supersedes_a_pairing_here_so_removal_takes_every_way_in() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let (http, _, _) = relay().await;
    let (mut a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    // b was paired with a over Bluetooth before the fleet: a desktop
    // device on a with b's own tunnel and SSH keys.
    let keys = crate::desktop::keys::Keys::load_or_create(b.dir.path()).unwrap();
    let ssh = crate::desktop::keys::ssh_public_key(b.dir.path()).unwrap();
    let phone_key = SigningKey::from_slice(&[9; 32]).unwrap();
    let point = phone_key.verifying_key().to_sec1_point(false);
    let legacy = crate::control::DeviceId("legacy".into());
    a.state
        .auth()
        .devices_mut()
        .add(
            crate::auth::NewDevice {
                app: None,
                id: legacy.clone(),
                name: "b over Bluetooth".into(),
                public_key: crate::auth::PublicKey::parse(
                    &URL_SAFE_NO_PAD.encode(point.as_bytes()),
                )
                .unwrap(),
                relay: Some(crate::control::host_relay::Keys {
                    dial: URL_SAFE_NO_PAD
                        .encode(keys.dial.verifying_key().to_sec1_point(true).as_bytes()),
                    tunnel: URL_SAFE_NO_PAD.encode(keys.tunnel.identity().x25519()),
                }),
                ssh: Some(crate::ssh_access::SshKey::parse(&ssh).unwrap()),
            },
            now_ms(),
        )
        .unwrap();
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    join(&a, &b);
    a.start().await;
    // The fleet lets b in now; the pairing beside it goes, so removing b
    // from the fleet leaves it no way in.
    until(10, || a.state.auth().devices().get(&legacy).is_none()).await;
    assert_eq!(a.lines().len(), 1);
}
