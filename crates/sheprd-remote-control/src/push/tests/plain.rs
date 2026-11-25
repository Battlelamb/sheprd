use crate::push::plain::plain;

#[test]
fn size_marks_and_code_marks_leave_their_text() {
    assert_eq!(
        plain(":big[Nothing is uncommitted here.] This worktree (`port/mobile`) is clean"),
        "Nothing is uncommitted here. This worktree (port/mobile) is clean"
    );
}

#[test]
fn markdown_reads_as_one_plain_line() {
    assert_eq!(
        plain("# Done\n\n**Merged** [PR #12](https://example.com/12), ~~old~~ _new_."),
        "Done Merged PR #12, old new."
    );
    assert_eq!(plain("- one\n- two\n\n> quoted"), "one two quoted");
    assert_eq!(plain("```sh\njust ci\n```\nran"), "just ci ran");
    assert_eq!(plain("![a chart](/tmp/c.png) here"), "a chart here");
}

#[test]
fn size_marks_nest_and_only_open_where_the_chat_opens_them() {
    assert_eq!(plain(":huge[a :big[b] c] d"), "a b c d");
    assert_eq!(plain(":big[see [x] here]"), "see [x] here");
    assert_eq!(plain("http://x:big[y]"), "http://x:big[y]");
    assert_eq!(plain("a:big[y]"), "a:big[y]");
    assert_eq!(plain(":big[unclosed"), "unclosed");
    assert_eq!(plain(":tiny[x]"), ":tiny[x]");
}
