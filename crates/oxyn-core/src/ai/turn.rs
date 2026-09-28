//! The vocabulary of a conversation turn that Oxyn **persists**.
//!
//! # Why these three types live here and not in `oxyn-llm`
//!
//! [`ReasoningBlock`] and [`StopReason`] are written to the database by
//! `oxyn-store`, **as they are**: a reasoning block is sent back to the provider
//! identically on the next turn, and the stop reason keeps the provider's word.
//! Copying them downstream would make two definitions; and because they are
//! `#[non_exhaustive]`, the conversion would have to go through a `_ =>` arm —
//! that is, silently lose the variant added later, for a failure that would
//! only show up at the user's, as "the provider refuses my turn".
//!
//! Letting `oxyn-store` depend on `oxyn-llm` to read them brought an HTTP
//! client into the tree of `oxyn-exec`, which never does network itself: the
//! direction of dependencies of
//! [ARCHITECTURE](../../../../docs/ARCHITECTURE.md) was broken. It is **shared
//! vocabulary**, not protocol, so it belongs here — it is the exact precedent
//! of [`ProviderId`](super::ProviderId). `oxyn-llm` re-exports them: they have
//! a single definition in the repository.
//!
//! [`Role`] follows for a more modest reason, and it is stated: it is not
//! persisted, but `oxyn-store` converts it to its own open role. Leaving it
//! behind would have kept the dependency for a single conversion.
//!
//! # What is **not** here
//!
//! The translation from a protocol string (`stop`, `end_turn`, `pause_turn`…).
//! It belongs to each provider, and it stays in `oxyn-llm`: the core knows no
//! protocol.
//!
//! # The serialized form is a persistence format
//!
//! `oxyn-store` calls `serde_json::to_string` directly on these types. Their
//! `serde` attributes **are** therefore the on-disk format of conversations:
//! renaming a variant, changing a `tag` or a `rename_all` makes the turns
//! already written unreadable, without an error. This module's tests freeze
//! the exact shape; a failure there is a migration to write, not a test to
//! adjust.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Who speaks in a conversation turn.
///
/// The enumeration is **closed**: these four roles are the shared vocabulary
/// of the three supported protocol families, and each translation to a
/// protocol must cover them all. Adding a role must break the compilation of
/// every provider, not be silently ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Framing instruction, set by Oxyn and never by the user.
    System,
    /// The user's turn — or the context Oxyn assembles for them.
    User,
    /// The model's turn.
    Assistant,
    /// Result of a tool execution, sent back to the model.
    Tool,
}

impl Role {
    /// Stable name, the one that goes on the wire for OpenAI-compatible
    /// protocols.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a stream stopped.
///
/// **Default external `serde` representation, without renaming**: it is the
/// format of the `stop_reason` column of persisted conversations. See the
/// module note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum StopReason {
    /// The model finished its turn.
    EndTurn,
    /// The token ceiling was reached: the reply is **truncated**.
    MaxTokens,
    /// The model asks for tools to be run before continuing.
    ToolCalls,
    /// The provider interrupted because of content filtering.
    ContentFilter,
    /// A stop sequence requested by the caller was met.
    ///
    /// Distinct from [`MaxTokens`](Self::MaxTokens): the reply stops where the
    /// caller wanted it to, it is not truncated by accident.
    StopSequence,
    /// The model refused to answer.
    ///
    /// Distinct from [`ContentFilter`](Self::ContentFilter), which is an
    /// intervention of the provider **on** a produced reply: here there is no
    /// reply. What the interface must say is therefore not the same.
    Refusal,
    /// The turn is **paused** and waits to be resumed.
    ///
    /// The provider reached an internal limit — a number of server-side tool
    /// turns, typically — and yields without the reply being finished. Nothing
    /// failed, and nothing is complete.
    Paused,
    /// The reply filled the model's context window.
    ///
    /// The reply is truncated, and raising the token ceiling will not change
    /// anything: it is the input that must be reduced.
    ContextWindowExceeded,
    /// The caller cancelled through the `CancelToken`.
    Cancelled,
    /// The stream stopped **before the end of the turn**, without the provider
    /// announcing it: dropped connection, buffer exceeded, unreadable frames in
    /// a row.
    ///
    /// It is the **ambiguous** case of [I-13](../../../../CLAUDE.md#i-13), and
    /// the only one in this enumeration: the server may have finished the
    /// generation — and so billed it — while nothing arrived here. Replaying
    /// the turn pays a second time for a reply already produced.
    /// See [`is_ambiguous`](Self::is_ambiguous).
    Interrupted,
    /// The provider reported an error **within** the stream, after starting
    /// to reply.
    ///
    /// Distinct from [`Interrupted`](Self::Interrupted): here the provider
    /// announced its failure, so there is no doubt about what happened on its
    /// side. The reply stays incomplete.
    ProviderError,
    /// The stream ended without the provider giving the reason.
    Unspecified,
    /// Provider-specific reason, kept as is rather than folded into a
    /// neighboring variant.
    Other(String),
}

impl StopReason {
    /// Is the reply incomplete because of the stop?
    ///
    /// To be shown in the interface: a reply cut at the token ceiling that does
    /// not say so looks like a wrong reply.
    ///
    /// [`StopSequence`](Self::StopSequence) answers `false`: the reply stops
    /// where the caller asked. [`Paused`](Self::Paused) answers `true` —
    /// nothing failed, but the rest is missing.
    ///
    /// [`Other`](Self::Other) answers `false` **deliberately**: it is a reason
    /// the provider named and that this version does not know, hence an
    /// ordinary end of turn until proven otherwise. A cut does not fall into
    /// it: it has its variant, [`Interrupted`](Self::Interrupted). Doing the
    /// opposite — a cut filed under `Other` — would present a reply sliced in
    /// the middle as a complete reply.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        matches!(
            self,
            Self::MaxTokens
                | Self::ContentFilter
                | Self::Refusal
                | Self::Paused
                | Self::ContextWindowExceeded
                | Self::Cancelled
                | Self::Interrupted
                | Self::ProviderError
        )
    }

    /// Do we not know what the provider actually produced?
    ///
    /// It is the question of [I-13](../../../../CLAUDE.md#i-13) asked of a
    /// generation: a turn whose stream was cut may have been finished — and
    /// billed — on the server side. **Replaying an ambiguous turn pays twice.**
    ///
    /// A single case answers `true`, and that is intended: everywhere else,
    /// either the provider announced the end, or the caller decided to stop.
    /// No crate of the repository replays a turn on its own; this method exists
    /// so that a caller who would consider it has the data, rather than
    /// deducing it from a character string.
    #[must_use]
    pub const fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Interrupted)
    }
}

/// A reasoning block produced by the model.
///
/// **This type is a transport, not content to read.** It exists to be handed
/// back to the provider on the next turn, identically: that is what protocols
/// require when a reasoning turn precedes a tool call. Rebuilding it from its
/// pieces, reordering or removing one makes the next request be refused.
///
/// Its `serde` shape (`tag = "kind"`, `snake_case`) is the format of the
/// `reasoning` column of persisted conversations. See the module note.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReasoningBlock {
    /// Written-out reasoning, as the provider agrees to show it.
    ///
    /// The text can be **empty**: several protocols send the block back without
    /// its text when the caller did not ask to see it. The block is still
    /// needed for the next turn, which is why we keep it anyway.
    Summarized {
        /// What the model agrees to show. Empty is legal.
        text: String,
        /// Opaque payload that authenticates the block to the provider.
        ///
        /// Not interpreted, not compared, not displayed: it only makes sense
        /// to the issuer.
        signature: Option<String>,
    },
    /// Reasoning encrypted by the provider.
    ///
    /// It has **no** text, and it is never displayed. It is sent back as is,
    /// otherwise the next turn loses the thread of the reasoning.
    Redacted {
        /// Opaque payload. See the variant note: it is not displayed.
        data: String,
    },
}

impl ReasoningBlock {
    /// Builds a written-out block.
    #[must_use]
    pub fn summarized(text: impl Into<String>, signature: Option<String>) -> Self {
        Self::Summarized {
            text: text.into(),
            signature,
        }
    }

    /// Builds an encrypted block.
    #[must_use]
    pub fn redacted(data: impl Into<String>) -> Self {
        Self::Redacted { data: data.into() }
    }

    /// The text to show, if there is one.
    ///
    /// Returns `None` for an encrypted block **and** for a written-out block
    /// whose text is empty: in both cases there is nothing to display, and an
    /// interface that rendered an empty string would draw an empty frame.
    #[must_use]
    pub fn display_text(&self) -> Option<&str> {
        match self {
            Self::Summarized { text, .. } if !text.is_empty() => Some(text),
            _ => None,
        }
    }

    /// Is the block encrypted?
    #[must_use]
    pub const fn is_redacted(&self) -> bool {
        matches!(self, Self::Redacted { .. })
    }
}

impl fmt::Debug for ReasoningBlock {
    /// Shows the text, never the opaque payload.
    ///
    /// A signature weighs hundreds of bytes and helps nobody diagnose a stream;
    /// copying it into a log would only make the log unreadable. The text, on
    /// the other hand, is the model's output.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Summarized { text, signature } => f
                .debug_struct("Summarized")
                .field("text", text)
                .field("signature", &signature.as_ref().map(|s| Opaque(s.len())))
                .finish(),
            Self::Redacted { data } => f
                .debug_struct("Redacted")
                .field("data", &Opaque(data.len()))
                .finish(),
        }
    }
}

/// Opaque payload marker, rendered `<opaque, N bytes>`.
struct Opaque(usize);

impl fmt::Debug for Opaque {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<opaque, {} bytes>", self.0)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // ── Persistence format, frozen ─────────────────────────────────────────
    //
    // These tests do not check a behavior: they freeze an on-disk format. A
    // failure here is not a test to adjust, it is a migration of persisted
    // conversations to write.

    #[test]
    fn the_persisted_shape_of_a_stop_reason_is_frozen() {
        for (reason, expected) in [
            (StopReason::EndTurn, json!("EndTurn")),
            (StopReason::MaxTokens, json!("MaxTokens")),
            (StopReason::ToolCalls, json!("ToolCalls")),
            (StopReason::ContentFilter, json!("ContentFilter")),
            (StopReason::StopSequence, json!("StopSequence")),
            (StopReason::Refusal, json!("Refusal")),
            (StopReason::Paused, json!("Paused")),
            (
                StopReason::ContextWindowExceeded,
                json!("ContextWindowExceeded"),
            ),
            (StopReason::Cancelled, json!("Cancelled")),
            (StopReason::Interrupted, json!("Interrupted")),
            (StopReason::ProviderError, json!("ProviderError")),
            (StopReason::Unspecified, json!("Unspecified")),
            (
                StopReason::Other("guardrail_intervened".to_owned()),
                json!({"Other": "guardrail_intervened"}),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(&reason).expect("serialization"),
                expected,
                "{reason:?}"
            );
            let read_back: StopReason = serde_json::from_value(expected).expect("deserialization");
            assert_eq!(read_back, reason);
        }
    }

    #[test]
    fn the_persisted_shape_of_a_reasoning_block_is_frozen() {
        for (block, expected) in [
            (
                ReasoningBlock::summarized("I am counting", Some("SIG".to_owned())),
                json!({"kind": "summarized", "text": "I am counting", "signature": "SIG"}),
            ),
            (
                ReasoningBlock::summarized("", None),
                json!({"kind": "summarized", "text": "", "signature": null}),
            ),
            (
                ReasoningBlock::redacted("CIPHERTEXT"),
                json!({"kind": "redacted", "data": "CIPHERTEXT"}),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(&block).expect("serialization"),
                expected,
                "{block:?}"
            );
            let read_back: ReasoningBlock =
                serde_json::from_value(expected).expect("deserialization");
            assert_eq!(read_back, block);
        }
    }

    #[test]
    fn the_serialized_shape_of_a_role_is_frozen() {
        for (role, expected) in [
            (Role::System, "system"),
            (Role::User, "user"),
            (Role::Assistant, "assistant"),
            (Role::Tool, "tool"),
        ] {
            assert_eq!(
                serde_json::to_value(role).expect("serialization"),
                json!(expected)
            );
            assert_eq!(
                role.as_str(),
                expected,
                "the wire name and the serde shape coincide"
            );
        }
    }

    // ── Stop reasons ───────────────────────────────────────────────────────

    #[test]
    fn a_cut_reply_is_reported() {
        assert!(StopReason::MaxTokens.is_truncated());
        assert!(StopReason::Cancelled.is_truncated());
        assert!(!StopReason::EndTurn.is_truncated());
        assert!(!StopReason::ToolCalls.is_truncated());
    }

    #[test]
    fn a_refused_or_paused_reply_is_reported_as_incomplete() {
        assert!(StopReason::Refusal.is_truncated());
        assert!(StopReason::Paused.is_truncated());
        assert!(StopReason::ContextWindowExceeded.is_truncated());
        assert!(
            !StopReason::StopSequence.is_truncated(),
            "a stop requested by the caller is not a truncation"
        );
    }

    #[test]
    fn a_stream_cut_is_the_only_ambiguous_reason() {
        // I-13: replaying a turn whose fate on the server side is unknown pays
        // twice for a reply that may already have been produced.
        assert!(StopReason::Interrupted.is_ambiguous());
        assert!(StopReason::Interrupted.is_truncated());

        for certain in [
            StopReason::EndTurn,
            StopReason::MaxTokens,
            StopReason::StopSequence,
            StopReason::ToolCalls,
            StopReason::ContentFilter,
            StopReason::Refusal,
            StopReason::Paused,
            StopReason::ContextWindowExceeded,
            StopReason::Cancelled,
            StopReason::ProviderError,
            StopReason::Unspecified,
        ] {
            assert!(
                !certain.is_ambiguous(),
                "{certain:?}: the turn's fate is known, or the caller decided"
            );
        }
    }

    #[test]
    fn an_error_announced_by_the_provider_truncates_without_being_ambiguous() {
        assert!(StopReason::ProviderError.is_truncated());
        assert!(
            !StopReason::ProviderError.is_ambiguous(),
            "the provider announced its failure: there is no doubt about what it did"
        );
    }

    #[test]
    fn a_reason_unknown_to_the_provider_stays_an_ordinary_end() {
        // `Other` carries a reason **named** by the provider: treating it as a
        // truncation would make every protocol evolution look like a cut
        // reply.
        let future = StopReason::Other("guardrail_intervened".to_owned());
        assert!(!future.is_truncated());
        assert!(!future.is_ambiguous());
    }

    // ── Reasoning blocks ───────────────────────────────────────────────────

    #[test]
    fn an_encrypted_block_has_nothing_to_show() {
        let block = ReasoningBlock::redacted("EncRypTeD");
        assert!(block.is_redacted());
        assert_eq!(block.display_text(), None);
    }

    #[test]
    fn a_block_rendered_empty_stays_transportable_but_is_not_displayed() {
        // The common case when the caller did not ask to see the reasoning:
        // the block arrives without text and must still be sent back on the
        // next turn.
        let block = ReasoningBlock::summarized("", Some("sig".to_owned()));
        assert_eq!(block.display_text(), None);
        assert!(!block.is_redacted());
    }

    #[test]
    fn the_debug_does_not_copy_the_opaque_payload() {
        let rendered = format!(
            "{:?}",
            ReasoningBlock::summarized("adding it up", Some("VERY_LONG_SIGNATURE".to_owned()))
        );
        assert!(!rendered.contains("VERY_LONG_SIGNATURE"), "{rendered}");
        assert!(rendered.contains("adding it up"), "{rendered}");

        let encrypted = format!("{:?}", ReasoningBlock::redacted("ENCRYPTED_PAYLOAD"));
        assert!(!encrypted.contains("ENCRYPTED_PAYLOAD"), "{encrypted}");
        assert!(encrypted.contains("opaque"), "{encrypted}");
    }
}
