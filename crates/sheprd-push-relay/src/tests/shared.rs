use super::*;
use crate::relay::Reply;
use serde_json::Value;

const GROUP: &str = "1234567890abcdef1234567890abcdef";

fn scoped(group: &str, slot: &str, kind: &str, byte: u8) -> String {
    seal_to(
        &relay_key(),
        &json!({
            "credential": issuer().issue(PHONE, Host::Production, NOW_S),
            "id": format!("{byte:02x}").repeat(16), "token": token_hex(byte),
            "group": group, "slot": slot, "kind": kind
        }),
        seal::HANDLE_INFO,
    )
}

fn report(revision: u64, end: bool, question: bool) -> Value {
    json!({"revision": revision, "run": 1, "end": end, "question": question,
        "done": u32::from(end), "total": 1, "state": {"sealed":"opaque"},
        "latest": end.then(|| json!({"sealed":"finished"}))})
}

async fn send(f: &Fixture, handle: &str, payload: Value) -> Reply {
    let mut push = request(handle.into(), PushType::LiveActivity);
    push.payload = payload.to_string();
    f.relay.push(source(), push, NOW_MS).await
}

fn last(f: &Fixture) -> Value {
    let sent = f.apple.sent();
    serde_json::from_slice::<Value>(&sent.last().unwrap().1.body).unwrap()["aps"].clone()
}

#[tokio::test]
async fn overlapping_hosts_flush_one_question_then_one_end_to_the_late_token() {
    let f = fixture();
    let a = "a".repeat(32);
    let b = "b".repeat(32);
    let first = scoped(GROUP, &a, "push_to_start", 1);
    let second = scoped(GROUP, &b, "push_to_start", 2);
    let (left, right) = tokio::join!(
        send(&f, &first, report(1, false, false)),
        send(&f, &second, report(1, false, true))
    );
    assert_eq!((left, right), (Reply::Delivered, Reply::Delivered));
    let start = last(&f);
    let card = start["attributes"]["shared"]["card"].as_str().unwrap();
    assert_eq!(start["event"], "start");
    assert!(!start.to_string().contains(GROUP));
    assert_eq!(
        send(&f, &first, report(2, true, false)).await,
        Reply::Delivered
    );
    assert_eq!(
        send(&f, &first, report(1, false, false)).await,
        Reply::Delivered
    );
    assert_eq!(f.apple.sent().len(), 1);
    let token = scoped(GROUP, card, "activity", 3);
    assert_eq!(send(&f, &token, json!({})).await, Reply::Delivered);
    let update = last(&f);
    let state = &update["content-state"]["shared"];
    assert_eq!(update["event"], "update");
    assert_eq!(state["slot"], b);
    assert_eq!(state["latest"]["slot"], a);
    assert_eq!(
        (state["done"].as_u64(), state["total"].as_u64()),
        (Some(1), Some(2))
    );
    assert_eq!(
        send(&f, &second, report(2, true, false)).await,
        Reply::Delivered
    );
    assert_eq!(last(&f)["event"], "end");
    assert_eq!(f.apple.sent().last().unwrap().0, super::token(3));
    assert_eq!(
        send(&f, &second, report(2, true, false)).await,
        Reply::Delivered
    );
    assert_eq!(send(&f, &token, json!({})).await, Reply::Gone);
    assert_eq!(f.apple.sent().len(), 3);
}

#[tokio::test]
async fn retries_reuse_the_start_nonce_and_a_finish_waits_for_the_right_generation() {
    let f = fixture();
    let slot = "a".repeat(32);
    let host = scoped(GROUP, &slot, "push_to_start", 1);
    f.apple.answer(Ok(Outcome::RateLimited));
    assert_eq!(
        send(&f, &host, report(1, false, false)).await,
        Reply::Limited
    );
    let first = last(&f);
    assert_eq!(
        send(&f, &host, report(1, false, false)).await,
        Reply::Delivered
    );
    assert_eq!(first["attributes"], last(&f)["attributes"]);
    assert_eq!(
        send(&f, &host, report(2, true, false)).await,
        Reply::Delivered
    );
    let nonce = first["attributes"]["shared"]["card"].as_str().unwrap();
    let wrong = scoped(&"f".repeat(32), nonce, "activity", 2);
    assert_eq!(send(&f, &wrong, json!({})).await, Reply::Gone);
    let right = scoped(GROUP, nonce, "activity", 3);
    assert_eq!(send(&f, &right, json!({})).await, Reply::Delivered);
    assert_eq!(last(&f)["event"], "end");
    assert_eq!(
        send(&f, &host, report(3, false, false)).await,
        Reply::Delivered
    );
    assert_ne!(first["attributes"], last(&f)["attributes"]);
    assert_eq!(send(&f, &right, json!({})).await, Reply::Gone);
}
