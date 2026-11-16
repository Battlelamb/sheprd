use super::*;
use control::{ControlCommand, MemoryCommand};
use sheprd_orchestrator::memory::Kind;

fn memory(args: &[&str]) -> Result<ControlCommand, CliError> {
    let mut all = vec!["memory"];
    all.extend_from_slice(args);
    match LaunchOptions::parse(all)?.mode {
        LaunchMode::Control(invocation) => Ok(invocation.command),
        mode => panic!("{mode:?}"),
    }
}

#[test]
fn memory_lists_by_default_and_takes_scope_and_limit() {
    assert_eq!(
        memory(&[]).unwrap(),
        ControlCommand::Memory(MemoryCommand::List {
            all: false,
            limit: 20
        })
    );
    assert_eq!(
        memory(&["list", "--all", "--limit", "5"]).unwrap(),
        ControlCommand::Memory(MemoryCommand::List {
            all: true,
            limit: 5
        })
    );
    assert!(memory(&["list", "--limit", "0"]).is_err());
    assert!(memory(&["list", "--limit", "201"]).is_err());
}

#[test]
fn memory_search_joins_its_words() {
    assert_eq!(
        memory(&["search", "test", "--all", "core"]).unwrap(),
        ControlCommand::Memory(MemoryCommand::Search {
            query: "test core".into(),
            all: true,
            limit: 20,
        })
    );
    assert!(memory(&["search"]).is_err());
}

#[test]
fn memory_add_takes_a_kind_and_a_branch_scope() {
    assert_eq!(
        memory(&["add", "--kind", "gotcha", "--branch", "CI", "is", "slow"]).unwrap(),
        ControlCommand::Memory(MemoryCommand::Add {
            branch: true,
            kind: Kind::Gotcha,
            text: "CI is slow".into(),
        })
    );
    assert_eq!(
        memory(&["add", "plain", "fact"]).unwrap(),
        ControlCommand::Memory(MemoryCommand::Add {
            branch: false,
            kind: Kind::Fact,
            text: "plain fact".into(),
        })
    );
    assert!(memory(&["add", "--kind", "note", "x"]).is_err());
    assert!(memory(&["add"]).is_err());
    let long = "x".repeat(2001);
    assert!(memory(&["add", &long]).is_err());
}

#[test]
fn memory_forget_needs_a_numeric_id() {
    assert_eq!(
        memory(&["forget", "12"]).unwrap(),
        ControlCommand::Memory(MemoryCommand::Forget { id: 12 })
    );
    assert!(memory(&["forget", "x"]).is_err());
    assert!(memory(&["forget"]).is_err());
    assert!(memory(&["remember", "x"]).is_err());
}
