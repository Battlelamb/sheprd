//! The question before borrowing a virtual Mac on another computer: it
//! takes that computer's memory, CPU, and one of its two slots, so the user
//! answers in the app's own dialog, as for other outward actions. An agent
//! cannot answer it; `--host` only asks. Closing it unanswered is a no.

use crate::{
    SheprdWindow,
    i18n::fl,
    menu::Page,
    notifications::safe_text,
    remote::{DialogTone as Tone, dialog_button as button},
};
use gpui::{prelude::*, *};
use std::sync::{Arc, Mutex, PoisonError};

/// What the user chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Borrow this time.
    Once,
    /// Borrow, and from this host from now on without asking.
    Always,
    /// Wait for a slot on this computer instead.
    WaitHere,
    /// Closed without an answer.
    Dismissed,
}

/// Where the answer lands, for the request that asked.
#[derive(Clone, Debug, Default)]
pub(crate) struct Answered(Arc<Mutex<Option<Answer>>>);

impl Answered {
    pub(crate) fn take(&self) -> Option<Answer> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn set(&self, answer: Answer) {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(answer);
        }
    }
}

/// The question on screen. Dropping it unanswered, as closing the dialog
/// does, declines.
pub(crate) struct Ask {
    /// The host's label, which the dialog shows.
    host: String,
    /// The ID or label `borrow_from` keeps for **Always**.
    key: String,
    machine: String,
    answered: Answered,
}

impl Drop for Ask {
    fn drop(&mut self) {
        self.answered.set(Answer::Dismissed);
    }
}

impl SheprdWindow {
    /// Asks whether to borrow `machine` on host `host`, kept as `key`
    /// when the user always borrows from it. `None` when another dialog
    /// holds the window, which the caller reports as busy.
    pub(crate) fn ask_to_borrow(
        &mut self,
        host: String,
        key: String,
        machine: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Answered> {
        if self.menu.page.is_some() || !self.open_menu(window, cx) {
            return None;
        }
        let answered = Answered::default();
        self.menu.machine_borrow = Some(Ask {
            host,
            key,
            machine,
            answered: answered.clone(),
        });
        self.menu.page = Some(Page::BorrowMachine);
        window.activate_window();
        cx.notify();
        Some(answered)
    }

    pub(crate) fn answer_borrow(
        &mut self,
        answer: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ask) = self.menu.machine_borrow.take() {
            if answer == Answer::Always
                && let Some(hosts) = self.config.machine.adding(&ask.key)
            {
                self.save_preference(move || save_borrow_from(&hosts), cx);
            }
            ask.answered.set(answer);
        }
        self.dismiss_menu(window, cx);
    }

    pub(crate) fn render_borrow_machine(&self, cx: &mut Context<Self>) -> Div {
        let theme = &self.theme;
        let Some(ask) = &self.menu.machine_borrow else {
            return div();
        };
        let host = safe_text(&ask.host, 64);
        let machine = safe_text(&ask.machine, 64);
        let choice = |id: &'static str, label: String, tone: Tone, answer: Answer| {
            button(theme, id, label, tone, true).on_click(cx.listener(
                move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.answer_borrow(answer, window, cx);
                },
            ))
        };
        div()
            .debug_selector(|| "machine-borrow".into())
            .flex()
            .flex_col()
            .p(px(8.))
            .gap(px(12.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(fl!("machine-borrow-title", host = host.clone())),
            )
            .child(div().text_color(rgb(theme.subtext())).child(fl!(
                "machine-borrow-detail",
                host = host.clone(),
                machine = machine
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .child(choice(
                        "machine-borrow-wait",
                        fl!("machine-borrow-wait"),
                        Tone::Plain,
                        Answer::WaitHere,
                    ))
                    .child(choice(
                        "machine-borrow-always",
                        fl!("machine-borrow-always", host = host),
                        Tone::Plain,
                        Answer::Always,
                    ))
                    .child(choice(
                        "machine-borrow-once",
                        fl!("machine-borrow-once"),
                        Tone::Primary,
                        Answer::Once,
                    )),
            )
    }
}

/// Keeps `hosts` as `[machine] borrow_from`.
fn save_borrow_from(hosts: &[String]) -> crate::Result<()> {
    let mut array = toml_edit::Array::new();
    for host in hosts {
        array.push(host.as_str());
    }
    crate::config::Config::set_key("machine.borrow_from", toml_edit::Value::Array(array))
}

#[cfg(test)]
mod tests;
