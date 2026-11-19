//! Plain hosts are shared and kept current only over sessions that check
//! the host key strictly.

use super::*;

fn ssh(target: &str) -> ConnectTarget {
    ConnectTarget::Ssh {
        target: target.into(),
        session: "default".into(),
    }
}

#[test]
fn a_machine_whose_key_is_not_checked_is_refused() {
    let target = "deploy@unchecked-plain.example";
    sheprd_client::set_host_keys(target, sheprd_client::HostKeys::Skip);
    let refused = crate::fleet::plain::strict_target(&ssh(target));
    sheprd_client::set_host_keys(target, sheprd_client::HostKeys::Verify);
    assert!(matches!(refused, Err(Error::Unchecked)));
    assert_eq!(
        crate::fleet::plain::strict_target(&ssh(target)).unwrap(),
        target
    );
}

#[test]
fn only_an_ssh_machine_can_be_a_plain_host() {
    assert!(matches!(
        crate::fleet::plain::strict_target(&ConnectTarget::Local),
        Err(Error::Unsupported)
    ));
}

#[test]
fn a_machine_whose_key_is_not_checked_is_asked_rarely() {
    let mine = vec!["h1".to_owned()];
    let now = Instant::now() + Duration::from_secs(7200);
    let link = Link {
        standing: Standing::Unchecked,
        last: Some(now - Duration::from_secs(600)),
        ..Link::default()
    };
    assert!(!link.due(&mine, now));
}
