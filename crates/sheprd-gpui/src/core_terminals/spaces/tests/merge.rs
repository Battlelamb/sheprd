use super::*;

#[test]
fn a_new_title_here_does_not_hold_back_changes_from_elsewhere() {
    let mut core = FakeCore::default();
    let mut desk = Client::new();
    let api = desk.keep("api", "/src/api");
    let shown = desk.open(&api, "t1");
    desk.settle(&mut core);
    let mut laptop = Client::new();
    laptop.settle(&mut core);

    desk.store.visited(shown, None, Some("vim"));
    assert!(laptop.store.rename_kept(&scope(), &api, "backend"));
    laptop.settle(&mut core);
    desk.settle(&mut core);
    assert_eq!(desk.labels(), ["backend"]);
    assert_eq!(desk.store.get(shown).unwrap().title, "vim");
}

#[test]
fn changes_on_both_sides_merge_record_by_record() {
    let mut core = FakeCore::default();
    let mut desk = Client::new();
    let api = desk.keep("api", "/src/api");
    let web = desk.keep("web", "/src/web");
    desk.settle(&mut core);
    let mut laptop = Client::new();
    laptop.settle(&mut core);

    let read = desk.send().unwrap();
    assert_eq!(read, Job::Read);
    assert!(laptop.store.rename_kept(&scope(), &api, "backend"));
    laptop.settle(&mut core);
    // Changed here while the read was out: kept, and written after.
    assert!(desk.store.rename_kept(&scope(), &web, "site"));
    let answered = core.run(read);
    desk.finish(answered);
    assert_eq!(desk.labels(), ["backend", "site"]);
    desk.settle(&mut core);
    laptop.settle(&mut core);
    assert_eq!(laptop.labels(), ["backend", "site"]);
}

#[test]
fn a_full_store_keeps_every_tab_it_had() {
    let mut core = FakeCore::default();
    let mut desk = Client::new();
    let api = desk.keep("api", "/src/api");
    let shown = desk.open(&api, "t1");
    desk.settle(&mut core);
    // Another computer keeps a workspace first in the document.
    let mut laptop = Client::new();
    laptop.settle(&mut core);
    let early = laptop.keep("early", "/src/early");
    laptop.settle(&mut core);
    let mut spaces = core.spaces();
    spaces
        .apply(Edit::MoveWorkspace {
            id: early.clone(),
            before: Some(api.clone()),
        })
        .unwrap();
    core.value = spaces.to_value().unwrap();
    core.version += 1;
    // This app has room for one more workspace of this host only.
    let elsewhere = Scope::endpoint("ssh:other");
    while desk.store.kept_room() > 1 {
        desk.store
            .keep_workspace(&elsewhere, "other", "", false)
            .unwrap();
    }
    let page = desk.store.open(scope(), &api, None, None).unwrap();
    desk.store
        .keep_workspace(&elsewhere, "full", "", false)
        .unwrap();
    desk.settle(&mut core);
    assert_eq!(desk.labels(), ["api"], "no room for the new one");
    assert!(desk.store.get(shown).is_some());
    assert!(desk.store.get(page).is_some());
    assert!(
        core.spaces().workspace(&early).is_some(),
        "nor is it forgotten"
    );
}
