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
    /// Once none is left, the prompts queued meanwhile go.
    pub(super) fn carry_next(&mut self) {
        let harness = self.config.harness;
        loop {
            let Some(settings) = self
                .config
                .settings
                .as_mut()
                .filter(|settings| harness.is_some_and(|agent| settings.is_for(agent)))
            else {
                if self.config.settings.take().is_some() {
                    self.next_queued();
                }
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
            if std::mem::take(&mut settings.auto_accept) {
                self.set_auto_accept(true);
                continue;
            }
            let mode = settings.mode.take();
            if let Some(mode) = mode
                && self
                    .transcript
                    .modes
                    .as_ref()
                    .is_some_and(|modes| modes.current != mode)
                // An unknown mode is refused and the session's stays.
                && self.set_mode(mode).is_ok()
            {
                // Its answer comes back here, to end the carrying.
                return;
            }
            self.config.settings = None;
            self.next_queued();
            return;
        }
    }

    /// Whether carried settings are still being set: prompts wait for
    /// them, so the first one runs with the model and mode asked for.
    pub(super) fn carrying(&self) -> bool {
        self.config.settings.is_some()
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
