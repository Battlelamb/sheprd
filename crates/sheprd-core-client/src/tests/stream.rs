use crate::{
    End,
    stream::{Attachment, Link, Route, Stream},
};
use std::{sync::Arc, time::Duration};

/// Bytes that arrived just before the connection dropped are read even
/// when the credit for them can no longer be sent; the loss shows next.
#[test]
fn bytes_taken_as_the_connection_drops_are_still_read() {
    let (outbound, queue) = crossbeam_channel::bounded(1);
    drop(queue);
    let route = Arc::new(Route::new());
    let stream = Stream {
        channel: 5,
        link: Link { outbound },
        route: Arc::clone(&route),
    };
    let terminal = serde_json::from_value(super::terminal("t1")).unwrap();
    let attachment = Attachment::new(stream, terminal);
    route.push(b"held").unwrap();
    let chunk = attachment.read(Duration::ZERO).unwrap();
    assert_eq!(chunk.bytes, b"held");
    assert_eq!(chunk.end, None);
    route.end(End::Lost);
    let chunk = attachment.read(Duration::ZERO).unwrap();
    assert!(chunk.bytes.is_empty());
    assert_eq!(chunk.end, Some(End::Lost));
}
