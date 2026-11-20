//! The settings carried from the chat that asked for this one
//! ([`sheprd_chat_protocol::Settings`]), set once the agent's new session
//! opens.

use sheprd_chat_protocol::{ConfigOption, OptionKind, OptionValue, Setting};

use super::{Session, Update};

impl<R: FnMut(Update)> Session<R> {
    /// Sends the next carried setting: one at a time, each after the agent
    /// answered the one before, so it is checked against the settings that
    /// one left, as a model's thinking levels follow the model. A setting
    /// the agent does not offer, or already has, is skipped; auto-accepting
    /// and the mode come last. Settings read from another agent are dropped.
    pub(super) fn carry_next(&mut self) {
        let harness = self.config.harness;
        loop {
            let Some(settings) = self
                .config
                .settings
                .as_mut()
                .filter(|settings| harness.is_some_and(|agent| settings.is_for(agent)))
            else {
                self.config.settings = None;
                return;
            };
            if !settings.options.is_empty() {
                let Setting { id, value } = settings.options.remove(0);
                let has = self
                    .transcript
                    .options
                    .iter()
                    .any(|option| option.id == id && holds(option, &value));
                // A setting the agent does not take is refused here and
                // left as the new session has it.
                if !has && self.set_option(id, &value).is_ok() {
                    return;
                }
                continue;
            }
            let Some(settings) = self.config.settings.take() else {
                return;
            };
            if settings.auto_accept {
                self.set_auto_accept(true);
            }
            if let Some(mode) = settings.mode
                && self
                    .transcript
                    .modes
                    .as_ref()
                    .is_some_and(|modes| modes.current != mode)
            {
                // An unknown mode is refused and the session's stays.
                let _ = self.set_mode(mode);
            }
            return;
        }
    }
}

/// Whether `option` is set to `value` now.
fn holds(option: &ConfigOption, value: &OptionValue) -> bool {
    match (&option.kind, value) {
        (OptionKind::Select { current, .. }, OptionValue::Choice { id }) => current == id,
        (OptionKind::Toggle { current }, OptionValue::Toggle { value }) => current == value,
        _ => false,
    }
}
