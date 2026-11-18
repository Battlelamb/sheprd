//! The Git popup's restack: once the layer directly below the focused
//! branch's pull request merged into its GitHub stack's base, the branch
//! still carries that layer's old commits. The confirmation says what moves;
//! the Git worker then replays the branch's own commits onto the base as the
//! remote has it now, locally, and never pushes.
use super::{Page, git::Row};
use crate::{SheprdWindow, git::Action, i18n::fl, pull_request::State};
use gpui::{prelude::*, *};

/// What a restack moves the focused branch onto, and past.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Restack {
    /// The merged layer's number and head commit.
    pub(super) below: u64,
    pub(super) oid: String,
    /// The stack's base branch.
    pub(super) base: String,
}

impl SheprdWindow {
    /// The restack the focused branch's open pull request calls for, if the
    /// layer directly below it merged.
    pub(super) fn git_restack(&self) -> Option<Restack> {
        let pr = self.git_open_pull_request()?;
        if pr.state != State::Open {
            return None;
        }
        let (stack, position) = pr.stack()?;
        let below = stack.merged_below(position)?;
        Some(Restack {
            below: below.number,
            oid: below.head_oid.clone(),
            base: stack.base.clone(),
        })
    }

    pub(super) fn open_git_restack(&mut self) {
        self.menu.page = Some(Page::GitRestack);
        self.menu.error = None;
    }

    /// The confirmation itself: the restack named by the pull request the
    /// popup shows now, so a lookup that moved since cannot name another.
    pub(super) fn submit_git_restack(&mut self, cx: &mut Context<Self>) {
        if self.git.running().is_some() || self.pr_actions.running().is_some() {
            return;
        }
        let Some(restack) = self.git_restack() else {
            self.menu.page = Some(Page::Git);
            cx.notify();
            return;
        };
        self.start_git(Action::Restack {
            below: restack.oid,
            base: restack.base,
        });
        if self.menu.error.is_none() {
            self.menu.page = Some(Page::Git);
        }
        cx.notify();
    }

    pub(super) fn render_git_restack(&self, cx: &mut Context<Self>) -> Div {
        let theme = &self.theme;
        let restack = self.git_restack();
        let base = restack
            .as_ref()
            .map_or(String::new(), |restack| restack.base.clone());
        let body = restack.as_ref().map(|restack| {
            let branch = self
                .git
                .tracked()
                .map(|input| input.branch.clone())
                .unwrap_or_default();
            fl!(
                "git-menu-restack-body",
                number = restack.below,
                branch = branch.as_str(),
                base = restack.base.as_str()
            )
        });
        let armed = restack.is_some() && self.git.running().is_none();
        div()
            .flex()
            .flex_col()
            .child(self.pr_dialog_header(
                Row::Restack,
                fl!("git-menu-restack-title", base = base.as_str()),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(12.))
                    .children(body.map(|body| {
                        div()
                            .debug_selector(|| "git-restack-body".into())
                            .text_color(rgb(theme.muted))
                            .child(crate::sidebar::label_text(&body))
                    }))
                    .children(self.pr_dialog_error()),
            )
            .child(
                self.pr_footer().child(self.pr_back_button(cx)).child(
                    self.pr_button(
                        "git-restack-confirm",
                        fl!("git-menu-restack-confirm"),
                        armed,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.submit_git_restack(cx);
                    })),
                ),
            )
    }
}
