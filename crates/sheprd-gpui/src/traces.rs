//! Traces: every agent session this computer's agents ran (Claude Code,
//! Codex, Pi, omp), as a section of the activity bar drawn across the
//! window's body. It shows totals, messages per day by agent, the sessions
//! with a filter and search, and one session's events.
//!
//! [`service::Service`] keeps the index current on its own thread; each
//! window picks up a new snapshot on its tick (`poll_traces`), so a message
//! an agent writes shows within a frame of being indexed. A session's events
//! are read off the UI thread when it is selected. Drawing reads prepared
//! state only.
// i18n: migrated

mod chart;
mod detail;
pub(crate) mod model;
pub(crate) mod service;
mod view;

#[cfg(feature = "integration-test")]
pub(crate) mod native;

#[cfg(test)]
mod tests;

use crate::i18n::fl;
use crate::{
    SheprdWindow,
    search_input::{self, SearchInput},
};
use gpui::{prelude::*, *};
use model::Filter;
use service::{Service, Status};
use sheprd_control_protocol::ActivityPanel;
use sheprd_traces::Event;

/// The most events a session's detail reads.
const DETAIL_EVENTS: usize = 400;

/// A selected session's events.
pub(crate) enum Detail {
    Loading,
    Loaded(Vec<Event>),
    Failed(String),
}

/// The section's own state, made the first time it shows.
pub(crate) struct Page {
    pub(crate) search: Entity<SearchInput>,
    pub(crate) scroll: ScrollHandle,
    pub(crate) detail_scroll: ScrollHandle,
    pub(crate) filter: Filter,
    /// The session whose events show, by id.
    pub(crate) selected: Option<String>,
    pub(crate) detail: Option<Detail>,
    /// The read of `detail` in flight; replacing it drops the old one.
    load: Option<Task<()>>,
    _changed: Subscription,
}

/// Traces in a window: the latest snapshot and the section's page.
pub(crate) struct State {
    pub(crate) status: Status,
    seen: u64,
    pub(crate) page: Option<Page>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            status: Status::Starting,
            seen: 0,
            page: None,
        }
    }
}

impl SheprdWindow {
    /// Gives the section its page while it shows, in the window's face.
    fn ensure_traces_page(&mut self, cx: &mut Context<Self>) {
        let (font, theme) = (self.config.ui.clone(), self.theme.clone());
        if let Some(page) = &self.traces.page {
            page.search
                .update(cx, |input, cx| input.set_appearance(font, theme, cx));
            return;
        }
        let search = cx.new(|cx| {
            let mut input = SearchInput::new(cx);
            input.set_placeholder_with(|| fl!("traces-search"), cx);
            input.set_appearance(font, theme, cx);
            input
        });
        let changed = cx.subscribe(&search, |this, search, _: &search_input::Changed, cx| {
            let query = search.read(cx).text().to_owned();
            if let Some(page) = &mut this.traces.page {
                page.filter.query = query;
            }
            cx.notify();
        });
        self.traces.page = Some(Page {
            search,
            scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            filter: Filter::default(),
            selected: None,
            detail: None,
            load: None,
            _changed: changed,
        });
        cx.notify();
    }

    /// Runs every window tick: takes a new snapshot from the watcher, and
    /// keeps the page while the section shows.
    pub(crate) fn poll_traces(&mut self, cx: &mut Context<Self>) {
        if self.activity_section() == Some(ActivityPanel::Traces) {
            self.ensure_traces_page(cx);
        }
        let Some(service) = cx.try_global::<Service>() else {
            return;
        };
        let generation = service.generation();
        if generation == self.traces.seen {
            return;
        }
        self.traces.seen = generation;
        self.traces.status = service.status();
        // An open session's new events.
        if let Some(session) = self
            .traces
            .page
            .as_ref()
            .and_then(|page| page.selected.clone())
        {
            self.load_trace_events(session, false, cx);
        }
        cx.notify();
    }

    /// Shows `session`'s events, or closes them when it is already shown.
    pub(crate) fn select_trace(&mut self, session: String, cx: &mut Context<Self>) {
        let Some(page) = &mut self.traces.page else {
            return;
        };
        if page.selected.as_deref() == Some(session.as_str()) {
            page.selected = None;
            page.detail = None;
            page.load = None;
            cx.notify();
            return;
        }
        page.selected = Some(session.clone());
        self.load_trace_events(session, true, cx);
        cx.notify();
    }

    /// Reads `session`'s events off the UI thread. A reload keeps the events
    /// shown until the new ones arrive, so a live session does not flicker.
    fn load_trace_events(&mut self, session: String, fresh: bool, cx: &mut Context<Self>) {
        let Some(page) = &mut self.traces.page else {
            return;
        };
        if fresh || page.detail.is_none() {
            page.detail = Some(Detail::Loading);
            page.detail_scroll = ScrollHandle::new();
        }
        let wanted = session.clone();
        let read = cx.background_executor().spawn(async move {
            let path = sheprd_traces::path().ok_or(sheprd_traces::Error::NoStateDirectory)?;
            sheprd_traces::Store::open(&path)?.events(&session, 0, DETAIL_EVENTS)
        });
        page.load = Some(cx.spawn(async move |this, cx| {
            let events = read.await;
            let _ = this.update(cx, |this, cx| {
                let Some(page) = &mut this.traces.page else {
                    return;
                };
                // A later selection owns the pane.
                if page.selected.as_deref() != Some(wanted.as_str()) {
                    return;
                }
                page.detail = Some(match events {
                    Ok(events) => Detail::Loaded(events),
                    Err(error) => Detail::Failed(error.to_string()),
                });
                page.load = None;
                cx.notify();
            });
        }));
    }
}
