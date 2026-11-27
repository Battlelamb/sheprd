use super::commands::{id, window_with_split};
use crate::sidebar::layout_tests::full_draw;
use gpui::{Modifiers, TestAppContext};

#[gpui::test]
fn the_split_close_button_only_closes_the_pane_it_is_on(cx: &mut TestAppContext) {
    check_split_close(cx, crate::config::TabLayoutMode::Classic);
}

#[gpui::test]
fn the_arc_split_close_button_only_closes_the_pane_it_is_on(cx: &mut TestAppContext) {
    check_split_close(cx, crate::config::TabLayoutMode::Arc);
}

fn check_split_close(cx: &mut TestAppContext, layout: crate::config::TabLayoutMode) {
    let (view, cx, tab) = window_with_split(cx, &["t1", "t2"]);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.config.tab_layout = layout;
            view.config.confirm_close_pane = false;
            // The fixture announcement covers the right pane's close button.
            std::sync::Arc::make_mut(view.live.snapshot.as_mut().unwrap()).product_announcement =
                None;
            view.show_browser_tab(tab, window, cx);
        });
        full_draw(window, cx).clear(cx);
    });
    let selector = format!("close-core-split-{}-t2", u64::from(tab)).leak();
    let pane = view.read_with(cx, |view, cx| {
        view.core_terminals.views[&(tab, id("t2"))]
            .read(cx)
            .bounds()
            .unwrap()
    });
    assert!(cx.debug_bounds(selector).is_none());
    cx.simulate_mouse_move(pane.center(), None, Modifiers::default());
    cx.update(|window, cx| full_draw(window, cx).clear(cx));
    let button = cx.debug_bounds(selector).unwrap().center();
    cx.simulate_click(button, Modifiers::default());
    view.read_with(cx, |view, _| {
        let shown = &view.core_terminals.tabs[&tab];
        assert_eq!(shown.panes.count(), 1);
        assert_eq!(shown.panes.first(), &id("t1"));
    });
}
