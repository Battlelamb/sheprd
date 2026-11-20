use super::{
    window::{checked, merged_window},
    *,
};
use crate::{menu::Page, sidebar::layout_tests::fixture_window};

#[gpui::test]
fn the_broom_asks_once_for_every_merged_worktree(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(fixture_window);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let candidate = merged_window(view);
            // Only the verified local daemon's checkouts are looked up.
            assert!(view.merged_worktrees().is_empty());
            view.live.local_daemon_peer = true;
            assert_eq!(view.merged_worktrees(), [candidate]);
            view.open_merged_sweep(window, cx);
            assert_eq!(view.menu.page, Some(Page::RemoveMerged));
            // Asking removes nothing yet.
            assert!(view.merged_cleanup.sweep.is_none());
            assert!(view.merged_cleanup.probe.is_none());
        })
    });
}

#[gpui::test]
fn nothing_merged_says_so_instead_of_asking(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(fixture_window);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.live.status = crate::state::ConnectionStatus::Connected;
            assert!(view.merged_worktrees().is_empty());
            view.open_merged_sweep(window, cx);
            assert_eq!(view.menu.page, None);
            assert_eq!(
                view.flash.as_ref().map(|(flash, _)| flash),
                Some(&crate::window::Flash::success(
                    "No merged worktrees to remove"
                ))
            );
        })
    });
}

#[gpui::test]
fn a_sweep_removes_the_focused_worktree_without_asking_again(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(fixture_window);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let candidate = merged_window(view);
            let mut snapshot = (**view.live.snapshot.as_ref().unwrap()).clone();
            snapshot.focused_workspace_id = Some(candidate.workspace.clone());
            view.live.snapshot = Some(Arc::new(snapshot));
            assert!(!view.config.remove_merged_worktrees);
            view.merged_cleanup.sweep = Some(crate::merged_cleanup::sweep::Sweep::default());
            view.merged_cleanup.probe = Some(checked(&candidate));
            view.finish_merged_probe(window, cx);
            // No delete dialog: the sweep was confirmed. This fixture cannot
            // send, so the removal counts as kept and waits for a retry.
            assert_eq!(view.menu.page, None);
            let sweep = view.merged_cleanup.sweep.as_ref().unwrap();
            assert_eq!((sweep.skipped, sweep.done), (1, 1));
            assert!(
                view.merged_cleanup
                    .ledger
                    .waiting(&candidate, Instant::now())
            );
        })
    });
}

#[gpui::test]
fn a_finished_sweep_says_how_many_stayed(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(fixture_window);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.merged_cleanup.sweep = Some(crate::merged_cleanup::sweep::Sweep {
                skipped: 2,
                done: 3,
                total: 3,
            });
            view.menu.page = Some(Page::RemoveMerged);
            // A list still being scanned keeps the sweep going.
            view.merged_cleanup.idle = true;
            view.end_merged_sweep(window, cx);
            assert!(view.merged_cleanup.sweep.is_some());
            view.merged_cleanup.kept.idle = true;
            view.end_merged_sweep(window, cx);
            assert!(view.merged_cleanup.sweep.is_none());
            // Its progress dialog closes with it.
            assert_eq!(view.menu.page, None);
            assert_eq!(
                view.flash.as_ref().map(|(flash, _)| flash),
                Some(&crate::window::Flash::warning(crate::i18n::fl!(
                    "merged-sweep-kept",
                    count = 2
                )))
            );
        })
    });
}

#[gpui::test]
fn confirming_keeps_the_dialog_open_as_progress(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(fixture_window);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            merged_window(view);
            view.live.local_daemon_peer = true;
            view.open_merged_sweep(window, cx);
            view.confirm_merged_sweep(window, cx);
            assert_eq!(view.menu.page, Some(Page::RemoveMerged));
            let sweep = view.merged_cleanup.sweep.as_ref().unwrap();
            assert_eq!((sweep.done, sweep.total), (0, 1));
            // Its check is running, and the dialog names it.
            assert!(view.merged_cleanup.current().is_some());
            // Confirming again starts nothing new.
            view.confirm_merged_sweep(window, cx);
            assert_eq!(view.merged_cleanup.sweep.as_ref().unwrap().total, 1);
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("merged-sweep-bar").is_some());
    assert!(cx.debug_bounds("merged-sweep-count").is_some());
    assert!(cx.debug_bounds("merged-sweep-submit").is_none());
}
