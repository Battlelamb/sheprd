//! The `[remote] push_relay` setting reaches the daemon as `--push-relay`,
//! and leaves the service definition alone while off.

use super::*;

#[test]
fn the_push_relay_setting_becomes_a_flag() {
    let mut plan = plan();
    assert!(!words(&plan.args()).contains(&"--push-relay"));
    plan.config.push_relay = true;
    assert!(words(&plan.args()).contains(&"--push-relay"));
}

#[test]
fn the_config_file_turns_it_on() {
    let config: RemoteConfig = toml::from_str("push_relay = true").unwrap();
    assert!(config.push_relay);
    assert!(!RemoteConfig::default().push_relay);
}
