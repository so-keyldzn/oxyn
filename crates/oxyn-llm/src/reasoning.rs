//! A model's reasoning: what is asked of it, what it returns.
//!
//! A subject apart from [`crate::types`] because it has its own rules, and
//! they look like nothing else in this crate:
//!
//! 1. **A reasoning block is sent back as is.** The protocols sign it;
//!    modifying it, reordering it or losing one makes the next turn fail. Oxyn
//!    therefore carries it without ever rebuilding it.
//! 2. **An encrypted block is not displayed.** It has no text: only an opaque
//!    payload, meaningful only to the provider that produced it.
//! 3. **Effort is not a budget.** One says *how much work* the model puts into
//!    its whole response, the other *how many tokens* it may spend thinking.
//!    Both exist, they do not replace each other, and not every provider has
//!    both.
//!
//! # Two types, two homes
//!
//! [`ReasoningBlock`] is **persisted** with the conversation, so it lives in
//! `oxyn-core` ([`oxyn_core::ai`]): that is what lets persistence read it
//! without pulling an HTTP client into its dependency tree. It is re-exported
//! here, and has a single definition.
//!
//! [`ReasoningEffort`] stays here: it is a **request** setting, it crosses no
//! persistence boundary.
//!
//! # The vocabulary is Oxyn's, not a provider's
//!
//! Each protocol names these things its own way — and the names do not match
//! field for field. The translation happens in the provider's module; it is
//! dated and sourced in
//! [`RESEARCH-NOTES`](../../../docs/RESEARCH-NOTES.md) (I-12).

use std::fmt;

use serde::{Deserialize, Serialize};

pub use oxyn_core::ai::ReasoningBlock;

/// How much work the model is asked to put into producing its response.
///
/// The scale is ordered, from the most frugal to the most expensive. It is
/// `#[non_exhaustive]`: providers publish others — a "minimal" level here, a
/// "none" there — and adding them must not be a breaking change.
///
/// **It is not a cost promise.** A level is a behavior signal: the model
/// thinks less at a low level, it does not stop at a ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ReasoningEffort {
    /// The most frugal: short answers, fewer tool calls.
    Low,
    /// A compromise between speed, cost and quality.
    Medium,
    /// The de facto default at providers that expose this setting.
    High,
    /// Beyond `High`, for long work. Written `xhigh` on the wire.
    XHigh,
    /// No spending constraint.
    Max,
}

impl ReasoningEffort {
    /// Stable name, the one that goes on the wire.
    ///
    /// The two protocols that expose this setting use the same strings; that
    /// is what allows a single table here rather than one per provider.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// Reads a level coming from a provider.
    ///
    /// Returns `None` on a level this version does not know, rather than
    /// falling back to a neighbor: asking for `high` where the user wanted
    /// `minimal` is a decision, not a decoding.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_levels_are_ordered_from_most_frugal_to_most_expensive() {
        assert!(ReasoningEffort::Low < ReasoningEffort::Medium);
        assert!(ReasoningEffort::Medium < ReasoningEffort::High);
        assert!(ReasoningEffort::High < ReasoningEffort::XHigh);
        assert!(ReasoningEffort::XHigh < ReasoningEffort::Max);
    }

    #[test]
    fn wire_names_are_the_protocols_ones() {
        assert_eq!(ReasoningEffort::XHigh.as_str(), "xhigh");
        assert_eq!(ReasoningEffort::Max.to_string(), "max");
        for niveau in [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
            ReasoningEffort::Max,
        ] {
            assert_eq!(ReasoningEffort::parse(niveau.as_str()), Some(niveau));
        }
    }

    #[test]
    fn an_unknown_level_does_not_fall_back_to_a_neighbor() {
        // `minimal` and `none` exist at one provider and not at the other:
        // translating them to `low` would change the user's request.
        assert_eq!(ReasoningEffort::parse("minimal"), None);
        assert_eq!(ReasoningEffort::parse("none"), None);
        assert_eq!(ReasoningEffort::parse(""), None);
    }

    #[test]
    fn the_reexported_block_is_the_domain_one() {
        // A single definition in the repository: if this re-export became a
        // second definition, persistence and transport would diverge without
        // either of them noticing.
        let ici: ReasoningBlock = ReasoningBlock::redacted("x");
        let domaine: oxyn_core::ai::ReasoningBlock = ici.clone();
        assert_eq!(ici, domaine);
    }
}
