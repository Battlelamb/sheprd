#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

const PHONE: Origin = Origin::new(Device::PhoneApp, Via::Relay);
const DESKTOP: Origin = Origin::new(Device::Desktop, Via::Local);

#[test]
fn the_system_prompt_goes_once_with_the_first_prompt() {
    let mut context = Context::new(Some("Answer in French.".to_owned()));
    let note = context.before(None).unwrap();
    assert!(note.starts_with("<sheprd>\n"));
    assert!(note.ends_with("</sheprd>"));
    assert!(note.contains("Answer in French."));
    assert_eq!(context.before(None), None);
}

#[test]
fn the_origin_is_told_again_only_when_it_changes() {
    let mut context = Context::default();
    assert!(
        context
            .before(Some(DESKTOP))
            .unwrap()
            .contains("desktop app")
    );
    assert_eq!(context.before(Some(DESKTOP)), None);
    // A client that does not say where it is leaves the last origin standing.
    assert_eq!(context.before(None), None);
    assert!(context.before(Some(PHONE)).unwrap().contains("relay"));
    assert_eq!(context.before(Some(PHONE)), None);
    assert!(context.before(Some(DESKTOP)).is_some());
}

#[test]
fn a_phone_is_told_to_attach_pictures_rather_than_give_paths() {
    let text = describe(PHONE);
    assert!(text.contains("cannot open files or folders"));
    assert!(text.contains("![what it shows](/absolute/path.png)"));
    assert!(text.contains("PNG screenshot"));
    assert!(text.contains("small"));
}

#[test]
fn the_desktop_on_this_computer_may_use_paths_and_browser_tabs() {
    let text = describe(DESKTOP);
    assert!(text.contains("paths are fine"));
    assert!(text.contains("sheprd browser open"));
    let terminal = describe(Origin::new(Device::Terminal, Via::Local));
    assert!(!terminal.contains("sheprd browser open"));
}

#[test]
fn the_desktop_on_another_computer_cannot_open_paths_here() {
    let text = describe(Origin::new(Device::Desktop, Via::Remote));
    assert!(text.contains("over SSH"));
    assert!(text.contains("cannot open files or folders"));
    assert!(!text.contains("small"));
}

#[test]
fn every_connection_is_named() {
    for (via, said) in [
        (Via::Lan, "local network"),
        (Via::Tailscale, "Tailscale"),
        (Via::Relay, "relay"),
        (Via::Other, "remotely"),
    ] {
        assert!(describe(Origin::new(Device::PhoneWeb, via)).contains(said));
    }
}

#[test]
fn the_system_prompt_file_is_bounded_and_trimmed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prompt.md");
    assert_eq!(read_system_prompt(&path), None);
    std::fs::write(&path, "  \n").unwrap();
    assert_eq!(read_system_prompt(&path), None);
    std::fs::write(&path, "\nBe brief.\n").unwrap();
    assert_eq!(read_system_prompt(&path).as_deref(), Some("Be brief."));
    std::fs::write(&path, "x".repeat(MAX_SYSTEM_PROMPT as usize + 1)).unwrap();
    assert_eq!(read_system_prompt(&path), None);
    std::fs::write(&path, [0xff, 0xfe]).unwrap();
    assert_eq!(read_system_prompt(&path), None);
}
