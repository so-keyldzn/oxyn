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
        let read_back: ReasoningBlock = serde_json::from_value(expected).expect("deserialization");
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
