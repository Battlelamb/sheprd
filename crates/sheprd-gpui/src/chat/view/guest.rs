//! A guest's message: words the person allowed someone else to send into
//! the chat through a tool (`docs/chat-guests.md`). Its row is named, so it
//! never reads as the person's own.

use super::*;
use crate::notifications::safe_text;
use sheprd_chat_protocol::{
    Entry, EntryKind, GuestPrompt, MAX_GUEST_NAME, MAX_GUEST_PROMPT_SIZE, MAX_GUEST_TOOL,
};

/// The height a waiting guest prompt's text takes before it scrolls.
const ASK_HEIGHT: f32 = 160.;

impl SheprdWindow {
    /// `row`, a prompt's row, under the name of the guest who wrote it.
    pub(super) fn with_chat_guest(&self, entry: &Entry, row: AnyElement) -> AnyElement {
        let EntryKind::User {
            guest: Some(guest), ..
        } = &entry.kind
        else {
            return row;
        };
        let theme = &self.theme;
        let id = entry.id.0;
        let label = fl!(
            "chat-guest-label",
            name = safe_text(&guest.name, MAX_GUEST_NAME),
            tool = safe_text(&guest.tool, MAX_GUEST_TOOL)
        );
        let line = div()
            .debug_selector(move || format!("chat-guest-{id}"))
            .mx(px(12.))
            .mt(px(8.))
            .flex()
            .justify_end()
            .items_center()
            .gap(px(4.))
            .text_size(self.chat_small_text())
            .text_color(rgb(Hues::of(theme).purple))
            .child(
                svg()
                    .path("icons/user.svg")
                    .size(px(11.))
                    .text_color(rgb(Hues::of(theme).purple)),
            )
            .child(label);
        div()
            .flex()
            .flex_col()
            .child(line)
            .child(row)
            .into_any_element()
    }

    /// A guest's prompt waiting for the person: who asks, and every word
    /// they would send, so nothing reaches the agent unread.
    pub(super) fn render_chat_guest_ask(&self, id: u64, prompt: &GuestPrompt) -> Div {
        let theme = &self.theme;
        let heading = fl!(
            "chat-guest-asks",
            name = safe_text(&prompt.guest.name, MAX_GUEST_NAME),
            tool = safe_text(&prompt.guest.tool, MAX_GUEST_TOOL)
        );
        let lines = prompt
            .text
            .lines()
            .map(|line| {
                let line = guest_line(line);
                div().child(if line.is_empty() {
                    " ".to_owned()
                } else {
                    line
                })
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        svg()
                            .path("icons/user.svg")
                            .size(px(12.))
                            .text_color(rgb(Hues::of(theme).purple)),
                    )
                    .child(heading),
            )
            .child(
                div()
                    .id(("chat-guest-ask", id))
                    .debug_selector(move || format!("chat-guest-ask-{id}"))
                    .max_h(px(ASK_HEIGHT))
                    .overflow_y_scroll()
                    .pl(px(8.))
                    .border_l_2()
                    .border_color(rgb(Hues::of(theme).purple))
                    .text_color(rgb(theme.subtext()))
                    .flex()
                    .flex_col()
                    .children(lines),
            )
    }
}

/// A line of a guest's prompt as the approval card shows it. Tabs separate
/// words the agent gets, so they show as spaces rather than being dropped
/// with the control characters `safe_text` removes.
pub(in crate::chat) fn guest_line(line: &str) -> String {
    safe_text(&line.replace('\t', "    "), MAX_GUEST_PROMPT_SIZE * 4)
}
