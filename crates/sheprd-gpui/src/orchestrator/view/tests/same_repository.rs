//! Folders of one repository whose Git common directory was not read in
//! time are each a checkout, keyed by folder; the projects menu, header,
//! and picks still treat them as the one project their workers found.
use super::*;
use crate::orchestrator::{
    Repository, Request, Timing,
    repo::RepoInfo,
    view::{RepoSpec, projects::Entry, repo::Repo, rows::Tab},
};
use std::sync::Arc;

fn info(common: &str) -> RepoInfo {
    RepoInfo {
        repository: Repository::Local(common.into()),
        main_root: common.trim_end_matches("/.git").into(),
        beads: false,
        beads_root: None,
        remote: None,
    }
}

#[gpui::test]
fn folders_of_one_repository_are_one_project(cx: &mut gpui::TestAppContext) {
    let (dir, view, cx) = empty::open(cx);
    cx.update(|_, cx| {
        view.update(cx, |view, _| {
            for folder in ["apps", "packages", "other"] {
                let request = Request {
                    target: sheprd_client::ConnectTarget::Local,
                    checkout: dir.path().join(folder).to_string_lossy().into_owned(),
                    workspace_id: folder.into(),
                    token: None,
                    data_root: Some(dir.path().join(format!("{folder}-data"))),
                    azure_store: Default::default(),
                    gitlab_store: Default::default(),
                    timing: Timing::default(),
                };
                view.repos.push(Repo::start(RepoSpec {
                    key: folder.into(),
                    host: "Local".into(),
                    request,
                }));
            }
            // The first three are folders of /code/hub; "other" is not.
            let first = view.repos[0].key.clone();
            for (index, repo) in view.repos.iter_mut().enumerate() {
                let common = if index < 3 {
                    "/code/hub/.git"
                } else {
                    "/code/other/.git"
                };
                repo.snapshot.repo = Some(info(common));
                let issue = item(
                    Provider::Github,
                    &(index + 1).to_string(),
                    "Issue",
                    "2026-10-04T00:00:00Z",
                );
                repo.snapshot.items = Arc::new(vec![issue]);
            }
            view.tab = Tab::Issues;
            view.refresh_rows();

            let entries = view.project_entries();
            let keys: Vec<_> = entries
                .iter()
                .map(|entry| match entry {
                    Entry::Repo { key, .. } => key.as_str(),
                    other => panic!("{other:?}"),
                })
                .collect();
            assert_eq!(keys, [first.as_str(), "other"]);
            assert_eq!(view.projects_listing().count(), 2);
            for folder in ["apps", "packages"] {
                assert_eq!(view.pick_of(folder), Some(first.clone()));
            }

            // Picking the repository lists every folder's items.
            view.projects = std::collections::HashSet::from([first.clone()]);
            view.refresh_rows();
            assert_eq!(view.keys().len(), 3);
        })
    });
}
