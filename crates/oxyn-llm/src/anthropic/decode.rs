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
mod tests;

#[cfg(test)]
mod budget_tests;
