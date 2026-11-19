#![allow(clippy::unwrap_used)]

use super::*;
use crate::{
    access::tests::{dial, id, member},
    authorized::Entry,
};

const FLEET: u8 = 0xf1;

fn file(members: &[(u8, bool, u64)], removed: &[u8], settled: &[u8]) -> Access {
    Access {
        fleet: id(FLEET),
        members: members
            .iter()
            .map(|&(seed, allow, rev)| (id(seed), member(seed, allow, rev)))
            .collect(),
        removed: removed.iter().map(|&seed| id(seed)).collect(),
        settled: settled.iter().map(|&seed| id(seed)).collect(),
    }
}

fn removal(op: u8, targets: &[u8]) -> Removed {
    Removed {
        id: [op; 32],
        fleet: [FLEET; 32],
        signer: [0x99; 32],
        seq: 9,
        targets: targets.iter().map(|&seed| [seed; 32]).collect(),
    }
}

struct Host {
    dir: tempfile::TempDir,
}

impl Host {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn state(&self) -> &Path {
        self.dir.path()
    }

    fn keys(&self) -> std::path::PathBuf {
        self.dir.path().join("authorized_keys")
    }

    fn apply(&self, access: Access) -> Result<Held> {
        Held::apply(self.state(), Some(&self.keys()), access)
    }

    /// `sender` passes `removals`, each signed by itself.
    fn suspend(&self, sender: u8, removals: &[Removed]) -> Vec<[u8; 32]> {
        let signed: Vec<Removed> = removals
            .iter()
            .cloned()
            .map(|removal| Removed {
                signer: [sender; 32],
                ..removal
            })
            .collect();
        self.pass(sender, &signed)
    }

    /// `sender` passes `removals` as they are.
    fn pass(&self, sender: u8, removals: &[Removed]) -> Vec<[u8; 32]> {
        Held::suspend(self.state(), Some(&self.keys()), id(sender), removals).unwrap()
    }

    /// Enrolls the desktop with member `seed`'s keys one by one.
    fn enroll(&self, seed: u8) {
        let keys = member(seed, true, 0);
        let mut desktops = Authorized::load(self.state()).unwrap();
        desktops
            .insert(Entry {
                dial: keys.dial,
                tunnel: keys.tunnel,
                label: "laptop".to_owned(),
            })
            .unwrap();
        desktops.save(self.state()).unwrap();
    }

    fn admitted(&self) -> Admitted {
        Admitted::load(self.state()).unwrap()
    }

    /// The members whose `authorized_keys` line is there, by short id.
    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.keys())
            .unwrap_or_default()
            .lines()
            .filter_map(authorized_keys::fleet_tag)
            .map(|tag| tag.split(':').nth(1).unwrap().to_owned())
            .collect()
    }
}

#[test]
fn applying_admits_every_member_and_lets_only_the_allowed_reach_ssh() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, false, 1)], &[], &[]))
        .unwrap();
    let admitted = host.admitted();
    assert_eq!(
        admitted.peer(&dial(1), &[1; 32]),
        Some(Peer::Member {
            id: id(1),
            ssh: true,
            desktop: false
        })
    );
    assert!(matches!(
        admitted.peer(&dial(2), &[2; 32]),
        Some(Peer::Member { ssh: false, .. })
    ));
    assert_eq!(admitted.peer(&dial(3), &[3; 32]), None);
    assert_eq!(admitted.dial_keys().len(), 2);
    assert_eq!(host.lines(), [id(1).short()]);
}

#[test]
fn the_held_file_is_private() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0)], &[], &[])).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in [access::FILE, SUSPENDED] {
            let mode = std::fs::metadata(host.state().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }
    assert_eq!(
        Held::load(host.state())
            .unwrap()
            .access
            .unwrap()
            .members
            .len(),
        1
    );
}

#[test]
fn a_second_fleet_is_refused_and_changes_nothing() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0)], &[], &[])).unwrap();
    let mut other = file(&[(2, true, 0)], &[], &[]);
    other.fleet = id(0xf2);
    assert!(matches!(
        host.apply(other),
        Err(Error::Access(access::Error::OtherFleet { .. }))
    ));
    assert_eq!(host.lines(), [id(1).short()]);
}

#[test]
fn a_removal_suspends_its_targets_but_never_the_sender() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, true, 0), (3, true, 0)], &[], &[]))
        .unwrap();
    let taken = host.suspend(1, &[removal(0x51, &[2, 1, 7])]);
    assert_eq!(taken, [[0x51; 32]]);
    let admitted = host.admitted();
    assert_eq!(admitted.peer(&dial(2), &[2; 32]), None);
    assert!(admitted.peer(&dial(1), &[1; 32]).is_some());
    assert!(!admitted.held().suspended.contains_key(&id(7)));
    assert_eq!(admitted.dial_keys().len(), 2);
    assert_eq!(host.lines(), [id(1).short(), id(3).short()]);
}

#[test]
fn a_suspended_member_passes_no_removal() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, true, 0), (3, true, 0)], &[], &[]))
        .unwrap();
    host.suspend(1, &[removal(0x51, &[2])]);
    assert!(host.suspend(2, &[removal(0x52, &[3])]).is_empty());
    assert!(host.admitted().peer(&dial(3), &[3; 32]).is_some());
}

#[test]
fn a_removal_for_another_fleet_is_neither_taken_nor_acted_on() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    let mut foreign = removal(0x51, &[2]);
    foreign.fleet = [0xf2; 32];
    assert!(host.suspend(1, &[foreign]).is_empty());
    assert!(host.admitted().peer(&dial(2), &[2; 32]).is_some());
}

#[test]
fn the_next_file_confirms_a_suspension_whose_target_was_removed() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    host.suspend(1, &[removal(0x51, &[2])]);
    let held = host.apply(file(&[(1, true, 0)], &[2], &[])).unwrap();
    assert!(held.suspended.is_empty());
    assert!(held.access.as_ref().unwrap().removed.contains(&id(2)));
    // A member that never heard of the removal cannot bring 2 back.
    let held = host
        .apply(file(&[(1, true, 0), (2, true, 5)], &[], &[]))
        .unwrap();
    assert!(!held.admits(&id(2)));
    assert_eq!(host.lines(), [id(1).short()]);
}

#[test]
fn the_next_file_lifts_a_suspension_the_log_voided() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    host.suspend(1, &[removal(0x51, &[2])]);
    // A file from a log that does not hold the removal settles nothing.
    let held = host
        .apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    assert!(held.suspended.contains_key(&id(2)));
    // One whose log holds it, with 2 still a member, lifts it.
    let held = host
        .apply(file(&[(1, true, 0), (2, true, 0)], &[], &[0x51]))
        .unwrap();
    assert!(held.suspended.is_empty());
    assert!(host.admitted().peer(&dial(2), &[2; 32]).is_some());
    assert_eq!(host.lines(), [id(1).short(), id(2).short()]);
}

#[test]
fn a_suspension_lifts_only_when_every_removal_naming_it_is_settled() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, true, 0), (3, true, 0)], &[], &[]))
        .unwrap();
    host.suspend(1, &[removal(0x51, &[2])]);
    host.suspend(3, &[removal(0x52, &[2])]);
    let held = host
        .apply(file(
            &[(1, true, 0), (2, true, 0), (3, true, 0)],
            &[],
            &[0x51],
        ))
        .unwrap();
    assert!(held.suspended.contains_key(&id(2)));
}

#[test]
fn a_member_named_by_too_many_removals_stays_suspended() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    let removals: Vec<Removed> = (0..=u8::try_from(MAX_CAUSES).unwrap())
        .map(|op| removal(0x40 + op, &[2]))
        .collect();
    host.suspend(1, &removals);
    let suspension = &Held::load(host.state()).unwrap().suspended[&id(2)];
    assert!(suspension.overflow);
    assert_eq!(suspension.removals.len(), MAX_CAUSES);
    let every: Vec<u8> = (0..=u8::try_from(MAX_CAUSES).unwrap())
        .map(|op| 0x40 + op)
        .collect();
    let held = host
        .apply(file(&[(1, true, 0), (2, true, 0)], &[], &every))
        .unwrap();
    assert!(held.suspended.contains_key(&id(2)));
}

#[test]
fn a_desktop_with_a_suspended_members_pair_is_cut_off_with_it() {
    let host = Host::new();
    host.enroll(2);
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    assert!(matches!(
        host.admitted().peer(&dial(2), &[2; 32]),
        Some(Peer::Member { desktop: true, .. })
    ));
    host.suspend(1, &[removal(0x51, &[2])]);
    let admitted = host.admitted();
    assert_eq!(admitted.peer(&dial(2), &[2; 32]), None);
    assert!(!admitted.keeps_ssh(&Peer::Desktop, &dial(2), &[2; 32]));
    // Removed for good, the desktop stays enrolled, cut off.
    host.apply(file(&[(1, true, 0)], &[2], &[])).unwrap();
    assert_eq!(Authorized::load(host.state()).unwrap().entries().len(), 1);
    assert_eq!(host.admitted().peer(&dial(2), &[2; 32]), None);
}

#[test]
fn a_denied_member_loses_a_live_ssh_session() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0)], &[], &[])).unwrap();
    let peer = host.admitted().peer(&dial(1), &[1; 32]).unwrap();
    assert!(host.admitted().keeps_ssh(&peer, &dial(1), &[1; 32]));
    host.apply(file(&[(1, false, 1)], &[], &[])).unwrap();
    assert!(!host.admitted().keeps_ssh(&peer, &dial(1), &[1; 32]));
    assert!(host.lines().is_empty());
}

#[test]
fn the_report_names_the_fleet_and_the_digest_of_what_is_held() {
    let host = Host::new();
    let sent = file(&[(1, true, 0), (2, false, 1)], &[9], &[]);
    let held = host.apply(sent.clone()).unwrap();
    let report = held.report(&Authorized::default()).unwrap().unwrap();
    assert_eq!(report.fleet, id(FLEET));
    assert_eq!(report.digest, sent.digest().unwrap());
    assert_eq!((report.allowed, report.denied, report.removed), (1, 1, 1));
    assert!(
        Held::default()
            .report(&Authorized::default())
            .unwrap()
            .is_none()
    );
}

#[test]
fn scrubbing_forgets_the_fleet_and_its_lines_and_keeps_the_rest() {
    let host = Host::new();
    std::fs::write(
        host.keys(),
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOwn me@laptop\n",
    )
    .unwrap();
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    host.suspend(1, &[removal(7, &[2])]);
    assert_eq!(host.lines().len(), 1);
    Held::scrub(host.state(), Some(&host.keys())).unwrap();
    assert!(host.lines().is_empty());
    let text = std::fs::read_to_string(host.keys()).unwrap();
    assert!(text.contains("me@laptop"), "{text}");
    let admitted = host.admitted();
    assert_eq!(admitted.peer(&dial(1), &[1; 32]), None);
    assert!(admitted.held().access.is_none());
    assert!(admitted.held().suspended.is_empty());
    // Another fleet may take the account now.
    let mut other = file(&[(3, true, 0)], &[], &[]);
    other.fleet = id(0xf2);
    assert!(host.apply(other).is_ok());
}

#[test]
fn an_unbound_desktop_with_a_suspended_members_keys_fails_closed() {
    let host = Host::new();
    host.enroll(2);
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    host.suspend(1, &[removal(0x51, &[2])]);
    // Never bound, still cut off: a suspended member's key is no one's.
    let admitted = host.admitted();
    assert_eq!(admitted.peer(&dial(2), &[2; 32]), None);
    assert!(!admitted.keeps_ssh(&Peer::Desktop, &dial(2), &[2; 32]));
    assert!(!admitted.dial_keys().contains(&member(2, true, 0).dial));
}

#[test]
fn an_unbound_desktop_a_removed_member_named_stays_enrolled_cut_off_and_listed() {
    let host = Host::new();
    host.enroll(2);
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    host.apply(file(&[(1, true, 0)], &[2], &[])).unwrap();
    // Not proven member 2's device: not forgotten, but cut off.
    let desktops = Authorized::load(host.state()).unwrap();
    assert_eq!(desktops.entries().len(), 1);
    assert_eq!(host.admitted().peer(&dial(2), &[2; 32]), None);
    let report = Held::load(host.state())
        .unwrap()
        .report(&desktops)
        .unwrap()
        .unwrap();
    assert_eq!(report.revoked.len(), 1);
    assert_eq!(report.revoked[0].tunnel, Id([2; 32]));
    // A person enrolls it again.
    let keys = member(2, true, 0);
    Held::release(host.state(), keys.dial, keys.tunnel).unwrap();
    assert_eq!(
        host.admitted().peer(&dial(2), &[2; 32]),
        Some(Peer::Desktop)
    );
}

#[test]
fn a_desktop_sharing_only_a_tunnel_key_with_a_removed_member_is_not_cut_off() {
    let host = Host::new();
    let mut desktops = Authorized::default();
    desktops
        .insert(Entry {
            dial: member(7, true, 0).dial,
            tunnel: [2; 32],
            label: "other".to_owned(),
        })
        .unwrap();
    desktops.save(host.state()).unwrap();
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    host.apply(file(&[(1, true, 0)], &[2], &[])).unwrap();
    assert_eq!(
        host.admitted().peer(&dial(7), &[2; 32]),
        Some(Peer::Desktop)
    );
    assert_eq!(host.admitted().peer(&dial(2), &[2; 32]), None);
}

#[test]
fn a_removal_another_member_signed_is_taken_but_acted_on_by_no_one() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, true, 0), (3, true, 0)], &[], &[]))
        .unwrap();
    // Member 1 passes a removal of 2 it says 3 signed.
    let mut forged = removal(0x51, &[2]);
    forged.signer = [3; 32];
    assert_eq!(host.pass(1, &[forged]), [[0x51; 32]]);
    assert!(Held::load(host.state()).unwrap().suspended.is_empty());
}

#[test]
fn a_member_denied_ssh_here_suspends_only_members_denied_too() {
    let host = Host::new();
    host.apply(file(
        &[(1, false, 1), (2, true, 0), (3, false, 1)],
        &[],
        &[],
    ))
    .unwrap();
    host.suspend(1, &[removal(0x51, &[2, 3])]);
    let held = Held::load(host.state()).unwrap();
    assert!(!held.suspended.contains_key(&id(2)));
    assert!(held.suspended.contains_key(&id(3)));
}

#[test]
fn a_suspension_for_removals_the_log_never_held_lifts_once_settled() {
    let host = Host::new();
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    // Member 1 names a removal no log will hold.
    host.suspend(1, &[removal(0x5f, &[2])]);
    assert!(host.admitted().peer(&dial(2), &[2; 32]).is_none());
    // A current log settles ids it does not hold too.
    let held = host
        .apply(file(&[(1, true, 0), (2, true, 0)], &[], &[0x5f]))
        .unwrap();
    assert!(held.suspended.is_empty());
}

#[test]
fn a_desktop_sharing_only_the_tunnel_key_with_a_suspended_member_is_still_admitted() {
    let host = Host::new();
    let mut desktops = Authorized::default();
    desktops
        .insert(Entry {
            dial: dial(7),
            tunnel: [2; 32],
            label: "other".to_owned(),
        })
        .unwrap();
    desktops.save(host.state()).unwrap();
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    host.suspend(1, &[removal(0x51, &[2])]);
    let admitted = host.admitted();
    assert_eq!(admitted.peer(&dial(7), &[2; 32]), Some(Peer::Desktop));
    assert!(admitted.keeps_ssh(&Peer::Desktop, &dial(7), &[2; 32]));
    // The member's own pair is no one's.
    assert_eq!(admitted.peer(&dial(2), &[2; 32]), None);
}

#[test]
fn a_member_naming_an_enrolled_desktops_keys_cannot_make_it_forgotten() {
    let host = Host::new();
    host.enroll(2);
    host.apply(file(&[(1, true, 0), (2, true, 0)], &[], &[]))
        .unwrap();
    // However often the desktop dials in, nothing binds it to member 2.
    assert_eq!(
        host.admitted().peer(&dial(2), &[2; 32]),
        Some(Peer::Member {
            id: id(2),
            ssh: true,
            desktop: true
        })
    );
    host.apply(file(&[(1, true, 0)], &[2], &[])).unwrap();
    let desktops = Authorized::load(host.state()).unwrap();
    assert_eq!(desktops.entries().len(), 1);
    assert_eq!(desktops.entries()[0].label, "laptop");
}
