//! The wire of carrying a phone's relay keys through another computer: what
//! the phone signs, how long a signature lasts, and the bounds on answers.

use super::*;
use crate::control::host_relay::{
    self, Info, Job, Keys, MAX_AGE_MS, MAX_FIELD, MAX_RELAYS, MAX_SKEW_MS, Outcome, PREFIX, State,
    fingerprint_bytes,
};

const FINGERPRINT: &str = "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff";

fn request() -> host_relay::Request {
    host_relay::Request {
        fingerprint: FINGERPRINT.into(),
        device: DeviceId("dev".into()),
        keys: Keys {
            dial: "DIAL".into(),
            tunnel: "TUN".into(),
        },
        signed_ms: 0x0102_0304_0506_0708,
        signature: "SIG".into(),
    }
}

fn info() -> Info {
    Info {
        identity: "identity".into(),
        relays: vec!["https://relay.example.org".into()],
        route: "route".into(),
    }
}

#[test]
fn the_signed_message_is_the_prefix_fingerprint_length_prefixed_fields_and_time() {
    let mut expected = PREFIX.to_vec();
    expected.extend(
        (0..=0xff_u8)
            .step_by(0x11)
            .chain((0..=0xff_u8).step_by(0x11)),
    );
    expected.extend([0, 3]);
    expected.extend(b"dev");
    expected.extend([0, 4]);
    expected.extend(b"DIAL");
    expected.extend([0, 3]);
    expected.extend(b"TUN");
    expected.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(request().message().unwrap(), expected);
}

#[test]
fn a_request_out_of_bounds_has_no_message() {
    let mut bad = request();
    bad.fingerprint = "00:11".into();
    assert!(bad.message().is_none());
    let mut bad = request();
    bad.keys.dial = String::new();
    assert!(bad.message().is_none());
    let mut bad = request();
    bad.device = DeviceId("a\nb".into());
    assert!(bad.message().is_none());
    // The signature is not signed, but a request needs one in bounds.
    let mut unsigned = request();
    unsigned.signature = String::new();
    assert!(unsigned.message().is_some());
    assert!(!unsigned.valid());
    unsigned.signature = "x".repeat(MAX_FIELD + 1);
    assert!(!unsigned.valid());
}

#[test]
fn fingerprints_read_with_or_without_colons_in_either_case() {
    let plain = FINGERPRINT.replace(':', "").to_lowercase();
    assert_eq!(fingerprint_bytes(FINGERPRINT), fingerprint_bytes(&plain));
    assert!(fingerprint_bytes(&plain).is_some());
    assert!(fingerprint_bytes("zz").is_none());
    assert!(fingerprint_bytes(&"g".repeat(64)).is_none());
}

#[test]
fn a_signature_lasts_a_day_and_may_run_a_little_ahead() {
    let mut request = request();
    let now = 10 * MAX_AGE_MS;
    request.signed_ms = now - MAX_AGE_MS;
    assert!(request.fresh(now));
    request.signed_ms = now - MAX_AGE_MS - 1;
    assert!(!request.fresh(now));
    request.signed_ms = now + MAX_SKEW_MS;
    assert!(request.fresh(now));
    request.signed_ms = now + MAX_SKEW_MS + 1;
    assert!(!request.fresh(now));
}

#[test]
fn relay_info_is_bounded() {
    assert!(info().valid());
    let mut none = info();
    none.relays.clear();
    assert!(!none.valid());
    let mut many = info();
    many.relays = vec!["https://relay.example.org".into(); MAX_RELAYS + 1];
    assert!(!many.valid());
    let mut long = info();
    long.identity = "x".repeat(MAX_FIELD + 1);
    assert!(!long.valid());
    assert!(!Outcome::Relayed(none).valid());
    assert!(Outcome::Kept.valid());
}

#[test]
fn host_relay_messages_keep_their_wire_names() {
    assert_eq!(
        serde_json::to_value(Request::HostRelays).unwrap(),
        json!({"request": "host_relays"})
    );
    assert_eq!(
        serde_json::to_value(Response::Relayed(Outcome::Relayed(info()))).unwrap(),
        json!({
            "response": "relayed",
            "outcome": "relayed",
            "identity": "identity",
            "relays": ["https://relay.example.org"],
            "route": "route",
        })
    );
    assert_eq!(
        serde_json::to_value(State::Done(Outcome::Failed {
            reason: "no".into()
        }))
        .unwrap(),
        json!({"state": "done", "outcome": "failed", "reason": "no"})
    );
    assert_eq!(
        serde_json::to_value(State::Pending).unwrap(),
        json!({"state": "pending"})
    );
    // A job for a phone paired with that computer directly names no host.
    let job = Job {
        device: DeviceId("near".into()),
        host: None,
        request: request(),
    };
    let value = serde_json::to_value(&job).unwrap();
    assert!(value.get("host").is_none());
    assert_eq!(serde_json::from_value::<Job>(value).unwrap(), job);
    let relayed = Request::HostRelayed {
        device: DeviceId("near".into()),
        relay: request(),
        outcome: Outcome::Elsewhere,
    };
    let value = serde_json::to_value(&relayed).unwrap();
    assert_eq!(value["outcome"], json!({"outcome": "elsewhere"}));
    assert_eq!(serde_json::from_value::<Request>(value).unwrap(), relayed);
}
