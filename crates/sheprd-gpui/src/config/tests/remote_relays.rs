use super::*;
use crate::config::remote::Relay;
use sheprd_remote_control::relay_label::{self, RelayLabel};

#[test]
fn a_relay_is_an_address_or_a_labelled_table() -> anyhow::Result<()> {
    let config = Config::parse(
        "[remote]\nrelays = [\n  'https://sfo3.example.org',\n  \
         { endpoint = 'https://fra3.example.org', name = 'fra3', \
         description = 'not for public use', tags = ['eu', 'private'] },\n  \
         { endpoint = 'https://ams3.example.org', name = 'ams3' },\n]\n",
    )?;
    assert!(config.unknown_keys.is_empty(), "{:?}", config.unknown_keys);
    assert_eq!(
        config.remote.relays,
        [
            Relay::from("https://sfo3.example.org"),
            Relay {
                endpoint: "https://fra3.example.org".into(),
                label: RelayLabel {
                    description: Some("not for public use".into()),
                    name: Some("fra3".into()),
                    tags: vec!["eu".into(), "private".into()],
                },
            },
            Relay {
                endpoint: "https://ams3.example.org".into(),
                label: RelayLabel {
                    name: Some("ams3".into()),
                    ..RelayLabel::default()
                },
            },
        ]
    );
    Ok(())
}

#[test]
fn a_relay_label_the_daemon_cannot_use_is_refused() {
    let Err(error) =
        Config::parse("[remote]\nrelays = [{ endpoint = 'https://fra3.example.org', name = '' }]")
    else {
        panic!("a blank name loaded");
    };
    assert!(
        matches!(
            &error,
            crate::Error::RemoteControl(crate::remote::Error::RelayLabel {
                relay,
                source: relay_label::Error::Blank { field: "name" },
            }) if relay == "https://fra3.example.org"
        ),
        "{error:?}"
    );
    for text in [
        // A table needs its address.
        "relays = [{ name = 'fra3' }]",
        // A misspelt key would otherwise drop the label without a word.
        "relays = [{ endpoint = 'https://fra3.example.org', tag = 'eu' }]",
        "relays = [{ endpoint = 'https://fra3.example.org', tags = 'eu' }]",
        "relays = [7]",
    ] {
        assert!(
            Config::parse(&format!("[remote]\n{text}")).is_err(),
            "{text} loaded"
        );
    }
}
