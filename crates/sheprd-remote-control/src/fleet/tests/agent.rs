//! A plain host's agent in the fleet: a removal reaches it from a member's
//! daemon with no Sheprd running, which suspends the target there, and
//! the next access file the log computes confirms it.

use super::*;
use crate::control::fleet::{
    HostId, HostIntro, HostKind, Reason, SelfRecord, Verified, access as host_access, sync,
};
use sheprd_relay_agent::{
    access::Id,
    agent::{Agent, Options, Relay},
    fleet::Held,
    status::Report,
};

/// An agent listening on the relay at `http`, and the intro a member that
/// read its status over SSH would add it with.
async fn plain_host(
    http: SocketAddr,
) -> (
    tempfile::TempDir,
    Arc<Agent>,
    watch::Sender<bool>,
    HostIntro,
) {
    let dir = tempfile::tempdir().unwrap();
    let relay = format!("http://{http}");
    let report = Report::read(dir.path(), std::time::SystemTime::now()).unwrap();
    let agent = Agent::load(Options {
        authorized_keys: Some(dir.path().join("authorized_keys")),
        forward: ssh_server().await,
        relays: vec![Relay {
            endpoint: Endpoint::parse(&relay).unwrap(),
            token: Vec::new(),
        }],
        state_dir: dir.path().to_owned(),
    })
    .unwrap();
    let (stop, stopped) = watch::channel(false);
    tokio::spawn(Arc::clone(&agent).run(stopped));
    let mut intro = HostIntro {
        kind: HostKind::Agent,
        name: "plain".to_owned(),
        user: "deploy".to_owned(),
        ca: None,
        record: SelfRecord {
            host: HostId([0; 32]),
            rev: 1,
            route: report.route,
            relays: vec![relay],
            identity: report.identity,
            direct: Vec::new(),
            ssh_host_keys: vec![format!(
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI{}",
                "P".repeat(43)
            )],
            signed_ms: now_ms(),
        },
        record_signature: None,
        verified: Verified::KnownHosts,
    };
    intro.record.host = intro.id().unwrap();
    (dir, agent, stop, intro)
}

/// What Sheprd would run over SSH: `fleet apply` with the file `node`'s
/// log computes, settling what the agent reported suspending.
fn fleet_apply(node: &Node, host: &HostId, dir: &Path, agent: &Agent) -> Held {
    let suspended: Vec<OpId> = Held::load(dir)
        .unwrap()
        .suspended
        .values()
        .flat_map(|suspension| suspension.removals.iter().map(|id| OpId(id.0)))
        .collect();
    let file = {
        let inner = node.fleet.inner();
        host_access::compute(inner.log.as_ref().unwrap(), host, suspended).unwrap()
    };
    let held = Held::apply(dir, Some(&dir.join("authorized_keys")), file).unwrap();
    agent.reload().unwrap();
    held
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removal_reaches_a_plain_hosts_agent_with_no_sheprd_running() {
    let (http, _, _) = relay().await;
    let (mut a, b, c) = (
        Node::new(http, 1).await,
        Node::new(http, 2).await,
        Node::new(http, 3).await,
    );
    a.ask(Ask::Found);
    join(&a, &b);
    join(&a, &c);
    let (dir, agent, _stop, intro) = plain_host(http).await;
    let host = intro.id().unwrap();
    a.ask(Ask::AddHost {
        intro: sync::text(&intro.to_bytes().unwrap()),
        deny: Vec::new(),
    });
    fleet_apply(&a, &host, dir.path(), &agent);
    let c_id = Id(c.fleet.me().0);
    assert!(Held::load(dir.path()).unwrap().admits(&c_id));

    // a removes c with Sheprd closed everywhere; a's daemon passes it to
    // the agent, which suspends c at once.
    a.start().await;
    a.ask(Ask::Remove {
        member: c.me(),
        also_members: Vec::new(),
        also_hosts: Vec::new(),
        reason: Reason::Compromised,
    });
    until(20, || {
        Held::load(dir.path())
            .unwrap()
            .suspended
            .contains_key(&c_id)
    })
    .await;
    until(10, || {
        a.fleet.inner().held.contains_key(&Holder::Host(host))
    })
    .await;

    // The next access file confirms it for good.
    let held = fleet_apply(&a, &host, dir.path(), &agent);
    assert!(held.suspended.is_empty());
    assert!(held.access.unwrap().removed.contains(&c_id));
    drop(b);
}
