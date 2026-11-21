//! Through the relay, activities go to handles; tokens only a key reaches
//! are skipped, and nothing waits for one.

use super::*;
use serde_json::json;

fn relay() -> Delivery {
    Delivery::Relay(Box::new(crate::apns::Relays::new().unwrap()))
}

fn handle(byte: char) -> serde_json::Value {
    json!({"handle": format!("1a2b3c4d.{}", byte.to_string().repeat(160)), "environment": "sandbox"})
}

#[test]
fn the_relay_sends_to_handles() {
    let relay = relay();
    let registered = registration(
        &json!({
            "push_to_start": handle('A'),
            "activities": [{"pane": "w1-p3", "token": handle('B')}],
        })
        .to_string(),
    );
    let end = live(Event::End { dismissal_s: 0 });
    let Route::Send(Recipient::Handle(sent)) = route(&end, &registered, &relay) else {
        panic!("expected a send to a handle");
    };
    assert_eq!(
        sent.secret().expose_secret(),
        format!("1a2b3c4d.{}", "B".repeat(160))
    );

    // Without the activity's handle yet, the end waits for it.
    let start_only = registration(&json!({"push_to_start": handle('A')}).to_string());
    assert_eq!(route(&end, &start_only, &relay), Route::Hold);
}

#[test]
fn the_relay_skips_tokens_only_a_key_reaches() {
    let tokens = registration(
        &json!({
            "push_to_start": "ab".repeat(32),
            "activities": [{"pane": "w1-p3", "token": "cd".repeat(40)}],
        })
        .to_string(),
    );
    let end = live(Event::End { dismissal_s: 0 });
    assert_eq!(route(&end, &tokens, &relay()), Route::Skip);
    // And the other way round.
    let handles = registration(&json!({"push_to_start": handle('A')}).to_string());
    let own_key = Delivery::OwnKey(Box::new(crate::apns::sender_for_tests()));
    assert_eq!(route(&end, &handles, &own_key), Route::Skip);
}
