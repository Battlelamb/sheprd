use super::*;
use gpui::VisualTestContext;

fn remote(id: &str, view: &SheprdWindow) -> crate::endpoint::Endpoint {
    let mut remote = crate::endpoint::Endpoint::new(
        id.into(),
        "Remote".into(),
        ConnectTarget::Ssh {
            target: "example.invalid".into(),
            session: "default".into(),
        },
        true,
    );
    remote.live.snapshot = view.live.snapshot.clone();
    remote
}

/// A window with Local and `remotes` more hosts, drawn.
fn open<'a>(
    cx: &'a mut gpui::TestAppContext,
    remotes: &[&str],
) -> (Entity<SheprdWindow>, &'a mut VisualTestContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = fixture_window(window, cx);
        for id in remotes {
            let endpoint = remote(id, &view);
            view.endpoints.push(endpoint);
        }
        view
    });
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.run_until_parked();
    cx.update(|window, cx| full_draw(window, cx).clear(cx));
    (view, cx)
}

fn collapsed(view: &Entity<SheprdWindow>, cx: &mut VisualTestContext) -> Vec<bool> {
    view.read_with(cx, |view, _| {
        view.endpoints.iter().map(|e| e.collapsed).collect()
    })
}

fn click(cx: &mut VisualTestContext, selector: &'static str, modifiers: Modifiers) {
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), modifiers);
    cx.update(|window, cx| full_draw(window, cx).clear(cx));
}

#[gpui::test]
fn the_heading_button_collapses_every_host_then_expands_them(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open(cx, &["ssh:a", "ssh:b"]);
    let heading = cx.debug_bounds("header-spaces").unwrap();
    let button = cx.debug_bounds("sidebar-fold-hosts").unwrap();
    assert!(
        heading.right() <= button.left(),
        "the button follows the heading"
    );
    click(cx, "sidebar-fold-hosts", Modifiers::default());
    assert_eq!(collapsed(&view, cx), [true; 3]);
    assert!(cx.debug_bounds("host-ssh:b").is_some(), "headers stay");
    click(cx, "sidebar-fold-hosts", Modifiers::default());
    assert_eq!(collapsed(&view, cx), [false; 3]);
}

#[gpui::test]
fn a_partly_collapsed_list_collapses_the_rest_first(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open(cx, &["ssh:a"]);
    click(cx, "collapse-host-local", Modifiers::default());
    assert_eq!(collapsed(&view, cx), [true, false]);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.command(crate::controls::Command::ToggleAllHosts, window, cx);
        });
    });
    assert_eq!(collapsed(&view, cx), [true, true]);
}

#[gpui::test]
fn alt_clicking_a_host_arrow_folds_every_host(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open(cx, &["ssh:a", "ssh:b"]);
    click(cx, "collapse-host-local", Modifiers::alt());
    assert_eq!(collapsed(&view, cx), [true; 3]);
}

#[gpui::test]
fn a_single_host_offers_no_button(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open(cx, &[]);
    assert!(cx.debug_bounds("sidebar-fold-hosts").is_none());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.command(crate::controls::Command::ToggleAllHosts, window, cx);
        });
    });
    assert_eq!(collapsed(&view, cx), [false]);
}
