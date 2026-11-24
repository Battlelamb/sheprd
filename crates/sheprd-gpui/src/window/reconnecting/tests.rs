//! Which surface carries a Tailscale check: the prompt in the overlay layer is
//! the one that shows whether or not a terminal exists, so the card over the
//! terminal stands down while a check is pending. The check also reaches the
//! window as a sentence rather than as a typed failure — the transport reports
//! `PermissionDenied` — and a surface reading only the failure would offer
//! nothing for it, which is what these tests pin.

#![allow(clippy::unwrap_used)]

// Named imports, not `super::*`: the parent pulls `gpui` in with a glob, and a
// glob-imported macro of the same name would take a plain `#[test]` over.
use super::SheprdWindow;
use crate::{LiveState, sidebar::layout_tests, state::ConnectionStatus};
use gpui::TestAppContext;
use sheprd_client::{
    SshFailure, TailscaleCheck,
    protocol::{FrameData, PaneSurfaceFrame},
};
use std::sync::Arc;

const URL: &str = "https://login.tailscale.com/a/o84652-335960037";

/// The sentence a `PermissionDenied` refusal leaves, in the shape the client
/// writes it: the address sits mid-line, with advice after it.
fn refusal_sentence() -> String {
    format!(
        "Tailscale SSH wants you to verify your identity: open {URL} to authenticate, then try again"
    )
}

/// A connection with a frame on screen, and then lost: the window keeps the
/// picture, dimmed, which is what puts the card over the terminal at all.
fn lose_a_presented_frame(view: &mut SheprdWindow) {
    let snapshot = Arc::new(layout_tests::snapshot(1));
    let mut live = LiveState::default();
    live.status = ConnectionStatus::Connected;
    live.surface = Some(Arc::new(PaneSurfaceFrame {
        boot_id: snapshot.boot_id.clone(),
        projection_revision: snapshot.revision,
        surface_revision: snapshot.revision,
        frame: FrameData {
            cells: Vec::new(),
            width: 0,
            height: 0,
            cursor: None,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        },
        panes: Vec::new(),
        splits: Vec::new(),
        popup: None,
        graphics: Default::default(),
    }));
    live.snapshot = Some(snapshot);
    view.presentation.frame(&live);
    view.presentation.hold();
}

fn pending(view: &mut SheprdWindow, failure: Option<SshFailure>, error: Option<String>) {
    let endpoint = &mut view.endpoints[0];
    endpoint.live.ssh_failure = failure;
    endpoint.live.error = error;
    endpoint.observe_tailscale();
}

#[gpui::test]
fn a_pending_check_has_the_prompt_and_not_the_card(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(layout_tests::fixture_window);
    view.update(cx, |view, _| {
        lose_a_presented_frame(view);
        assert!(view.reconnecting(), "a lost picture puts the card up");
        pending(
            view,
            Some(SshFailure::Tailscale(
                TailscaleCheck::from_address(URL).unwrap(),
            )),
            None,
        );
    });
    view.update(cx, |view, cx| {
        assert!(view.awaiting_tailscale());
        assert!(
            view.render_reconnecting(cx).is_none(),
            "the card must not be a second, dimmer copy of the prompt"
        );
        assert!(
            view.render_tailscale_prompt().is_some(),
            "the address is the one thing the user can act on"
        );
    });
}

/// The same check, arriving the way the transport actually reports it.
#[gpui::test]
fn a_check_the_transport_reported_as_text_is_still_a_check(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(layout_tests::fixture_window);
    view.update(cx, |view, _| pending(view, None, Some(refusal_sentence())));
    view.update(cx, |view, _| {
        assert!(view.awaiting_tailscale(), "the sentence names a check");
        assert!(view.render_tailscale_prompt().is_some());
    });
}

/// Nothing waits: the card is free to say why the host dropped, and the prompt
/// stays out of the way.
#[gpui::test]
fn without_a_check_the_card_speaks_for_itself(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(layout_tests::fixture_window);
    view.update(cx, |view, _| lose_a_presented_frame(view));
    view.update(cx, |view, cx| {
        assert!(!view.awaiting_tailscale());
        assert!(view.render_reconnecting(cx).is_some());
        assert!(view.render_tailscale_prompt().is_none());
    });
}
