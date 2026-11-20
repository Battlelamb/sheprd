//! Checkpoints signed before members carried `rotated` still check out,
//! prune, save, and load.

use super::*;
use crate::control::fleet::state::Layout;

/// a and b, b with a rotation behind it.
fn rotated_history() -> (Log, Device, Device) {
    let (a, b) = (computer(1), computer(2));
    let mut log = found(&a);
    admit(&mut log, &a, &b);
    let mut bundle = b.bundle.clone();
    bundle.tunnel = [0x62; 32];
    act(&mut log, &b, Body::Rotate(bundle));
    (log, a, b)
}

#[test]
fn a_checkpoint_naming_the_older_layout_is_taken_and_loads() {
    let (mut log, a, b) = rotated_history();
    let older = log.state().write(Layout::V1).unwrap();
    let hash = sha256(&[&older]);
    assert_ne!(hash, log.state().hash().unwrap());
    // Signed by a build that did not know `rotated`.
    let checkpoint = act(&mut log, &a, Body::Checkpoint(hash));
    // A joiner handed the older bytes starts from them.
    let joined = Log::from_checkpoint(checkpoint.clone(), &older).unwrap();
    let member = &joined.state().members[&b.id()];
    assert_eq!(member.bundle.tunnel, [0x62; 32]);
    assert_eq!(member.rotated, 0, "the older layout has no rotation");
    // A log pruned to it saves and loads again.
    log.prune(&checkpoint.id()).unwrap();
    let saved = saved::encode(&log).unwrap();
    let loaded = saved::decode(&saved, NOW).unwrap();
    assert_eq!(loaded.fleet(), log.fleet());
    assert_eq!(loaded.state().members.len(), 2);
}

#[test]
fn either_layout_reads_back_and_nothing_else_does() {
    let (log, ..) = rotated_history();
    for layout in [Layout::V1, Layout::V2] {
        let bytes = log.state().write(layout).unwrap();
        let read = State::decode(&bytes).unwrap();
        assert_eq!(read.write(layout).unwrap(), bytes);
    }
    let mut other = log.state().encode().unwrap();
    other[DOMAIN.len() + 1] = b'x';
    assert_eq!(State::decode(&other), Err(Error::Encoding));
    // A checkpoint naming neither layout is refused.
    assert!(!log.state().named_by(&[7; 32]).unwrap());
}

#[test]
fn a_log_pruned_to_a_checkpoint_saves_and_loads() {
    let (mut log, a, _) = rotated_history();
    let hash = log.state().hash().unwrap();
    let checkpoint = act(&mut log, &a, Body::Checkpoint(hash));
    log.prune(&checkpoint.id()).unwrap();
    let loaded = saved::decode(&saved::encode(&log).unwrap(), NOW).unwrap();
    assert_eq!(loaded.state(), log.state());
}
