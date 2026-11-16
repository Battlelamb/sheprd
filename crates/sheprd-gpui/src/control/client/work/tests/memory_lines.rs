use super::*;
use sheprd_orchestrator::memory::{Entry, Kind};

#[test]
fn memory_entries_read_one_per_line() {
    let entry = |id, branch: Option<&str>, kind, text: &str| Entry {
        id,
        branch: branch.map(str::to_owned),
        kind,
        text: text.to_owned(),
        created: 0,
        used: 0,
    };
    assert_eq!(
        memory::describe(&[
            entry(3, Some("feat/x"), Kind::Handoff, "Parser half done"),
            entry(1, None, Kind::Command, "Run\n  just test-core\n"),
        ]),
        "#3 [handoff] (feat/x) Parser half done\n#1 [command] Run just test-core"
    );
}
