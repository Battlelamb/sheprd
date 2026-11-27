use crate::{SheprdWindow, close_modal::CloseConfirmation, controls::Command, i18n::fl};
use gpui::{prelude::*, *};
use sheprd_client::protocol::PaneSurfaceFrame;

pub(crate) const ACTION_HINT_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

pub(crate) fn hint_label(label: &str, shortcut: Option<&str>) -> SharedString {
    match shortcut {
        Some(shortcut) => format!("{label}  ·  {shortcut}").into(),
        None => label.to_owned().into(),
    }
}

pub(crate) fn tooltip(
    text: SharedString,
    theme: &crate::config::Theme,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let (foreground, surface) = (theme.foreground, theme.surface);
    move |_, cx| {
        cx.new(|_| crate::usage::Hint {
            text: text.clone(),
            foreground,
            surface,
        })
        .into()
    }
}

impl SheprdWindow {
    pub(crate) fn action_shortcut(&self, command: Command) -> Option<String> {
        self.keymap().shortcuts(command).next().map(|key| {
            if cfg!(target_os = "macos") {
                key.to_owned()
            } else {
                key.replace("cmd-", if cfg!(windows) { "win-" } else { "super-" })
            }
        })
    }

    pub(crate) fn action_hint(
        &self,
        label: impl AsRef<str>,
        command: Option<Command>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let shortcut = command.and_then(|command| self.action_shortcut(command));
        let text = hint_label(label.as_ref(), shortcut.as_deref());
        tooltip(text, &self.theme)
    }

    pub(crate) fn split_pane_hover(
        &self,
        id: SharedString,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        // Track over the whole pane, including children that take mouse input.
        // This layer itself never blocks events from reaching the terminal.
        div()
            .id(id.clone())
            .absolute()
            .inset_0()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.split_button_hover.as_ref() != Some(&id) {
                    this.split_button_hover = Some(id.clone());
                    cx.notify();
                } else if !*hovered && this.split_button_hover.as_ref() == Some(&id) {
                    this.split_button_hover = None;
                    cx.notify();
                }
            }))
    }

    pub(crate) fn split_close_button(&self, id: SharedString) -> Option<Stateful<Div>> {
        if self.split_button_hover.as_ref() != Some(&id) {
            return None;
        }
        let label = fl!("pane-menu-close");
        Some(
            div()
                .id((id.clone(), 1))
                .debug_selector(move || id.to_string())
                .absolute()
                .top(px(3.))
                .right(px(5.))
                .role(Role::Button)
                .aria_label(label.clone())
                .block_mouse_except_scroll()
                .flex()
                .items_center()
                .justify_center()
                .size(px(24.))
                .rounded(px(crate::config::corners::CONTROL))
                .bg(rgb(self.theme.surface))
                .hover(|style| style.bg(rgb(self.theme.active)))
                .cursor_pointer()
                .tooltip(self.action_hint(label, Some(Command::ClosePane)))
                .tooltip_show_delay(ACTION_HINT_DELAY)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    svg()
                        .path("icons/close.svg")
                        .size(px(12.))
                        .text_color(rgb(self.theme.foreground)),
                ),
        )
    }

    pub(super) fn legacy_split_buttons(
        &self,
        surface: Option<&PaneSurfaceFrame>,
        cell_width: f32,
        cell_height: f32,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let Some(surface) = surface.filter(|surface| {
            surface.panes.len() > 1 && surface.popup.is_none() && self.input_ready()
        }) else {
            return Vec::new();
        };
        let endpoint = self.endpoints[self.selected_endpoint].id.clone();
        surface
            .panes
            .iter()
            .filter_map(|pane| {
                let r = pane.rect;
                let w = f32::from(r.width) * cell_width;
                let h = f32::from(r.height) * cell_height;
                if w < 32. || h < 30. {
                    return None;
                }
                let pane = pane.pane_id.clone();
                let boot = surface.boot_id.clone();
                let endpoint = endpoint.clone();
                let id = SharedString::from(format!("close-split-{pane}"));
                Some(
                    div()
                        .absolute()
                        .left(px(f32::from(r.x) * cell_width))
                        .top(px(f32::from(r.y) * cell_height))
                        .w(px(w))
                        .h(px(h))
                        .children(self.split_close_button(id.clone()).map(|button| {
                            button.on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                if !this.input_ready()
                                    || this.endpoints[this.selected_endpoint].id != endpoint
                                    || !this.live.surface.as_ref().is_some_and(|surface| {
                                        surface.boot_id == boot
                                            && surface.popup.is_none()
                                            && surface.panes.len() > 1
                                            && surface
                                                .panes
                                                .iter()
                                                .any(|shown| shown.pane_id == pane)
                                    })
                                {
                                    return;
                                }
                                if let Some(close) = this
                                    .live
                                    .snapshot
                                    .as_ref()
                                    .filter(|snapshot| snapshot.boot_id == boot)
                                    .and_then(|snapshot| {
                                        CloseConfirmation::capture_pane(snapshot, &pane)
                                    })
                                {
                                    this.present_close(close, window, cx);
                                }
                            }))
                        }))
                        .child(self.split_pane_hover(id, cx)),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
