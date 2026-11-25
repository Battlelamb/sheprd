//! Which pull request a finished turn created or pushed to.

use crate::push::TurnPull;
use sheprd_chat_protocol::{
    Change, Entry, EntryId, EntryKind, PullRequest, PullRequestAction, TurnEnd, TurnSummary,
};

fn pull(number: u64, actions: &[PullRequestAction]) -> Change {
    Change::PullRequest {
        pull_request: PullRequest::new("o/r", number, actions.to_vec()).unwrap(),
    }
}

fn push(kind: EntryKind) -> Change {
    Change::Push {
        entry: Entry {
            id: EntryId(10),
            kind,
        },
    }
}

fn user() -> Change {
    push(EntryKind::User {
        text: "go".to_owned(),
        files: 0,
        note: None,
    })
}

fn ended() -> Change {
    push(EntryKind::Turn(TurnSummary {
        ended_ms: None,
        duration_ms: 1_000,
        end: TurnEnd::Done,
        tokens: None,
    }))
}

#[test]
fn a_turn_ends_with_the_last_pull_request_it_created_or_pushed_to() {
    let mut turn = TurnPull::default();
    assert_eq!(turn.step(&user()), None);
    assert_eq!(turn.step(&pull(7, &[PullRequestAction::Pushed])), None);
    turn.step(&pull(8, &[PullRequestAction::Created]));
    // Read or commented on: no push, no button.
    turn.step(&pull(9, &[PullRequestAction::Commented]));
    turn.step(&pull(10, &[]));
    assert_eq!(
        turn.step(&ended()).as_deref(),
        Some("https://github.com/o/r/pull/8")
    );
    assert_eq!(turn.step(&ended()), None, "taken once");
}

#[test]
fn the_person_speaking_starts_a_turn_without_one() {
    let mut turn = TurnPull::default();
    turn.step(&pull(7, &[PullRequestAction::Pushed]));
    turn.step(&user());
    assert_eq!(turn.step(&ended()), None);
}
