use super::*;

const SHEPRD: &str = "/Applications/Herdr.app/Contents/MacOS/sheprd";

#[test]
fn the_memory_guide_goes_once_with_the_first_prompt() {
    let mut context = Context::default().with_memory(Some(Path::new(SHEPRD)));
    let note = context.before(None).unwrap();
    assert!(note.contains("<memory>\n"), "{note}");
    assert!(note.contains(&format!("`{SHEPRD} memory search WORDS`")));
    assert!(note.contains("memory add --kind"));
    assert!(note.ends_with("</sheprd>"));
    assert_eq!(context.before(None), None);
}

#[test]
fn without_sheprd_the_note_says_nothing_of_memory() {
    let mut context = Context::default().with_memory(None);
    assert_eq!(context.before(None), None);
    let mut context = Context::new(Some("Be brief.".to_owned()), Told::default()).with_memory(None);
    assert!(!context.before(None).unwrap().contains("<memory>"));
}

#[test]
fn a_continued_chat_that_heard_the_guide_is_not_told_again() {
    let mut first = Context::default().with_memory(Some(Path::new(SHEPRD)));
    let sent = first.before(None).unwrap();
    let mut told = Told::default();
    told.heard(split(&sent).unwrap().0);
    assert!(told.memory);

    let mut continued = Context::new(None, told).with_memory(Some(Path::new(SHEPRD)));
    assert_eq!(continued.before(None), None);

    // A note replayed after the context was made counts too.
    let mut replayed = Context::default().with_memory(Some(Path::new(SHEPRD)));
    replayed.heard(&sent);
    assert_eq!(replayed.before(None), None);
}

#[test]
fn the_guide_quotes_a_path_with_spaces() {
    let guide = memory_guide(Path::new("C:/Program Files/Sheprd/sheprd.exe"));
    assert!(guide.contains("`\"C:/Program Files/Sheprd/sheprd.exe\" memory search WORDS`"));
}
