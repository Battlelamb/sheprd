use super::*;
use crate::{EntryId, NoticeLevel, ToolKind};

fn entry(id: u64, kind: EntryKind) -> Entry {
    Entry {
        id: EntryId(id),
        kind,
    }
}

fn user(id: u64, text: &str) -> Entry {
    entry(
        id,
        EntryKind::User {
            text: text.to_owned(),
            files: 0,
        },
    )
}

fn agent(id: u64, text: &str) -> Entry {
    entry(
        id,
        EntryKind::Agent {
            text: text.to_owned(),
        },
    )
}

fn full() -> Scope<'static> {
    Scope::Full { first_prompt: None }
}

#[test]
fn nothing_worth_handing_over_gives_no_handoff() {
    assert_eq!(build(&[], full(), &[Agent::Claude], BUDGET), None);
    let notice = entry(
        1,
        EntryKind::Notice {
            answers: Vec::new(),
            details: None,
            level: NoticeLevel::Info,
            text: "Switched".to_owned(),
        },
    );
    assert_eq!(build(&[notice], full(), &[Agent::Claude], BUDGET), None);
}

#[test]
fn a_short_conversation_is_quoted_whole_in_order_without_reasoning() {
    let entries = [
        user(1, "Fix the parser"),
        entry(
            2,
            EntryKind::Thought {
                text: "private".to_owned(),
            },
        ),
        entry(
            3,
            EntryKind::Tool(Tool {
                call_id: "c1".to_owned(),
                content: vec![ToolContent::Diff {
                    new: "b".to_owned(),
                    old: Some("a".to_owned()),
                    path: "src/parse.rs".to_owned(),
                }],
                kind: ToolKind::Edit,
                locations: vec!["src/parse.rs".to_owned()],
                parent: None,
                started_ms: None,
                status: ToolStatus::Completed,
                title: "Edit parse.rs".to_owned(),
            }),
        ),
        agent(4, "Fixed."),
    ];
    let handoff = build(&entries, full(), &[Agent::Claude], BUDGET).unwrap();
    assert_eq!((handoff.kept, handoff.omitted), (3, 0));
    assert!(handoff.text.contains("with Claude"));
    assert!(!handoff.text.contains("private"));
    let parse = handoff.text.find("User: Fix the parser").unwrap();
    let tool = handoff
        .text
        .find("Tool: Edit parse.rs (done); files: src/parse.rs\nchanged src/parse.rs")
        .unwrap();
    let reply = handoff.text.find("Agent: Fixed.").unwrap();
    assert!(parse < tool && tool < reply);
    assert!(handoff.text.ends_with("The user's new message follows.]"));
}

#[test]
fn a_long_conversation_keeps_its_first_and_latest_exchange_and_the_newest_rest() {
    let mut entries = vec![user(1, "The first ask")];
    for id in 2..200 {
        entries.push(agent(id, &"x".repeat(1000)));
    }
    entries.push(user(200, "The latest ask"));
    entries.push(agent(201, "The latest answer"));
    let handoff = build(&entries, full(), &[Agent::Codex], 20 * 1024).unwrap();
    assert!(handoff.text.contains("User: The first ask"));
    assert!(handoff.text.contains("User: The latest ask"));
    assert!(handoff.text.contains("Agent: The latest answer"));
    assert!(handoff.omitted > 0);
    assert_eq!(handoff.kept + handoff.omitted, entries.len());
    assert!(
        handoff
            .text
            .contains(&format!("{} earlier entries are left out", handoff.omitted))
    );
    // The gap sits between the first prompt and the newest entries kept.
    let first = handoff.text.find("User: The first ask").unwrap();
    let gap = handoff.text.find(" left out]").unwrap();
    let latest = handoff.text.find("User: The latest ask").unwrap();
    assert!(first < gap && gap < latest);
    assert!(handoff.text.len() < 21 * 1024 + 1024);
}

#[test]
fn a_first_prompt_the_transcript_dropped_is_still_quoted_first() {
    let entries = [user(50, "Later ask"), agent(51, "Later answer")];
    let scope = Scope::Full {
        first_prompt: Some("The very first ask"),
    };
    let handoff = build(&entries, scope, &[Agent::Claude], BUDGET).unwrap();
    let first = handoff.text.find("User: The very first ask").unwrap();
    assert!(first < handoff.text.find("User: Later ask").unwrap());
    assert_eq!(handoff.kept, 3);
    // Held in the transcript too, it is quoted once.
    let entries = [user(1, "The very first ask"), agent(2, "Answer")];
    let handoff = build(&entries, scope, &[Agent::Claude], BUDGET).unwrap();
    assert_eq!(handoff.text.matches("The very first ask").count(), 1);
}

#[test]
fn a_resumed_agent_hears_only_what_happened_since() {
    let entries = [user(9, "Now this"), agent(10, "Done by Codex")];
    let handoff = build(&entries, Scope::Since, &[Agent::Codex, Agent::Pi], BUDGET).unwrap();
    assert!(handoff.text.contains("your earlier session"));
    assert!(handoff.text.contains("with Codex and Pi"));
    assert_eq!(handoff.kept, 2);
}

#[test]
fn a_long_entry_is_cut_on_a_character_boundary() {
    let long = "é".repeat(ENTRY_CAP);
    let handoff = build(&[agent(1, &long)], full(), &[], BUDGET).unwrap();
    let quoted = handoff.text.lines().nth(1).unwrap();
    assert!(quoted.ends_with(" [cut]"));
    assert!(quoted.len() <= ENTRY_CAP);
    assert!(handoff.text.contains("with another agent"));
}
