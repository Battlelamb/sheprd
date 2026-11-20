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
fn past_the_bound_only_that_member_is_held_to_the_keys_it_had() {
    let host = suspended();
    let rounds = u8::try_from(MAX_SUSPENDED_PAIRS + 2).unwrap();
    for round in 1..=rounds {
        host.apply(rotated(round)).unwrap();
    }
    let held = Held::load(host.state()).unwrap();
    assert!(!held.refused);
    let suspension = &held.suspended[&id(2)];
    assert!(suspension.overflow, "only its removal ends it now");
    assert!(suspension.pairs.len() <= MAX_SUSPENDED_PAIRS);
    // Its rotation past the bound was refused: the file keeps the keys it
    // had, every one of them kept and cut off.
    let member = held.access.as_ref().unwrap().members[&id(2)].clone();
    assert!(member.pairs().all(|pair| suspension.pairs.contains(&pair)));
    let admitted = host.admitted();
    for pair in suspension.pairs.iter().chain(
        [Pair {
            dial: dial(100 + rounds),
            tunnel: [100 + rounds; 32],
        }]
        .iter(),
    ) {
        assert_eq!(admitted.peer(&pair.dial, &pair.tunnel), None);
    }
    assert!(!admitted.admits_member(&id(2)));
    // A settling file does not lift it.
    let mut settling = rotated(rounds);
    settling.settled.insert(id(0x51));
    let held = host.apply(settling).unwrap();
    assert!(held.suspended.contains_key(&id(2)));
    // Member 1 is untouched.
    assert!(held.reaches_ssh(&id(1)));
    assert_eq!(host.lines(), [id(1).short()]);
    assert!(host.admitted().admits_member(&id(1)));
}
