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
