//! File cards reuse the explicit link opener, including its session fence and
//! background checks. Merely displaying an attachment never reads or opens it.

use crate::chat::view::rich::links::Links;
use crate::{SheprdWindow, config::corners, i18n::fl};
use gpui::{prelude::*, *};

impl SheprdWindow {
    pub(super) fn render_chat_file(
        &self,
        pane_id: &str,
        destination: &str,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let local = self
            .chats
            .pane(pane_id)
            .is_some_and(|pane| pane.target.is_local());
        let links = Links::new(self, pane_id, cx);
        let destination = destination.to_owned();
        div()
            .debug_selector(|| "chat-file-card".into())
            .min_w_0()
            .max_w_full()
            .p(px(10.))
            .rounded(px(corners::CONTROL))
            .border_1()
            .border_color(rgb(self.theme.active))
            .bg(rgb(self.theme.surface))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(content)
            .child(
                div()
                    .id("chat-file-open")
                    .debug_selector(|| "chat-file-open".into())
                    .text_size(self.chat_small_text())
                    .text_color(rgb(self.theme.muted))
                    .child(if local {
                        fl!("chat-file-open")
                    } else {
                        fl!("chat-file-on-host")
                    })
                    .when(local, |button| {
                        button
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                links.open_attachment(&destination, cx);
                            })
                    }),
            )
            .into_any_element()
    }
}
