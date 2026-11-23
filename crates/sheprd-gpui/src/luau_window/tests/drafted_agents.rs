//! Moonframe's agents list draws a pencil on the row of an agent whose chat
//! holds a draft not sent, and none on the others.
#![allow(clippy::unwrap_used)]

use moonframe::{
    Data,
    moonframe_core::{Roots, Workspace},
};
use std::path::PathBuf;

fn agents_view(drafted: bool) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("moonframe/app");
    let mut workspace = Workspace::open(Roots::new([root]), true);
    let agent = Data::map([
        ("name", "Claude".into()),
        ("pane", "p1".into()),
        ("endpoint", "local".into()),
        ("status", "idle".into()),
        ("drafted", drafted.into()),
    ]);
    workspace.set_host(
        "sheprd",
        Data::map([
            ("agents", Data::List(vec![agent])),
            ("agents_list", true.into()),
        ]),
    );
    workspace.flush();
    let slot = workspace.slot("agents").unwrap();
    assert!(slot.error().is_none(), "{:?}", slot.error());
    format!("{:?}", slot.view())
}

#[test]
fn a_drafted_agent_has_a_pencil() {
    assert!(agents_view(true).contains("agent-drafted"));
    assert!(!agents_view(false).contains("agent-drafted"));
}
