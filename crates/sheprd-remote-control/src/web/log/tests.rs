#![allow(clippy::unwrap_used)]

use super::*;
use crate::web::tests::{post, send, state};
use serde_json::json;

#[test]
fn a_report_is_cut_at_a_character_boundary() {
    assert_eq!(clip("short"), "short");
    let long = "é".repeat(MAX_REPORT_CHARS + 5);
    assert_eq!(clip(&long).chars().count(), MAX_REPORT_CHARS);
}

#[tokio::test]
async fn a_phone_reports_without_a_session_until_its_limit() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let report = || {
        post(
            "/api/log",
            json!({ "context": "pair", "message": "Load failed" }),
        )
    };
    for _ in 0..REPORT_BURST {
        assert_eq!(send(&state, report()).await.0, StatusCode::NO_CONTENT);
    }
    assert_eq!(
        send(&state, report()).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_report_must_be_a_context_and_a_message() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let (status, _, _) = send(&state, post("/api/log", json!({ "message": "x" }))).await;
    assert!(status.is_client_error());
}

#[test]
fn only_a_short_plain_call_tag_is_kept() {
    let tagged = |value: &str| {
        let mut headers = HeaderMap::new();
        headers.insert(CALL_HEADER, value.parse().unwrap());
        call_tag(&headers)
    };
    assert_eq!(tagged("3fa9c2d1").as_deref(), Some("3fa9c2d1"));
    assert_eq!(tagged(""), None);
    assert_eq!(tagged("3fa9 c2d1"), None);
    assert_eq!(tagged("a=b"), None);
    assert_eq!(tagged(&"a".repeat(MAX_CALL_CHARS + 1)), None);
    assert_eq!(call_tag(&HeaderMap::new()), None);
}

/// What `run` logs on this thread while it runs.
fn logged(run: impl FnOnce()) -> String {
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Log {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // A second live dispatcher keeps a parallel test without a subscriber
    // from caching "never" for these callsites (see `lend.rs`).
    let _other = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    let _default = tracing::subscriber::set_default(subscriber);
    run();
    String::from_utf8(log.0.lock().unwrap().clone()).unwrap()
}

fn line(status: Option<u16>) -> Line {
    Line {
        method: Method::POST,
        route: "/api/chat/{pane}/command".to_owned(),
        peer: Some(IpAddr::from([192, 168, 1, 50])),
        call: Some("3fa9c2d1".to_owned()),
        started: Instant::now(),
        status,
    }
}

#[test]
fn an_answered_request_is_logged_with_its_call() {
    let log = logged(|| drop(line(Some(200))));
    assert!(log.contains(" request "), "{log}");
    assert!(log.contains("call=\"3fa9c2d1\""), "{log}");
    assert!(log.contains("status=200"), "{log}");
}

/// A client that stops waiting drops the handler; the line still comes.
#[test]
fn a_request_dropped_before_its_answer_is_logged() {
    let log = logged(|| drop(line(None)));
    assert!(log.contains("request ended unanswered"), "{log}");
    assert!(log.contains("call=\"3fa9c2d1\""), "{log}");
    assert!(log.contains("peer=192.168.1.50"), "{log}");
}
