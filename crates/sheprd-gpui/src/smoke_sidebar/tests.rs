use crate::{
    config::{Density, LayoutMode, Style},
    sidebar::layout_tests::{PaintedProbes, fixture_window, full_draw},
    smoke::sidebar_fixture::{GLYPH_SIZES, glyph_fixture_size},
};
use gpui::TestAppContext;

#[gpui::test]
fn glyph_fixture_keeps_probed_rows_inside_both_lists(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(fixture_window);
    for style in [Style::Flat, Style::Rounded] {
        for density in [Density::Comfortable, Density::Normal, Density::Compact] {
            for &(width, height) in GLYPH_SIZES {
                let size = cx.update(|window, cx| {
                    view.update(cx, |view, cx| {
                        view.config.layout.mode = LayoutMode::new(density, style);
                        cx.notify();
                        glyph_fixture_size(view, window, width, height, style)
                    })
                });
                cx.simulate_resize(size);
                cx.update(|window, cx| full_draw(window, cx).clear(cx));
                cx.update(|window, cx| {
                    cx.default_global::<PaintedProbes>().0.clear();
                    full_draw(window, cx).clear(cx);
                    let probes = &cx.global::<PaintedProbes>().0;
                    for label in [
                        "herdr",
                        "main",
                        "Claude Code",
                        "agent",
                        "1256789",
                        "herdr-gpui-sidebar-rendering-regression-investigation",
                        "fix/sidebar-label-width-and-overflow-regression",
                    ] {
                        if density == Density::Compact
                            && matches!(
                                label,
                                "main"
                                    | "1256789"
                                    | "fix/sidebar-label-width-and-overflow-regression"
                            )
                        {
                            assert!(!probes.contains_key(label));
                            continue;
                        }
                        let probe = probes
                            .get(label)
                            .unwrap_or_else(|| panic!("missing {label}"));
                        assert!(
                            probe.mask.top() <= probe.bounds.top()
                                && probe.mask.bottom() >= probe.bounds.bottom(),
                            "{style:?}/{density:?} at {size:?}: {label}: {probe:?}"
                        );
                    }
                });
            }
        }
    }
}
