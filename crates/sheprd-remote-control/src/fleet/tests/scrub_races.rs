//! `fleet scrub` against what runs beside it: a starting daemon or an
//! offline update holding the fleet lock, a grants write already under
//! way, and the SSH session that asked for the scrub.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn an_offline_scrub_waits_for_the_fleet_lock() {
    let dir = tempfile::tempdir().unwrap();
    // A daemon starting, or an update writing, holds the lock.
    let held = lock::Lock::try_acquire(dir.path()).unwrap().unwrap();
    let ran = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let scrub = scope.spawn(|| {
            scrub_with(dir.path(), |state_dir| {
                // Run only while holding the lock itself.
                assert!(lock::Lock::try_acquire(state_dir).unwrap().is_none());
                ran.store(true, Ordering::SeqCst);
                Ok(())
            })
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !ran.load(Ordering::SeqCst),
            "nothing is forgotten under the lock"
        );
        drop(held);
        scrub.join().unwrap().unwrap();
    });
    assert!(ran.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scrub_waits_for_a_grants_write_under_way() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    join(&a, &b);
    let log = b.dir.path().join("fleet/log.bin");
    // A grants write has read the fleet and not yet written it.
    let writing = b.fleet.writing();
    let (state, fleet) = (b.state.clone(), b.fleet.clone());
    let scrub =
        tokio::task::spawn_blocking(move || ask::answer(&state, &fleet, Ask::Scrub, now_ms()));
    // Blocking on purpose: the guard stands for a write on a blocking
    // thread, never held across an await.
    std::thread::sleep(Duration::from_millis(300));
    assert!(log.exists(), "the scrub waits for the write to finish");
    drop(writing);
    assert_eq!(scrub.await.unwrap().unwrap(), Answer::Done);
    assert!(!log.exists());
    // A write after it finds no fleet, and grants nothing.
    grants::apply(&b.state, &b.fleet).unwrap();
    assert!(b.lines().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_session_that_asked_for_a_scrub_may_read_its_answer() {
    let (http, _, _) = relay().await;
    let (a, b) = (Node::new(http, 1).await, Node::new(http, 2).await);
    assert_eq!(a.ask(Ask::Found), Answer::Done);
    join(&a, &b);
    assert_eq!(b.fleet.scrub_grace(), None);
    assert_eq!(b.ask(Ask::Scrub), Answer::Done);
    let left = b.fleet.scrub_grace().unwrap();
    assert!(left > Duration::ZERO && left <= SCRUB_GRACE);
}
