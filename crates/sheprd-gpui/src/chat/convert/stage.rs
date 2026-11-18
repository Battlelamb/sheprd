//! How far a move's worker has gone, for typing that may stop an automatic
//! move. Typing into the pane before the first Ctrl+C keeps the draft and
//! the agent, so the move must not press it; typing after it changes
//! nothing, since the agent is already on its way out and only the chat can
//! continue its conversation.
//!
//! One atomic step decides which came first, so the worker never presses a
//! key after the window was told the move stopped, and the window never
//! stops a move that has pressed one.

use std::sync::atomic::{AtomicU8, Ordering};

const PREPARING: u8 = 0;
const INTERRUPTING: u8 = 1;
const TYPED: u8 = 2;

#[derive(Debug, Default)]
pub(crate) struct Stage(AtomicU8);

impl Stage {
    /// The worker is about to press Ctrl+C. False when typing reached the
    /// pane first, and no key may be pressed.
    pub(in crate::chat) fn interrupt(&self) -> bool {
        match self
            .0
            .compare_exchange(PREPARING, INTERRUPTING, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => true,
            Err(stage) => stage == INTERRUPTING,
        }
    }

    /// Typing reached the pane. True when the worker had pressed no key,
    /// and now never will; false when the move is past its first key and
    /// finishes.
    pub(crate) fn typed(&self) -> bool {
        self.0
            .compare_exchange(PREPARING, TYPED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Whether typing stopped the move before its first key.
    #[cfg(test)]
    pub(crate) fn stopped_by_typing(&self) -> bool {
        self.0.load(Ordering::Acquire) == TYPED
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
