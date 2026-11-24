use super::*;
use sheprd_control_protocol::protocol::tabs::AgentCapabilities;

#[test]
fn legacy_macos_hello_derives_common_capabilities() {
    let body = br#"{"ok":true,"version":1,"width":2560,"height":1600,"scale":2,
        "accessibility":false,"screen_recording":true}"#;
    let hello: Hello = parse(body).unwrap();
    assert_eq!(hello.capabilities, None);
    assert_eq!(
        hello.effective_capabilities(),
        AgentCapabilities {
            exec: true,
            screenshot: true,
            pointer: false,
            keyboard: false,
            text: false,
            desktop_ready: true,
        }
    );
}

#[test]
fn linux_hello_parses_explicit_capabilities_without_macos_permission_flags() {
    let body = br#"{"ok":true,"version":1,"width":1280,"height":800,"scale":1,
        "capabilities":{"exec":true,"screenshot":true,"pointer":true,"keyboard":true,"text":true,"desktop_ready":true}}"#;
    let hello: Hello = parse(body).unwrap();
    assert_eq!(
        hello.effective_capabilities(),
        AgentCapabilities {
            exec: true,
            screenshot: true,
            pointer: true,
            keyboard: true,
            text: true,
            desktop_ready: true,
        }
    );
}
