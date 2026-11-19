use super::*;
use crate::control::fleet::{
    Method,
    sync::{Answer, Apply, Ask},
};

#[test]
fn fleet_requests_keep_their_wire_names() {
    assert_eq!(
        serde_json::to_value(Request::Fleet(Box::new(Ask::Status))).unwrap(),
        json!({"request": "fleet", "ask": "status"})
    );
    assert_eq!(
        serde_json::to_value(Request::Fleet(Box::new(Ask::Admit {
            bundle: "b".into(),
            signature: "s".into(),
            method: Method::Bluetooth,
        })))
        .unwrap(),
        json!({"request": "fleet", "ask": "admit", "bundle": "b", "signature": "s", "method": "bluetooth"})
    );
    let apply: Request = serde_json::from_value(json!({
        "request": "fleet", "ask": "apply", "heads": ["h"], "ops": []
    }))
    .unwrap();
    assert_eq!(
        apply,
        Request::Fleet(Box::new(Ask::Apply(Apply {
            fleet: None,
            heads: vec!["h".into()],
            ops: Vec::new(),
        })))
    );
    assert_eq!(
        serde_json::to_value(Response::Fleet(Box::new(Answer::Done))).unwrap(),
        json!({"response": "fleet", "answer": "done"})
    );
    assert_eq!(
        serde_json::to_value(Response::Fleet(Box::new(Answer::Status { view: None }))).unwrap(),
        json!({"response": "fleet", "answer": "status"})
    );
}
