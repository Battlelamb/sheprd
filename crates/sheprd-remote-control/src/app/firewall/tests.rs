use super::*;

#[test]
fn firewall_state_is_read_from_its_number() {
    assert!(firewall_on("Firewall is enabled. (State = 1)\n"));
    assert!(firewall_on(
        "Firewall is blocking all non-essential. (State = 2)"
    ));
    assert!(!firewall_on("Firewall is disabled. (State = 0)\n"));
}

#[test]
fn unknown_firewall_wording_is_taken_as_off() {
    assert!(!firewall_on(""));
    assert!(!firewall_on("Firewall is enabled."));
    assert!(!firewall_on("(State = on)"));
    assert!(!firewall_on("(State = 1"));
}

#[test]
fn a_developer_id_signature_has_a_team() {
    let text = "Executable=/Applications/Herdr.app/Contents/MacOS/sheprd-remote-control\n\
                Authority=Developer ID Application: Someone (VJAHQVZ96V)\n\
                TeamIdentifier=VJAHQVZ96V\n";
    assert!(has_team(text));
}

#[test]
fn ad_hoc_and_unsigned_builds_have_no_team() {
    let ad_hoc = "CodeDirectory v=20400 size=168584 flags=0x20002(adhoc,linker-signed)\n\
                  Signature=adhoc\n\
                  TeamIdentifier=not set\n";
    assert!(!has_team(ad_hoc));
    assert!(!has_team(
        "sheprd-remote-control: code object is not signed at all\n"
    ));
    assert!(!has_team("TeamIdentifier=\n"));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn approval_lasts_without_a_macos_firewall() {
    assert_eq!(
        approval(Path::new("/nowhere/sheprd-remote-control")).unwrap(),
        Approval::Lasting
    );
}
