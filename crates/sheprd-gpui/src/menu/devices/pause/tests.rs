use crate::sidebar::layout_tests::{fixture_window, full_draw};
use gpui::{TestAppContext, px, size};
use sheprd_client::ConnectTarget;

const HOST: &str = "ssh:0123456789abcdef0123456789abcdef";
const WSL: &str = "wsl:Ubuntu";

fn add_devices(view: &mut crate::SheprdWindow, enabled: bool) {
    view.endpoints[0].connection.target = ConnectTarget::Local;
    view.endpoints[0].collapsed = true;
    view.endpoints.push(crate::endpoint::Endpoint::new(
        HOST.into(),
        "studio".into(),
        ConnectTarget::Ssh {
            target: "penso@studio".into(),
            session: "default".into(),
        },
        enabled,
    ));
    view.endpoints.push(crate::endpoint::Endpoint::new(
        WSL.into(),
        "Ubuntu".into(),
        ConnectTarget::Wsl {
            distro: "Ubuntu".into(),
            session: "default".into(),
        },
        true,
    ));
}

#[gpui::test]
fn only_saved_ssh_and_cloud_devices_pause(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = fixture_window(window, cx);
        add_devices(&mut view, true);
        view
    });
    view.update(cx, |view, cx| {
        assert!(view.device_pausable(HOST));
        assert!(!view.device_pausable(crate::endpoint::LOCAL));
        assert!(!view.device_pausable(WSL));
        assert!(!view.device_pausable("ssh:missing"));
        assert!(!view.device_paused(HOST));
        // Nothing runs for a device that cannot pause.
        assert!(!view.set_device_paused(WSL, true, cx));
        assert!(!view.set_device_paused(crate::endpoint::LOCAL, true, cx));
        // A development catalog is not the one `herdr machine` edits.
        view.endpoints[0].connection.target = ConnectTarget::Session {
            name: "default".into(),
            development: true,
        };
        assert!(!view.device_pausable(HOST));
    });
}

#[gpui::test]
fn a_paused_device_stays_listed_and_says_so(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = fixture_window(window, cx);
        add_devices(&mut view, false);
        view
    });
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.update(|window, cx| full_draw(window, cx).clear(cx));
    view.read_with(cx, |view, _| assert!(view.device_paused(HOST)));
    assert!(
        cx.debug_bounds("host-ssh:0123456789abcdef0123456789abcdef")
            .is_some()
    );
    assert!(
        cx.debug_bounds("host-working-ssh:0123456789abcdef0123456789abcdef")
            .is_some()
    );
    // A running device has no such word in the Workspaces layout.
    view.update(cx, |view, _| view.endpoints[1].enabled = true);
    cx.update(|window, cx| full_draw(window, cx).clear(cx));
    assert!(
        cx.debug_bounds("host-working-ssh:0123456789abcdef0123456789abcdef")
            .is_none()
    );
}
