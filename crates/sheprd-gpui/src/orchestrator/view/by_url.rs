//! A GitHub pull request the section lists, found by its address, for what
//! names one only that way, such as a chat's rail.

use super::OrchestratorView;
use crate::orchestrator::Item;
use gpui::Context;

impl OrchestratorView {
    /// The listed GitHub pull request at `url`, from any repository.
    pub(crate) fn pull_request_at(&self, url: &str) -> Option<&Item> {
        self.listed()
            .flat_map(|(_, items, _)| items)
            .filter(|item| item.github_pull_request().is_some())
            .find(|item| {
                item.url
                    .as_deref()
                    .is_some_and(|known| known.eq_ignore_ascii_case(url))
            })
    }

    /// Opens the listed pull request at `url` on its conversation; false
    /// when the section does not list it.
    pub(crate) fn open_pull_request_at(&mut self, url: &str, cx: &mut Context<Self>) -> bool {
        let Some(key) = self.pull_request_at(url).map(|item| item.key.canonical()) else {
            return false;
        };
        self.selected = Some(key.clone());
        self.open_detail(key, cx);
        true
    }
}
