//! Counting devices in memory: bounded, and never naming one.

use crate::devices::{DAY_S, Devices, MAX_DEVICES, WEEK_S};

fn device(index: usize) -> [u8; 16] {
    let mut id = [0xee; 16];
    id[..8].copy_from_slice(&(index as u64).to_be_bytes());
    id
}

#[test]
fn a_device_counts_once_however_often_it_appears() {
    let mut devices = Devices::default();
    for _ in 0..3 {
        devices.attested(&device(1));
        devices.pushed(&device(1), 1_000);
    }
    let counts = devices.counts(1_000);
    assert_eq!(
        (counts.attested, counts.active_day, counts.active_week),
        (1, 1, 1)
    );
}

#[test]
fn full_counts_forget_quiet_devices_then_stop_counting_new_ones() {
    let mut devices = Devices::default();
    for index in 0..MAX_DEVICES {
        devices.pushed(&device(index), 0);
        devices.attested(&device(index));
    }
    devices.attested(&device(MAX_DEVICES));
    assert_eq!(devices.counts(0).untracked, 1);
    // A week on, every device is quiet, so a new one makes room.
    devices.pushed(&device(MAX_DEVICES), WEEK_S);
    let counts = devices.counts(WEEK_S);
    assert_eq!((counts.active_week, counts.untracked), (1, 1));
    // Within the week none can go: the newcomer is not counted.
    for index in 0..MAX_DEVICES {
        devices.pushed(&device(index), WEEK_S + DAY_S);
    }
    devices.pushed(&device(MAX_DEVICES + 1), WEEK_S + DAY_S);
    assert_eq!(devices.counts(WEEK_S + DAY_S).untracked, 3);
}

#[test]
fn debug_shows_counts_alone() {
    let mut devices = Devices::default();
    devices.attested(&device(1));
    let text = format!("{devices:?}");
    assert_eq!(text, "Devices { attested: 1, pushed: 0, untracked: 0 }");
}
