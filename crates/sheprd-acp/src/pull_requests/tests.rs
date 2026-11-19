#![allow(clippy::unwrap_used)]

use super::*;

fn call(title: &str, output: &str) -> Tool {
    Tool {
        call_id: "call".to_owned(),
        content: vec![ToolContent::Text {
            text: output.to_owned(),
        }],
        kind: ToolKind::Execute,
        locations: Vec::new(),
        parent: None,
        started_ms: None,
        status: ToolStatus::Completed,
        title: title.to_owned(),
    }
}

/// What `found` read, as `(OWNER/REPO#NUMBER, actions)`.
fn read(title: &str, output: &str) -> Vec<(String, Vec<Action>)> {
    found(&call(title, output))
        .into_iter()
        .map(|pull| (format!("{}#{}", pull.repo, pull.number), pull.actions))
        .collect()
}

fn one(reference: &str, action: Action) -> Vec<(String, Vec<Action>)> {
    vec![(reference.to_owned(), vec![action])]
}

#[test]
fn a_created_pull_request_is_read_from_the_address_gh_prints() {
    assert_eq!(
        read(
            "`gh pr create --title \"feat: rail\" --body \"$(cat <<'EOF'\nSummary\nEOF\n)\"`",
            "```console\nhttps://github.com/sheprd-labs/herdr-gpui/pull/1446\n```",
        ),
        one("sheprd-labs/herdr-gpui#1446", Action::Created)
    );
}

#[test]
fn each_changing_gh_pr_command_names_what_it_did() {
    for (command, output, action) in [
        (
            "gh pr edit 7 -R o/r --add-label bug",
            "https://github.com/o/r/pull/7",
            Action::Edited,
        ),
        (
            "gh pr merge 7 --squash",
            "✓ Squashed and merged pull request o/r#7 (Fix)",
            Action::Merged,
        ),
        (
            "gh pr merge --auto --squash 7 --repo o/r",
            "",
            Action::Edited,
        ),
        (
            "gh pr comment 7 --body hi",
            "https://github.com/o/r/pull/7#issuecomment-99",
            Action::Commented,
        ),
        (
            "gh pr review 7 --approve",
            "✓ Approved pull request o/r#7",
            Action::Reviewed,
        ),
        (
            "gh pr ready",
            "✓ Pull request o/r#7 (Fix) is marked as \"ready for review\"",
            Action::Ready,
        ),
        (
            "gh pr close https://github.com/o/r/pull/7",
            "",
            Action::Closed,
        ),
        ("gh pr reopen --repo=o/r #7", "", Action::Reopened),
    ] {
        assert_eq!(read(command, output), one("o/r#7", action), "{command}");
    }
}

#[test]
fn commands_that_only_read_name_no_pull_request() {
    for command in [
        "gh pr view 7 --repo o/r",
        "gh pr list",
        "gh pr checks 7",
        "gh pr diff https://github.com/o/r/pull/7",
        "gh pr checkout 7",
        "gh api repos/o/r/pulls/7",
        "gh api repos/o/r/pulls/7 --method GET -f per_page=5",
        "gh issue comment 7 --body https://github.com/o/r/pull/7",
        "cat notes.md",
        "echo 'gh pr merge 7 --repo o/r'",
        "git log --oneline",
    ] {
        assert_eq!(
            read(command, "see https://github.com/o/r/pull/7"),
            [],
            "{command}"
        );
    }
}

#[test]
fn a_call_that_failed_or_did_not_run_a_command_names_none() {
    let mut failed = call("gh pr merge 7 -R o/r", "");
    failed.status = ToolStatus::Failed;
    assert!(found(&failed).is_empty());
    let mut running = call("gh pr merge 7 -R o/r", "");
    running.status = ToolStatus::InProgress;
    assert!(found(&running).is_empty());
    let mut edit = call("gh pr merge 7 -R o/r", "");
    edit.kind = ToolKind::Edit;
    assert!(found(&edit).is_empty());
}

#[test]
fn gh_api_writes_to_a_pull_request_are_read_by_their_path() {
    for (command, action) in [
        ("gh api -X PUT repos/o/r/pulls/7/merge", Action::Merged),
        (
            "gh api repos/o/r/pulls/7/comments -f body=hi",
            Action::Commented,
        ),
        (
            "gh api --method=POST /repos/o/r/pulls/7/reviews",
            Action::Reviewed,
        ),
        (
            "gh api -X PATCH repos/o/r/pulls/7 -f state=closed",
            Action::Closed,
        ),
        (
            "gh api -X PATCH repos/o/r/pulls/7 -f state=open",
            Action::Reopened,
        ),
        (
            "gh api -X PATCH repos/o/r/pulls/7 -f title=New",
            Action::Edited,
        ),
        (
            "gh api repos/o/r/pulls/7/requested_reviewers -f reviewers[]=a",
            Action::Edited,
        ),
    ] {
        assert_eq!(read(command, ""), one("o/r#7", action), "{command}");
    }
    // A placeholder `gh` fills in is not a repository this can name.
    assert_eq!(
        read("gh api -X PUT repos/{owner}/{repo}/pulls/7/merge", ""),
        []
    );
}

#[test]
fn a_push_names_the_pull_request_its_output_links_to() {
    let output = "remote:\nremote: View pull request for 'feat':\nremote:   \
                  https://github.com/o/r/pull/7\nTo github.com:o/r.git";
    assert_eq!(
        read("git -C /tmp/r push origin feat", output),
        one("o/r#7", Action::Pushed)
    );
    // A new branch's link opens a pull request; it names none yet.
    let new = "remote: Create a pull request for 'feat' on GitHub by visiting:\n\
               remote:      https://github.com/o/r/pull/new/feat";
    assert_eq!(read("git push -u origin feat", new), []);
}

#[test]
fn chained_commands_are_read_one_by_one_and_each_pull_request_once() {
    let pulls = read(
        "cd repo && git push && gh pr create --fill",
        "https://github.com/o/r/pull/7\nhttps://github.com/o/r/pull/7",
    );
    assert_eq!(
        pulls,
        vec![("o/r#7".to_owned(), vec![Action::Pushed, Action::Created])]
    );
    // GitHub reads names in any case.
    assert_eq!(
        read("gh pr close 7 -R O/R", "✓ Closed pull request o/r#7 (Fix)").len(),
        1
    );
}

#[test]
fn addresses_that_are_not_pull_requests_are_ignored() {
    let output = "https://github.com/o/r/issues/7 https://gitlab.com/o/r/pull/7 \
                  https://github.com/o/r/pull/7abc http://github.com/o/r/pull/8";
    assert_eq!(read("gh pr create --fill", output), []);
}

#[test]
fn a_call_names_a_bounded_number_of_pull_requests() {
    let output: String = (1..=MAX_FOUND as u64 + 5)
        .map(|number| format!("https://github.com/o/r/pull/{number}\n"))
        .collect();
    assert_eq!(read("gh pr edit --fill", &output).len(), MAX_FOUND);
}

#[test]
fn quotes_and_separators_split_words_as_a_shell_would() {
    let words = words("a 'b c' \"d \\\" e\" f\\ g;h&&i|j".to_owned());
    let text = |word: &str| Word::Text(word.to_owned());
    assert_eq!(
        words,
        [
            text("a"),
            text("b c"),
            text("d \" e"),
            text("f g"),
            Word::Break,
            text("h"),
            Word::Break,
            text("i"),
            Word::Break,
            text("j"),
        ]
    );
    assert_eq!(command_text("`echo \\`date\\``"), "echo `date`");
    assert!(runs("/opt/homebrew/bin/gh", "gh"));
    assert!(runs(r"C:\bin\gh.exe", "gh"));
    assert!(!runs("ghq", "gh"));
}
