//! Standalone file links become attachment cards without reading the file.
//! The Markdown stays in the transcript, so older clients can still open it.

use crate::chat::{
    prepared::Part,
    rich::{Block, Breaks, Inline, Kind, parse_with},
    visual::Visuals,
};
use crate::terminal::{RowTarget, file_target};

pub(super) fn attachment(line: &str, visuals: &mut Visuals) -> Option<Part> {
    if !line.trim_start().starts_with('[') {
        return None;
    }
    let blocks = parse_with(line, Breaks::Soft, visuals)?;
    let [
        Block::Paragraph {
            kind: Kind::Paragraph,
            indent: 0,
            content,
        },
    ] = blocks.as_slice()
    else {
        return None;
    };
    let mut destination = None;
    for part in content {
        let Inline::Text(_, style) = part else {
            return None;
        };
        let link = style.link.as_deref()?;
        if destination.is_some_and(|known| known != link) {
            return None;
        }
        destination = Some(link);
    }
    let destination = destination?;
    // Source references with a line number remain ordinary prose links.
    let RowTarget::Path { line: None, .. } = file_target(destination)? else {
        return None;
    };
    Some(Part::File {
        destination: destination.to_owned(),
        blocks: blocks.into(),
    })
}
