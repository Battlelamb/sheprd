//! A suspended member's key pairs are kept with its suspension and only
//! ever added to: a rotation never lets a desktop with an older pair back
//! in, and past the bound the agent refuses rather than forget one.

use super::*;

/// Members 1 and 2, member 2 on its `round`th rotated keys.
fn rotated(round: u8) -> Access {
    let mut access = file(&[(1, true, 0), (2, true, 0)], &[], &[]);
    if round > 0 {
        let member = access.members.get_mut(&id(2)).unwrap();
        member.dial = dial(100 + round);
        member.tunnel = [100 + round; 32];
        member.keys = u64::from(round);
    }
    access
}

/// A host where member 2, with a desktop enrolled on its first keys, is
/// suspended by member 1.
fn suspended() -> Host {
    let host = Host::new();
    host.enroll(2);
    host.apply(rotated(0)).unwrap();
    host.suspend(1, &[removal(0x51, &[2])]);
    assert_eq!(host.admitted().peer(&dial(2), &[2; 32]), None);
    host
}

#[test]
fn a_suspended_member_that_rotates_leaves_its_old_desktop_cut_off() {
    let host = suspended();
    // More rotations than the agent's own key history remembers.
    let rounds = u8::try_from(access::MAX_FORMER + 1).unwrap();
    for round in 1..=rounds {
        host.apply(rotated(round)).unwrap();
    }
    let held = Held::load(host.state()).unwrap();
    let kept = &held.suspended[&id(2)].pairs;
    let first = Pair {
        dial: dial(2),
        tunnel: [2; 32],
    };
    assert!(kept.contains(&first));
    assert!(
        !held.access.unwrap().members[&id(2)]
            .pairs()
            .any(|pair| pair == first)
    );
    let admitted = host.admitted();
    assert_eq!(admitted.peer(&dial(2), &[2; 32]), None);
    assert!(!admitted.keeps_ssh(&Peer::Desktop, &dial(2), &[2; 32]));
}

#[test]
fn past_the_bound_the_agent_refuses_rather_than_forget_a_pair() {
    let host = suspended();
    let rounds = u8::try_from(MAX_SUSPENDED_PAIRS).unwrap();
    let mut refused = None;
    for round in 1..=rounds {
        if let Err(error) = host.apply(rotated(round)) {
            refused = Some(error);
            break;
        }
    }
    assert!(matches!(refused, Some(Error::SuspendedKeys)), "{refused:?}");
    let held = Held::load(host.state()).unwrap();
    assert!(held.refused);
    assert!(held.suspended[&id(2)].pairs.len() <= MAX_SUSPENDED_PAIRS);
    assert!(host.lines().is_empty());
    assert_eq!(host.admitted().peer(&dial(2), &[2; 32]), None);
}
