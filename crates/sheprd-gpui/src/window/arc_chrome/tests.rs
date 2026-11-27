#![allow(clippy::unwrap_used)]

use crate::{
    browser::{Pick, Store},
    config::TabLayoutMode,
    sidebar::layout_tests::full_draw,
};
use gpui::{Modifiers, TestAppContext, VisualTestContext, px};

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| full_draw(window, cx).clear(cx));
}

#[gpui::test]
fn arc_tabs_are_always_visible_at_the_top_of_the_terminal_frame(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = crate::palette::fixture_window(window, cx);
        view.config.tab_layout = TabLayoutMode::Arc;
        view.settings.shared = Some(
            crate::herdr_settings::Settings::parse_text(
                "[ui]\ntab_bar_position = 'bottom'\nhide_tab_bar_when_single_tab = true",
            )
            .unwrap(),
        );
        view
    });
    draw(cx);
    let sidebar = cx.debug_bounds("arc-sidebar").unwrap();
    let tabs = cx.debug_bounds("tab-scroller").unwrap();
    let tab = cx.debug_bounds("tab-w1:t1").unwrap();
    let frame = cx.debug_bounds("arc-terminal-frame").unwrap();
    assert!(sidebar.right() < frame.left());
    assert!(tabs.top() >= frame.top() && tabs.bottom() < frame.top() + px(56.));
    assert!(tab.left() >= tabs.left() && tab.right() <= tabs.right());
    assert!(cx.debug_bounds("arc-split-pane").is_some());

    let page = cx.update(|_, cx| {
        let (scope, workspace) = view.read(cx).browser_key().unwrap();
        Store::update(cx, |store| store.open(scope, &workspace, None, None)).unwrap()
    });
    draw(cx);
    let page_row = format!("browser-tab-{page}").leak();
    let page_row = cx.debug_bounds(page_row).unwrap().center();
    cx.simulate_click(page_row, Modifiers::default());
    assert_eq!(
        view.read_with(cx, |view, _| view
            .active_group()
            .and_then(|id| view.group_pick(id))),
        Some(Pick::Page(page))
    );

    let search = cx.debug_bounds("arc-search").unwrap().center();
    cx.simulate_click(search, Modifiers::default());
    assert_eq!(
        view.read_with(cx, |view, _| view.menu.page),
        Some(crate::menu::Page::Palette)
    );
}

#[gpui::test]
fn the_configured_arc_palette_shortcut_opens_the_real_palette(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = crate::palette::fixture_window(window, cx);
        view.config.tab_layout = TabLayoutMode::Arc;
        let overrides = toml::from_str("command_palette = ['ctrl-t', 'ctrl-shift-p']").unwrap();
        view.config.keybindings = crate::keymap::Keymap::with_overrides(
            &overrides,
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        cx.set_global(crate::app::InitialAppearance {
            config: view.config.clone(),
            ..Default::default()
        });
        crate::bind_keys(cx);
        window.focus(&view.focus, cx);
        view
    });
    draw(cx);
    assert_eq!(
        view.read_with(cx, |view, _| view
            .action_shortcut(crate::controls::Command::Palette)),
        Some("ctrl-t".into())
    );
    cx.simulate_keystrokes("ctrl-t");
    assert_eq!(
        view.read_with(cx, |view, _| view.menu.page),
        Some(crate::menu::Page::Palette)
    );
}

#[gpui::test]
fn the_arc_rail_peeks_then_pins_without_resizing_on_hover(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = crate::palette::fixture_window(window, cx);
        view.config.tab_layout = TabLayoutMode::Arc;
        view.toggle_sidebar();
        view
    });
    draw(cx);
    let rail = cx.debug_bounds("sidebar-rail").unwrap();
    let frame = cx.debug_bounds("arc-terminal-frame").unwrap();
    assert!(cx.debug_bounds("tab-w1:t1").is_some());
    cx.simulate_mouse_move(rail.center(), None, Modifiers::default());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    draw(cx);
    assert!(cx.debug_bounds("arc-search").is_some());
    assert!(cx.debug_bounds("tab-w1:t1").is_some());
    assert_eq!(cx.debug_bounds("arc-terminal-frame"), Some(frame));
    let toggle = cx.debug_bounds("toggle-sidebar").unwrap().center();
    cx.simulate_click(toggle, Modifiers::default());
    draw(cx);
    assert!(view.read_with(cx, |view, _| view.sidebar_visible));
    assert!(cx.debug_bounds("arc-terminal-frame").unwrap().left() > frame.left());
}

#[gpui::test]
fn hiding_the_right_pane_leaves_tabs_available(cx: &mut TestAppContext) {
    let (_view, cx) = cx.add_window_view(|window, cx| {
        let mut view = crate::palette::fixture_window(window, cx);
        view.config.tab_layout = TabLayoutMode::Arc;
        view.mods.set_panel_hidden(true);
        view
    });
    draw(cx);
    assert!(cx.debug_bounds("mod-panel-rail").is_none());
    assert!(cx.debug_bounds("tab-scroller").is_some());
    assert!(cx.debug_bounds("tab-w1:t1").is_some());
}
