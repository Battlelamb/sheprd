#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

const NOW: i64 = 1_700_000_000;

fn texts(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.text.as_str()).collect()
}

#[test]
fn repository_entries_reach_every_branch_and_branch_entries_stay_there() {
    let mut memory = Memory::in_memory().unwrap();
    memory
        .add(
            None,
            Kind::Command,
            "Run just test-core with an absolute path",
            NOW,
        )
        .unwrap();
    memory
        .add(
            Some("feat/a"),
            Kind::Handoff,
            "Halfway through the parser",
            NOW + 1,
        )
        .unwrap();
    memory
        .add(
            Some("feat/b"),
            Kind::Gotcha,
            "The fixture is stale",
            NOW + 2,
        )
        .unwrap();

    let on_a = memory
        .list(
            Scope::Here {
                branch: Some("feat/a"),
            },
            50,
        )
        .unwrap();
    assert_eq!(
        texts(&on_a),
        [
            "Halfway through the parser",
            "Run just test-core with an absolute path"
        ]
    );
    let detached = memory.list(Scope::Here { branch: None }, 50).unwrap();
    assert_eq!(
        texts(&detached),
        ["Run just test-core with an absolute path"]
    );
    assert_eq!(memory.list(Scope::All, 50).unwrap().len(), 3);
}

#[test]
fn the_same_text_in_the_same_scope_is_kept_once() {
    let mut memory = Memory::in_memory().unwrap();
    let first = memory
        .add(None, Kind::Fact, "  CI runs on macOS  ", NOW)
        .unwrap();
    let again = memory
        .add(None, Kind::Gotcha, "CI runs on macOS", NOW + 5)
        .unwrap();
    assert_eq!(again, Added::Again(first.id()));
    let entries = memory.list(Scope::All, 10).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].kind, entries[0].used), (Kind::Gotcha, NOW + 5));
    // The same words on a branch are a separate entry.
    assert!(matches!(
        memory
            .add(Some("main"), Kind::Fact, "CI runs on macOS", NOW)
            .unwrap(),
        Added::New(_)
    ));
}

#[test]
fn search_matches_word_prefixes_and_marks_what_it_found_used() {
    let mut memory = Memory::in_memory().unwrap();
    memory
        .add(
            None,
            Kind::Gotcha,
            "Snapshot tests need HERDR_TEST_TMPDIR short",
            NOW,
        )
        .unwrap();
    memory
        .add(None, Kind::Fact, "The relay speaks QUIC", NOW)
        .unwrap();
    memory
        .add(
            Some("other"),
            Kind::Fact,
            "Snapshot fixtures live in tests",
            NOW,
        )
        .unwrap();

    let found = memory
        .search(
            Scope::Here {
                branch: Some("mine"),
            },
            "snap",
            10,
            NOW + 9,
        )
        .unwrap();
    assert_eq!(
        texts(&found),
        ["Snapshot tests need HERDR_TEST_TMPDIR short"]
    );
    assert_eq!(found[0].used, NOW);
    let listed = memory.list(Scope::Here { branch: None }, 1).unwrap();
    assert_eq!(listed[0].used, NOW + 9);

    assert_eq!(
        memory
            .search(Scope::All, "snapshot", 10, NOW)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn search_takes_query_syntax_as_plain_words() {
    let mut memory = Memory::in_memory().unwrap();
    memory
        .add(None, Kind::Fact, "Use NEAR quotes carefully", NOW)
        .unwrap();
    for query in ["\"unbalanced", "NEAR(a b)", "a OR AND", "col:text", "*"] {
        memory.search(Scope::All, query, 10, NOW).unwrap();
    }
    assert!(
        memory
            .search(Scope::All, "  !! ", 10, NOW)
            .unwrap()
            .is_empty()
    );
    assert_eq!(memory.search(Scope::All, "near", 10, NOW).unwrap().len(), 1);
}

#[test]
fn forgotten_entries_leave_search_too() {
    let mut memory = Memory::in_memory().unwrap();
    let id = memory
        .add(None, Kind::Fact, "Ephemeral fact", NOW)
        .unwrap()
        .id();
    assert!(memory.forget(id).unwrap());
    assert!(!memory.forget(id).unwrap());
    assert!(
        memory
            .search(Scope::All, "ephemeral", 10, NOW)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn text_is_bounded_and_required() {
    let mut memory = Memory::in_memory().unwrap();
    assert!(matches!(
        memory.add(None, Kind::Fact, " \n ", NOW),
        Err(Error::Empty)
    ));
    let long = "x".repeat(MAX_TEXT + 1);
    assert!(matches!(
        memory.add(None, Kind::Fact, &long, NOW),
        Err(Error::TooLong { max: MAX_TEXT })
    ));
    memory
        .add(None, Kind::Fact, &"x".repeat(MAX_TEXT), NOW)
        .unwrap();
}

#[test]
fn the_least_recently_used_go_first_past_the_cap() {
    let mut memory = Memory::in_memory().unwrap();
    for n in 0..=MAX_ENTRIES {
        let used = NOW + i64::from(n);
        memory
            .add(None, Kind::Fact, &format!("entry {n}"), used)
            .unwrap();
    }
    let all = memory.list(Scope::All, MAX_LIMIT).unwrap();
    assert_eq!(all[0].text, format!("entry {MAX_ENTRIES}"));
    let count: i64 = memory
        .connection
        .query_row("SELECT count(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, i64::from(MAX_ENTRIES));
    assert!(memory.search(Scope::All, "entry 0", 1, NOW).unwrap()[0].text != "entry 0");
}

#[test]
fn listing_is_bounded() {
    let mut memory = Memory::in_memory().unwrap();
    for n in 0..5 {
        memory
            .add(None, Kind::Fact, &format!("fact {n}"), NOW + n)
            .unwrap();
    }
    assert_eq!(memory.list(Scope::All, 2).unwrap().len(), 2);
    assert_eq!(memory.list(Scope::All, 0).unwrap().len(), 1);
}

#[test]
fn kinds_parse_from_their_names() {
    for kind in KINDS {
        assert_eq!(kind.as_str().parse::<Kind>().unwrap(), *kind);
    }
    assert!(matches!("note".parse::<Kind>(), Err(Error::Kind(name)) if name == "note"));
}

#[test]
fn every_worktree_of_a_repository_opens_one_file() {
    let state = tempfile::tempdir().unwrap();
    let repository = Repository::Local(PathBuf::from("/code/app/.git"));
    let path = path_in(state.path(), &repository);
    assert_eq!(
        path.parent(),
        crate::location::database_path_in(state.path(), &repository).parent()
    );
    Memory::open(&path)
        .unwrap()
        .add(Some("one"), Kind::Fact, "Shared", NOW)
        .unwrap();
    let mut other = Memory::open(&path).unwrap();
    assert_eq!(other.search(Scope::All, "shared", 5, NOW).unwrap().len(), 1);
}

#[test]
fn a_newer_schema_is_refused() {
    let state = tempfile::tempdir().unwrap();
    let path = path_in(state.path(), &Repository::Local(PathBuf::from("/r/.git")));
    drop(Memory::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    assert!(matches!(
        Memory::open(&path),
        Err(Error::NewerSchema { found, known: SCHEMA_VERSION }) if found == SCHEMA_VERSION + 1
    ));
}

#[test]
fn a_forgotten_id_is_never_given_to_a_new_entry() {
    let mut memory = Memory::in_memory().unwrap();
    memory.add(None, Kind::Fact, "first", NOW).unwrap();
    let last = memory.add(None, Kind::Fact, "second", NOW).unwrap().id();
    assert!(memory.forget(last).unwrap());
    let next = memory.add(None, Kind::Fact, "third", NOW).unwrap().id();
    assert!(next > last, "{next} reused {last}");
    // A stale forget of the old ID leaves the new entry alone.
    assert!(!memory.forget(last).unwrap());
    assert_eq!(memory.list(Scope::All, 10).unwrap().len(), 2);
}
