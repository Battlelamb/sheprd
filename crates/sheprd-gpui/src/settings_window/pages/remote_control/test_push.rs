//! Settings > Remote control's Send Test Push: the daemon sends each paired
//! iPhone registered for alerts one test alert now, through the push relay
//! (`Request::TestPush`), as the phone's own Send Test Push does, and the
//! page says what came of each. Asked on the background executor; it only
//! notifies the user's own phones.
// i18n: migrated

use super::{SettingsWindow, ui};
use crate::{
    direction::Logical,
    i18n::{Localize, fl},
    remote::Supervisor,
};
use gpui::{prelude::*, *};
use sheprd_remote_control::control::{
    self, Request,
    push::{Mode, Outcome, TestPush, TestPushes},
};

/// Where the last Send Test Push stands, until the page goes away.
#[derive(Debug, Default)]
pub(in crate::settings_window) enum TestPushState {
    #[default]
    Idle,
    Sending,
    Sent(TestPushes),
    /// Why the daemon could not be asked, as shown.
    Failed(String),
}

impl SettingsWindow {
    pub(in crate::settings_window) fn send_test_push(&mut self, cx: &mut Context<Self>) {
        if matches!(self.remote.test_push, TestPushState::Sending) {
            return;
        }
        let Some(client) = Supervisor::client(cx) else {
            self.remote.test_push =
                TestPushState::Failed(crate::remote::Error::Unexpected.localize());
            cx.notify();
            return;
        };
        let request = client.request(Request::TestPush);
        self.remote.test_push = TestPushState::Sending;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let answer = request.await;
            let _ = this.update(cx, |this, cx| {
                this.remote.test_push = match answer {
                    Ok(control::Response::TestPushed(sent)) => TestPushState::Sent(sent),
                    Ok(_) => TestPushState::Failed(crate::remote::Error::Unexpected.localize()),
                    Err(error) => {
                        tracing::warn!(%error, "Could not send a test push");
                        TestPushState::Failed(error.localize())
                    }
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// The button, while remote control and the push relay are on, and
    /// what the last test came to.
    pub(super) fn test_push_row(&self, cx: &mut Context<Self>) -> Option<Div> {
        let remote = &self.config.remote;
        if !remote.enabled || !remote.push_relay {
            return None;
        }
        let theme = &self.theme;
        let sending = matches!(self.remote.test_push, TestPushState::Sending);
        let button = ui::button(
            theme,
            "remote-test-push",
            fl!("settings-push-relay-test"),
            !sending,
        )
        .when(!sending, |button| {
            button.on_click(cx.listener(|this, _, _, cx| this.send_test_push(cx)))
        });
        let lines = test_push_lines(&self.remote.test_push);
        Some(
            div()
                .ps(px(16.))
                .pb(px(6.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .debug_selector(|| "remote-test-push-result".into())
                .child(div().flex().flex_row().child(button))
                .children(lines.into_iter().map(|line| self.control_note(line))),
        )
    }
}

/// What the page says about the last test: one line per phone, or why
/// none was sent.
pub(in crate::settings_window) fn test_push_lines(state: &TestPushState) -> Vec<String> {
    match state {
        TestPushState::Idle => Vec::new(),
        TestPushState::Sending => vec![fl!("settings-push-relay-test-sending")],
        TestPushState::Failed(error) => {
            vec![fl!(
                "settings-push-relay-test-failed",
                error = error.as_str()
            )]
        }
        TestPushState::Sent(sent) if sent.mode != Mode::PushRelay => {
            vec![fl!("settings-push-relay-test-off")]
        }
        TestPushState::Sent(sent) if sent.pushes.is_empty() => {
            vec![fl!("settings-push-relay-test-none")]
        }
        TestPushState::Sent(sent) => sent.pushes.iter().map(line).collect(),
    }
}

fn line(push: &TestPush) -> String {
    let device = crate::notifications::safe_text(&push.device, 64);
    let (outcome, status) = match &push.outcome {
        Outcome::Delivered => ("delivered", String::new()),
        Outcome::Gone => ("gone", String::new()),
        Outcome::Limited => ("limited", String::new()),
        // Digits as they are, without grouping.
        Outcome::Refused { status, .. } => ("refused", status.to_string()),
        Outcome::Unreachable => ("unreachable", String::new()),
        Outcome::Unsent => ("unsent", String::new()),
        Outcome::Other => ("other", String::new()),
    };
    fl!(
        "settings-push-relay-test-result",
        device = device.as_str(),
        outcome = outcome,
        status = status.as_str()
    )
}
