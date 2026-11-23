//! Exercise the notifier entry point, including discovery of a delegated
//! chat that already started by the time its snapshot arrives.
use super::*;
use crate::hub::{ChatRow, Hub, Layout, Workspace};
use sheprd_chat_protocol::{Entry, EntryId, Turn, TurnEnd, TurnSummary};

fn workspace(pane: &str, endpoint: EndpointId) -> Workspace {
    Workspace {
        id: pane.into(),
        label: pane.into(),
        branch: None,
        cwd: String::new(),
        worktree: None,
        session: None,
        chats: vec![ChatRow {
            pane: pane.into(),
            tab: pane.into(),
            endpoint,
            agent: None,
            title: "Codex".into(),
            named: false,
            status: sheprd_client::protocol::AgentStatus::Working,
        }],
        terminals: vec![],
    }
}

#[test]
fn a_child_in_another_space_updates_only_its_parent() {
    let parent = EndpointId::random();
    let child = EndpointId::random();
    let hub = Hub::offline(Layout {
        connected: true,
        workspaces: vec![
            workspace("parent", parent.clone()),
            workspace("child", child.clone()),
        ],
        ..Layout::default()
    });
    let dir = tempfile::tempdir().unwrap();
    let mut state = crate::web::tests::state_with(dir.path(), hub);
    Arc::get_mut(&mut state).unwrap().apns = Delivery::new(true).unwrap();
    let mut live = LiveActivities::default();
    let turn = Some(Turn { started_ms: 1000 });
    live.change(
        &state,
        "parent",
        &Transcript::new("Codex"),
        &Change::Turn { turn },
    );
    let mut chat = Transcript::new("Codex");
    chat.delegation = Some(sheprd_chat_protocol::brief::DelegatedTask::new(
        Delegation {
            endpoint: parent.clone(),
            started_ms: 1000,
        },
        "iOS tests",
    ));
    chat.apply(&Change::Push {
        entry: Entry {
            id: EntryId(1),
            kind: EntryKind::User {
                text: "iOS tests".into(),
                files: 0,
                note: None,
            },
        },
    })
    .unwrap();

    live.change(&state, "child", &chat, &Change::Turn { turn });
    chat.turn = turn;
    live.snapshot(&state, "child", &chat);
    assert_eq!(live.groups.len(), 1);
    let ended = Change::Push {
        entry: Entry {
            id: EntryId(2),
            kind: EntryKind::Turn(TurnSummary {
                ended_ms: Some(2000),
                duration_ms: 1000,
                end: TurnEnd::Done,
                tokens: None,
            }),
        },
    };
    live.change(&state, "child", &chat, &ended);
    let updates = live.pacer.due(now_ms() + 60_000);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].pane, "parent");
    let agents = updates[0].state.detail.agents.as_ref().unwrap();
    assert_eq!((agents.done, agents.total), (1, 1));
    assert_eq!(agents.latest.as_deref(), Some("Finished · iOS tests"));
}
