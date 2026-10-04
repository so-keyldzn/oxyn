//! Streamed fragments held for the panel, sent together.
//!
//! A model streams a few characters at a time, and each fragment was one IPC
//! message — which Tauri delivers, under 8,192 bytes of JSON, by evaluating
//! script in the webview (`MAX_JSON_DIRECT_EXECUTE_THRESHOLD`, `tauri` 2.12.1,
//! `src/ipc/channel.rs`) — sent under the conversation's lock. Consecutive
//! fragments of one block merge here and go out every [`FLUSH_EVERY`] or
//! [`FLUSH_BYTES`], whichever comes first; any other event sends them first,
//! so the panel reads events in the order they came.
//!
//! The node's log has the fragments from the start: a view taken meanwhile
//! shows them, and empties the outbox so they do not arrive twice.

use std::time::{Duration, Instant};

use crate::ipc::ai::AiEvent;

/// How long fragments wait to be sent together. Under what reads as a
/// stutter, and it turns hundreds of messages a second into twenty-five.
pub(super) const FLUSH_EVERY: Duration = Duration::from_millis(40);

/// The size at which held fragments go without waiting for [`FLUSH_EVERY`]:
/// half of the bound under which Tauri evaluates a message rather than
/// fetching it, so a merged block stays on the cheap path.
const FLUSH_BYTES: usize = 4 * 1024;

/// What is held, for which node, since when.
#[derive(Default)]
pub(super) struct Outbox {
    held: Option<(u32, AiEvent)>,
    since: Option<Instant>,
    /// A timer will send what is held. One at a time, not one per fragment.
    pub(super) scheduled: bool,
}

impl Outbox {
    /// Takes what is held, to be sent now.
    pub(super) fn take(&mut self) -> Option<(u32, AiEvent)> {
        self.since = None;
        self.held.take()
    }

    /// Holds a fragment, merged into the block held when it continues it.
    ///
    /// Answers what it displaced — to send first — and whether what is held
    /// is due.
    pub(super) fn hold(&mut self, node: u32, event: AiEvent) -> (Option<(u32, AiEvent)>, bool) {
        let merged = match &mut self.held {
            Some((held_node, held)) if *held_node == node => merge(held, &event),
            _ => false,
        };
        let displaced = if merged {
            None
        } else {
            let displaced = self.take();
            self.held = Some((node, event));
            self.since = Some(Instant::now());
            displaced
        };
        let size = self.held.as_ref().map_or(0, |(_, held)| fragment_len(held));
        let due = size >= FLUSH_BYTES
            || self
                .since
                .is_some_and(|since| since.elapsed() >= FLUSH_EVERY);
        (displaced, due)
    }
}

/// Whether an event is a streamed fragment, merged before it is sent.
pub(super) fn is_fragment(event: &AiEvent) -> bool {
    matches!(
        event,
        AiEvent::TextDelta { .. } | AiEvent::ThinkingDelta { .. } | AiEvent::ToolArguments { .. }
    )
}

fn fragment_len(event: &AiEvent) -> usize {
    match event {
        AiEvent::TextDelta { text } | AiEvent::ThinkingDelta { text } => text.len(),
        AiEvent::ToolArguments { fragment, .. } => fragment.len(),
        _ => 0,
    }
}

/// Appends `more` to `held` when it continues the same block.
fn merge(held: &mut AiEvent, more: &AiEvent) -> bool {
    match (held, more) {
        (AiEvent::TextDelta { text }, AiEvent::TextDelta { text: more })
        | (AiEvent::ThinkingDelta { text }, AiEvent::ThinkingDelta { text: more }) => {
            text.push_str(more);
            true
        }
        (
            AiEvent::ToolArguments { index, fragment },
            AiEvent::ToolArguments {
                index: same,
                fragment: more,
            },
        ) if *index == *same => {
            fragment.push_str(more);
            true
        }
        _ => false,
    }
}
