//! The model provider abstraction.
//!
//! `oxyn-llm` knows how to talk to a model and to nothing else. It knows
//! neither agents, nor the catalog, nor privacy tiers: it is `oxyn-ai` that
//! assembles the context and applies the connection's tier
//! ([I-04](../../../CLAUDE.md#i-04)), and the `PolicyGate` that decides what a
//! response is allowed to trigger ([I-07](../../../CLAUDE.md#i-07)). This crate
//! is a **typed transport**, and this narrow scope is what makes it
//! reviewable.
//!
//! # What it contains
//!
//! | Module | Subject |
//! |---|---|
//! | [`types`] | the vocabulary of an exchange: messages, tools, events, models |
//! | [`provider`] | the [`LlmProvider`] trait and the [`ProviderRegistry`] |
//! | [`openai_compatible`] | **one** implementation for seven providers |
//! | [`anthropic`] | the `/v1/messages` protocol, complete |
//! | [`gemini`] | its own protocol — request written, sending still to do |
//! | [`reasoning`] | effort, budget, and the blocks sent back as they are |
//! | [`error`] | [`LlmError`] and its projection onto the domain |
//! | [`secret`] | [`ApiKey`], which is never displayed |
//! | [`reach`] | local or remote, decided after resolution and not on the name |
//!
//! # The four choices that govern this crate
//!
//! **No provider is required.** [`ProviderRegistry::default`] is empty, and it
//! is Oxyn's default installation: the AI workspace is then absent from the
//! interface and the product remains a complete database client
//! ([ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)). Nothing here probes
//! the machine looking for a local model, reads an environment variable at
//! startup, or builds a provider it was not asked for.
//!
//! **One implementation for the compatible protocols, one per real
//! protocol.** [`OpenAiCompatibleProvider`] covers Ollama, LM Studio,
//! `llama.cpp`, OpenAI, Azure and OpenRouter. Anthropic and Gemini have their
//! own: their protocols differ exactly where Oxyn needs them to be exact —
//! tool calls — and a common adapter would be wrong there
//! ([`ARCHITECTURE` §7.5](../../../docs/ARCHITECTURE.md)).
//!
//! What they share is **the stream driver**, not the decoding: the guarantee
//! "exactly one [`ChatEvent::Done`], last, whatever the exit" is held in a
//! single place, and each protocol only plugs its frame reading into it. Two
//! drivers side by side would diverge at the first addition.
//!
//! **What goes out is not displayed.** [`ApiKey`] masks its `Debug` and has no
//! `Display`; [`ChatMessage`] and [`ChatRequest`] mask their content, because
//! at the `Sampled` tier that content is made of real rows from the user's
//! database, and a `tracing::debug!` would write them in clear on disk
//! ([I-03](../../../CLAUDE.md#i-03)). Any response body quoted in an error is
//! truncated and scrubbed of the key.
//!
//! **Cancellation goes all the way.** The [`oxyn_core::CancelToken`] passed to
//! [`LlmProvider::stream`] is cloned into the returned stream: cancelling it
//! interrupts the reading, closes the connection and emits
//! `Done { stop_reason: Cancelled }`. No partially received tool call is
//! proposed — truncated arguments are not arguments.
//!
//! # Example
//!
//! ```
//! use oxyn_llm::prelude::*;
//!
//! // Without configuration: no provider. This is not a failure.
//! let registry = ProviderRegistry::default();
//! assert!(registry.is_empty());
//! assert!(registry.get(&ProviderId::ollama()).is_none());
//! ```
//!
//! Once a provider is configured by the user:
//!
//! ```no_run
//! use std::sync::Arc;
//! use oxyn_llm::prelude::*;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let registry = ProviderRegistry::new();
//! registry.register(Arc::new(OpenAiCompatibleProvider::ollama()?));
//!
//! let provider = registry
//!     .get(&ProviderId::ollama())
//!     .ok_or("provider missing")?;
//!
//! let request = ChatRequest::new(
//!     "llama3.2",
//!     vec![ChatMessage::user("list the tables of this schema")],
//! );
//! let token = CancelToken::new();
//! let _stream = provider.stream(request, &token);
//! # Ok(())
//! # }
//! ```

pub mod anthropic;
pub mod budget;
pub mod error;
pub mod gemini;
pub mod openai_compatible;
pub mod provider;
pub mod reach;
pub mod reasoning;
pub mod secret;
pub mod types;

/// `text/event-stream` decoding, shared by the three protocol families.
///
/// Internal: it is a transport detail, and exposing it would invite writing a
/// provider that bypasses [`LlmProvider`].
mod sse;

/// The HTTP transport: building the client, reading a failure response.
///
/// Internal: it is what refuses redirections, and this guarantee only holds
/// because no provider builds its client any other way.
mod http;

/// The cancellable stream driver, shared as well.
///
/// Internal for the same reason as [`sse`]: it is what guarantees that a
/// `Done` is emitted exactly once, and this guarantee only holds because no
/// provider can assemble its stream any other way.
mod stream;

/// Re-exported from `oxyn-core`: the token appears in the signature of
/// [`LlmProvider::stream`], and a caller should not have to depend on the
/// domain to build one.
pub use oxyn_core::CancelToken;

pub use anthropic::AnthropicProvider;
pub use budget::{BudgetExceeded, GenerationBudget};
pub use error::LlmError;
pub use gemini::GeminiProvider;
pub use openai_compatible::{AuthStyle, OpenAiCompatibleProvider};
pub use provider::{LlmProvider, ProviderId, ProviderRegistry, build_provider, list_models};
pub use reach::{Reach, endpoint_reach};
pub use reasoning::{ReasoningBlock, ReasoningEffort};
pub use secret::ApiKey;
pub use types::{
    ChatEvent, ChatMessage, ChatRequest, Cost, ModelInfo, Role, StopReason, Support, ToolCall,
    ToolSpec,
};

/// What is imported in one go to talk to a model.
///
/// ```
/// use oxyn_llm::prelude::*;
/// ```
pub mod prelude {
    pub use oxyn_core::CancelToken;

    pub use crate::error::LlmError;
    pub use crate::openai_compatible::OpenAiCompatibleProvider;
    pub use crate::provider::{LlmProvider, ProviderId, ProviderRegistry, build_provider};
    pub use crate::reach::{Reach, endpoint_reach};
    pub use crate::reasoning::{ReasoningBlock, ReasoningEffort};
    pub use crate::secret::ApiKey;
    pub use crate::types::{
        ChatEvent, ChatMessage, ChatRequest, Cost, ModelInfo, Role, StopReason, Support, ToolCall,
        ToolSpec,
    };
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    /// The nominal path of phase 2, reduced to what can be tested without a
    /// network: an empty registry, a registered provider, a built request.
    #[test]
    fn the_ai_workspace_is_absent_without_configuration() {
        // ADR-0006: this is Oxyn's default state, not a degraded state.
        let registry = ProviderRegistry::new();
        assert!(registry.is_empty());
        assert!(registry.providers().is_empty());
        for id in [
            ProviderId::ollama(),
            ProviderId::openai(),
            ProviderId::anthropic(),
            ProviderId::gemini(),
        ] {
            assert!(registry.get(&id).is_none(), "{id}");
        }
    }

    #[test]
    fn a_request_does_not_leak_the_context_into_traces() {
        // The failure I-03 targets: `tracing::debug!("{req:?}")` writing rows
        // of the customer database into a log file.
        let request = ChatRequest::new(
            "llama3.2",
            vec![
                ChatMessage::system("you answer in SQL"),
                ChatMessage::user("customer Dupont, IBAN FR7630006000011234567890189"),
            ],
        );
        let rendered = format!("{request:?}");
        assert!(!rendered.contains("Dupont"), "{rendered}");
        assert!(!rendered.contains("FR76"), "{rendered}");
    }

    #[test]
    fn provider_identifiers_are_distinct_and_stable() {
        let ids = [
            ProviderId::ollama(),
            ProviderId::lm_studio(),
            ProviderId::llama_cpp(),
            ProviderId::openai(),
            ProviderId::azure_openai(),
            ProviderId::openrouter(),
            ProviderId::anthropic(),
            ProviderId::gemini(),
        ];
        let mut seen: Vec<String> = ids.iter().map(ProviderId::to_string).collect();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "two providers share a name");
    }
}
