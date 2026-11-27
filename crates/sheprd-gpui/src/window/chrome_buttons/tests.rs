#![allow(clippy::unwrap_used)]

use super::{ACTION_HINT_DELAY, hint_label};
use crate::sidebar::layout_tests::{fixture_window, full_draw};
use gpui::{Modifiers, TestAppContext, VisualTestContext};

#[test]
fn shortcut_hints_omit_the_hint_separator_for_unbound_actions() {
    assert_eq!(hint_label("Close pane", None).as_ref(), "Close pane");
    assert_eq!(
        hint_label("Close pane", Some("ctrl-b x")).as_ref(),
        "Close pane  ·  ctrl-b x"
    );
}

#[gpui::test]
fn the_sidebar_button_waits_two_seconds_to_show_its_hint(cx: &mut TestAppContext) {
    let (_, cx) = cx.add_window_view(fixture_window);
    let draw = |cx: &mut VisualTestContext| cx.update(|window, cx| full_draw(window, cx).clear(cx));
    draw(cx);
    let button = cx.debug_bounds("toggle-sidebar").unwrap();
    cx.simulate_mouse_move(button.center(), None, Modifiers::default());
    draw(cx);
    let last_tick = std::time::Duration::from_millis(1);
    cx.executor().advance_clock(ACTION_HINT_DELAY - last_tick);
    draw(cx);
    assert!(cx.debug_bounds("hint").is_none());
    cx.executor().advance_clock(last_tick);
    draw(cx);
    assert!(cx.debug_bounds("hint").is_some());
}
