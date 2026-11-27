//! The Arc-style window: workspace navigation and a framed content surface.
use super::{SheprdWindow, chrome_buttons::ACTION_HINT_DELAY, chrome_focus::Region};
use crate::{
    config::{TabLayoutMode, mix},
    controls::Command,
    i18n::fl,
};
use gpui::{prelude::*, *};

pub(crate) const ARC_FRAME_INSET: f32 = 8.;

impl SheprdWindow {
    pub(crate) fn arc_chrome(&self) -> bool {
        self.config.tab_layout == TabLayoutMode::Arc && !self.zen_layout()
    }

    pub(crate) fn arc_shell_color(&self) -> u32 {
        mix(self.theme.sidebar_background(), self.theme.primary(), 17)
    }

    pub(super) fn arc_frame(
        &self,
        body: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body = body
            .when(self.arc_chrome(), |body| {
                body.debug_selector(|| "arc-terminal-frame".into())
                    .m(px(ARC_FRAME_INSET))
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(mix(self.theme.surface, self.theme.foreground, 13)))
                    .bg(rgb(self.theme.background))
            })
            .into_any_element();
        if self.arc_chrome() && self.mod_panel_shown() {
            self.with_mod_panel(body, window, cx)
        } else {
            body
        }
    }

    pub(super) fn arc_sidebar(
        &self,
        sidebar: AnyElement,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = Command::Palette;
        let search_label = fl!("tab-layout-arc-search");
        let search = self
            .arc_button(
                "arc-search",
                search_label.clone(),
                Some(palette),
                "icons/search.svg",
                Region::Sidebar,
            )
            .w_full()
            .justify_start()
            .gap_2()
            .px_2()
            .child(div().flex_1().min_w_0().truncate().child(search_label))
            .children(self.action_shortcut(palette).map(|shortcut| {
                div()
                    .flex_none()
                    .px_1()
                    .text_size(px(10.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(self.theme.active))
                    .text_color(rgb(self.theme.muted))
                    .child(shortcut)
            }))
            .bg(rgb(mix(
                self.theme.sidebar_background(),
                self.theme.foreground,
                9,
            )))
            .on_click(cx.listener(move |this, _, window, cx| this.command(palette, window, cx)));
        div()
            .debug_selector(|| "arc-sidebar".into())
            .flex()
            .flex_col()
            .flex_none()
            .w(px(width))
            .min_h_0()
            .child(div().px_2().pt_2().child(search))
            .child(div().flex().flex_1().min_h_0().child(sidebar))
            .into_any_element()
    }

    pub(crate) fn arc_button(
        &self,
        id: &'static str,
        label: String,
        command: Option<Command>,
        icon: &'static str,
        region: Region,
    ) -> Stateful<Div> {
        self.chrome_stop(region, div().id(id))
            .debug_selector(move || id.into())
            .role(Role::Button)
            .aria_label(label.clone())
            .tooltip(self.action_hint(label, command))
            .tooltip_show_delay(ACTION_HINT_DELAY)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(28.))
            .min_w(px(28.))
            .rounded(px(7.))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(self.theme.active)))
            .child(
                svg()
                    .path(icon)
                    .size(px(14.))
                    .flex_none()
                    .text_color(rgb(self.theme.foreground)),
            )
    }
}

#[cfg(test)]
mod tests;
