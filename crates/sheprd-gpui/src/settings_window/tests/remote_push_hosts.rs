//! Settings > Remote control > push delivery: each linked host's mode,
//! asked over SSH, under this computer's own.
use super::{
    remote_control::{draw, page},
    *,
};
use crate::{remote::Supervisor, settings_window::pages::remote_control::host_line};
use sheprd_control_protocol::protocol::remote::HostPush;
use sheprd_remote_control::control::push::{Mode, PushStatus};

/// `text` without Fluent's isolation marks around its arguments.
fn plain(text: String) -> String {
    text.replace(['\u{2068}', '\u{2069}'], "")
}

fn host(name: &str, mode: Option<Mode>, error: Option<&str>) -> HostPush {
    HostPush {
        name: name.into(),
        status: mode.map(|mode| PushStatus {
            mode,
            ..PushStatus::default()
        }),
        error: error.map(str::to_owned),
    }
}

#[gpui::test]
fn linked_hosts_show_only_once_asked(cx: &mut TestAppContext) {
    let (view, cx, _) = page(cx);
    assert!(cx.debug_bounds("remote-push-delivery-hosts").is_none());
    cx.update(|_, cx| {
        Supervisor::set_host_pushes(cx, vec![host("box", Some(Mode::PushRelay), None)]);
    });
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    assert!(cx.debug_bounds("remote-push-delivery-hosts").is_some());
}

#[test]
fn each_host_line_names_its_mode_or_why_it_is_unknown() {
    assert_eq!(
        plain(host_line(&host("box", Some(Mode::PushRelay), None))),
        "box: through the push relay."
    );
    assert_eq!(
        plain(host_line(&host("box", Some(Mode::Off), None))),
        "box: off, with no APNs key and the push relay off there."
    );
    assert_eq!(
        plain(host_line(&host("old", None, None))),
        "old: no remote control daemon answered there."
    );
    assert_eq!(
        plain(host_line(&host("far", None, Some("ssh failed")))),
        "far: could not be asked."
    );
}
