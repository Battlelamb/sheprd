use crate::{
    config::{Density, LayoutMode, Style},
    sidebar::layout_tests::{PaintedProbes, fixture_window, full_draw},
    smoke::sidebar_fixture::sidebar_fixture_size,
};

#[gpui::test]
fn smallest_sidebar_fixture_keeps_probed_rows_visible(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = fixture_window(window, cx);
        view.config.sidebar_style.activity_bar = true;
        view.config.show_window_controls = true;
        view
    });
    for style in [Style::Flat, Style::Rounded] {
        for density in [Density::Comfortable, Density::Normal, Density::Compact] {
            let size = cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.config.layout.mode = LayoutMode::new(density, style);
                    cx.notify();
                });
                sidebar_fixture_size(view.read(cx), window, 640., 400., style)
            });
            cx.simulate_resize(size);
            cx.run_until_parked();
            cx.update(|window, cx| {
                cx.default_global::<PaintedProbes>().0.clear();
                full_draw(window, cx).clear(cx);
                let probes = &cx.global::<PaintedProbes>().0;
                for input in ["herdr", "Claude Code", "agent", "1256789"] {
                    if density == Density::Compact && input == "1256789" {
                        assert!(!probes.contains_key(input));
                        continue;
                    }
                    let probe = &probes[input];
                    assert!(
                        probe.bounds.top() >= probe.mask.top()
                            && probe.bounds.bottom() <= probe.mask.bottom(),
                        "{style:?}/{density:?} cropped {input}: {probe:?}"
                    );
                }
            });
        }
    }
}
