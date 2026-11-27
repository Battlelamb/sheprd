use super::{SheprdWindow, herdr_mark};
use crate::{
    browser::{Pick, Slot},
    i18n::fl,
    notification_mutes::Silenced,
    tab_layout::{Probe, TabContext, TabFace},
};
use gpui::{prelude::*, *};

impl SheprdWindow {
    pub(super) fn tab_rows(
        &self,
        slot: Slot,
        faces: &TabContext<'_>,
        silenced: &Silenced<'_>,
        cx: &mut Context<Self>,
    ) -> Vec<(Pick, Stateful<Div>)> {
        let pick = self.group_pick(slot.id);
        let mut built = Vec::new();
        let layout = self.tab_layout();
        if let Some(snapshot) = &self.live.snapshot {
            for tab in self
                .herdr_tabs_in_front()
                .filter(|t| self.group_lists(slot.id, &Pick::Herdr(t.tab_id.clone())))
            {
                let id = tab.tab_id.clone();
                let context_id = id.clone();
                let close_id = id.clone();
                let selected = matches!(&pick, Some(Pick::Herdr(picked)) if *picked == id);
                let muted = silenced.tab(&id);
                let label = self.strip_label(snapshot, tab);
                let base = div()
                    .id(SharedString::from(format!("tab-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || slot.selector(&format!("tab-{id}"))
                    })
                    .role(Role::Tab)
                    .aria_label(if muted {
                        fl!("tab-strip-muted", label = label.as_ref())
                    } else {
                        label.to_string()
                    })
                    .aria_selected(selected)
                    .map(|control| faces.stop(control));
                let face = TabFace {
                    label,
                    mark: herdr_mark(snapshot, tab),
                    zoomed: tab.zoomed,
                    muted,
                    probe: Probe::Herdr(id.clone()),
                    close: Box::new(cx.listener(move |this, _, window, cx| {
                        this.close_strip_tab(Pick::Herdr(close_id.clone()), window, cx);
                    })),
                };
                built.push((
                    Pick::Herdr(id.clone()),
                    layout
                        .tab(base, face, self.tab_state(selected, slot.id), faces)
                        .map(|tab| self.grow_tab(tab, slot.id, &Pick::Herdr(id.clone())))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_tab_menu(
                                    &context_id,
                                    Some(slot.id),
                                    event.position,
                                    window,
                                    cx,
                                );
                                this.menu.opening_right_click =
                                    this.menu.page == Some(crate::menu::Page::Tab);
                            }),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_herdr_tab(slot.id, &id, window, cx);
                        })),
                ));
            }
        }
        let shown = match pick {
            Some(Pick::Page(id)) => Some(id),
            _ => None,
        };
        built.extend(
            self.browser_tab_entries(slot, shown, faces, cx)
                .into_iter()
                .map(|(id, tab)| (Pick::Page(id), tab)),
        );
        built
    }
}
