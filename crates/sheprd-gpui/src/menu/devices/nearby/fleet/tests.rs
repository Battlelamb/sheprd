use super::Offer;
use sheprd_remote_control::control::fleet::{Kind, MemberView, View, bluetooth::Hint};

const OURS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const THEIRS: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn view() -> View {
    View {
        fleet: OURS.into(),
        me: "me".into(),
        members: vec![MemberView {
            id: "me".into(),
            name: "laptop".into(),
            kind: Kind::Computer,
            admitted_by: None,
        }],
        hosts: Vec::new(),
        removed: 0,
        equivocating: Vec::new(),
        suspended: Vec::new(),
        pending: 0,
        heads: Vec::new(),
        hosted: None,
    }
}

fn hint(fleet: &str) -> Hint {
    Hint {
        fleet: fleet.into(),
        computers: 1,
        hosts: 0,
    }
}

#[test]
fn the_offer_follows_both_fleets() {
    let ours = view();
    assert_eq!(Offer::of(Some(&ours), true, Some(&hint(OURS))), Offer::Same);
    assert_eq!(
        Offer::of(Some(&ours), true, Some(&hint(THEIRS))),
        Offer::Other(hint(OURS))
    );
    assert_eq!(
        Offer::of(Some(&ours), true, None),
        Offer::Invite(hint(OURS))
    );
    assert_eq!(
        Offer::of(None, false, Some(&hint(THEIRS))),
        Offer::Join(hint(THEIRS))
    );
    assert_eq!(Offer::of(None, false, None), Offer::Found);
    // A fleet held as a shared host, or after a removal, offers nothing.
    assert_eq!(
        Offer::of(Some(&ours), false, Some(&hint(THEIRS))),
        Offer::None
    );
    assert_eq!(Offer::of(Some(&ours), false, None), Offer::None);
}

#[test]
fn only_a_choice_holds_the_claim_back() {
    assert!(Offer::Invite(hint(OURS)).asks());
    assert!(Offer::Join(hint(THEIRS)).asks());
    assert!(Offer::Found.asks());
    assert!(!Offer::Same.asks());
    assert!(!Offer::Other(hint(OURS)).asks());
    assert!(!Offer::None.asks());
}
