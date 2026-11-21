//! A chat the phone follows in the background stays current without
//! counting as shown, so what finishes in it still waits for the person.

use super::{live_chat::linked, *};
use sheprd_chat_protocol::Cursor;
use std::time::Duration;
use wire::View;

fn chat(background: bool) -> ClientMessage {
    ClientMessage::Subscribe {
        view: View::Chat { pane: "p1".into() },
        since: None,
        size: None,
        background,
    }
}

/// Waits until whether `p1` is shown is `shown`.
async fn shown(session: &Session, shown: bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while session.state.opened.watched("p1") != shown {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("p1 never became shown: {shown}"));
}

#[tokio::test]
async fn a_background_chat_is_followed_but_not_shown() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _burst) = linked(dir.path(), Cursor::new_epoch()).await;

    session.send(chat(true)).await;
    assert!(matches!(
        session.next().await,
        ServerMessage::ChatSnapshot(_)
    ));
    assert!(!session.state.opened.watched("p1"));

    // Opened on a screen, then left to the background again.
    session.send(chat(false)).await;
    assert!(matches!(
        session.next().await,
        ServerMessage::ChatSnapshot(_)
    ));
    shown(&session, true).await;
    session.send(chat(true)).await;
    assert!(matches!(
        session.next().await,
        ServerMessage::ChatSnapshot(_)
    ));
    shown(&session, false).await;
}

#[test]
fn background_is_sent_only_when_set() {
    let text = serde_json::to_value(chat(true)).unwrap();
    assert_eq!(text["background"], true);
    let text = serde_json::to_value(chat(false)).unwrap();
    assert!(text.get("background").is_none());
}
