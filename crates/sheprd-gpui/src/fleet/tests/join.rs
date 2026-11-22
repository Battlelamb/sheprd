use super::*;
use sheprd_remote_control::control::fleet::{Kind, MemberView};

fn member(id: &str) -> MemberView {
    MemberView {
        id: id.into(),
        name: id.into(),
        kind: Kind::Computer,
        admitted_by: None,
    }
}

fn machine(id: &str) -> (String, Machine) {
    (
        id.to_owned(),
        Machine {
            target: ConnectTarget::Ssh {
                target: id.to_owned(),
                session: "default".into(),
            },
            label: id.to_owned(),
        },
    )
}

#[test]
fn only_a_member_of_its_fleet_offers_it() {
    let mut view = fleet_view(&[]);
    view.members = vec![member("a"), member("b")];
    let offer = crate::fleet::join::Offer::of(&view).unwrap();
    assert_eq!(offer.fleet, view.fleet);
    assert_eq!((offer.members, offer.hosts), (2, 0));
    // A shared host holds the log but cannot admit anyone.
    view.me = "host".into();
    view.hosted = Some("host".into());
    assert_eq!(crate::fleet::join::Offer::of(&view), None);
}

#[test]
fn each_fleet_is_offered_once_and_only_to_a_computer_in_none() {
    let offer = |fleet: char| crate::fleet::join::Offer {
        fleet: fleet.to_string().repeat(64),
        members: 1,
        hosts: 0,
    };
    let mut fleet = Fleet::default();
    for (id, offered) in [
        ("laptop", Some(offer('a'))),
        ("studio", Some(offer('a'))),
        ("lab", Some(offer('b'))),
        ("box", None),
    ] {
        fleet.links.insert(
            id.to_owned(),
            Link {
                offer: offered,
                ..Link::default()
            },
        );
    }
    let reached: BTreeMap<_, _> = ["box", "lab", "laptop", "studio"].map(machine).into();
    let offered: Vec<_> = fleet
        .offered(reached.clone(), true)
        .into_iter()
        .map(|(id, _, offer)| (id, offer.fleet.chars().next()))
        .collect();
    assert_eq!(
        offered,
        vec![
            ("lab".to_owned(), Some('b')),
            ("laptop".to_owned(), Some('a'))
        ]
    );
    // Named by the command line, any machine offering one counts.
    assert_eq!(fleet.offered(reached.clone(), false).len(), 3);
    fleet.view = Some(fleet_view(&[]));
    assert!(fleet.offered(reached, false).is_empty());
}

#[test]
fn a_machine_in_another_fleet_is_asked_rarely() {
    let (elsewhere, now) = link(Standing::Elsewhere, Duration::from_secs(600), &[]);
    assert!(!elsewhere.due(&[], now));
    let (elsewhere, now) = link(Standing::Elsewhere, OUTSIDE, &[]);
    assert!(elsewhere.due(&[], now));
}
