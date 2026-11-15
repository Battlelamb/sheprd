use super::*;

#[test]
fn only_claude_in_a_herdr_1_pane_is_observed() {
    assert!(observable("w1:p1", Some("claude")));
    assert!(!observable("w1:p1", Some("codex")));
    assert!(!observable("w1:p1", Some("bash")));
    assert!(!observable("w1:p1", None));
}

#[cfg(unix)]
mod running {
    use super::*;

    /// A chat whose "observer" only waits, so nothing ever arrives.
    fn idle(pane: &str) -> Arc<Chat> {
        let mut command = sheprd_client::new_command("/bin/sleep");
        command.arg("30");
        Chat::observe(pane.to_owned(), command).unwrap()
    }

    fn binding(pane: &str, session: &str) -> Binding {
        Binding {
            pane: pane.to_owned(),
            session: session.to_owned(),
            pid: 10,
            started: 20,
        }
    }

    #[test]
    fn the_same_conversation_reuses_its_observer() {
        let observed = Observed::default();
        let chat = idle("w1:p1");
        observed.keep(binding("w1:p1", "a"), chat.clone());
        let found = observed.find(&binding("w1:p1", "a")).unwrap();
        assert!(Arc::ptr_eq(&found, &chat));
        observed.retain(|_| false);
        assert!(chat.is_closed());
    }

    #[test]
    fn another_session_in_the_pane_ends_the_old_observer() {
        let observed = Observed::default();
        let old = idle("w1:p1");
        observed.keep(binding("w1:p1", "a"), old.clone());
        assert!(observed.find(&binding("w1:p1", "b")).is_none());
        assert!(old.is_closed());
    }

    #[test]
    fn past_the_limit_the_oldest_unfollowed_observer_ends() {
        let observed = Observed::default();
        let followed = idle("w0:p1");
        let (_, _phone) = followed.subscribe();
        observed.keep(binding("w0:p1", "a"), followed.clone());
        let chats: Vec<_> = (1..=MAX_OBSERVED)
            .map(|n| {
                let pane = format!("w{n}:p1");
                let chat = idle(&pane);
                observed.keep(binding(&pane, "a"), chat.clone());
                chat
            })
            .collect();
        assert!(!followed.is_closed(), "a phone still follows it");
        assert!(chats[0].is_closed());
        assert!(chats[1..].iter().all(|chat| !chat.is_closed()));
        observed.retain(|_| false);
    }
}
