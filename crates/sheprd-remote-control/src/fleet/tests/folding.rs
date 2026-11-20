//! However many removals a member sends whose history never comes, the
//! suspensions kept stay bounded and `peers.json` readable. Folded ones
//! still settle by their own history, end on their own time, and stand
//! by their own removal's signer rule.

use super::*;
use crate::control::fleet::{
    Access, MAX_COMPUTERS, MAX_OP, MAX_PHONES, MAX_TOMBSTONES, Reason, Remove,
};
use suspensions::{FoldedTarget, MAX_FOLDED_IDS, MAX_UNCONFIRMED_PER_SIGNER};

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
        None,
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
        let folded = &inner.folded[&a.fleet.me()][&c.fleet.me()];
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
        b.fleet.inner().folded[&a.fleet.me()][&c.fleet.me()]
            .ids
            .contains_key(&invalid.id())
    );
    let mut cut: Vec<MemberId> = vec![c.fleet.me(), d.fleet.me()];
    cut.sort();
    assert_eq!(b.fleet.suspended(), cut);
    // Its history arrives: refused, and c is let back in; d stays out.
    b.fleet.receive(vec![missing], now_ms() + 2).unwrap();
    let inner = b.fleet.inner();
    assert!(
        inner
            .folded
            .get(&a.fleet.me())
            .is_none_or(|folded| !folded.contains_key(&c.fleet.me()))
    );
    drop(inner);
    assert_eq!(b.fleet.suspended(), vec![d.fleet.me()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removal_dropped_for_room_keeps_its_target_out() {
    let (a, b, c, d) = four().await;
    // a's removal of c, on an operation b has not seen.
    let missing = unseen(&a, 0);
    let kept = removal(&a, &c, None);
    b.fleet.receive(vec![kept], now_ms()).unwrap();
    // More of a's removals of d than the pool holds, each on its own
    // unseen parent, arrive later: the first, c's, is dropped for room.
    let mut flood = Vec::new();
    for turn in 1..=crate::control::fleet::MAX_PENDING + 4 {
        unseen(&a, turn);
        flood.push(removal(&a, &d, None));
    }
    b.fleet.receive(flood, now_ms() + 1).unwrap();
    // Its history arrives: nothing refused it, so c is never let back in.
    b.fleet.receive(vec![missing], now_ms() + 2).unwrap();
    let out = {
        let inner = b.fleet.inner();
        !inner.log.as_ref().unwrap().is_member(&c.fleet.me())
    };
    assert!(out || b.fleet.suspended().contains(&c.fleet.me()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newer_fold_never_extends_an_older_suspension() {
    let (a, b, c, d) = four().await;
    let start = now_ms();
    let (signer, older, newer) = (a.fleet.me(), c.fleet.me(), d.fleet.me());
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
        inner.folded[&signer][&older]
            .ids
            .values()
            .copied()
            .collect::<Vec<_>>(),
        vec![start]
    );
    // Seven days after the first, c's own time is up, d's is not.
    suspensions::settle(&mut inner, start + 7 * DAY + 1, BTreeSet::new());
    assert!(!inner.folded[&signer].contains_key(&older));
    assert!(inner.folded[&signer].contains_key(&newer));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_overflow_lasts_as_long_as_its_latest_removal() {
    let (a, b, c, d) = four().await;
    let start = now_ms();
    let (signer, target, other) = (a.fleet.me(), c.fleet.me(), d.fleet.me());
    let mut inner = b.fleet.inner();
    // Removals of c: more than a folded target keeps ids for, the last
    // three days after the rest; then the newest few, of d, kept apart.
    let mut turn = 0_u8;
    let mut add = |inner: &mut Inner, target, since_ms| {
        turn += 1;
        inner.unconfirmed.insert(
            OpId([turn; 32]),
            Unconfirmed {
                signer,
                targets: vec![target],
                since_ms,
                op: None,
            },
        );
    };
    for _ in 0..MAX_FOLDED_IDS + 2 {
        add(&mut inner, target, start);
    }
    add(&mut inner, target, start + 3 * DAY);
    for _ in 0..MAX_UNCONFIRMED_PER_SIGNER {
        add(&mut inner, other, start + 4 * DAY);
    }
    suspensions::settle(&mut inner, start + 4 * DAY, BTreeSet::new());
    assert_eq!(
        inner.folded[&signer][&target].overflowed,
        Some(start + 3 * DAY)
    );
    // A week after the first removals of c, the latest still holds it.
    suspensions::settle(&mut inner, start + 7 * DAY + 1, BTreeSet::new());
    assert!(inner.folded[&signer].contains_key(&target));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folded_target_stands_by_what_its_removals_named() {
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
    let mut inner = a.fleet.inner();
    // Folded from c: d, by a removal that also named b, which removed c,
    // so it stands; a, by one that named a alone, so it does not.
    let folded = |named_with: Vec<MemberId>| FoldedTarget {
        ids: BTreeMap::from([(OpId([named_with.len() as u8; 32]), now_ms())]),
        overflowed: None,
        named_with: named_with.into_iter().collect(),
        pinned: false,
    };
    inner.folded.insert(
        signer,
        BTreeMap::from([
            (d.fleet.me(), folded(vec![b.fleet.me(), d.fleet.me()])),
            (a.fleet.me(), folded(vec![a.fleet.me()])),
        ]),
    );
    suspensions::settle(&mut inner, now_ms(), BTreeSet::new());
    assert!(inner.folded[&signer].contains_key(&d.fleet.me()));
    assert!(!inner.folded[&signer].contains_key(&a.fleet.me()));
    // c is no member: what stands now is pinned and needs no list.
    assert!(inner.folded[&signer][&d.fleet.me()].pinned);
    drop(inner);
    assert_eq!(a.fleet.suspended(), vec![d.fleet.me()]);
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
    // Every live signer's removals kept apart and folded, naming every
    // live member, and every member no longer live with one folded,
    // pinned, for every live member.
    let mut inner = Inner::default();
    for seed in 0..live + MAX_TOMBSTONES {
        let mut signer = [0xee; 32];
        signer[..8].copy_from_slice(&u64::try_from(seed).unwrap().to_be_bytes());
        let signer = MemberId(signer);
        let live = seed < live;
        let (separate, ids) = if live {
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
        let folded = inner.folded.entry(signer).or_default();
        for target in &targets {
            folded.insert(
                *target,
                FoldedTarget {
                    ids: (0..ids)
                        .map(|turn| (id(&signer, 100 + turn), u64::MAX))
                        .collect(),
                    overflowed: Some(u64::MAX),
                    named_with: if live {
                        targets.iter().copied().collect()
                    } else {
                        BTreeSet::new()
                    },
                    pinned: !live,
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

#[test]
fn only_a_removals_own_fault_counts_as_refused() {
    use crate::control::fleet::{Error as LogError, Inserted};
    for room in [LogError::Full, LogError::Bounds, LogError::ParentMissing] {
        assert!(!suspensions::refuses(&Err(room)));
    }
    for fault in [
        LogError::Sequence,
        LogError::Signature,
        LogError::Permission,
        LogError::Unknown,
    ] {
        assert!(suspensions::refuses(&Err(fault)));
    }
    assert!(!suspensions::refuses(&Ok(Inserted::Applied)));
}
