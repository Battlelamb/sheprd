//! However many removals a member sends whose history never comes, the
//! suspensions kept stay bounded and `peers.json` readable. Folded ones
//! still settle by their own history, end on their own time, and stand
//! by their own removal's signer rule.

use super::*;
use crate::control::fleet::{
    Access, MAX_COMPUTERS, MAX_OP, MAX_PHONES, MAX_TOMBSTONES, Reason, Remove,
};
use suspensions::{Folded, MAX_FOLDED_IDS, MAX_UNCONFIRMED_PER_SIGNER};

const DAY: u64 = 24 * 60 * 60 * 1000;

/// a, b, c, and d in one fleet, none started.
async fn four() -> (Node, Node, Node, Node) {
    let (a, b, c) = idle().await;
    let d = Node::new("127.0.0.1:9".parse().unwrap(), 4).await;
    join(&a, &d);
    for node in [&b, &c] {
        node.fleet.receive(a.ops(), now_ms()).unwrap();
    }
    (a, b, c, d)
}

/// `by`'s removal of `member` on everything `by` holds, kept out of its
/// own log, with `seq` when given.
fn removal(by: &Node, member: &Node, seq: Option<u64>) -> Op {
    let inner = by.fleet.inner();
    let mut unsigned = inner.log.as_ref().unwrap().draft(
        by.fleet.me(),
        now_ms(),
        Body::Remove(Remove {
            member: member.fleet.me(),
            also_members: Vec::new(),
            also_hosts: Vec::new(),
            reason: Reason::Compromised,
        }),
    );
    if let Some(seq) = seq {
        unsigned.seq = seq;
    }
    unsigned.sign(by.fleet.key()).unwrap()
}

/// An operation `by` signs and keeps, which no one else has seen.
fn unseen(by: &Node, turn: usize) -> Op {
    let host = grants::own_host(
        by.fleet.inner().log.as_ref().unwrap().state(),
        &by.fleet.me(),
    )
    .unwrap();
    by.fleet
        .sign(
            Body::Access(Access {
                host,
                member: by.fleet.me(),
                allow: turn % 2 == 1,
                rev: u64::try_from(turn).unwrap() + 100,
            }),
            now_ms(),
        )
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_members_many_removals_stay_bounded_and_suspended() {
    let (a, b, c, _) = four().await;
    for turn in 0..3 * MAX_UNCONFIRMED_PER_SIGNER {
        unseen(&a, turn);
        b.fleet
            .receive(vec![removal(&a, &c, None)], now_ms())
            .unwrap();
    }
    {
        let inner = b.fleet.inner();
        assert!(inner.unconfirmed.len() <= MAX_UNCONFIRMED_PER_SIGNER);
        let folded = &inner.folded[&(a.fleet.me(), c.fleet.me())];
        assert!(folded.ids.len() <= MAX_FOLDED_IDS);
    }
    assert_eq!(b.fleet.suspended(), vec![c.fleet.me()]);
    let again = Fleet::load(b.dir.path(), now_ms()).unwrap();
    assert_eq!(again.suspended(), vec![c.fleet.me()]);
    assert_eq!(again.inner().folded, b.fleet.inner().folded);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folded_removal_its_history_refuses_lifts_its_target() {
    let (a, b, c, d) = four().await;
    // a's removal of c on an operation b has not seen, reusing that
    // operation's number: its history refuses it.
    let missing = unseen(&a, 0);
    let invalid = removal(&a, &c, Some(missing.unsigned().seq));
    b.fleet.receive(vec![invalid.clone()], now_ms()).unwrap();
    // Enough of a's removals of d after it to fold it.
    for turn in 1..=MAX_UNCONFIRMED_PER_SIGNER {
        unseen(&a, turn);
        b.fleet
            .receive(vec![removal(&a, &d, None)], now_ms() + 1)
            .unwrap();
    }
    assert!(
        b.fleet.inner().folded[&(a.fleet.me(), c.fleet.me())]
            .ids
            .contains_key(&invalid.id())
    );
    let mut cut: Vec<MemberId> = vec![c.fleet.me(), d.fleet.me()];
    cut.sort();
    assert_eq!(b.fleet.suspended(), cut);
    // Its history arrives: refused, and c is let back in; d stays out.
    b.fleet.receive(vec![missing], now_ms() + 2).unwrap();
    assert!(
        !b.fleet
            .inner()
            .folded
            .contains_key(&(a.fleet.me(), c.fleet.me()))
    );
    assert_eq!(b.fleet.suspended(), vec![d.fleet.me()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newer_fold_never_extends_an_older_suspension() {
    let (a, b, c, d) = four().await;
    let start = now_ms();
    let (signer, older, newer) = (a.fleet.me(), c.fleet.me(), d.fleet.me());
    {
        let mut inner = b.fleet.inner();
        // a's oldest removal cuts c off; the rest, three days on, d.
        for turn in 0..=MAX_UNCONFIRMED_PER_SIGNER + 1 {
            let (target, since_ms) = if turn == 0 {
                (older, start)
            } else {
                (newer, start + 3 * DAY)
            };
            inner.unconfirmed.insert(
                OpId([u8::try_from(turn).unwrap() + 1; 32]),
                Unconfirmed {
                    signer,
                    targets: vec![target],
                    since_ms,
                    op: None,
                },
            );
        }
        suspensions::settle(&mut inner, start + 3 * DAY, BTreeSet::new());
        assert_eq!(
            inner.folded[&(signer, older)]
                .ids
                .values()
                .copied()
                .collect::<Vec<_>>(),
            vec![start]
        );
        // Seven days after the first, c's own time is up, d's is not.
        suspensions::settle(&mut inner, start + 7 * DAY + 1, BTreeSet::new());
        assert!(!inner.folded.contains_key(&(signer, older)));
        assert!(inner.folded.contains_key(&(signer, newer)));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_target_its_signer_cannot_suspend_is_not_suspended_by_a_fold() {
    let (a, b, c, d) = four().await;
    // b removes c: c is no member, and only b removed it.
    let removed = b
        .fleet
        .sign(
            Body::Remove(Remove {
                member: c.fleet.me(),
                also_members: Vec::new(),
                also_hosts: Vec::new(),
                reason: Reason::Compromised,
            }),
            now_ms(),
        )
        .unwrap();
    a.fleet.receive(vec![removed], now_ms()).unwrap();
    let signer = c.fleet.me();
    {
        let mut inner = a.fleet.inner();
        // Folded: c's removal of b, which still stands (b removed c), and
        // c's removal of d, which does not.
        for (id, target) in [(1, b.fleet.me()), (2, d.fleet.me())] {
            inner.folded.insert(
                (signer, target),
                Folded {
                    ids: BTreeMap::from([(OpId([id; 32]), now_ms())]),
                    overflowed: None,
                },
            );
        }
        suspensions::settle(&mut inner, now_ms(), BTreeSet::new());
        assert!(inner.folded.contains_key(&(signer, b.fleet.me())));
        assert!(!inner.folded.contains_key(&(signer, d.fleet.me())));
    }
    assert_eq!(a.fleet.suspended(), vec![b.fleet.me()]);
}

#[test]
fn the_most_suspensions_kept_fit_in_the_peers_file() {
    let dir = tempfile::tempdir().unwrap();
    let live = MAX_COMPUTERS + MAX_PHONES;
    let targets: Vec<MemberId> = (0..live)
        .map(|seed| MemberId([u8::try_from(seed).unwrap(); 32]))
        .collect();
    let id = |signer: &MemberId, turn: usize| {
        let mut id = signer.0;
        id[31] = u8::try_from(turn).unwrap();
        OpId(id)
    };
    // Every live signer's separate removals and folded ones, and every
    // member no longer live with one folded removal, each naming every
    // live member.
    let mut inner = Inner::default();
    for seed in 0..live + MAX_TOMBSTONES {
        let mut signer = [0xee; 32];
        signer[..8].copy_from_slice(&u64::try_from(seed).unwrap().to_be_bytes());
        let signer = MemberId(signer);
        let (separate, folded) = if seed < live {
            (MAX_UNCONFIRMED_PER_SIGNER, MAX_FOLDED_IDS)
        } else {
            (0, 1)
        };
        for turn in 0..separate {
            inner.unconfirmed.insert(
                id(&signer, turn),
                Unconfirmed {
                    signer,
                    targets: targets.clone(),
                    since_ms: u64::MAX,
                    op: None,
                },
            );
        }
        for target in &targets {
            inner.folded.insert(
                (signer, *target),
                Folded {
                    ids: (0..folded)
                        .map(|turn| (id(&signer, 100 + turn), u64::MAX))
                        .collect(),
                    overflowed: Some(u64::MAX),
                },
            );
        }
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
