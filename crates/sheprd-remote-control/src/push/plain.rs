//! An agent's Markdown as the plain text a notification shows: no emphasis
//! or code marks, links as their text, and no `:big[…]` size marks
//! (`docs/rich-chat.md`, Text size and effects), as a lock screen draws
//! none of them and has few characters to spare.

use pulldown_cmark::{Event, Options, Parser, TagEnd};
use std::ops::Range;

/// `markdown` as plain text on one line. Whole directive lines are
/// [`super::directives::without_directives`]'s business.
pub(super) fn plain(markdown: &str) -> String {
    sized(markdown).0
}

/// [`plain`], led by the sentence the agent set larger: a `:big[…]`,
/// `:huge[…]` or `:giant[…]` span marks the outcome, which a short excerpt
/// should not cut off. The rest follows in its order.
pub(super) fn verdict_first(markdown: &str) -> String {
    let (text, span) = sized(markdown);
    let Some(span) = span else { return text };
    let sentence = sentence(&text, span);
    if sentence.start == 0 {
        return text;
    }
    let parts = [
        &text[sentence.clone()],
        &text[..sentence.start],
        &text[sentence.end..],
    ];
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The plain line and where its first larger span ended up.
fn sized(markdown: &str) -> (String, Option<Range<usize>>) {
    let mut text = String::new();
    for event in Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Text(part) | Event::Code(part) => text.push_str(&part),
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::BlockQuote(_),
            ) => text.push(' '),
            _ => {}
        }
    }
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    without_sizes(&line)
}

/// Whether `ch` ends a sentence when a space or the end follows it.
fn ends_sentence(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?')
}

/// The sentence of `text` holding `span`: from just after the last
/// sentence end before it to the first one at or after its end.
fn sentence(text: &str, span: Range<usize>) -> Range<usize> {
    let before = &text[..span.start];
    let start = before
        .char_indices()
        .rev()
        .find(|(at, ch)| ends_sentence(*ch) && before[at + ch.len_utf8()..].starts_with(' '))
        .map_or(0, |(at, ch)| at + ch.len_utf8());
    if text[..span.end]
        .chars()
        .next_back()
        .is_some_and(ends_sentence)
    {
        return start..span.end;
    }
    let after = &text[span.end..];
    let end = after
        .char_indices()
        .find(|(at, ch)| {
            let next = &after[at + ch.len_utf8()..];
            ends_sentence(*ch) && (next.is_empty() || next.starts_with(' '))
        })
        .map_or(text.len(), |(at, ch)| span.end + at + ch.len_utf8());
    start..end
}

/// The steps a size mark may name, as the chat draws them; only `small`
/// is no larger than the prose.
const SIZES: [&str; 4] = ["small", "big", "huge", "giant"];

/// Spans open at once; deeper openings stay text, as in the chat.
const MAX_OPEN: usize = 8;

/// `text` with each `:big[…]` mark reduced to what it holds, and where the
/// first outermost span larger than the prose ended up. A mark opens only
/// at the start or after a character that is not part of a word, so
/// `http://x:big[` stays text; brackets inside a span pair up, and a span
/// never closed loses only its opening.
fn without_sizes(text: &str) -> (String, Option<Range<usize>>) {
    let mut out = String::with_capacity(text.len());
    // Each open span's count of its own brackets still open.
    let mut open: Vec<usize> = Vec::new();
    let mut larger: Option<Range<usize>> = None;
    // Where the outermost span open now started, when it is larger.
    let mut opening: Option<usize> = None;
    let mut at = 0;
    while let Some(ch) = text[at..].chars().next() {
        let rest = &text[at + ch.len_utf8()..];
        match ch {
            ':' if open.len() < MAX_OPEN && starts_mark(&out) => {
                if let Some(name) = SIZES
                    .into_iter()
                    .find(|name| rest.strip_prefix(name).is_some_and(|r| r.starts_with('[')))
                {
                    if open.is_empty() && name != "small" && larger.is_none() {
                        opening = Some(out.len());
                    }
                    open.push(1);
                    at += 1 + name.len() + 1;
                    continue;
                }
                out.push(ch);
            }
            '[' if !open.is_empty() => {
                if let Some(depth) = open.last_mut() {
                    *depth += 1;
                }
                out.push(ch);
            }
            ']' if !open.is_empty() => {
                let closed = open.last_mut().is_some_and(|depth| {
                    *depth -= 1;
                    *depth == 0
                });
                if closed {
                    open.pop();
                    if open.is_empty()
                        && let Some(start) = opening.take()
                        && start < out.len()
                    {
                        larger = Some(start..out.len());
                    }
                } else {
                    out.push(ch);
                }
            }
            _ => out.push(ch),
        }
        at += ch.len_utf8();
    }
    (out, larger)
}

/// Whether a mark may open after `before`.
fn starts_mark(before: &str) -> bool {
    before
        .chars()
        .next_back()
        .is_none_or(|ch| !ch.is_alphanumeric() && ch != '/')
}
