//! The relay records the fleet writes: it never deletes one pairing took
//! over, and it follows a host's signed new tunnel identity.

use super::*;
use crate::desktop::record::Record;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

fn record(identity: &str, fleet: bool) -> Record {
    Record {
        computer: "Studio".into(),
        identity: identity.into(),
        relays: vec!["https://relay.example.org".into()],
        route: URL_SAFE_NO_PAD.encode([7; 32]),
        fleet,
    }
}

fn keys() -> Vec<String> {
    vec!["ssh-ed25519 AAAA".to_owned()]
}

#[test]
fn a_host_paired_after_the_fleet_wrote_it_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let fleet = record("identity-one", true);
    let id = fleet.id().unwrap();
    grants::write_records(dir.path(), &[(fleet, keys())]).unwrap();
    // Paired here over Bluetooth since: pairing owns the record now.
    let paired = record("identity-one", false);
    paired.save(dir.path(), &keys()).unwrap();
    // Granted still, then no longer: neither deletes it.
    grants::write_records(dir.path(), &[(record("identity-one", true), keys())]).unwrap();
    assert_eq!(Record::load(dir.path(), &id).unwrap(), paired);
    grants::write_records(dir.path(), &[]).unwrap();
    assert_eq!(Record::load(dir.path(), &id).unwrap(), paired);
}

#[test]
fn a_fleet_host_that_changed_its_tunnel_identity_keeps_its_record() {
    let dir = tempfile::tempdir().unwrap();
    let id = record("identity-one", true).id().unwrap();
    grants::write_records(dir.path(), &[(record("identity-one", true), keys())]).unwrap();
    let moved = record("identity-two", true);
    grants::write_records(dir.path(), &[(moved.clone(), keys())]).unwrap();
    assert_eq!(Record::load(dir.path(), &id).unwrap(), moved);
}

#[test]
fn pairing_still_cannot_replace_an_identity() {
    let dir = tempfile::tempdir().unwrap();
    let id = record("identity-one", true).id().unwrap();
    grants::write_records(dir.path(), &[(record("identity-one", true), keys())]).unwrap();
    assert!(matches!(
        record("identity-two", false).save(dir.path(), &keys()),
        Err(crate::desktop::Error::RecordConflict(_))
    ));
    // Nor can the fleet take over another identity pairing wrote.
    let dir = tempfile::tempdir().unwrap();
    record("identity-one", false)
        .save(dir.path(), &keys())
        .unwrap();
    assert!(matches!(
        record("identity-two", true).save(dir.path(), &keys()),
        Err(crate::desktop::Error::RecordConflict(_))
    ));
    assert!(!Record::load(dir.path(), &id).unwrap().fleet);
}
