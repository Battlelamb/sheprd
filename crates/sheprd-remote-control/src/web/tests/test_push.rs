//! `POST /api/apns/test`, up to the point it would reach APNs.

use super::*;
use axum::http::{Request as HttpRequest, StatusCode, header};

fn test_post(token: Option<&str>, body: serde_json::Value) -> HttpRequest<Body> {
    let mut request =
        HttpRequest::post("/api/apns/test").header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    request.body(Body::from(body.to_string())).unwrap()
}

/// A state whose daemon has an APNs key.
fn with_apns(dir: &std::path::Path) -> Shared {
    let mut state = state(dir);
    Arc::get_mut(&mut state).unwrap().apns = Some(crate::apns::sender_for_tests());
    state
}

#[tokio::test]
async fn a_test_push_needs_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let state = with_apns(dir.path());
    let (status, _, _) = send(&state, test_post(None, json!({"id": "a"}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn without_an_apns_key_the_phone_hears_push_is_not_set_up() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let (_, token) = signed_in(&state, 1);
    let (status, _, _) = send(&state, test_post(Some(&token), json!({"id": "a"}))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn a_bad_id_or_an_unregistered_phone_is_refused_before_apns() {
    let dir = tempfile::tempdir().unwrap();
    let state = with_apns(dir.path());
    let (_, token) = signed_in(&state, 1);
    for body in [
        json!({"id": "not valid"}),
        json!({"id": "a", "extra": 1}),
        json!({}),
        json!({"id": "a"}),
    ] {
        let (status, _, text) = send(&state, test_post(Some(&token), body.clone())).await;
        assert!(status.is_client_error(), "{body}: {status} {text}");
    }
    let (_, _, text) = send(&state, test_post(Some(&token), json!({"id": "a"}))).await;
    assert!(text.contains("not registered for alerts"), "{text}");
}
