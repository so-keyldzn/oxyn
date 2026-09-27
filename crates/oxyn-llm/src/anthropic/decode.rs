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
    /// Appel d'outil en cours de reconstruction.
    ToolUse {
        id: String,
        name: String,
        arguments: String,
    },
    /// Raisonnement en cours de reconstruction.
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

        let trame: Envelope = match serde_json::from_str(data) {
            Ok(trame) => trame,
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

        let compte = match trame.r#type.as_deref() {
            Some("content_block_start") => self.on_block_start(&trame, out),
            Some("content_block_delta") => self.on_block_delta(&trame, out),
            _ => Ok(()),
        };
        if let Err(limite) = compte {
            self.exceed(limite, out);
            return;
        }

        match trame.r#type.as_deref() {
            Some("message_start") => self.on_message_start(&trame, out),
            // Handled above, where their cost is counted.
            Some("content_block_start" | "content_block_delta") => {}
            Some("content_block_stop") => self.on_block_stop(&trame, out),
            Some("message_delta") => self.on_message_delta(&trame, out),
            Some("message_stop") => self.on_message_stop(out),
            Some("error") => self.on_error(trame.error.as_ref(), out),
            // `ping` is a connection keep-alive: it has nothing to say.
            Some("ping") => {}
            // The documentation announces that new event types can appear. An
            // unknown type is ignored: refusing it would break the stream at
            // the protocol's first evolution.
            _ => {}
        }
    }

    /// `message_start`: the input usage is already known.
    fn on_message_start(&mut self, trame: &Envelope, out: &mut Vec<ChatEvent>) {
        let Some(usage) = trame.message.as_ref().and_then(|m| m.usage.as_ref()) else {
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
        trame: &Envelope,
        out: &mut Vec<ChatEvent>,
    ) -> Result<(), BudgetExceeded> {
        let Some(index) = trame.index else {
            return Ok(());
        };
        let Some(bloc) = trame.content_block.as_ref() else {
            return Ok(());
        };
        let longueur = |champ: &Option<String>| champ.as_ref().map_or(0, String::len);
        if bloc.r#type.as_deref() == Some("tool_use") {
            self.budget.open_tool_call()?;
            self.budget
                .charge_tool_name(index, 0, longueur(&bloc.name))?;
            self.budget.charge(longueur(&bloc.id))?;
        } else {
            self.budget.open_block()?;
            self.budget.charge(
                longueur(&bloc.thinking)
                    .saturating_add(longueur(&bloc.signature))
                    .saturating_add(longueur(&bloc.data)),
            )?;
        }

        let partiel = match bloc.r#type.as_deref() {
            Some("text") => PartialBlock::Text,
            Some("tool_use") => {
                let id = bloc.id.clone().unwrap_or_else(|| synthetic_id(index));
                let name = bloc.name.clone().unwrap_or_default();
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
                text: bloc.thinking.clone().unwrap_or_default(),
                signature: bloc.signature.clone(),
            },
            Some("redacted_thinking") => PartialBlock::RedactedThinking {
                data: bloc.data.clone().unwrap_or_default(),
            },
            // `server_tool_use`, web search results, blocks to come: they
            // exist, Oxyn offers none of them, and they produce nothing here.
            _ => PartialBlock::Unknown,
        };
        self.blocks.insert(index, partiel);
        Self::note_unused(bloc);
        Ok(())
    }

    /// Accepts without doing anything a block whose fields are not used.
    ///
    /// Only serves to make the intent explicit to the reader: all the fields
    /// of [`ContentBlock`] are read above, and the one that is not is left
    /// deliberately.
    const fn note_unused(_bloc: &ContentBlock) {}

    /// `content_block_delta`: a fragment arrives for an open block.
    ///
    /// # Errors
    /// The budget this fragment would exceed; nothing of it is kept then.
    fn on_block_delta(
        &mut self,
        trame: &Envelope,
        out: &mut Vec<ChatEvent>,
    ) -> Result<(), BudgetExceeded> {
        let (Some(index), Some(delta)) = (trame.index, trame.delta.as_ref()) else {
            return Ok(());
        };
        // A delta targeting a block never opened is ignored: inventing the
        // block would amount to inventing its type, hence the meaning of what
        // is accumulated.
        let Some(bloc) = self.blocks.get_mut(&index) else {
            return Ok(());
        };
        let budget = &mut self.budget;

        match (delta.r#type.as_deref(), bloc) {
            (Some("text_delta"), PartialBlock::Text) => {
                if let Some(texte) = delta.text.clone()
                    && !texte.is_empty()
                {
                    budget.charge(texte.len())?;
                    out.push(ChatEvent::TextDelta(texte));
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
                if let Some(valeur) = delta.signature.clone() {
                    budget.charge(valeur.len())?;
                    *signature = Some(valeur);
                }
            }
            // A delta whose type does not match the block's is an
            // inconsistency of the server, not data to save.
            _ => {}
        }
        Ok(())
    }

    /// `content_block_stop`: the block is complete.
    fn on_block_stop(&mut self, trame: &Envelope, out: &mut Vec<ChatEvent>) {
        let Some(index) = trame.index else {
            return;
        };
        let Some(bloc) = self.blocks.remove(&index) else {
            return;
        };
        Self::close_block(index, bloc, out);
    }

    /// Closes a block and emits what it produces.
    fn close_block(index: u32, bloc: PartialBlock, out: &mut Vec<ChatEvent>) {
        match bloc {
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
                match build_tool_call(id, name, &arguments) {
                    Ok(appel) => out.push(ChatEvent::ToolCallComplete(appel)),
                    Err(message) => out.push(ChatEvent::Error(message)),
                }
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
    fn on_message_delta(&mut self, trame: &Envelope, out: &mut Vec<ChatEvent>) {
        if let Some(raison) = trame.delta.as_ref().and_then(|d| d.stop_reason.as_deref()) {
            // The generation is finished; the stream, not yet.
            self.stop = Some(stop_reason(raison));
        }
        if let Some(usage) = trame.usage.as_ref()
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
    fn on_error(&mut self, erreur: Option<&WireError>, out: &mut Vec<ChatEvent>) {
        let message = erreur.map_or_else(
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
    fn exceed(&mut self, limite: BudgetExceeded, out: &mut Vec<ChatEvent>) {
        out.push(ChatEvent::Error(limite.to_string()));
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
        for (index, bloc) in std::mem::take(&mut self.blocks) {
            match bloc {
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
        autre => StopReason::Other(autre.to_owned()),
    }
}

/// Makes up a call identifier when the server gives none.
fn synthetic_id(index: u32) -> String {
    format!("toolu_{index}")
}

/// Rebuilds a complete tool call from its fragments.
///
/// # Errors
/// Returns the parse error message — **without** the arguments string, which
/// is a model output and can copy what it was given.
fn build_tool_call(id: String, name: String, arguments: &str) -> Result<ToolCall, String> {
    let brut = arguments.trim();
    if brut.is_empty() {
        // A tool without parameters: the block opens with `input: {}` and no
        // delta follows.
        return Ok(ToolCall::new(id, name, serde_json::json!({})));
    }
    match serde_json::from_str::<serde_json::Value>(brut) {
        Ok(valeur) => Ok(ToolCall::new(id, name, valeur)),
        Err(err) => Err(format!(
            "cannot read the arguments of tool `{name}`: {} (line {}, column {})",
            classify_json_error(&err),
            err.line(),
            err.column()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_stop_reasons_are_translated() {
        for (brut, attendu) in [
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
            assert_eq!(stop_reason(brut), attendu, "{brut}");
        }
        assert_eq!(
            stop_reason("raison_future"),
            StopReason::Other("raison_future".to_owned()),
            "la documentation annonce que cette liste peut grandir"
        );
    }

    /// Plays a sequence of `data` fields and returns all the events produced.
    fn jouer(trames: &[&str]) -> Vec<ChatEvent> {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        for trame in trames {
            decodeur.on_data(trame, &mut sorties);
        }
        if !decodeur.finished() {
            decodeur.finish(&mut sorties);
        }
        sorties
    }

    fn textes(evenements: &[ChatEvent]) -> String {
        evenements
            .iter()
            .filter_map(|e| match e {
                ChatEvent::TextDelta(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    fn raisonnement(evenements: &[ChatEvent]) -> String {
        evenements
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ReasoningDelta { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn appels(evenements: &[ChatEvent]) -> Vec<&ToolCall> {
        evenements
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ToolCallComplete(appel) => Some(appel),
                _ => None,
            })
            .collect()
    }

    fn blocs(evenements: &[ChatEvent]) -> Vec<&ReasoningBlock> {
        evenements
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ReasoningComplete { block, .. } => Some(block),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_text_stream_is_reassembled_in_order() {
        let evenements = jouer(&[
            r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":25,"output_tokens":1}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"SELECT "}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"1"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":15}}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(textes(&evenements), "SELECT 1");
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::EndTurn
            })
        );
    }

    #[test]
    fn a_keep_alive_produces_nothing() {
        let evenements = jouer(&[
            r#"{"type":"ping"}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(
            evenements.len(),
            1,
            "seul `Done` doit rester : {evenements:?}"
        );
        assert!(evenements[0].is_terminal());
    }

    #[test]
    fn done_is_emitted_exactly_once() {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            &mut sorties,
        );
        decodeur.on_data(r#"{"type":"message_stop"}"#, &mut sorties);
        // The server closes after `message_stop`: `finish` must add nothing.
        decodeur.finish(&mut sorties);
        assert_eq!(sorties.iter().filter(|e| e.is_terminal()).count(), 1);
    }

    #[test]
    fn cache_usage_is_reported_both_ways() {
        let evenements = jouer(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":50,"output_tokens":1,"cache_creation_input_tokens":148,"cache_read_input_tokens":2000}}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(
            evenements.contains(&ChatEvent::Usage {
                prompt_tokens: 50,
                completion_tokens: 1,
                cache_write_tokens: Some(148),
                cache_read_tokens: Some(2000),
                reasoning_tokens: None,
            }),
            "{evenements:?}"
        );
    }

    #[test]
    fn an_undeclared_usage_is_not_invented() {
        let evenements = jouer(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        let Some(ChatEvent::Usage {
            cache_write_tokens,
            cache_read_tokens,
            ..
        }) = evenements
            .iter()
            .find(|e| matches!(e, ChatEvent::Usage { .. }))
        else {
            panic!("no usage: {evenements:?}");
        };
        assert_eq!(*cache_write_tokens, None);
        assert_eq!(*cache_read_tokens, None);
    }

    #[test]
    fn a_fragmented_tool_call_is_reassembled() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"execute_query","input":{}}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"sql\":"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"SELECT 1\"}"}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        let complets = appels(&evenements);
        assert_eq!(complets.len(), 1, "{evenements:?}");
        assert_eq!(complets[0].id, "toolu_01");
        assert_eq!(complets[0].name, "execute_query");
        assert_eq!(complets[0].arguments["sql"], "SELECT 1");

        assert!(evenements.contains(&ChatEvent::ToolCallStarted {
            index: 1,
            id: "toolu_01".to_owned(),
            name: "execute_query".to_owned(),
        }));
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::ToolCalls
            })
        );
    }

    #[test]
    fn two_tool_blocks_do_not_mix() {
        // This protocol closes a block before opening another, but nothing
        // forces it to: the state is kept by index, not by order of arrival.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"lire"}}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"b","name":"ecrire"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"t\":1}"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"u\":2}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        let complets = appels(&evenements);
        assert_eq!(complets.len(), 2, "{evenements:?}");
        assert_eq!(complets[0].arguments, serde_json::json!({"t": 1}));
        assert_eq!(complets[1].arguments, serde_json::json!({"u": 2}));
    }

    #[test]
    fn a_tool_without_arguments_receives_an_empty_object() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"lister","input":{}}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(appels(&evenements)[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn invalid_arguments_produce_an_error_and_not_a_call() {
        // The fine-grained stream is not validated by the server: the
        // accumulated string may not be JSON.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"execute"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\": \"SELECT secret"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(appels(&evenements).is_empty(), "{evenements:?}");
        let erreur = evenements
            .iter()
            .find_map(|e| match e {
                ChatEvent::Error(m) => Some(m.as_str()),
                _ => None,
            })
            .expect("an error must be reported");
        assert!(erreur.contains("execute"), "{erreur}");
        assert!(
            !erreur.contains("secret"),
            "the model output must not be copied: {erreur}"
        );
    }

    #[test]
    fn a_reasoning_block_is_reassembled_with_its_signature() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"je pose 1071 = 2 × 462 + 147"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":", puis 462 = 3 × 147 + 21"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCgIYAhIM"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(
            raisonnement(&evenements),
            "je pose 1071 = 2 × 462 + 147, puis 462 = 3 × 147 + 21"
        );
        let blocs = blocs(&evenements);
        assert_eq!(blocs.len(), 1, "{evenements:?}");
        assert_eq!(
            blocs[0],
            &ReasoningBlock::Summarized {
                text: "je pose 1071 = 2 × 462 + 147, puis 462 = 3 × 147 + 21".to_owned(),
                signature: Some("EqQBCgIYAhIM".to_owned()),
            }
        );
    }

    #[test]
    fn a_masked_reasoning_remains_transportable() {
        // The default of several models: the block arrives without text, but
        // with its signature. It must still be rendered, otherwise the next
        // turn is refused.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EosnCkYICxIM"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(raisonnement(&evenements), "", "nothing to display");
        let blocs = blocs(&evenements);
        assert_eq!(blocs.len(), 1, "the block must exist all the same");
        assert_eq!(blocs[0].display_text(), None);
    }

    #[test]
    fn an_encrypted_reasoning_produces_no_text() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"EvwBCoYBGAIiQL"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(raisonnement(&evenements), "");
        let blocs = blocs(&evenements);
        assert_eq!(blocs.len(), 1, "{evenements:?}");
        assert!(blocs[0].is_redacted());
        assert_eq!(blocs[0].display_text(), None);
    }

    #[test]
    fn a_cancellation_in_the_middle_of_a_reasoning_returns_no_incomplete_block() {
        // A block without its signature would be refused at the next turn:
        // rendering it would make the following request fail, far from here.
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            &mut sorties,
        );
        decodeur.on_data(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"je commence à"}}"#,
            &mut sorties,
        );
        decodeur.cancel(&mut sorties);

        assert!(blocs(&sorties).is_empty(), "{sorties:?}");
        assert_eq!(
            sorties.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            })
        );
    }

    #[test]
    fn a_cancellation_in_the_middle_of_a_tool_call_proposes_nothing() {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"drop_table"}}"#,
            &mut sorties,
        );
        decodeur.on_data(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"nom\":"}}"#,
            &mut sorties,
        );
        decodeur.cancel(&mut sorties);

        assert!(
            appels(&sorties).is_empty(),
            "a truncated call never becomes a proposed action: {sorties:?}"
        );
        assert_eq!(
            sorties.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            })
        );
    }

    #[test]
    fn a_refusal_is_distinct_from_an_end_of_turn() {
        let evenements = jouer(&[
            r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Refusal
            })
        );
    }

    #[test]
    fn a_turn_pause_reports_itself_as_incomplete() {
        let evenements = jouer(&[
            r#"{"type":"message_delta","delta":{"stop_reason":"pause_turn"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        let Some(ChatEvent::Done { stop_reason }) = evenements.last() else {
            panic!("{evenements:?}");
        };
        assert_eq!(*stop_reason, StopReason::Paused);
        assert!(stop_reason.is_truncated());
    }

    #[test]
    fn an_error_in_the_stream_is_terminal() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ]);
        assert!(
            evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("Overloaded"))),
            "{evenements:?}"
        );
        assert_eq!(
            evenements.iter().filter(|e| e.is_terminal()).count(),
            1,
            "{evenements:?}"
        );
    }

    #[test]
    fn a_stream_closed_cleanly_without_message_stop_is_interrupted() {
        // A load balancer that cuts at its duration limit closes the
        // connection cleanly: no transport error, only a stream that stops.
        // Presenting this text as complete would be lying.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
        ]);
        assert_eq!(textes(&evenements), "a");
        let Some(ChatEvent::Done { stop_reason }) = evenements.last() else {
            panic!("the stream must end: {evenements:?}");
        };
        assert_eq!(*stop_reason, StopReason::Interrupted);
        assert!(stop_reason.is_truncated() && stop_reason.is_ambiguous());
        assert!(
            evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "the cut must show: {evenements:?}"
        );
    }

    #[test]
    fn an_announced_reason_without_message_stop_remains_a_cut() {
        // `message_delta` announces the end of the generation, not that of the
        // message: what followed was not received.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
        ]);
        assert_eq!(
            evenements.last(),
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
        let evenements = jouer(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":10}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"execute","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\":\"DELETE FROM t\"}"}}"#,
        ]);
        assert!(
            !evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
            "{evenements:?}"
        );
        assert!(
            evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("tool call #0"))),
            "la disparition de l'appel doit se voir : {evenements:?}"
        );
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn a_block_not_closed_before_message_stop_is_thrown_away() {
        // Server inconsistency: the message says it is finished, the block is not.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"execute","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(
            !evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
            "{evenements:?}"
        );
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::ToolCalls
            })
        );
    }

    #[test]
    fn an_unknown_event_does_not_break_the_stream() {
        // The documentation announces that new types can appear.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"un_evenement_futur","charge":{"quoi":"que ce soit"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(textes(&evenements), "a");
        assert!(
            !evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "un type inconnu n'est pas une erreur : {evenements:?}"
        );
    }

    #[test]
    fn a_block_of_unknown_type_produces_nothing_but_absorbs_its_deltas() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"x\"}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(appels(&evenements).is_empty(), "{evenements:?}");
        assert!(
            !evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "{evenements:?}"
        );
    }

    #[test]
    fn a_delta_without_an_open_block_is_ignored() {
        let evenements = jouer(&[
            r#"{"type":"content_block_delta","index":7,"delta":{"type":"text_delta","text":"fantome"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(textes(&evenements), "", "{evenements:?}");
    }

    #[test]
    fn an_isolated_unreadable_frame_does_not_kill_the_stream() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            "{ceci n'est pas du json",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"b"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(textes(&evenements), "b");
        assert!(evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))));
    }

    #[test]
    fn a_series_of_unreadable_frames_gives_up_the_stream() {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        for _ in 0..MAX_DECODE_ERRORS {
            decodeur.on_data("pas du json", &mut sorties);
        }
        assert!(decodeur.finished(), "{sorties:?}");
        assert!(sorties.last().is_some_and(ChatEvent::is_terminal));
    }

    /// A real SSE trace, as the documentation publishes it: event names,
    /// interleaved `ping`, blocks opened and closed.
    const TRACE: &str = concat!(
        "event: message_start\n",
        r#"data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-modele","stop_reason":null,"usage":{"input_tokens":472,"output_tokens":2}}}"#,
        "\n\n",
        "event: content_block_start\n",
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        "\n\n",
        "event: ping\n",
        r#"data: {"type": "ping"}"#,
        "\n\n",
        "event: content_block_delta\n",
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Je vérifie le café"}}"#,
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
    fn rejouer(morceaux: Vec<bytes::Bytes>) -> Vec<ChatEvent> {
        use futures::stream::StreamExt as _;

        let octets = morceaux
            .into_iter()
            .map(Ok::<bytes::Bytes, String>)
            .collect::<Vec<_>>();
        let flux = crate::stream::events_stream(
            Box::pin(futures::stream::iter(octets)),
            MessageDecoder::new(),
            oxyn_core::CancelToken::new(),
            None,
        );
        futures::executor::block_on(flux.collect())
    }

    #[test]
    fn a_real_trace_decodes_in_one_block() {
        let evenements = rejouer(vec![bytes::Bytes::from_static(TRACE.as_bytes())]);

        assert_eq!(textes(&evenements), "Je vérifie le café");
        let complets = appels(&evenements);
        assert_eq!(complets.len(), 1, "{evenements:?}");
        assert_eq!(complets[0].id, "toolu_01");
        assert_eq!(complets[0].arguments["ville"], "Besançon");
        assert_eq!(
            evenements.last(),
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
        let par_octet: Vec<bytes::Bytes> = TRACE
            .as_bytes()
            .iter()
            .map(|octet| bytes::Bytes::copy_from_slice(&[*octet]))
            .collect();

        assert_eq!(
            rejouer(par_octet),
            rejouer(vec![bytes::Bytes::from_static(TRACE.as_bytes())]),
            "the network split must change nothing"
        );
    }

    #[test]
    fn a_stream_cut_mid_turn_is_truncated_and_ambiguous() {
        // The case that costs money, and that shows neither at compile time
        // nor in review: the connection drops while the model has started to
        // answer. The server may have finished — and billed — the turn.
        // Replaying would pay twice (I-13).
        let coupee: Vec<std::result::Result<bytes::Bytes, String>> = vec![
            Ok(bytes::Bytes::from_static(
                concat!(
                    "event: content_block_start\n",
                    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
                    "\n\n",
                    "event: content_block_delta\n",
                    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"la table clients "}}"#,
                    "\n\n",
                )
                .as_bytes(),
            )),
            // Ni `message_delta`, ni `message_stop` : le transport lâche.
            Err("lost the connection while receiving the stream".to_owned()),
        ];

        let evenements = {
            use futures::stream::StreamExt as _;
            let flux = crate::stream::events_stream(
                Box::pin(futures::stream::iter(coupee)),
                MessageDecoder::new(),
                oxyn_core::CancelToken::new(),
                None,
            );
            futures::executor::block_on(flux.collect::<Vec<_>>())
        };

        // What was received stays shown: the user sees where it stopped rather
        // than losing the beginning.
        assert_eq!(textes(&evenements), "la table clients ");

        let Some(ChatEvent::Done { stop_reason }) = evenements.last() else {
            panic!("the stream must end: {evenements:?}");
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
            evenements.iter().filter(|e| e.is_terminal()).count(),
            1,
            "{evenements:?}"
        );
    }

    #[test]
    fn a_tool_call_cut_by_the_transport_is_never_proposed() {
        // The cut falls in the middle of the arguments. Proposing them would
        // amount to submitting an action whose reach no one knows.
        let coupee: Vec<std::result::Result<bytes::Bytes, String>> = vec![
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

        let evenements = {
            use futures::stream::StreamExt as _;
            let flux = crate::stream::events_stream(
                Box::pin(futures::stream::iter(coupee)),
                MessageDecoder::new(),
                oxyn_core::CancelToken::new(),
                None,
            );
            futures::executor::block_on(flux.collect::<Vec<_>>())
        };

        assert!(
            appels(&evenements).is_empty(),
            "truncated arguments are not arguments: {evenements:?}"
        );
        // And the partial SQL must not come out in an error message: what a
        // model writes can copy what it was given.
        for evenement in &evenements {
            if let ChatEvent::Error(message) = evenement {
                assert!(!message.contains("DELETE FROM"), "{message}");
            }
        }
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn nothing_is_decoded_after_the_end() {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(r#"{"type":"message_stop"}"#, &mut sorties);
        let apres = sorties.len();
        decodeur.on_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            &mut sorties,
        );
        assert_eq!(sorties.len(), apres, "{sorties:?}");
    }
}

#[cfg(test)]
mod budget_tests;
