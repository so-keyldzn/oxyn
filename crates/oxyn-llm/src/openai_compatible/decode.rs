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
//! **interleaved**. The only thing that links them is the `index` field. A
//! decoder that concatenated in order of arrival would produce a JSON mixing
//! two calls — and the model would have asked for two distinct actions.
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

use std::collections::BTreeMap;

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
    id: Option<String>,
    name: String,
    arguments: String,
    started: bool,
}

/// State of an OpenAI-compatible completion stream.
#[derive(Debug, Default)]
pub(crate) struct ChunkDecoder {
    calls: BTreeMap<u32, PartialCall>,
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
        let index = raw.index;
        if !self.calls.contains_key(&index) {
            // A new index is a new call: it is the one the ceiling on the
            // number of calls counts, before the state exists.
            self.budget.open_tool_call()?;
        }
        let entry = self.calls.entry(index).or_default();

        if let Some(id) = raw.id.filter(|value| !value.is_empty())
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

        if !entry.started && !entry.name.is_empty() {
            let id = entry.id.clone().unwrap_or_else(|| synthetic_id(index));
            entry.id = Some(id.clone());
            entry.started = true;
            let name = entry.name.clone();
            out.push(ChatEvent::ToolCallStarted { index, id, name });
        }

        if let Some(arguments) = fragment {
            out.push(ChatEvent::ToolCallDelta { index, arguments });
        }
        Ok(())
    }

    /// Throws away the tool calls without an end announcement, reporting it.
    ///
    /// A call that disappears silently would let the user believe the model
    /// asked for nothing.
    fn discard_calls(&mut self, out: &mut Vec<ChatEvent>) {
        for (index, partial) in std::mem::take(&mut self.calls) {
            if !partial.name.is_empty() || !partial.arguments.is_empty() {
                out.push(ChatEvent::Error(format!(
                    "tool call #{index} was not finished by the provider and was dropped"
                )));
            }
        }
    }

    /// Closes all gathered tool calls, on an end announcement.
    fn flush_calls(&mut self, out: &mut Vec<ChatEvent>) {
        for (index, partial) in std::mem::take(&mut self.calls) {
            if partial.name.is_empty() {
                out.push(ChatEvent::Error(format!(
                    "tool call #{index} has no name and was dropped"
                )));
                continue;
            }
            let id = partial.id.unwrap_or_else(|| synthetic_id(index));
            if !partial.started {
                out.push(ChatEvent::ToolCallStarted {
                    index,
                    id: id.clone(),
                    name: partial.name.clone(),
                });
            }
            match wire::build_tool_call(id, partial.name, &partial.arguments) {
                Ok(call) => out.push(ChatEvent::ToolCallComplete(call)),
                Err(message) => out.push(ChatEvent::Error(message)),
            }
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
mod tests {
    use super::*;

    #[test]
    fn openai_stop_reasons_are_translated() {
        assert_eq!(stop_reason("stop"), StopReason::EndTurn);
        assert_eq!(stop_reason("length"), StopReason::MaxTokens);
        assert_eq!(stop_reason("tool_calls"), StopReason::ToolCalls);
        assert_eq!(stop_reason("function_call"), StopReason::ToolCalls);
        assert_eq!(stop_reason("content_filter"), StopReason::ContentFilter);
        assert_eq!(
            stop_reason("guardrail_intervened"),
            StopReason::Other("guardrail_intervened".to_owned()),
            "an unknown reason is kept, it is not folded into EndTurn"
        );
    }

    /// Plays a sequence of `data` fields and returns all the events produced.
    fn play(frames: &[&str]) -> Vec<ChatEvent> {
        let mut decoder = ChunkDecoder::new();
        let mut outputs = Vec::new();
        for frame in frames {
            decoder.on_data(frame, &mut outputs);
        }
        if !decoder.is_done() {
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

    fn calls(events: &[ChatEvent]) -> Vec<&crate::types::ToolCall> {
        events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ToolCallComplete(call) => Some(call),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_text_stream_is_reassembled_in_order() {
        let events = play(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
            r#"{"choices":[{"delta":{"content":"SELECT "}}]}"#,
            r#"{"choices":[{"delta":{"content":"1"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            DONE_SENTINEL,
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
    fn done_is_emitted_exactly_once() {
        let mut decoder = ChunkDecoder::new();
        let mut outputs = Vec::new();
        decoder.on_data(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            &mut outputs,
        );
        decoder.on_data(DONE_SENTINEL, &mut outputs);
        // The server closes after the sentinel: `finish` must add nothing.
        decoder.finish(&mut outputs);
        let ends = outputs.iter().filter(|e| e.is_terminal()).count();
        assert_eq!(ends, 1, "{outputs:?}");
    }

    #[test]
    fn usage_arriving_after_the_end_of_generation_is_kept() {
        // The trap: emitting `Done` on `finish_reason` would lose this frame.
        let events = play(&[
            r#"{"choices":[{"delta":{"content":"ok"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":120,"completion_tokens":8}}"#,
            DONE_SENTINEL,
        ]);
        assert!(
            events.contains(&ChatEvent::Usage {
                prompt_tokens: 120,
                completion_tokens: 8,
                cache_write_tokens: None,
                cache_read_tokens: None,
                reasoning_tokens: None,
            }),
            "{events:?}"
        );
        assert!(
            events.last().is_some_and(ChatEvent::is_terminal),
            "Done must remain the last event"
        );
    }

    #[test]
    fn a_fragmented_tool_call_is_reassembled() {
        let events = play(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"execute_query","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"sql\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"SELECT 1\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            DONE_SENTINEL,
        ]);

        let complete = calls(&events);
        assert_eq!(complete.len(), 1, "{events:?}");
        assert_eq!(complete[0].id, "call_abc");
        assert_eq!(complete[0].name, "execute_query");
        assert_eq!(complete[0].arguments["sql"], "SELECT 1");

        assert!(events.contains(&ChatEvent::ToolCallStarted {
            index: 0,
            id: "call_abc".to_owned(),
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
    fn two_interleaved_calls_do_not_mix() {
        // The defect this module exists to avoid: concatenating in order of
        // arrival would produce a single incoherent JSON.
        let events = play(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"lire"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"ecrire"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"t\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"u\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"2}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);

        let complete = calls(&events);
        assert_eq!(complete.len(), 2, "{events:?}");
        assert_eq!(complete[0].name, "lire");
        assert_eq!(complete[0].arguments, serde_json::json!({"t": 1}));
        assert_eq!(complete[1].name, "ecrire");
        assert_eq!(complete[1].arguments, serde_json::json!({"u": 2}));
    }

    #[test]
    fn a_call_without_identifier_receives_one() {
        // Ollama and llama.cpp omit `id`.
        let events = play(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"ping","arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        let complete = calls(&events);
        assert_eq!(complete.len(), 1);
        assert!(
            !complete[0].id.is_empty(),
            "without an identifier, the tool result cannot be attached"
        );
    }

    #[test]
    fn a_tool_without_arguments_receives_an_empty_object() {
        let events = play(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"lister"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        assert_eq!(calls(&events)[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn invalid_arguments_produce_an_error_and_not_a_call() {
        // A model sometimes produces broken JSON. Emitting a call with null
        // arguments would be a silent lie.
        let events = play(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"execute","arguments":"{\"sql\": "}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        assert!(calls(&events).is_empty(), "{events:?}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("execute"))),
            "{events:?}"
        );
    }

    #[test]
    fn a_stream_closed_after_finish_reason_without_sentinel_is_complete() {
        // The generation was announced finished: the absence of `[DONE]` only
        // costs the usage frame.
        let events = play(&[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ]);
        assert_eq!(texts(&events), "a");
        assert!(
            !events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "{events:?}"
        );
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::EndTurn
            })
        );
    }

    #[test]
    fn a_stream_closed_cleanly_without_any_announcement_is_interrupted() {
        // Neither `finish_reason` nor `[DONE]`: a proxy closed in the middle of
        // the generation. Nothing failed on the transport side, and yet the
        // response is cut.
        let events = play(&[r#"{"choices":[{"delta":{"content":"a"}}]}"#]);
        let Some(ChatEvent::Done { stop_reason }) = events.last() else {
            panic!("the stream must end: {events:?}");
        };
        assert_eq!(*stop_reason, StopReason::Interrupted);
        assert!(stop_reason.is_ambiguous());
        assert!(
            events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "the cut must show: {events:?}"
        );
    }

    #[test]
    fn a_readable_tool_call_without_end_announcement_is_never_proposed() {
        let events = play(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"execute","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"sql\":\"DELETE FROM t\"}"}}]}}]}"#,
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
            "{events:?}"
        );
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn an_error_in_the_stream_throws_away_the_calls_in_progress() {
        let events = play(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"execute","arguments":"{}"}}]}}]}"#,
            r#"{"error":{"message":"overloaded"}}"#,
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
                stop_reason: StopReason::ProviderError
            })
        );
    }

    #[test]
    fn an_isolated_unreadable_frame_does_not_kill_the_stream() {
        let events = play(&[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            "{this is not json",
            r#"{"choices":[{"delta":{"content":"b"}}]}"#,
            DONE_SENTINEL,
        ]);
        assert_eq!(texts(&events), "ab");
        assert!(events.iter().any(|e| matches!(e, ChatEvent::Error(_))));
    }

    #[test]
    fn a_series_of_unreadable_frames_gives_up_the_stream() {
        let mut decoder = ChunkDecoder::new();
        let mut outputs = Vec::new();
        for _ in 0..MAX_DECODE_ERRORS {
            decoder.on_data("not json", &mut outputs);
        }
        assert!(decoder.is_done(), "{outputs:?}");
        assert!(outputs.last().is_some_and(ChatEvent::is_terminal));
    }

    #[test]
    fn an_error_carried_in_the_stream_is_terminal() {
        let events = play(&[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            r#"{"error":{"message":"upstream refused","code":502}}"#,
        ]);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("upstream refused"))),
            "{events:?}"
        );
        assert_eq!(
            events.iter().filter(|e| e.is_terminal()).count(),
            1,
            "{events:?}"
        );
    }

    #[test]
    fn a_cancellation_invents_no_half_described_call() {
        let mut decoder = ChunkDecoder::new();
        let mut outputs = Vec::new();
        decoder.on_data(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"drop_table","arguments":"{\"name\":"}}]}}]}"#,
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
    fn a_transport_break_ends_the_stream() {
        let mut decoder = ChunkDecoder::new();
        let mut outputs = Vec::new();
        decoder.on_data(r#"{"choices":[{"delta":{"content":"a"}}]}"#, &mut outputs);
        decoder.transport_error("connection reset".to_owned(), &mut outputs);
        assert!(decoder.is_done());
        assert!(outputs.last().is_some_and(ChatEvent::is_terminal));
    }

    #[test]
    fn nothing_is_decoded_after_the_end() {
        let mut decoder = ChunkDecoder::new();
        let mut outputs = Vec::new();
        decoder.on_data(DONE_SENTINEL, &mut outputs);
        let after = outputs.len();
        decoder.on_data(
            r#"{"choices":[{"delta":{"content":"fantome"}}]}"#,
            &mut outputs,
        );
        assert_eq!(outputs.len(), after, "{outputs:?}");
    }
}

#[cfg(test)]
mod budget_tests;
