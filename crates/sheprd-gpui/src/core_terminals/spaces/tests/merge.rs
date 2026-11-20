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

#[test]
fn a_zoom_here_and_a_rename_elsewhere_of_one_tab_both_stay() {
    let mut core = FakeCore::default();
    let mut desk = Client::new();
    let api = desk.keep("api", "/src/api");
    let shown = desk.open(&api, "t1");
    let mut panes = Panes::single(id("t1"));
    panes
        .split(&id("t1"), crate::core_terminals::panes::Axis::Row, id("t2"))
        .unwrap();
    desk.store.visited(
        shown,
        Some(Location::core_terminal(panes.clone(), false)),
        None,
    );
    desk.settle(&mut core);
    let mut laptop = Client::new();
    laptop.settle(&mut core);

    let read = desk.send().unwrap();
    assert_eq!(read, Job::Read);
    // The laptop names the tab while the desk zooms a pane.
    let theirs = laptop.store.in_scope(&scope()).next().unwrap().id;
    let mut named = Location::core_terminal(panes.clone(), false);
    if let Location::CoreTerminal { name, .. } = &mut named {
        *name = Some("api".into());
    }
    laptop.store.visited(theirs, Some(named), None);
    laptop.settle(&mut core);
    let mut zoomed = Location::core_terminal(panes, false);
    if let Location::CoreTerminal { zoomed: zoom, .. } = &mut zoomed {
        *zoom = Some(id("t2"));
    }
    desk.store.visited(shown, Some(zoomed), None);
    let answered = core.run(read);
    desk.finish(answered);
    desk.settle(&mut core);
    laptop.settle(&mut core);

    let tab = core.spaces().tabs()[0].clone();
    assert_eq!(tab.name.as_deref(), Some("api"), "the rename stays");
    assert_eq!(tab.zoomed, Some(id("t2")), "and so does the zoom");
    let Some(Location::CoreTerminal { name, zoomed, .. }) =
        &desk.store.get(shown).unwrap().location
    else {
        panic!("a terminal tab");
    };
    assert_eq!(name.as_deref(), Some("api"));
    assert_eq!(*zoomed, Some(id("t2")));
}
