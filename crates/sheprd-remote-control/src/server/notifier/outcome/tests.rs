use super::*;
use std::process::Stdio;

fn run(repository: &Path, args: &[&str]) {
    let status = sheprd_client::new_command("git")
        .current_dir(repository)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn commit(repository: &Path, file: &str, date: &str) {
    std::fs::write(repository.join(file), file).unwrap();
    run(repository, &["add", file]);
    let date = format!("--date={date}");
    let status = sheprd_client::new_command("git")
        .current_dir(repository)
        .env("GIT_COMMITTER_DATE", &date[7..])
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            file,
            &date,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "commit {file}");
}

/// 2026-01-01T00:00:00Z, the run's start in these tests.
const START_S: u64 = 1_767_225_600;

#[test]
fn counts_the_runs_commits_and_what_is_left_uncommitted() {
    let parent = tempfile::tempdir().unwrap();
    let repository = parent.path().join("project");
    std::fs::create_dir(&repository).unwrap();
    run(&repository, &["init", "--quiet"]);
    commit(&repository, "before.txt", "@1767225000 +0000");
    commit(&repository, "during.txt", "@1767225700 +0000");
    std::fs::write(repository.join("before.txt"), "edited").unwrap();
    std::fs::write(repository.join("new.txt"), "new").unwrap();
    assert_eq!(
        read(&repository, START_S * 1000),
        Some(RunOutcome {
            commits: 1,
            pushed: false,
            uncommitted: 2,
        })
    );
}

#[test]
fn a_branch_its_upstream_has_is_pushed() {
    let parent = tempfile::tempdir().unwrap();
    let remote = parent.path().join("remote.git");
    let repository = parent.path().join("project");
    std::fs::create_dir(&repository).unwrap();
    run(parent.path(), &["init", "--quiet", "--bare", "remote.git"]);
    run(&repository, &["init", "--quiet"]);
    commit(&repository, "during.txt", "@1767225700 +0000");
    run(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run(&repository, &["push", "--quiet", "-u", "origin", "main"]);
    let outcome = read(&repository, START_S * 1000).unwrap();
    assert!(outcome.pushed);
    commit(&repository, "later.txt", "@1767225800 +0000");
    let outcome = read(&repository, START_S * 1000).unwrap();
    assert_eq!((outcome.commits, outcome.pushed), (2, false));
}

#[test]
fn each_new_file_in_a_new_folder_counts() {
    let parent = tempfile::tempdir().unwrap();
    let repository = parent.path().join("project");
    std::fs::create_dir_all(repository.join("new/deeper")).unwrap();
    run(&repository, &["init", "--quiet"]);
    commit(&repository, "before.txt", "@1767225000 +0000");
    for file in ["new/a.txt", "new/b.txt", "new/deeper/c.txt"] {
        std::fs::write(repository.join(file), file).unwrap();
    }
    let outcome = read(&repository, START_S * 1000).unwrap();
    assert_eq!(outcome.uncommitted, 3, "not the one folder");
}

#[test]
fn outside_a_checkout_there_is_none() {
    let folder = tempfile::tempdir().unwrap();
    assert_eq!(read(folder.path(), START_S * 1000), None);
}
