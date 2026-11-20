//! However many removals a member sends whose history never comes, the
//! suspensions kept stay bounded, none is lost, and `peers.json` stays
//! readable.

use super::*;
use crate::control::fleet::{
    Access, MAX_COMPUTERS, MAX_OP, MAX_PHONES, MAX_TOMBSTONES, Reason, Remove,
};

#[tokio::test(flavor = "multi_thread")]
async fn a_members_many_removals_fold_into_one_suspension() {
    let (a, b, c) = idle().await;
    let host = grants::own_host(
        a.fleet.inner().log.as_ref().unwrap().state(),
        &a.fleet.me(),
        None,
    )
    .unwrap();
    for turn in 0..3 * MAX_UNCONFIRMED_PER_SIGNER {
        // Each removal on a parent b has not heard of, signed apart from
        // a's own log.
        let access = Body::Access(Access {
            host,
            member: b.fleet.me(),
            allow: turn % 2 == 1,
            rev: u64::try_from(turn).unwrap() + 100,
        });
        a.fleet.sign(access, now_ms()).unwrap();
        let removal = a
            .fleet
            .inner()
            .log
            .as_ref()
            .unwrap()
            .draft(
                a.fleet.me(),
                now_ms(),
                Body::Remove(Remove {
                    member: c.fleet.me(),
                    also_members: Vec::new(),
                    also_hosts: Vec::new(),
                    reason: Reason::Compromised,
                }),
            )
            .sign(a.fleet.key())
            .unwrap();
        b.fleet.receive(vec![removal], now_ms()).unwrap();
    }
    let inner = b.fleet.inner();
    assert!(inner.unconfirmed.len() <= MAX_UNCONFIRMED_PER_SIGNER + 1);
    let fold = &inner.unconfirmed[&folded(&a.fleet.me())];
    assert_eq!(fold.targets, vec![c.fleet.me()]);
    assert!(fold.op.is_none());
    drop(inner);
    // Still cut off, and after a restart too.
    assert_eq!(b.fleet.suspended(), vec![c.fleet.me()]);
    let again = Fleet::load(b.dir.path(), now_ms()).unwrap();
    assert_eq!(again.suspended(), vec![c.fleet.me()]);
    assert!(again.inner().unconfirmed.len() <= MAX_UNCONFIRMED_PER_SIGNER + 1);
}

#[test]
fn the_most_suspensions_kept_fit_in_the_peers_file() {
    let dir = tempfile::tempdir().unwrap();
    let live = MAX_COMPUTERS + MAX_PHONES;
    let targets: Vec<MemberId> = (0..live)
        .map(|seed| MemberId([u8::try_from(seed).unwrap(); 32]))
        .collect();
    // Every live signer's separate and folded suspensions, and one folded
    // suspension for every member no longer live, each naming every
    // live member.
    let signers = (0..live + MAX_TOMBSTONES).map(|seed| {
        let mut id = [0xee; 32];
        id[..8].copy_from_slice(&u64::try_from(seed).unwrap().to_be_bytes());
        (seed < live, MemberId(id))
    });
    let mut inner = Inner::default();
    for (live, signer) in signers {
        let kept = if live { MAX_UNCONFIRMED_PER_SIGNER } else { 0 };
        for turn in 0..kept {
            let mut id = signer.0;
            id[31] = u8::try_from(turn).unwrap();
            inner.unconfirmed.insert(
                OpId(id),
                Unconfirmed {
                    signer,
                    targets: targets.clone(),
                    since_ms: u64::MAX,
                    op: None,
                },
            );
        }
        inner.unconfirmed.insert(
            folded(&signer),
            Unconfirmed {
                signer,
                targets: targets.clone(),
                since_ms: u64::MAX,
                op: None,
            },
        );
    }
    crate::fleet::store::save(dir.path(), &inner, &mut Default::default()).unwrap();
    let written = std::fs::metadata(dir.path().join("fleet/peers.json"))
        .unwrap()
        .len();
    // The removals kept to check again, at most a full operation each,
    // base64url, with room to spare for their keys and quotes.
    let removals = u64::try_from(MAX_KEPT_REMOVALS * (MAX_OP * 4 / 3 + 64)).unwrap();
    assert!(
        written + removals < crate::fleet::store::MAX_PEERS,
        "{written} + {removals}"
    );
}
