use super::*;

/// Switching to another tab hides the pane a file was dropped on, and a
/// popup can open over it. Neither cancels the copy: the pane still exists,
/// so the copied paths are pasted into it by its ID.
#[gpui::test]
fn a_copy_keeps_going_and_pastes_into_a_pane_another_tab_hides(cx: &mut TestAppContext) {
    for covered in [false, true] {
        let (fixture, cx) = cx.add_window_view(fixture);
        let view = fixture.read_with(cx, |fixture, _| fixture.view.clone().unwrap());
        let mut peer = Peer::new();
        view.update(cx, |view, cx| {
            peer.prepare(view, cx);
            view.start_file_transfer_with(
                InputTarget::Pane("w1:p1".into()),
                vec!["source".into()],
                |_, _, _, progress| {
                    progress(9, 9);
                    Ok(vec![REMOTE.into()])
                },
                |_, _| panic!("accepted upload was removed"),
                cx,
            );
            if covered {
                popup(view, "popup");
            } else {
                Arc::make_mut(view.live.surface.as_mut().unwrap())
                    .panes
                    .clear();
            }
            view.poll_file_transfer(cx);
            let transfer = view.file_transfer.as_ref().unwrap();
            assert!(!transfer.cancelled.load(Ordering::Acquire), "{covered}");
            assert!(!transfer.shown.2, "{covered}");
        });
        cx.run_until_parked();
        assert_eq!(
            peer.receive(),
            ClientMessage::ClientShellPaneInput {
                pane_id: "w1:p1".into(),
                events: vec![ClientPaneInputEvent::Paste(
                    "/tmp/herdr-upload.ABCDEF123456/it\\'s\\ a\\ file".into(),
                )],
            }
        );
        view.read_with(cx, |view, _| {
            assert!(view.file_transfer.is_none());
            assert_eq!(
                view.endpoints[0].toasts.entries.back().unwrap().1.title,
                "Copy complete"
            );
        });
        peer.sentinel(&view, cx);
    }
}
