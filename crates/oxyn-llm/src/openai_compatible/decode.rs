//! From the SSE frame to domain events.
//!
//! This module holds the state of a completion stream. It is **pure**: no
//! input-output, no network. That is what allows testing it on the servers'
//! real sequences, quirks included, without starting one.
//!
//! # Reassembling tool calls
//!
//! A tool call does not arrive in one block: the name comes in one frame, the
//! arguments in a dozen fragments, and the fragments of several calls are
//! **interleaved**. What links them is the `index` field and, on the first
//! fragment, the `id`. A decoder that concatenated in order of arrival would
//! produce a JSON mixing two calls — and the model would have asked for two
//! distinct actions.
//!
//! Neither key is enough alone. Some gateways omit `index`, others give every
//! call the same one; only the first fragment carries the `id`. So a fragment
//! continues the call of the same `id` if there is one; otherwise the last
//! call of the same `index` — or the last call at all when there is no
//! `index` —, **unless** that call already has another `id`: a new `id` is a
//! new call, whatever the `index` says. Merging them produced one call with
//! the two names glued together and the arguments `{…}{…}`, which parses as
//! nothing.
//!
//! The index the events carry is the call's position in the turn, never the
//! server's `index`: two calls that share a server `index` must not share a
//! card.
//!
//! # When `Done` is emitted
//!
//! `finish_reason` marks the end of the generation, but **not** the end of the
//! stream: OpenAI sends the usage in a later frame. So the reason is recorded,
//! the tool calls are closed, and `Done` is only emitted at the `[DONE]`
//! sentinel or when the stream closes. `Done` is emitted exactly once.
//!
//! # An observed end is not an announced end
//!
//! This protocol has **two** end announcements, and they do not say the same
//! thing: `finish_reason` on the last content fragment says the generation is
//! finished; `data: [DONE]` says the stream is. OpenAI documents both
//! ([RESEARCH-NOTES](../../../../docs/RESEARCH-NOTES.md)), but a compatible
//! server can omit the sentinel.
//!
//! The chosen rule therefore does not depend on the sentinel:
//!
//! * **`finish_reason` received, then closing**: the generation was announced
//!   finished, the content is whole; only the usage frame may have been lost.
//!   The announced reason is kept;
//! * **neither `finish_reason` nor `[DONE]`, then closing** — clean or not: it
//!   is a cut, [`StopReason::Interrupted`], and the tool calls in progress are
//!   **thrown away**. A proxy that closes cleanly in the middle of a generation
//!   produces no transport error, and arguments that happen to parse are not
//!   arguments the model finished writing.
//!
//! # Everything that accumulates is counted
//!
//! Text, refusal, identifiers, tool names and arguments go through a
//! [`GenerationBudget`] **before** being accumulated or emitted — including
//! the name of an already announced call, which some servers keep fragmenting
//! without anything being emitted. At the first overrun, the generation stops:
//! see [`crate::budget`].

use super::wire::{self, ChatChunk, DeltaToolCall};
use crate::budget::{BudgetExceeded, GenerationBudget};
use crate::sse::SseFrame;
use crate::stream::EventDecoder;
use crate::types::{ChatEvent, StopReason};

/// End sentinel of the OpenAI-compatible protocols.
pub(crate) const DONE_SENTINEL: &str = "[DONE]";

/// Number of unreadable frames tolerated before giving up the stream.
///
/// An isolated unreadable frame happens (a gateway inserting an undocumented
/// object); a series means we do not speak the same protocol, and continuing
/// would only flood the interface with errors.
pub(crate) const MAX_DECODE_ERRORS: usize = 8;

/// A tool call being reassembled.
#[derive(Debug, Default)]
struct PartialCall {
    /// The server's `index`, when it gives one: a lookup key, not an
    /// identity.
    wire_index: Option<u32>,
    /// The server's `id`, when it gave one.
    id: Option<String>,
    name: String,
    arguments: String,
    /// The identifier announced by `ToolCallStarted`, once it is.
    started: Option<String>,
}

/// State of an OpenAI-compatible completion stream.
#[derive(Debug, Default)]
pub(crate) struct ChunkDecoder {
    /// In order of first appearance: the position is the call's index in the
    /// events.
    calls: Vec<PartialCall>,
    budget: GenerationBudget,
    stop: Option<StopReason>,
    done: bool,
    errors: usize,
}

impl ChunkDecoder {
    /// Decoder for a fresh stream.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Is the stream finished? Nothing more must be decoded afterwards.
    pub(crate) fn is_done(&self) -> bool {
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
        if data == DONE_SENTINEL {
            self.flush_calls(out);
            self.emit_done(out);
            return;
        }

        let chunk: ChatChunk = match serde_json::from_str(data) {
            Ok(chunk) => chunk,
            Err(err) => {
                // The faulty frame is not copied: we do not know what a
                // third-party endpoint puts in it.
                self.errors += 1;
                out.push(ChatEvent::Error(format!(
                    "unreadable stream frame ({}, line {}, column {})",
                    wire::classify_label(&err),
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

        // An error carried in the stream is terminal: the gateway will produce
        // nothing more after it.
        if let Some(error) = &chunk.error {
            out.push(ChatEvent::Error(error.describe()));
            self.stop = Some(StopReason::ProviderError);
            // Thrown away, not closed: a call interrupted by an error is not a
            // proposed action.
            self.discard_calls(out);
            self.emit_done(out);
            return;
        }

        if let Some(usage) = &chunk.usage {
            out.push(ChatEvent::Usage {
                prompt_tokens: usage.prompt(),
                completion_tokens: usage.completion(),
                // This protocol does not distinguish cache writes: it only
                // reports the tokens **read**. Declaring `Some(0)` for writes
                // would suggest no prefix was cached, when we know nothing
                // about it.
                cache_write_tokens: None,
                cache_read_tokens: usage.cache_read(),
                reasoning_tokens: usage.reasoning(),
            });
        }

        for choice in chunk.choices {
            if let Err(limit) = self.on_delta(choice.delta, out) {
                self.exceed(limit, out);
                return;
            }
            if let Some(reason) = choice.finish_reason {
                // The generation is finished; the stream, not necessarily.
                self.stop = Some(stop_reason(&reason));
                self.flush_calls(out);
            }
        }
    }

    /// Emits the content of a choice fragment, counted before it is.
    fn on_delta(
        &mut self,
        delta: wire::Delta,
        out: &mut Vec<ChatEvent>,
    ) -> Result<(), BudgetExceeded> {
        if let Some(text) = delta.content
            && !text.is_empty()
        {
            self.budget.charge(text.len())?;
            out.push(ChatEvent::TextDelta(text));
        }
        // A refusal is a response, not an error: the request succeeded and the
        // model said it would not answer. Confusing it with text would present
        // it as an answer.
        if let Some(refusal) = delta.refusal
            && !refusal.is_empty()
        {
            self.budget.charge(refusal.len())?;
            out.push(ChatEvent::RefusalDelta(refusal));
        }
        for call in delta.tool_calls {
            self.accumulate(call, out)?;
        }
        Ok(())
    }

    /// Stops the generation on an exceeded budget.
    ///
    /// The error names the limit; the calls in progress are **thrown away** —
    /// a cut call is not a proposed action — and the end is a cut: the
    /// provider may have continued, and billed, what we stopped reading
    /// (I-13).
    fn exceed(&mut self, limit: BudgetExceeded, out: &mut Vec<ChatEvent>) {
        out.push(ChatEvent::Error(limit.to_string()));
        self.discard_calls(out);
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
    }

    /// Signals the closing of the stream by the server, without `[DONE]`.
    ///
    /// See the module note: only an already received `finish_reason` makes
    /// this closing an ordinary end.
    pub(crate) fn finish(&mut self, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        if self.stop.is_some() {
            // The calls were closed at `finish_reason`; a fragment arrived
            // afterwards has no end announcement.
            self.discard_calls(out);
            self.emit_done(out);
            return;
        }
        out.push(ChatEvent::Error(
            "the stream ended before the provider announced the end of the generation".to_owned(),
        ));
        self.discard_calls(out);
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
    }

    /// Signals a cancellation requested by the caller.
    ///
    /// The tool calls in progress are **not** closed: truncated arguments are
    /// not arguments, and proposing a half-described action would be worse
    /// than proposing nothing.
    pub(crate) fn cancel(&mut self, out: &mut Vec<ChatEvent>) {
        self.calls.clear();
        self.stop = Some(StopReason::Cancelled);
        self.emit_done(out);
    }

    /// Reports a transport break mid-stream.
    pub(crate) fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        out.push(ChatEvent::Error(detail));
        self.calls.clear();
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
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

    /// Stores a tool call fragment and emits what has become certain.
    ///
    /// # Errors
    /// The budget this fragment would exceed; nothing of it is kept then.
    fn accumulate(
        &mut self,
        raw: DeltaToolCall,
        out: &mut Vec<ChatEvent>,
    ) -> Result<(), BudgetExceeded> {
        let id = raw.id.filter(|value| !value.is_empty());
        let position = match self.continued(raw.index, id.as_deref()) {
            Some(position) => position,
            None => {
                // A new call: it is the one the ceiling on the number of
                // calls counts, before the state exists.
                self.budget.open_tool_call()?;
                self.calls.push(PartialCall {
                    wire_index: raw.index,
                    ..PartialCall::default()
                });
                self.calls.len().saturating_sub(1)
            }
        };
        // The ceiling on the number of calls keeps the position far below.
        let index = u32::try_from(position).unwrap_or(u32::MAX);
        let Some(entry) = self.calls.get_mut(position) else {
            return Ok(());
        };

        if let Some(id) = id
            && entry.id.is_none()
        {
            self.budget.charge(id.len())?;
            entry.id = Some(id);
        }

        let mut fragment = None;
        if let Some(function) = raw.function {
            if let Some(tool_name) = function.name.filter(|value| !value.is_empty()) {
                // Concatenation and not assignment: a few servers fragment the
                // name too.
                self.budget
                    .charge_tool_name(index, entry.name.len(), tool_name.len())?;
                entry.name.push_str(&tool_name);
            }
            if let Some(arguments) = function.arguments.filter(|value| !value.is_empty()) {
                self.budget
                    .charge_tool_arguments(index, entry.arguments.len(), arguments.len())?;
                entry.arguments.push_str(&arguments);
                fragment = Some(arguments);
            }
        }

        if entry.started.is_none() && !entry.name.is_empty() {
            let id = entry.id.clone().unwrap_or_else(|| synthetic_id(index));
            entry.started = Some(id.clone());
            let name = entry.name.clone();
            out.push(ChatEvent::ToolCallStarted { index, id, name });
        }

        if let Some(arguments) = fragment {
            out.push(ChatEvent::ToolCallDelta { index, arguments });
        }
        Ok(())
    }

    /// The position of the call a fragment continues, or `None` when it opens
    /// a new one. See the module note for the rule.
    fn continued(&self, index: Option<u32>, id: Option<&str>) -> Option<usize> {
        if let Some(id) = id
            && let Some(same) = self
                .calls
                .iter()
                .rposition(|call| call.id.as_deref() == Some(id))
        {
            return Some(same);
        }
        let candidate = match index {
            Some(index) => self
                .calls
                .iter()
                .rposition(|call| call.wire_index == Some(index))?,
            None => self.calls.len().checked_sub(1)?,
        };
        match (id, self.calls.get(candidate)?.id.as_deref()) {
            (Some(new), Some(known)) if new != known => None,
            _ => Some(candidate),
        }
    }

    /// Throws away the tool calls without an end announcement, reporting it.
    ///
    /// A call that disappears silently would let the user believe the model
    /// asked for nothing.
    fn discard_calls(&mut self, out: &mut Vec<ChatEvent>) {
        for (index, partial) in std::mem::take(&mut self.calls).into_iter().enumerate() {
            if !partial.name.is_empty() || !partial.arguments.is_empty() {
                out.push(ChatEvent::Error(format!(
                    "tool call #{index} was not finished by the provider and was dropped"
                )));
            }
        }
    }

    /// Closes all gathered tool calls, on an end announcement.
    fn flush_calls(&mut self, out: &mut Vec<ChatEvent>) {
        for (position, partial) in std::mem::take(&mut self.calls).into_iter().enumerate() {
            let index = u32::try_from(position).unwrap_or(u32::MAX);
            if partial.name.is_empty() {
                out.push(ChatEvent::Error(format!(
                    "tool call #{index} has no name and was dropped"
                )));
                continue;
            }
            // The identifier already announced wins: the card and the result
            // must name the same call.
            let id = match partial.started {
                Some(announced) => announced,
                None => {
                    let id = partial.id.unwrap_or_else(|| synthetic_id(index));
                    out.push(ChatEvent::ToolCallStarted {
                        index,
                        id: id.clone(),
                        name: partial.name.clone(),
                    });
                    id
                }
            };
            out.push(wire::tool_call_event(id, partial.name, &partial.arguments));
        }
    }
}

impl EventDecoder for ChunkDecoder {
    /// This protocol does not name its frames: only the `data` field counts,
    /// and a possible `event` field is ignored rather than given a meaning.
    fn on_frame(&mut self, frame: &SseFrame, out: &mut Vec<ChatEvent>) {
        self.on_data(&frame.data, out);
    }

    fn is_done(&self) -> bool {
        Self::is_done(self)
    }

    fn finish(&mut self, out: &mut Vec<ChatEvent>) {
        Self::finish(self, out);
    }

    fn cancel(&mut self, out: &mut Vec<ChatEvent>) {
        Self::cancel(self, out);
    }

    fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>) {
        Self::transport_error(self, detail, out);
    }
}

/// Translates the `finish_reason` of OpenAI-compatible protocols.
///
/// A function of this module and not a method of [`StopReason`]: the type
/// lives in `oxyn-core`, which knows no protocol. An unknown reason is
/// **kept** rather than folded into `EndTurn`, which would pass an incomplete
/// response off as a finished one.
pub(crate) fn stop_reason(raw: &str) -> StopReason {
    match raw {
        "stop" => StopReason::EndTurn,
        "length" => StopReason::MaxTokens,
        "tool_calls" | "function_call" => StopReason::ToolCalls,
        "content_filter" => StopReason::ContentFilter,
        other => StopReason::Other(other.to_owned()),
    }
}

/// Makes up a call identifier when the server gives none.
///
/// Ollama and `llama.cpp` regularly omit `id`. Without an identifier, the next
/// turn cannot attach the tool's result to its request: making it up from the
/// index is the only recourse, and it is stable for a given turn.
fn synthetic_id(index: u32) -> String {
    format!("call_{index}")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod budget_tests;
