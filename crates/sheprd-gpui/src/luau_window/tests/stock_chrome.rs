use super::Data;
use moonframe::moonframe_core::{Node, Paint, Roots, Workspace};

#[test]
fn stock_header_and_status_mods_host_the_classic_controls() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("moonframe/app");
    let workspace = Workspace::open(Roots::new([root]), true);
    for name in ["header", "status"] {
        let node = workspace.slot(name).and_then(|slot| slot.view());
        let Some(Node::View(view)) = node else {
            panic!("the stock {name} mod must host a native view");
        };
        assert_eq!(view.name, "sheprd");
        assert_eq!(view.props.get("region").and_then(Data::as_str), Some(name));
    }
}

#[test]
fn orca_cards_show_the_host_wash_repository_tile_and_primary_badge() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("moonframe/app");
    let mut workspace = Workspace::open(Roots::new([root]), true);
    let tile = Data::map([
        ("letters", "DE".into()),
        ("background", "#202020".into()),
        ("foreground", "#dedede".into()),
    ]);
    let checkout = Data::map([
        ("id", "w1".into()),
        ("group", "demo".into()),
        ("label", "demo".into()),
        ("branch", "main".into()),
        ("focused", true.into()),
        ("repo_tile", tile),
    ]);
    let host = Data::map([
        ("id", "local".into()),
        ("wash", "#ff356f38".into()),
        ("workspaces", Data::List(vec![checkout])),
    ]);
    workspace.set_host(
        "sheprd_theme",
        Data::map([
            ("foreground", "#dedede".into()),
            ("sidebar", "#181818".into()),
        ]),
    );
    workspace.set_host(
        "sheprd",
        Data::map([
            ("layout", Data::map([("name", "orca".into())])),
            ("primary_label", "primary".into()),
            ("devices", Data::List(vec![host])),
        ]),
    );
    workspace.flush();
    let Some(slot) = workspace.slot("worktrees") else {
        panic!("expected the stock worktrees mod");
    };
    assert!(slot.error().is_none(), "{:?}", slot.error());
    let Some(node) = slot.view() else {
        panic!("expected worktree rows");
    };
    assert!(contains_wash(node, 0xff356f38));
    let tree = format!("{node:?}");
    for marker in ["repository-chip", "worktree-primary"] {
        assert!(tree.contains(marker), "missing {marker}");
    }
}

fn contains_wash(node: &Node, rgba: u32) -> bool {
    let Node::Stack(stack) = node else {
        return false;
    };
    matches!(stack.look.style.background.map(|color| color.paint),
        Some(Paint::Fixed(shade)) if shade.dark == rgba)
        || stack
            .children
            .iter()
            .any(|child| contains_wash(child, rgba))
}
