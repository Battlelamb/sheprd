use super::*;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

mod scrub;

fn link(standing: Standing, ago: Duration, synced: &[&str]) -> (Link, Instant) {
    let now = Instant::now() + Duration::from_secs(7200);
    let link = Link {
        heads: Vec::new(),
        synced: synced.iter().map(|head| (*head).to_owned()).collect(),
        last: Some(now - ago),
        running: false,
        standing,
    };
    (link, now)
}

#[test]
fn a_member_syncs_when_this_computer_moved_or_now_and_then() {
    let mine = vec!["h2".to_owned()];
    let (fresh, now) = link(Standing::Member, Duration::from_secs(5), &["h2"]);
    assert!(!fresh.due(&mine, now));
    let (moved, now) = link(Standing::Member, Duration::from_secs(5), &["h1"]);
    assert!(moved.due(&mine, now));
    let (stale, now) = link(Standing::Member, REFRESH, &["h2"]);
    assert!(stale.due(&mine, now));
}

#[test]
fn machines_outside_the_fleet_are_asked_rarely() {
    let mine = vec!["h1".to_owned()];
    let (outside, now) = link(Standing::Outside, Duration::from_secs(600), &[]);
    assert!(!outside.due(&mine, now));
    let (outside, now) = link(Standing::Outside, OUTSIDE, &[]);
    assert!(outside.due(&mine, now));
    let (failed, now) = link(Standing::Failed, RETRY, &[]);
    assert!(failed.due(&mine, now));
    let (mut running, now) = link(Standing::Unknown, OUTSIDE, &[]);
    running.running = true;
    assert!(!running.due(&mine, now));
    assert!(Link::default().due(&mine, now));
}

#[test]
fn machines_show_their_target() {
    let ssh = ConnectTarget::Ssh {
        target: "me@build-01".into(),
        session: "default".into(),
    };
    assert_eq!(label("id", &ssh), "me@build-01");
    assert_eq!(label("id", &ConnectTarget::Local), "id");
}

#[test]
fn a_fleet_host_keeps_one_saved_device_id() {
    let host = "0123456789abcdef0123456789ABCDEF0123456789abcdef0123456789abcdef";
    assert_eq!(
        saved::device_id(host).as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(saved::device_id("short"), None);
}

#[test]
fn an_answer_reads_or_says_why_not() {
    let applied = remote::parse_applied(br#"{"fleet":"f","heads":["h"],"ops":[]}"#).unwrap();
    assert_eq!(applied.fleet, "f");
    assert!(matches!(
        remote::parse_applied(b"nope"),
        Err(Error::Answer(_))
    ));
}

#[test]
fn an_approval_at_pairing_is_used_once_and_expires() {
    use sheprd_remote_control::control::{DeviceId, fleet::Kind, fleet::sync::Join};
    let join = |device: &str| Join {
        member: device.into(),
        name: "phone".into(),
        kind: Kind::Phone,
        bundle: "b".into(),
        signature: "s".into(),
        device: "iPhone".into(),
        device_id: DeviceId(device.into()),
        received_ms: 1,
    };
    let now = Instant::now() + Duration::from_secs(7200);
    let mut expected = HashMap::from([
        (
            DeviceId("fresh".into()),
            ("f1".into(), now - Duration::from_secs(5)),
        ),
        (DeviceId("old".into()), ("f1".into(), now - joins::EXPECT)),
        // Given while this computer was in another fleet.
        (DeviceId("elsewhere".into()), ("f0".into(), now)),
    ]);
    let (admit, rest) = joins::settle(
        &mut expected,
        "f1",
        vec![join("fresh"), join("old"), join("elsewhere"), join("other")],
        now,
    );
    assert_eq!(admit, vec![join("fresh")]);
    assert_eq!(rest, vec![join("old"), join("elsewhere"), join("other")]);
    assert!(expected.is_empty());
}

fn fleet_view(heads: &[&str]) -> View {
    View {
        fleet: "f".repeat(64),
        me: "a".into(),
        members: Vec::new(),
        hosts: Vec::new(),
        removed: 0,
        equivocating: Vec::new(),
        suspended: Vec::new(),
        pending: 0,
        heads: heads.iter().map(|head| (*head).to_owned()).collect(),
    }
}

#[test]
fn an_older_read_never_replaces_a_newer_view() {
    let mut fleet = Fleet {
        reads: 2,
        asking: true,
        ..Fleet::default()
    };
    // The second read answered first; the first answers late.
    assert!(fleet.answered(2, Ok(Some(fleet_view(&["new"])))));
    assert!(!fleet.answered(1, Ok(Some(fleet_view(&["old"])))));
    assert!(!fleet.answered(1, Ok(None)));
    assert_eq!(fleet.view, Some(fleet_view(&["new"])));
    assert!(!fleet.asking);
}

#[test]
fn a_log_handed_over_asks_again_from_the_tips_of_what_it_sent() {
    let op = |id: &str, parents: &[&str]| {
        (
            id.to_owned(),
            parents.iter().map(|parent| (*parent).to_owned()).collect(),
        )
    };
    // A first part with the root and two concurrent children: both are
    // tips, the root is not.
    let first = flows::heads_after(&[], [op("r", &[]), op("x", &["r"]), op("y", &["r"])]);
    assert_eq!(first, vec!["x".to_owned(), "y".to_owned()]);
    // The next part merges them, and one more leaves a tip of the first.
    assert_eq!(
        flows::heads_after(&first, [op("m", &["x"])]),
        vec!["m".to_owned(), "y".to_owned()]
    );
    assert_eq!(flows::tips(&first, &[]).unwrap(), first);
    assert!(matches!(
        flows::tips(&[], &["not base64!".into()]),
        Err(Error::Log(_))
    ));
}

fn host(id: &str, target: &str) -> sheprd_client::SavedHost {
    sheprd_client::SavedHost {
        id: id.into(),
        label: id.into(),
        target: target.into(),
        session: "default".into(),
        enabled: true,
    }
}

#[test]
fn a_fleet_host_whose_route_moved_is_saved_again_not_forgotten() {
    let before = BTreeSet::from(["moved".to_owned(), "same".to_owned(), "gone".to_owned()]);
    let saved_now = [
        host("moved", "me@r-old.sheprd.invalid"),
        host("same", "me@r-same.sheprd.invalid"),
        host("gone", "me@r-gone.sheprd.invalid"),
        host("mine", "me@build-01"),
    ];
    let wanted = [
        host("moved", "me@r-new.sheprd.invalid"),
        host("same", "me@r-same.sheprd.invalid"),
        host("new", "me@r-fresh.sheprd.invalid"),
        // A person saved this id: it is left alone.
        host("mine", "me@r-other.sheprd.invalid"),
    ];
    let plan = saved::plan(&before, &saved_now, &wanted);
    assert_eq!(plan.forget, vec!["moved".to_owned(), "gone".to_owned()]);
    assert_eq!(
        plan.save,
        vec![
            host("moved", "me@r-new.sheprd.invalid"),
            host("new", "me@r-fresh.sheprd.invalid"),
        ]
    );
    assert_eq!(
        plan.kept,
        BTreeSet::from(["moved".to_owned(), "new".to_owned(), "same".to_owned()])
    );
}

#[test]
fn an_older_update_of_the_saved_devices_never_runs_after_a_newer_one() {
    let saving = saved::Saving::default();
    let ran = std::cell::RefCell::new(Vec::new());
    saving.run(2, || ran.borrow_mut().push(2));
    saving.run(1, || ran.borrow_mut().push(1));
    saving.run(3, || ran.borrow_mut().push(3));
    assert_eq!(*ran.borrow(), vec![2, 3]);
}
