//! The contract of a model provider, and the registry that holds them.
//!
//! # No provider is required
//!
//! [`ProviderRegistry::default`] is **empty**, and it is the nominal state:
//! without configuration, the AI workspace is absent from the interface and
//! Oxyn remains a complete database client
//! ([ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)). No path of this
//! crate builds a provider on its own, reads an environment variable at
//! startup, or "detects" an Ollama that would be running on the machine. A
//! provider exists because the user registered it.
//!
//! # `ProviderId` lives in `oxyn-core`, and that is the point
//!
//! [`ProviderId`] is defined in [`oxyn_core::ai`] and re-exported here. There
//! is **only one** definition in the repository, for the same reason as
//! [`PrivacyTier`](oxyn_core::PrivacyTier): a
//! [`Command`](oxyn_core::Command) carries the identity of a provider — it is
//! the bus that declares it, lists it and removes it —, and `oxyn-core` cannot
//! depend on `oxyn-llm`
//! ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
//!
//! This module therefore only keeps what needs a transport: the trait, the
//! registry, and the factory that translates an
//! [`oxyn_core::AiProviderKind`] into a concrete implementation.
//!
//! # Why a trait
//!
//! Three incompatible protocol families (OpenAI-compatible, Anthropic,
//! Gemini), plus the providers to come through plugins: it is a **boundary**,
//! not an indirection with a single caller. The trait is object-safe — it is
//! used behind `Arc<dyn LlmProvider>` — and this constraint is hard.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxyn_core::{AiProviderKind, CancelToken, Result};
use reqwest::Url;

use crate::error::LlmError;
use crate::secret::ApiKey;
use crate::types::{ChatEvent, ChatRequest, ModelInfo};

pub use oxyn_core::ai::ProviderId;

/// Normalizes a base URL so that [`Url::join`] **appends** instead of
/// replacing.
///
/// Without a final `/`, `Url::join` replaces the last segment of the path:
/// `http://host/v1` joined with `chat/completions` gives
/// `http://host/chat/completions` and the request goes astray. The trap only
/// shows at run time, against an endpoint configured by hand.
#[must_use]
pub(crate) fn normalize_base_url(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    url
}

/// What a model provider must be able to do.
///
/// The trait is object-safe: it lives behind `Arc<dyn LlmProvider>` in the
/// [`ProviderRegistry`]. `fmt::Debug` is a deliberate supertrait bound — a
/// provider that carries a key must **write** its `Debug` by hand rather than
/// be exempted from having one (I-03).
#[async_trait]
pub trait LlmProvider: fmt::Debug + Send + Sync {
    /// Identifier under which this provider is registered.
    fn id(&self) -> ProviderId;

    /// Network endpoint, when the provider has one.
    ///
    /// Used to classify the sending as local or remote ([`crate::reach`]):
    /// that is what lets the interface say at all times where a request goes,
    /// as [`AI-PROVIDERS`](../../../docs/AI-PROVIDERS.md) requires. The default
    /// returns `None`, for a provider that has no URL — an in-process model,
    /// for example.
    fn endpoint(&self) -> Option<&Url> {
        None
    }

    /// Lists the models offered.
    ///
    /// # Errors
    /// Any error exchanging with the provider: network, failure status,
    /// unreadable response. A local provider that is switched off produces a
    /// transient error, and that is what lets the interface offer "retry"
    /// rather than "reconfigure".
    async fn models(&self) -> Result<Vec<ModelInfo>>;

    /// Counts the input tokens of a request, if the provider knows how.
    ///
    /// Returns `Ok(None)` by default, and that is the answer of most providers:
    /// **no** OpenAI-compatible endpoint exposes this service. `None` means "I
    /// cannot count", never "zero" — a caller that treated both the same would
    /// display an empty prompt.
    ///
    /// The count is an **estimate** by the provider, not an invoice: it can
    /// differ from what will actually be counted, and it depends on the target
    /// model.
    ///
    /// # Errors
    /// The same as an ordinary exchange: network, failure status, unreadable
    /// response. A provider that cannot count produces **no** error — it
    /// returns `None`.
    async fn count_tokens(&self, request: &ChatRequest) -> Result<Option<u32>> {
        let _ = request;
        Ok(None)
    }

    /// Starts a generation and returns the stream of events.
    ///
    /// The returned stream is `'static`: it does not hold the provider, which
    /// allows handing it to a task. It emits exactly one [`ChatEvent::Done`],
    /// last.
    ///
    /// # Cancellation
    /// The token is **cloned into the stream**: cancelling it interrupts the
    /// reading, emits `Done { stop_reason: Cancelled }` and closes the HTTP
    /// connection. Dropping the stream without cancelling the token also closes
    /// the connection, but informs no one — prefer explicit cancellation.
    ///
    /// # Errors
    /// Failures **before** the first byte (configuration, authentication,
    /// failure status) are returned here. Those that happen mid-stream become
    /// [`ChatEvent::Error`]s: an error can no longer be a return value once
    /// text has been shown to the user.
    async fn stream(
        &self,
        request: ChatRequest,
        cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>>;
}

/// The providers registered by the user.
///
/// **Empty by default**, and an empty registry is not a failure: it is Oxyn's
/// default installation.
///
/// The registry is shareable and can be modified live (the user adds a
/// provider in the settings without restarting), hence the internal lock. It
/// is ordered by identifier: the list shown to the user must not reorder from
/// one opening to the next.
#[derive(Debug, Default)]
pub struct ProviderRegistry {
    providers: RwLock<BTreeMap<ProviderId, Arc<dyn LlmProvider>>>,
}

impl ProviderRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the table, ignoring a possible poisoning of the lock.
    ///
    /// A poisoning means a thread panicked while holding the lock. The table
    /// stays consistent — its operations are atomic insertions and removals —,
    /// and refusing to serve the list of providers because of a panic that
    /// happened elsewhere would help no one.
    fn read(&self) -> RwLockReadGuard<'_, BTreeMap<ProviderId, Arc<dyn LlmProvider>>> {
        self.providers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes into the table, same rule as for reading.
    fn write(&self) -> RwLockWriteGuard<'_, BTreeMap<ProviderId, Arc<dyn LlmProvider>>> {
        self.providers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a provider, or replaces the one that already carried its
    /// identifier.
    ///
    /// Returns the replaced provider, if there was one: the caller can thus
    /// know that a reconfiguration took place — which, according to
    /// [`AI-PROVIDERS`](../../../docs/AI-PROVIDERS.md), must trigger a new
    /// local/remote classification of the endpoint.
    pub fn register(&self, provider: Arc<dyn LlmProvider>) -> Option<Arc<dyn LlmProvider>> {
        let id = provider.id();
        self.write().insert(id, provider)
    }

    /// Removes a provider.
    pub fn remove(&self, id: &ProviderId) -> Option<Arc<dyn LlmProvider>> {
        self.write().remove(id)
    }

    /// Returns a registered provider.
    #[must_use]
    pub fn get(&self, id: &ProviderId) -> Option<Arc<dyn LlmProvider>> {
        self.read().get(id).map(Arc::clone)
    }

    /// Registered identifiers, in stable order.
    #[must_use]
    pub fn ids(&self) -> Vec<ProviderId> {
        self.read().keys().cloned().collect()
    }

    /// All registered providers, in a stable order.
    #[must_use]
    pub fn providers(&self) -> Vec<Arc<dyn LlmProvider>> {
        self.read().values().map(Arc::clone).collect()
    }

    /// Number of registered providers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// No provider is registered.
    ///
    /// It is what the interface asks to decide whether the AI workspace exists.
    /// Answering `true` is not a degraded state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// Empties the registry.
    pub fn clear(&self) {
        self.write().clear();
    }
}

/// Builds the transport of a provider declaration.
///
/// It is the only place in the repository that translates an
/// [`AiProviderKind`] into a concrete implementation: `oxyn-llm` is the only
/// crate that knows [`AnthropicProvider`](crate::AnthropicProvider),
/// [`GeminiProvider`](crate::GeminiProvider) and
/// [`OpenAiCompatibleProvider`](crate::OpenAiCompatibleProvider), and putting
/// this translation in the wiring would push a domain rule down into it.
///
/// # A missing key is not always an error
///
/// Ollama, LM Studio and `llama.cpp` do not require one: under
/// [`OpenAiCompatible`](AiProviderKind::OpenAiCompatible), `key` can be `None`
/// and the request goes out without an authentication header. The three other
/// families require it, and the lack is reported **here**, locally, rather
/// than through a `401` the user would read as an account problem
/// ([`LlmError::MissingApiKey`]).
///
/// # It classifies nothing
///
/// No DNS resolution, no call to [`resolve_reach`](crate::reach::resolve_reach):
/// the local/remote classification is recomputed elsewhere, at each runtime
/// opening, and is never persisted
/// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
/// [`OpenAiCompatible`](AiProviderKind::OpenAiCompatible) covers an Ollama on
/// the loopback as well as a gateway in the cloud; the factory cannot tell
/// them apart and does not try.
///
/// `reach` must be the measurement used by the privacy gate and egress audit.
/// A local measurement confines every subsequent connection to loopback;
/// changing DNS answers fail closed. This constructor performs no DNS I/O.
///
/// # Identity of the built provider
///
/// [`LlmProvider::id`] returns the identifier **of the family**, not that of
/// the declaration: two declarations of the same family registered in the same
/// [`ProviderRegistry`] therefore replace each other. The nominal path does not
/// go through there — an [`AgentRuntime`](../../oxyn_ai/runtime/struct.AgentRuntime.html)
/// receives **one** `Arc<dyn LlmProvider>`, the one the user chose.
///
/// # Errors
/// [`LlmError::MissingApiKey`] if the family requires a key and receives none;
/// [`LlmError::Config`] if the base URL is unreadable or the HTTP client does
/// not build; [`LlmError::Unsupported`] for a family this binary cannot
/// instantiate — the `match` is on a `#[non_exhaustive]` enumeration, and
/// refusing is better than instantiating an approximate transport.
pub fn build_provider(
    kind: AiProviderKind,
    base_url: &str,
    key: Option<ApiKey>,
    reach: crate::Reach,
) -> Result<Arc<dyn LlmProvider>> {
    Ok(match build(kind, base_url, key, reach)? {
        Built::Anthropic(provider) => Arc::new(provider),
        Built::Gemini(provider) => Arc::new(provider),
        Built::OpenAiCompatible(provider) => Arc::new(provider),
    })
}

/// Lists the models an endpoint serves, through the transport
/// [`build_provider`] would build for it — same base URL resolution, same
/// redirect refusal, same reach confinement. Listing never succeeds on an
/// address a conversation would not use.
///
/// Sends the key, if any, to that endpoint and nothing else: no prompt, no
/// database content, no third-party registry.
///
/// # Errors
/// The [`LlmError`] as such, not projected onto the domain: listing models
/// must tell a refused key (`401`) from a key without the right (`403`), which
/// the projection merges. Same construction errors as [`build_provider`].
pub async fn list_models(
    kind: AiProviderKind,
    base_url: &str,
    key: Option<ApiKey>,
    reach: crate::Reach,
) -> std::result::Result<Vec<ModelInfo>, LlmError> {
    match build(kind, base_url, key, reach)? {
        Built::Anthropic(provider) => provider.fetch_models().await,
        Built::Gemini(provider) => provider.fetch_models().await,
        Built::OpenAiCompatible(provider) => provider.fetch_models().await,
    }
}

/// A transport of a known family, before it is erased behind the trait.
enum Built {
    Anthropic(crate::anthropic::AnthropicProvider),
    Gemini(crate::gemini::GeminiProvider),
    OpenAiCompatible(crate::openai_compatible::OpenAiCompatibleProvider),
}

/// What [`build_provider`] and [`list_models`] share: one construction per
/// family, so the two can never resolve an endpoint differently.
fn build(
    kind: AiProviderKind,
    base_url: &str,
    key: Option<ApiKey>,
    reach: crate::Reach,
) -> std::result::Result<Built, LlmError> {
    match kind {
        AiProviderKind::Anthropic => {
            let key = require_key(&ProviderId::anthropic(), key)?;
            Ok(Built::Anthropic(
                crate::anthropic::AnthropicProvider::with_base_url_and_reach(key, base_url, reach)?,
            ))
        }
        AiProviderKind::Gemini => {
            let key = require_key(&ProviderId::gemini(), key)?;
            Ok(Built::Gemini(
                crate::gemini::GeminiProvider::with_base_url_and_reach(key, base_url, reach)?,
            ))
        }
        AiProviderKind::OpenAi => {
            let id = ProviderId::openai();
            let key = require_key(&id, key)?;
            Ok(Built::OpenAiCompatible(
                crate::openai_compatible::OpenAiCompatibleProvider::new_with_reach(
                    id, base_url, reach,
                )?
                .with_api_key(key)
                .requiring_api_key()
                .with_usage_reporting(true)
                .supporting_reasoning_effort(),
            ))
        }
        AiProviderKind::OpenAiCompatible => {
            let provider = crate::openai_compatible::OpenAiCompatibleProvider::new_with_reach(
                ProviderId::openai_compatible(),
                base_url,
                reach,
            )?;
            // A given key is presented; its absence cannot be required — that
            // is the case of a local model.
            Ok(Built::OpenAiCompatible(match key {
                Some(key) => provider.with_api_key(key),
                None => provider,
            }))
        }
        other => Err(LlmError::Unsupported {
            provider: ProviderId::openai_compatible(),
            capability: format!("provider family `{other}`"),
        }),
    }
}

/// Requires the key of a family that does not work without one.
fn require_key(id: &ProviderId, key: Option<ApiKey>) -> std::result::Result<ApiKey, LlmError> {
    key.ok_or_else(|| LlmError::MissingApiKey {
        provider: id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test provider: opens no connection.
    #[derive(Debug)]
    struct FakeProvider(ProviderId);

    #[async_trait]
    impl LlmProvider for FakeProvider {
        fn id(&self) -> ProviderId {
            self.0.clone()
        }

        async fn models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![ModelInfo::new("factice")])
        }

        async fn stream(
            &self,
            _request: ChatRequest,
            _cancel: &CancelToken,
        ) -> Result<BoxStream<'static, ChatEvent>> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    fn fake(id: &str) -> Arc<dyn LlmProvider> {
        Arc::new(FakeProvider(
            ProviderId::new(id).expect("valid test identifier"),
        ))
    }

    #[test]
    fn a_new_registry_is_empty() {
        // ADR-0006: this is the default installation, not a failure.
        let registry = ProviderRegistry::default();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.ids().is_empty());
        assert!(registry.get(&ProviderId::openai()).is_none());
    }

    #[test]
    fn a_registered_provider_is_found_again() {
        let registry = ProviderRegistry::new();
        assert!(registry.register(fake("ollama")).is_none());
        assert!(!registry.is_empty());
        assert!(registry.get(&ProviderId::ollama()).is_some());
        assert_eq!(registry.ids(), vec![ProviderId::ollama()]);
    }

    #[test]
    fn registering_again_returns_the_previous_one() {
        let registry = ProviderRegistry::new();
        registry.register(fake("openai"));
        let replaced = registry.register(fake("openai"));
        assert!(
            replaced.is_some(),
            "a reconfiguration must be visible to the caller"
        );
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn the_order_of_identifiers_is_stable() {
        let registry = ProviderRegistry::new();
        for id in ["openrouter", "ollama", "azure-openai", "openai"] {
            registry.register(fake(id));
        }
        let ids: Vec<String> = registry.ids().iter().map(ProviderId::to_string).collect();
        assert_eq!(ids, ["azure-openai", "ollama", "openai", "openrouter"]);
    }

    #[test]
    fn remove_then_clear() {
        let registry = ProviderRegistry::new();
        registry.register(fake("ollama"));
        registry.register(fake("openai"));
        assert!(registry.remove(&ProviderId::ollama()).is_some());
        assert!(registry.remove(&ProviderId::ollama()).is_none());
        registry.clear();
        assert!(registry.is_empty());
    }

    #[test]
    fn a_local_family_builds_without_key() {
        // Ollama, LM Studio, `llama.cpp`: a missing key is the nominal state,
        // not a configuration failure.
        let provider = build_provider(
            AiProviderKind::OpenAiCompatible,
            "http://localhost:11434/v1",
            None,
            crate::Reach::Local,
        )
        .expect("a local endpoint builds without a key");
        assert_eq!(provider.id(), ProviderId::openai_compatible());
        assert_eq!(
            provider.endpoint().map(reqwest::Url::as_str),
            Some("http://localhost:11434/v1/"),
            "the URL is normalized, and nothing was resolved"
        );
    }

    #[test]
    fn a_remote_family_without_key_is_refused_locally() {
        // The lack is stated here, not through a `401` the user would read as
        // an account problem.
        for (kind, base_url) in [
            (AiProviderKind::Anthropic, "https://api.anthropic.com"),
            (AiProviderKind::OpenAi, "https://api.openai.com/v1"),
            (
                AiProviderKind::Gemini,
                "https://generativelanguage.googleapis.com",
            ),
        ] {
            let error = build_provider(kind, base_url, None, crate::Reach::Remote)
                .expect_err("a remote family requires a key");
            assert!(
                matches!(error, oxyn_core::OxynError::Authentication(_)),
                "{kind} : {error:?}"
            );
            let message = error.to_string();
            assert!(message.contains("API key"), "{message}");

            // With a key, the same declaration builds.
            let provider = build_provider(
                kind,
                base_url,
                Some(ApiKey::new("sk-test")),
                crate::Reach::Remote,
            )
            .expect("a remote family builds with its key");
            let rendered = format!("{provider:?}");
            assert!(!rendered.contains("sk-test"), "leaked key: {rendered}");
        }
    }

    #[test]
    fn the_factory_does_not_classify_the_endpoint() {
        // ADR-0023: the classification is recomputed elsewhere. A name that
        // contains `localhost` proves nothing, and the factory resolves
        // nothing — so it accepts both without telling them apart.
        for base_url in [
            "http://localhost:11434/v1",
            "https://localhost.mon-nuage.example/v1",
        ] {
            assert!(
                build_provider(
                    AiProviderKind::OpenAiCompatible,
                    base_url,
                    None,
                    crate::Reach::Unresolved
                )
                .is_ok(),
                "{base_url}"
            );
        }
    }

    #[test]
    fn an_unreadable_url_is_refused_by_the_factory() {
        let error = build_provider(
            AiProviderKind::OpenAiCompatible,
            "not a url",
            None,
            crate::Reach::Unresolved,
        )
        .expect_err("unreadable URL");
        assert!(matches!(error, oxyn_core::OxynError::Config(_)), "{error}");
    }

    #[test]
    fn identifier_validation_remains_the_domain_one() {
        // The type lives in `oxyn-core` and is tested there; this test only
        // guards the link: the re-export must not become a second, more
        // permissive definition.
        assert!(ProviderId::new("OpenAI").is_err(), "majuscules");
        assert!(ProviderId::new("lm-studio").is_ok());
        assert_eq!(ProviderId::ollama().as_str(), ProviderId::OLLAMA);
    }
}
