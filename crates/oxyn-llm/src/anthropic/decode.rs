//! From the SSE frame to domain events, Anthropic side.
//!
//! This module holds the state of a generation stream. It is **pure**: no
//! input-output, no network. That is what allows testing it on the provider's
//! real sequences, quirks included, without calling one.
//!
//! # This protocol names its blocks, and that is the whole difference
//!
//! Where an OpenAI-compatible stream reassembles fragments linked by an array
//! `index`, this one explicitly opens and closes each content block:
//! `content_block_start`, `content_block_delta`s, `content_block_stop`. The
//! block's type is given at opening, and the deltas that follow depend on it —
//! an `input_json_delta` only makes sense in a `tool_use` block.
//!
//! Two rules follow:
//!
//! 1. **The state of a block is kept at the index the server announces**,
//!    never inferred from the order of arrival;
//! 2. **a delta whose block is unknown is ignored**, not guessed. The only
//!    other choice would be to invent a block, hence to invent its type.
//!
//! # When `Done` is emitted
//!
//! `message_delta` carries the stop reason but **does not end** the stream:
//! `message_stop` follows, and the cumulative usage arrives with the
//! `message_delta`. So the reason is recorded and `Done` is only emitted at
//! `message_stop`, when the stream closes, or on an error. Exactly once.
//!
//! # An observed end is not an announced end
//!
//! The protocol concludes every message with `message_stop`, and every block
//! with `content_block_stop`. Their absence is therefore **information**: a
//! proxy or a load balancer that cleanly closes the connection in the middle
//! of a generation produces no transport error, only a stream that stops.
//!
//! Hence two rules, which hold whatever the way the stream closed:
//!
//! 1. **a stream without `message_stop` is [`StopReason::Interrupted`]** — even
//!    if a `message_delta` had announced a reason: the server may have
//!    produced, and billed, more than what was received (I-13);
//! 2. **only `content_block_stop` closes a block.** A tool call whose JSON
//!    happens to parse is not a call the model finished writing: it is thrown
//!    away, never proposed.
//!
//! # Everything that accumulates is counted
//!
//! Each open block, each fragment of text, reasoning, signature or arguments
//! goes through a [`GenerationBudget`] **before** being kept or emitted. A
//! block opened without ever being closed costs too: it carries a state until
//! it closes. At the first overrun, the generation stops: see
//! [`crate::budget`].

use std::collections::BTreeMap;

use super::wire::{ContentBlock, Envelope, WireError};
use crate::budget::{BudgetExceeded, GenerationBudget};
use crate::error::classify_json_error;
use crate::reasoning::ReasoningBlock;
use crate::sse::SseFrame;
use crate::stream::EventDecoder;
use crate::types::{ChatEvent, StopReason, ToolCall};

/// Number of unreadable frames tolerated before giving up the stream.
///
/// An isolated unreadable frame happens; a series means we do not speak the
/// same protocol, and continuing would only flood the interface with errors.
pub(crate) const MAX_DECODE_ERRORS: usize = 8;

/// What a content block accumulates, according to its type.
#[derive(Debug)]
enum PartialBlock {
    /// Text block: nothing to accumulate, the fragments are emitted on the fly.
    Text,
    /// Tool call being reassembled.
    ToolUse {
        id: String,
        name: String,
        arguments: String,
    },
    /// Reasoning being reassembled.
    Thinking {
        text: String,
        signature: Option<String>,
    },
    /// Encrypted reasoning: complete as soon as it opens.
    RedactedThinking { data: String },
    /// Block of a type this version does not know.
    ///
    /// It is **tracked** rather than ignored: without it, the deltas targeting
    /// it would be counted as targeting an unknown block, and a perfectly valid
    /// frame would pass for an anomaly.
    Unknown,
}

/// State of an Anthropic generation stream.
#[derive(Debug, Default)]
pub(crate) struct MessageDecoder {
    blocks: BTreeMap<u32, PartialBlock>,
    budget: GenerationBudget,
    stop: Option<StopReason>,
    done: bool,
    errors: usize,
}

impl MessageDecoder {
    /// Decoder for a fresh stream.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Is the stream finished?
    pub(crate) fn finished(&self) -> bool {
        self.done
    }

    /// Consumes the `data` field of an SSE frame.
    pub(crate) fn on_data(&mut self, data: &str, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        let data = data.trim();
        if data.is_empty() {
            return;
        }

        let frame: Envelope = match serde_json::from_str(data) {
            Ok(frame) => frame,
            Err(err) => {
                // The faulty frame is not copied: we do not know what a proxy
                // puts in it.
                self.errors += 1;
                out.push(ChatEvent::Error(format!(
                    "unreadable stream frame ({}, line {}, column {})",
                    classify_json_error(&err),
                    err.line(),
                    err.column()
                )));
                if self.errors >= MAX_DECODE_ERRORS {
                    self.stop = Some(StopReason::Interrupted);
                    self.emit_done(out);
                }
                return;
            }
        };

        let count = match frame.r#type.as_deref() {
            Some("content_block_start") => self.on_block_start(&frame, out),
            Some("content_block_delta") => self.on_block_delta(&frame, out),
            _ => Ok(()),
        };
        if let Err(limit) = count {
            self.exceed(limit, out);
            return;
        }

        match frame.r#type.as_deref() {
            Some("message_start") => self.on_message_start(&frame, out),
            // Handled above, where their cost is counted.
            Some("content_block_start" | "content_block_delta") => {}
            Some("content_block_stop") => self.on_block_stop(&frame, out),
            Some("message_delta") => self.on_message_delta(&frame, out),
            Some("message_stop") => self.on_message_stop(out),
            Some("error") => self.on_error(frame.error.as_ref(), out),
            // `ping` is a connection keep-alive: it has nothing to say.
            Some("ping") => {}
            // The documentation announces that new event types can appear. An
            // unknown type is ignored: refusing it would break the stream at
            // the protocol's first evolution.
            _ => {}
        }
    }

    /// `message_start`: the input usage is already known.
    fn on_message_start(&mut self, frame: &Envelope, out: &mut Vec<ChatEvent>) {
        let Some(usage) = frame.message.as_ref().and_then(|m| m.usage.as_ref()) else {
            return;
        };
        if usage.is_empty() {
            return;
        }
        // Cache tokens are only announced here: the final `message_delta`
        // does not always repeat them. Withholding them would wait for a frame
        // that may never come.
        out.push(ChatEvent::Usage {
            prompt_tokens: usage.input(),
            completion_tokens: usage.output(),
            cache_write_tokens: usage.cache_write(),
            cache_read_tokens: usage.cache_read(),
            // This protocol does not bill reasoning separately: it is counted
            // in the output. Declaring `Some(0)` would be wrong.
            reasoning_tokens: None,
        });
    }

    /// `content_block_start`: a block opens, its type is given.
    ///
    /// # Errors
    /// The budget this block would exceed; it is then not opened.
    fn on_block_start(
        &mut self,
        frame: &Envelope,
        out: &mut Vec<ChatEvent>,
    ) -> Result<(), BudgetExceeded> {
        let Some(index) = frame.index else {
            return Ok(());
        };
        let Some(block) = frame.content_block.as_ref() else {
            return Ok(());
        };
        let length = |field: &Option<String>| field.as_ref().map_or(0, String::len);
        if block.r#type.as_deref() == Some("tool_use") {
            self.budget.open_tool_call()?;
            self.budget
                .charge_tool_name(index, 0, length(&block.name))?;
            self.budget.charge(length(&block.id))?;
        } else {
            self.budget.open_block()?;
            self.budget.charge(
                length(&block.thinking)
                    .saturating_add(length(&block.signature))
                    .saturating_add(length(&block.data)),
            )?;
        }

        let partial = match block.r#type.as_deref() {
            Some("text") => PartialBlock::Text,
            Some("tool_use") => {
                let id = block.id.clone().unwrap_or_else(|| synthetic_id(index));
                let name = block.name.clone().unwrap_or_default();
                if !name.is_empty() {
                    out.push(ChatEvent::ToolCallStarted {
                        index,
                        id: id.clone(),
                        name: name.clone(),
                    });
                }
                PartialBlock::ToolUse {
                    id,
                    name,
                    arguments: String::new(),
                }
            }
            Some("thinking") => PartialBlock::Thinking {
                text: block.thinking.clone().unwrap_or_default(),
                signature: block.signature.clone(),
            },
            Some("redacted_thinking") => PartialBlock::RedactedThinking {
                data: block.data.clone().unwrap_or_default(),
            },
            // `server_tool_use`, web search results, blocks to come: they
            // exist, Oxyn offers none of them, and they produce nothing here.
            _ => PartialBlock::Unknown,
        };
        self.blocks.insert(index, partial);
        Self::note_unused(block);
        Ok(())
    }

    /// Accepts without doing anything a block whose fields are not used.
    ///
    /// Only serves to make the intent explicit to the reader: all the fields
    /// of [`ContentBlock`] are read above, and the one that is not is left
    /// deliberately.
    const fn note_unused(_block: &ContentBlock) {}

    /// `content_block_delta`: a fragment arrives for an open block.
    ///
    /// # Errors
    /// The budget this fragment would exceed; nothing of it is kept then.
    fn on_block_delta(
        &mut self,
        frame: &Envelope,
        out: &mut Vec<ChatEvent>,
    ) -> Result<(), BudgetExceeded> {
        let (Some(index), Some(delta)) = (frame.index, frame.delta.as_ref()) else {
            return Ok(());
        };
        // A delta targeting a block never opened is ignored: inventing the
        // block would amount to inventing its type, hence the meaning of what
        // is accumulated.
        let Some(block) = self.blocks.get_mut(&index) else {
            return Ok(());
        };
        let budget = &mut self.budget;

        match (delta.r#type.as_deref(), block) {
            (Some("text_delta"), PartialBlock::Text) => {
                if let Some(text) = delta.text.clone()
                    && !text.is_empty()
                {
                    budget.charge(text.len())?;
                    out.push(ChatEvent::TextDelta(text));
                }
            }
            (Some("input_json_delta"), PartialBlock::ToolUse { arguments, .. }) => {
                if let Some(fragment) = delta.partial_json.clone()
                    && !fragment.is_empty()
                {
                    budget.charge_tool_arguments(index, arguments.len(), fragment.len())?;
                    arguments.push_str(&fragment);
                    out.push(ChatEvent::ToolCallDelta {
                        index,
                        arguments: fragment,
                    });
                }
            }
            (Some("thinking_delta"), PartialBlock::Thinking { text, .. }) => {
                if let Some(fragment) = delta.thinking.clone()
                    && !fragment.is_empty()
                {
                    budget.charge(fragment.len())?;
                    text.push_str(&fragment);
                    out.push(ChatEvent::ReasoningDelta {
                        index,
                        text: fragment,
                    });
                }
            }
            (Some("signature_delta"), PartialBlock::Thinking { signature, .. }) => {
                // The signature arrives in one go, just before the block
                // closes. It is not emitted: it only makes sense at the next
                // turn, and it is never displayed.
                if let Some(value) = delta.signature.clone() {
                    budget.charge(value.len())?;
                    *signature = Some(value);
                }
            }
            // A delta whose type does not match the block's is an
            // inconsistency of the server, not data to save.
            _ => {}
        }
        Ok(())
    }

    /// `content_block_stop`: the block is complete.
    fn on_block_stop(&mut self, frame: &Envelope, out: &mut Vec<ChatEvent>) {
        let Some(index) = frame.index else {
            return;
        };
        let Some(block) = self.blocks.remove(&index) else {
            return;
        };
        Self::close_block(index, block, out);
    }

    /// Closes a block and emits what it produces.
    fn close_block(index: u32, block: PartialBlock, out: &mut Vec<ChatEvent>) {
        match block {
            PartialBlock::Text | PartialBlock::Unknown => {}
            PartialBlock::ToolUse {
                id,
                name,
                arguments,
            } => {
                if name.is_empty() {
                    out.push(ChatEvent::Error(format!(
                        "tool call #{index} has no name and was dropped"
                    )));
                    return;
                }
                out.push(tool_call_event(id, name, &arguments));
            }
            PartialBlock::Thinking { text, signature } => {
                out.push(ChatEvent::ReasoningComplete {
                    index,
                    block: ReasoningBlock::Summarized { text, signature },
                });
            }
            PartialBlock::RedactedThinking { data } => {
                out.push(ChatEvent::ReasoningComplete {
                    index,
                    block: ReasoningBlock::Redacted { data },
                });
            }
        }
    }

    /// `message_delta`: the stop reason and the cumulative usage.
    fn on_message_delta(&mut self, frame: &Envelope, out: &mut Vec<ChatEvent>) {
        if let Some(reason) = frame.delta.as_ref().and_then(|d| d.stop_reason.as_deref()) {
            // The generation is finished; the stream, not yet.
            self.stop = Some(stop_reason(reason));
        }
        if let Some(usage) = frame.usage.as_ref()
            && !usage.is_empty()
        {
            out.push(ChatEvent::Usage {
                prompt_tokens: usage.input(),
                completion_tokens: usage.output(),
                cache_write_tokens: usage.cache_write(),
                cache_read_tokens: usage.cache_read(),
                reasoning_tokens: None,
            });
        }
    }

    /// `message_stop`: end of the stream, announced by the server.
    ///
    /// A block still open here is an inconsistency of the server: it is thrown
    /// away as when the stream closes, and the announced reason is kept.
    fn on_message_stop(&mut self, out: &mut Vec<ChatEvent>) {
        self.discard_open_blocks(out);
        self.emit_done(out);
    }

    /// An error arrived **in** the stream, after a `200` status.
    fn on_error(&mut self, error: Option<&WireError>, out: &mut Vec<ChatEvent>) {
        let message = error.map_or_else(
            || "no details given".to_owned(),
            super::wire::WireError::describe,
        );
        out.push(ChatEvent::Error(message));
        // The blocks in progress are **thrown away**, not closed: after an
        // error, a half-received tool call is not a proposed action.
        self.blocks.clear();
        self.stop = Some(StopReason::ProviderError);
        self.emit_done(out);
    }

    /// Stops the generation on an exceeded budget.
    ///
    /// The error names the limit; the open blocks are **thrown away** — a cut
    /// tool call is not a proposed action — and the end is a cut: the provider
    /// may have continued, and billed, what we stopped reading (I-13).
    fn exceed(&mut self, limit: BudgetExceeded, out: &mut Vec<ChatEvent>) {
        out.push(ChatEvent::Error(limit.to_string()));
        self.discard_open_blocks(out);
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
    }

    /// Throws away the blocks that did not receive their `content_block_stop`.
    ///
    /// They are **not** closed: a block the server did not close was cut. Tool
    /// arguments that happen to form valid JSON do not say the model had
    /// finished writing them, and a reasoning without its signature would be
    /// refused at the next turn.
    ///
    /// The text already emitted cannot be taken back; a thrown away tool call
    /// or reasoning is reported, so that its disappearance shows.
    fn discard_open_blocks(&mut self, out: &mut Vec<ChatEvent>) {
        for (index, block) in std::mem::take(&mut self.blocks) {
            match block {
                PartialBlock::Text | PartialBlock::Unknown => {}
                PartialBlock::ToolUse { .. } => out.push(ChatEvent::Error(format!(
                    "tool call #{index} was not closed by the provider and was dropped"
                ))),
                PartialBlock::Thinking { .. } | PartialBlock::RedactedThinking { .. } => {
                    out.push(ChatEvent::Error(format!(
                        "reasoning block #{index} was not closed by the provider and was dropped"
                    )));
                }
            }
        }
    }

    /// Emits `Done` once and only once.
    fn emit_done(&mut self, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        self.done = true;
        out.push(ChatEvent::Done {
            stop_reason: self.stop.take().unwrap_or(StopReason::Unspecified),
        });
    }
}

impl EventDecoder for MessageDecoder {
    /// The name of the SSE event is **ignored** in favor of the payload's
    /// `type` field: both are redundant in this protocol, and the payload is
    /// what a proxy has the least reason to alter.
    fn on_frame(&mut self, frame: &SseFrame, out: &mut Vec<ChatEvent>) {
        self.on_data(&frame.data, out);
    }

    fn is_done(&self) -> bool {
        self.finished()
    }

    /// The server closed the stream **without** `message_stop`.
    ///
    /// Otherwise the decoder would already be finished and the driver would
    /// not call it; the guard covers a direct call. Clean close or not, it is a
    /// cut: see the module note.
    fn finish(&mut self, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        out.push(ChatEvent::Error(
            "the stream ended before the provider announced the end of the message".to_owned(),
        ));
        self.discard_open_blocks(out);
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
    }

    /// The blocks in progress are **thrown away**.
    ///
    /// Truncated arguments are not arguments, and a reasoning without its
    /// signature would be refused at the next turn: proposing either would be
    /// worse than proposing nothing.
    fn cancel(&mut self, out: &mut Vec<ChatEvent>) {
        self.blocks.clear();
        self.stop = Some(StopReason::Cancelled);
        self.emit_done(out);
    }

    fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        out.push(ChatEvent::Error(detail));
        self.blocks.clear();
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
    }
}

/// Translates the `stop_reason` of Anthropic's protocol.
///
/// Checked on 2026-09-16, source in
/// [`RESEARCH-NOTES`](../../../../docs/RESEARCH-NOTES.md) (I-12). A function of
/// this module and not a method of [`StopReason`]: the type lives in
/// `oxyn-core`, which knows no protocol. An unknown value is **kept**: the
/// documentation announces this list can grow, and folding the unknown into
/// `EndTurn` would pass an incomplete response off as a finished one.
pub(crate) fn stop_reason(raw: &str) -> StopReason {
    match raw {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        "tool_use" => StopReason::ToolCalls,
        "pause_turn" => StopReason::Paused,
        "refusal" => StopReason::Refusal,
        "model_context_window_exceeded" => StopReason::ContextWindowExceeded,
        other => StopReason::Other(other.to_owned()),
    }
}

/// Makes up a call identifier when the server gives none.
fn synthetic_id(index: u32) -> String {
    format!("toolu_{index}")
}

/// Rebuilds a complete tool call from its fragments.
///
/// Unreadable arguments do not lose the call: it becomes
/// [`ChatEvent::ToolCallInvalid`], which the caller answers with a rejection,
/// and the turn's other calls stand.
fn tool_call_event(id: String, name: String, arguments: &str) -> ChatEvent {
    let raw = arguments.trim();
    if raw.is_empty() {
        // A tool without parameters: the block opens with `input: {}` and no
        // delta follows.
        return ChatEvent::ToolCallComplete(ToolCall::new(id, name, serde_json::json!({})));
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => ChatEvent::ToolCallComplete(ToolCall::new(id, name, value)),
        // The arguments string is not copied: it is a model output, and can
        // copy what the model was given.
        Err(err) => ChatEvent::ToolCallInvalid {
            id,
            name,
            detail: format!(
                "the arguments are not valid JSON: {} (line {}, column {})",
                classify_json_error(&err),
                err.line(),
                err.column()
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_stop_reasons_are_translated() {
        for (raw, expected) in [
            ("end_turn", StopReason::EndTurn),
            ("max_tokens", StopReason::MaxTokens),
            ("stop_sequence", StopReason::StopSequence),
            ("tool_use", StopReason::ToolCalls),
            ("pause_turn", StopReason::Paused),
            ("refusal", StopReason::Refusal),
            (
                "model_context_window_exceeded",
                StopReason::ContextWindowExceeded,
            ),
        ] {
            assert_eq!(stop_reason(raw), expected, "{raw}");
        }
        assert_eq!(
            stop_reason("future_reason"),
            StopReason::Other("future_reason".to_owned()),
            "the documentation says this list may grow"
        );
    }

    /// Plays a sequence of `data` fields and returns all the events produced.
    fn play(frames: &[&str]) -> Vec<ChatEvent> {
        let mut decoder = MessageDecoder::new();
        let mut outputs = Vec::new();
        for frame in frames {
            decoder.on_data(frame, &mut outputs);
        }
        if !decoder.finished() {
            decoder.finish(&mut outputs);
        }
        outputs
    }

    fn texts(events: &[ChatEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::TextDelta(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    fn reasoning(events: &[ChatEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ReasoningDelta { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn calls(events: &[ChatEvent]) -> Vec<&ToolCall> {
        events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ToolCallComplete(call) => Some(call),
                _ => None,
            })
            .collect()
    }

    fn blocks(events: &[ChatEvent]) -> Vec<&ReasoningBlock> {
        events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ReasoningComplete { block, .. } => Some(block),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_text_stream_is_reassembled_in_order() {
        let events = play(&[
            r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":25,"output_tokens":1}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"SELECT "}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"1"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":15}}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(texts(&events), "SELECT 1");
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::EndTurn
            })
        );
    }

    #[test]
    fn a_keep_alive_produces_nothing() {
        let events = play(&[
            r#"{"type":"ping"}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(events.len(), 1, "seul `Done` doit rester : {events:?}");
        assert!(events[0].is_terminal());
    }

    #[test]
    fn done_is_emitted_exactly_once() {
        let mut decoder = MessageDecoder::new();
        let mut outputs = Vec::new();
        decoder.on_data(
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            &mut outputs,
        );
        decoder.on_data(r#"{"type":"message_stop"}"#, &mut outputs);
        // The server closes after `message_stop`: `finish` must add nothing.
        decoder.finish(&mut outputs);
        assert_eq!(outputs.iter().filter(|e| e.is_terminal()).count(), 1);
    }

    #[test]
    fn cache_usage_is_reported_both_ways() {
        let events = play(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":50,"output_tokens":1,"cache_creation_input_tokens":148,"cache_read_input_tokens":2000}}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(
            events.contains(&ChatEvent::Usage {
                prompt_tokens: 50,
                completion_tokens: 1,
                cache_write_tokens: Some(148),
                cache_read_tokens: Some(2000),
                reasoning_tokens: None,
            }),
            "{events:?}"
        );
    }

    #[test]
    fn an_undeclared_usage_is_not_invented() {
        let events = play(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        let Some(ChatEvent::Usage {
            cache_write_tokens,
            cache_read_tokens,
            ..
        }) = events.iter().find(|e| matches!(e, ChatEvent::Usage { .. }))
        else {
            panic!("no usage: {events:?}");
        };
        assert_eq!(*cache_write_tokens, None);
        assert_eq!(*cache_read_tokens, None);
    }

    #[test]
    fn a_fragmented_tool_call_is_reassembled() {
        let events = play(&[
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"execute_query","input":{}}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"sql\":"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"SELECT 1\"}"}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        let complete = calls(&events);
        assert_eq!(complete.len(), 1, "{events:?}");
        assert_eq!(complete[0].id, "toolu_01");
        assert_eq!(complete[0].name, "execute_query");
        assert_eq!(complete[0].arguments["sql"], "SELECT 1");

        assert!(events.contains(&ChatEvent::ToolCallStarted {
            index: 1,
            id: "toolu_01".to_owned(),
            name: "execute_query".to_owned(),
        }));
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::ToolCalls
            })
        );
    }

    #[test]
    fn two_tool_blocks_do_not_mix() {
        // This protocol closes a block before opening another, but nothing
        // forces it to: the state is kept by index, not by order of arrival.
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"lire"}}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"b","name":"ecrire"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"t\":1}"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"u\":2}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        let complete = calls(&events);
        assert_eq!(complete.len(), 2, "{events:?}");
        assert_eq!(complete[0].arguments, serde_json::json!({"t": 1}));
        assert_eq!(complete[1].arguments, serde_json::json!({"u": 2}));
    }

    #[test]
    fn a_tool_without_arguments_receives_an_empty_object() {
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"lister","input":{}}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(calls(&events)[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn invalid_arguments_are_an_invalid_call_and_the_sibling_stands() {
        // The fine-grained stream is not validated by the server: the
        // accumulated string may not be JSON. It costs this call, not the
        // turn: the valid call next to it is still proposed.
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"execute"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\": \"SELECT secret"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"d","name":"lister","input":{}}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        let complete = calls(&events);
        assert_eq!(complete.len(), 1, "{events:?}");
        assert_eq!(complete[0].id, "d");
        let invalid = events
            .iter()
            .find_map(|e| match e {
                ChatEvent::ToolCallInvalid { id, name, detail } => Some((id, name, detail)),
                _ => None,
            })
            .expect("the invalid call is reported as such");
        assert_eq!((invalid.0.as_str(), invalid.1.as_str()), ("c", "execute"));
        assert!(
            !events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "not a stream error: {events:?}"
        );
        assert!(
            !invalid.2.contains("secret"),
            "the model output must not be copied: {}",
            invalid.2
        );
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::ToolCalls
            })
        );
    }

    #[test]
    fn a_reasoning_block_is_reassembled_with_its_signature() {
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"I set 1071 = 2 × 462 + 147"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":", then 462 = 3 × 147 + 21"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCgIYAhIM"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(
            reasoning(&events),
            "I set 1071 = 2 × 462 + 147, then 462 = 3 × 147 + 21"
        );
        let blocks = blocks(&events);
        assert_eq!(blocks.len(), 1, "{events:?}");
        assert_eq!(
            blocks[0],
            &ReasoningBlock::Summarized {
                text: "I set 1071 = 2 × 462 + 147, then 462 = 3 × 147 + 21".to_owned(),
                signature: Some("EqQBCgIYAhIM".to_owned()),
            }
        );
    }

    #[test]
    fn a_masked_reasoning_remains_transportable() {
        // The default of several models: the block arrives without text, but
        // with its signature. It must still be rendered, otherwise the next
        // turn is refused.
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EosnCkYICxIM"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(reasoning(&events), "", "nothing to display");
        let blocks = blocks(&events);
        assert_eq!(blocks.len(), 1, "the block must exist all the same");
        assert_eq!(blocks[0].display_text(), None);
    }

    #[test]
    fn an_encrypted_reasoning_produces_no_text() {
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"EvwBCoYBGAIiQL"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(reasoning(&events), "");
        let blocks = blocks(&events);
        assert_eq!(blocks.len(), 1, "{events:?}");
        assert!(blocks[0].is_redacted());
        assert_eq!(blocks[0].display_text(), None);
    }

    #[test]
    fn a_cancellation_in_the_middle_of_a_reasoning_returns_no_incomplete_block() {
        // A block without its signature would be refused at the next turn:
        // rendering it would make the following request fail, far from here.
        let mut decoder = MessageDecoder::new();
        let mut outputs = Vec::new();
        decoder.on_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            &mut outputs,
        );
        decoder.on_data(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"I start with"}}"#,
            &mut outputs,
        );
        decoder.cancel(&mut outputs);

        assert!(blocks(&outputs).is_empty(), "{outputs:?}");
        assert_eq!(
            outputs.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            })
        );
    }

    #[test]
    fn a_cancellation_in_the_middle_of_a_tool_call_proposes_nothing() {
        let mut decoder = MessageDecoder::new();
        let mut outputs = Vec::new();
        decoder.on_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"drop_table"}}"#,
            &mut outputs,
        );
        decoder.on_data(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"name\":"}}"#,
            &mut outputs,
        );
        decoder.cancel(&mut outputs);

        assert!(
            calls(&outputs).is_empty(),
            "a truncated call never becomes a proposed action: {outputs:?}"
        );
        assert_eq!(
            outputs.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            })
        );
    }

    #[test]
    fn a_refusal_is_distinct_from_an_end_of_turn() {
        let events = play(&[
            r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Refusal
            })
        );
    }

    #[test]
    fn a_turn_pause_reports_itself_as_incomplete() {
        let events = play(&[
            r#"{"type":"message_delta","delta":{"stop_reason":"pause_turn"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        let Some(ChatEvent::Done { stop_reason }) = events.last() else {
            panic!("{events:?}");
        };
        assert_eq!(*stop_reason, StopReason::Paused);
        assert!(stop_reason.is_truncated());
    }

    #[test]
    fn an_error_in_the_stream_is_terminal() {
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ]);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("Overloaded"))),
            "{events:?}"
        );
        assert_eq!(
            events.iter().filter(|e| e.is_terminal()).count(),
            1,
            "{events:?}"
        );
    }

    #[test]
    fn a_stream_closed_cleanly_without_message_stop_is_interrupted() {
        // A load balancer that cuts at its duration limit closes the
        // connection cleanly: no transport error, only a stream that stops.
        // Presenting this text as complete would be lying.
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
        ]);
        assert_eq!(texts(&events), "a");
        let Some(ChatEvent::Done { stop_reason }) = events.last() else {
            panic!("the stream must end: {events:?}");
        };
        assert_eq!(*stop_reason, StopReason::Interrupted);
        assert!(stop_reason.is_truncated() && stop_reason.is_ambiguous());
        assert!(
            events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "the cut must show: {events:?}"
        );
    }

    #[test]
    fn an_announced_reason_without_message_stop_remains_a_cut() {
        // `message_delta` announces the end of the generation, not that of the
        // message: what followed was not received.
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
        ]);
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn a_readable_but_unclosed_tool_call_is_never_proposed() {
        // The trap: the JSON received so far happens to be complete. The model
        // may not have finished — `{"sql":"DELETE FROM t"}` can be the start
        // of `{"sql":"DELETE FROM t", "where": …}`.
        let events = play(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":10}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"execute","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\":\"DELETE FROM t\"}"}}"#,
        ]);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("tool call #0"))),
            "the call vanishing must show: {events:?}"
        );
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn a_block_not_closed_before_message_stop_is_thrown_away() {
        // Server inconsistency: the message says it is finished, the block is not.
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"execute","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
            "{events:?}"
        );
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::ToolCalls
            })
        );
    }

    #[test]
    fn an_unknown_event_does_not_break_the_stream() {
        // The documentation announces that new types can appear.
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"a_future_event","payload":{"what":"ever"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(texts(&events), "a");
        assert!(
            !events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "an unknown type is not an error: {events:?}"
        );
    }

    #[test]
    fn a_block_of_unknown_type_produces_nothing_but_absorbs_its_deltas() {
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"x\"}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(calls(&events).is_empty(), "{events:?}");
        assert!(
            !events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "{events:?}"
        );
    }

    #[test]
    fn a_delta_without_an_open_block_is_ignored() {
        let events = play(&[
            r#"{"type":"content_block_delta","index":7,"delta":{"type":"text_delta","text":"fantome"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(texts(&events), "", "{events:?}");
    }

    #[test]
    fn an_isolated_unreadable_frame_does_not_kill_the_stream() {
        let events = play(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            "{this is not json",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"b"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(texts(&events), "b");
        assert!(events.iter().any(|e| matches!(e, ChatEvent::Error(_))));
    }

    #[test]
    fn a_series_of_unreadable_frames_gives_up_the_stream() {
        let mut decoder = MessageDecoder::new();
        let mut outputs = Vec::new();
        for _ in 0..MAX_DECODE_ERRORS {
            decoder.on_data("not json", &mut outputs);
        }
        assert!(decoder.finished(), "{outputs:?}");
        assert!(outputs.last().is_some_and(ChatEvent::is_terminal));
    }

    /// A real SSE trace, as the documentation publishes it: event names,
    /// interleaved `ping`, blocks opened and closed.
    const TRACE: &str = concat!(
        "event: message_start\n",
        r#"data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-model","stop_reason":null,"usage":{"input_tokens":472,"output_tokens":2}}}"#,
        "\n\n",
        "event: content_block_start\n",
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        "\n\n",
        "event: ping\n",
        r#"data: {"type": "ping"}"#,
        "\n\n",
        "event: content_block_delta\n",
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I check the café"}}"#,
        "\n\n",
        "event: content_block_stop\n",
        r#"data: {"type":"content_block_stop","index":0}"#,
        "\n\n",
        "event: content_block_start\n",
        r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"get_weather","input":{}}}"#,
        "\n\n",
        "event: content_block_delta\n",
        r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"ville\":"}}"#,
        "\n\n",
        "event: content_block_delta\n",
        r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"Besançon\"}"}}"#,
        "\n\n",
        "event: content_block_stop\n",
        r#"data: {"type":"content_block_stop","index":1}"#,
        "\n\n",
        "event: message_delta\n",
        r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":89}}"#,
        "\n\n",
        "event: message_stop\n",
        r#"data: {"type":"message_stop"}"#,
        "\n\n",
    );

    /// Replays a trace through the real SSE path, with a given split.
    fn replay(chunks: Vec<bytes::Bytes>) -> Vec<ChatEvent> {
        use futures::stream::StreamExt as _;

        let bytes = chunks
            .into_iter()
            .map(Ok::<bytes::Bytes, String>)
            .collect::<Vec<_>>();
        let flux = crate::stream::events_stream(
            Box::pin(futures::stream::iter(bytes)),
            MessageDecoder::new(),
            oxyn_core::CancelToken::new(),
            None,
        );
        futures::executor::block_on(flux.collect())
    }

    #[test]
    fn a_real_trace_decodes_in_one_block() {
        let events = replay(vec![bytes::Bytes::from_static(TRACE.as_bytes())]);

        assert_eq!(texts(&events), "I check the café");
        let complete = calls(&events);
        assert_eq!(complete.len(), 1, "{events:?}");
        assert_eq!(complete[0].id, "toolu_01");
        assert_eq!(complete[0].arguments["ville"], "Besançon");
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::ToolCalls
            })
        );
    }

    #[test]
    fn the_same_trace_split_at_every_byte_gives_the_same_result() {
        // The network does not respect frame boundaries, and the trace
        // contains multi-byte characters ("é", "ç"): the only split that
        // covers every case is the one that respects none.
        let byte_by_byte: Vec<bytes::Bytes> = TRACE
            .as_bytes()
            .iter()
            .map(|byte| bytes::Bytes::copy_from_slice(&[*byte]))
            .collect();

        assert_eq!(
            replay(byte_by_byte),
            replay(vec![bytes::Bytes::from_static(TRACE.as_bytes())]),
            "the network split must change nothing"
        );
    }

    #[test]
    fn a_stream_cut_mid_turn_is_truncated_and_ambiguous() {
        // The case that costs money, and that shows neither at compile time
        // nor in review: the connection drops while the model has started to
        // answer. The server may have finished — and billed — the turn.
        // Replaying would pay twice (I-13).
        let cut: Vec<std::result::Result<bytes::Bytes, String>> = vec![
            Ok(bytes::Bytes::from_static(
                concat!(
                    "event: content_block_start\n",
                    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
                    "\n\n",
                    "event: content_block_delta\n",
                    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"the customers table "}}"#,
                    "\n\n",
                )
                .as_bytes(),
            )),
            // Neither `message_delta` nor `message_stop`: the transport gives out.
            Err("lost the connection while receiving the stream".to_owned()),
        ];

        let events = {
            use futures::stream::StreamExt as _;
            let flux = crate::stream::events_stream(
                Box::pin(futures::stream::iter(cut)),
                MessageDecoder::new(),
                oxyn_core::CancelToken::new(),
                None,
            );
            futures::executor::block_on(flux.collect::<Vec<_>>())
        };

        // What was received stays shown: the user sees where it stopped rather
        // than losing the beginning.
        assert_eq!(texts(&events), "the customers table ");

        let Some(ChatEvent::Done { stop_reason }) = events.last() else {
            panic!("the stream must end: {events:?}");
        };
        assert_eq!(*stop_reason, StopReason::Interrupted);
        assert!(
            stop_reason.is_truncated(),
            "a response cut in the middle that claims to be complete is a false response"
        );
        assert!(
            stop_reason.is_ambiguous(),
            "the server may have produced the rest: replaying would pay twice (I-13)"
        );
        assert_eq!(
            events.iter().filter(|e| e.is_terminal()).count(),
            1,
            "{events:?}"
        );
    }

    #[test]
    fn a_tool_call_cut_by_the_transport_is_never_proposed() {
        // The cut falls in the middle of the arguments. Proposing them would
        // amount to submitting an action whose reach no one knows.
        let cut: Vec<std::result::Result<bytes::Bytes, String>> = vec![
            Ok(bytes::Bytes::from_static(
                concat!(
                    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"execute_query"}}"#,
                    "\n\n",
                    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\": \"DELETE FROM audit"}}"#,
                    "\n\n",
                )
                .as_bytes(),
            )),
            Err("lost the connection while receiving the stream".to_owned()),
        ];

        let events = {
            use futures::stream::StreamExt as _;
            let flux = crate::stream::events_stream(
                Box::pin(futures::stream::iter(cut)),
                MessageDecoder::new(),
                oxyn_core::CancelToken::new(),
                None,
            );
            futures::executor::block_on(flux.collect::<Vec<_>>())
        };

        assert!(
            calls(&events).is_empty(),
            "truncated arguments are not arguments: {events:?}"
        );
        // And the partial SQL must not come out in an error message: what a
        // model writes can copy what it was given.
        for event in &events {
            if let ChatEvent::Error(message) = event {
                assert!(!message.contains("DELETE FROM"), "{message}");
            }
        }
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn nothing_is_decoded_after_the_end() {
        let mut decoder = MessageDecoder::new();
        let mut outputs = Vec::new();
        decoder.on_data(r#"{"type":"message_stop"}"#, &mut outputs);
        let after = outputs.len();
        decoder.on_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            &mut outputs,
        );
        assert_eq!(outputs.len(), after, "{outputs:?}");
    }
}

#[cfg(test)]
mod budget_tests;
