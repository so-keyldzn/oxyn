use std::sync::Arc;

use oxyn_core::{AiProviderKind, PrivacyTier};
use oxyn_secrets::{ExposeSecret as _, MemorySecretStore, SecretRef, SecretStore};
use oxyn_store::Store;

use super::*;

#[test]
fn a_models_declared_efforts_reach_the_panel() {
    use oxyn_llm::ReasoningEffort::{High, Low};

    let choice = model_choice(
        oxyn_llm::ModelInfo::new("claude-opus-5").with_reasoning_efforts(vec![High, Low]),
    );
    assert_eq!(choice.reasoning_efforts, vec![Low, High]);
    // None declared stays none: no selector.
    assert!(
        model_choice(oxyn_llm::ModelInfo::new("m"))
            .reasoning_efforts
            .is_empty()
    );
}

fn draft(id: Option<String>) -> ProviderDraft {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "kind": "openai_compatible",
        "label": "  Ollama  ",
        "baseUrl": "",
        "model": " llama3 ",
    }))
    .expect("valid draft")
}

#[test]
fn a_local_tier_refuses_a_remote_or_unresolved_endpoint() {
    assert!(check_endpoint(PrivacyTier::Local, Reach::Remote).is_err());
    assert!(check_endpoint(PrivacyTier::Local, Reach::Unresolved).is_err());
    assert!(check_endpoint(PrivacyTier::Local, Reach::Local).is_ok());
    assert!(check_endpoint(PrivacyTier::Metadata, Reach::Remote).is_ok());
}

#[test]
fn an_edit_keeps_identity_endpoint_and_key_reference() {
    let previous = AiProviderConfig::new(
        ProviderId::for_new_declaration(AiProviderKind::OpenAiCompatible),
        AiProviderKind::OpenAiCompatible,
        "Old",
        "http://localhost:11434/v1",
        "old",
    )
    .with_secret_ref("oxyn:llm:openai_compatible-00000000");
    let config = compose_provider(&draft(Some(previous.id.to_string())), Some(&previous));
    assert_eq!(config.id, previous.id);
    assert_eq!(config.base_url, previous.base_url);
    assert_eq!(config.secret_ref, previous.secret_ref);
    assert_eq!(config.label, "Ollama");
    assert_eq!(config.model, "llama3");
}

#[test]
fn a_kind_change_drops_the_key_reference() {
    let previous = AiProviderConfig::new(
        ProviderId::for_new_declaration(AiProviderKind::Anthropic),
        AiProviderKind::Anthropic,
        "Old",
        "https://198.51.100.1/v1",
        "old",
    )
    .with_secret_ref("oxyn:llm:anthropic-00000000");
    let edited = provider_draft(
        Some(previous.id.to_string()),
        AiProviderKind::OpenAiCompatible,
        "https://198.51.100.1/v1",
        None,
    );
    let config = compose_provider(&edited, Some(&previous));
    assert_eq!(config.secret_ref, None);
}

/// A draft naming every field, unlike [`draft`]: the tests below edit
/// across a host change and must control the endpoint and the kind.
fn provider_draft(
    id: Option<String>,
    kind: AiProviderKind,
    base_url: &str,
    key: Option<&str>,
) -> ProviderDraft {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "kind": kind,
        "label": "Provider",
        "baseUrl": base_url,
        "model": "model",
        "key": key,
    }))
    .expect("valid draft")
}

fn backend_with_provider_store(path: &std::path::Path) -> (Backend, Arc<MemorySecretStore>) {
    let secrets = Arc::new(MemorySecretStore::new());
    let backend = Backend::assemble(
        Arc::new(Store::open_at(path).expect("temporary store")),
        Arc::clone(&secrets) as Arc<dyn SecretStore>,
    )
    .expect("temporary backend");
    (backend, secrets)
}

fn stored_provider(backend: &Backend, runtime: &tokio::runtime::Runtime) -> AiProviderConfig {
    runtime
        .block_on(backend.declared_providers())
        .expect("listed")
        .into_iter()
        .next()
        .expect("one declaration")
}

fn stored_key(store: &MemorySecretStore, config: &AiProviderConfig) -> Option<String> {
    let reference = SecretRef::parse(config.secret_ref.as_deref().expect("a reference"))
        .expect("valid reference");
    store
        .get(&reference)
        .expect("keychain read")
        .map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn a_failed_provider_edit_keeps_the_old_declaration_and_key() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime for the executor");
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("providers.sqlite3");
    let (backend, secrets) = backend_with_provider_store(&path);
    // An existing installation may still name the deterministic entry.
    let id = ProviderId::for_new_declaration(AiProviderKind::Anthropic);
    let reference = SecretRef::for_provider(id.as_str()).expect("legacy reference");
    secrets
        .put(
            &reference,
            oxyn_secrets::SecretString::from("old-test-key".to_owned()),
        )
        .expect("legacy key");
    let before = AiProviderConfig::new(
        id,
        AiProviderKind::Anthropic,
        "Provider",
        "https://198.51.100.1/v1",
        "model",
    )
    .with_secret_ref(reference.as_str());
    backend
        .inner
        .executor
        .store()
        .providers()
        .save(&before)
        .expect("legacy declaration");
    assert_eq!(
        backend
            .inner
            .credentials
            .provider_key(&before)
            .expect("legacy lookup")
            .expect("legacy key")
            .expose(),
        "old-test-key"
    );
    assert_eq!(
        stored_key(&secrets, &before).as_deref(),
        Some("old-test-key")
    );
    let connection = rusqlite::Connection::open(&path).expect("test connection");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_provider_update
                 BEFORE UPDATE ON ai_providers
                 BEGIN
                     SELECT RAISE(FAIL, 'injected provider save failure');
                 END;",
        )
        .expect("failing trigger");

    runtime
        .block_on(backend.save_ai_provider(provider_draft(
            Some(before.id.to_string()),
            AiProviderKind::Anthropic,
            "https://203.0.113.7/v1",
            Some("new-test-key"),
        )))
        .expect_err("the declaration save fails");

    let after = stored_provider(&backend, &runtime);
    assert_eq!(after.base_url, before.base_url);
    assert_eq!(after.secret_ref, before.secret_ref);
    assert_eq!(
        stored_key(&secrets, &after).as_deref(),
        Some("old-test-key")
    );
    assert_eq!(secrets.len(), 1, "the failed edit leaves no fresh orphan");
}

#[test]
fn a_successful_provider_key_edit_replaces_the_reference_and_forgets_the_old_entry() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime for the executor");
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("providers.sqlite3");
    let (backend, secrets) = backend_with_provider_store(&path);
    let declared = runtime
        .block_on(backend.save_ai_provider(provider_draft(
            None,
            AiProviderKind::Anthropic,
            "https://198.51.100.1/v1",
            Some("old-test-key"),
        )))
        .expect("declared");
    let mut before = stored_provider(&backend, &runtime);
    // Replacing on the same endpoint must isolate its key just like a move.
    for endpoint in ["https://203.0.113.7/v1", "https://203.0.113.7/v1"] {
        runtime
            .block_on(backend.save_ai_provider(provider_draft(
                Some(declared.id.clone()),
                AiProviderKind::Anthropic,
                endpoint,
                Some("new-test-key"),
            )))
            .expect("edited");

        let after = stored_provider(&backend, &runtime);
        assert_ne!(after.secret_ref, before.secret_ref);
        assert_eq!(
            stored_key(&secrets, &after).as_deref(),
            Some("new-test-key")
        );
        let old_reference = SecretRef::parse(before.secret_ref.as_deref().expect("old reference"))
            .expect("valid old reference");
        assert!(
            !secrets.contains(&old_reference),
            "the old key is forgotten"
        );
        assert_eq!(secrets.len(), 1);
        before = after;
    }
}

#[test]
fn overlapping_provider_key_edits_leave_only_the_saved_entry() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let directory = tempfile::tempdir().expect("temporary directory");
    let (backend, secrets) =
        backend_with_provider_store(&directory.path().join("providers.sqlite3"));
    runtime.block_on(async {
        let declared = backend
            .save_ai_provider(provider_draft(
                None,
                AiProviderKind::Anthropic,
                "https://198.51.100.1/v1",
                Some("initial-test-key"),
            ))
            .await
            .expect("declared");
        let before = backend
            .declared_providers()
            .await
            .expect("listed")
            .remove(0);
        let (left, right) = tokio::join!(
            backend.save_ai_provider(provider_draft(
                Some(declared.id.clone()),
                AiProviderKind::Anthropic,
                "https://203.0.113.7/v1",
                Some("left-test-key")
            )),
            backend.save_ai_provider(provider_draft(
                Some(declared.id.clone()),
                AiProviderKind::Anthropic,
                "https://198.51.100.2/v1",
                Some("right-test-key")
            )),
        );
        left.expect("left saved");
        right.expect("right saved");
        let saved = backend
            .declared_providers()
            .await
            .expect("listed")
            .remove(0);
        assert_ne!(saved.secret_ref, before.secret_ref);
        assert_eq!(
            secrets.len(),
            1,
            "each successful save retires its actual predecessor"
        );
        let key = stored_key(&secrets, &saved).expect("saved key");
        let expected = if saved.base_url == "https://203.0.113.7/v1" {
            "left-test-key"
        } else {
            "right-test-key"
        };
        assert_eq!(key, expected);
    });
}

#[test]
fn an_edit_to_another_host_without_a_key_drops_the_key_reference() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime for the executor");
    let backend = Backend::open_temporary().expect("temporary backend");
    runtime.block_on(async {
        let declared = backend
            .save_ai_provider(provider_draft(
                None,
                AiProviderKind::Anthropic,
                "https://198.51.100.1/v1",
                Some("not-a-real-key"),
            ))
            .await
            .expect("declared");
        assert!(declared.key_configured);

        let edited = backend
            .save_ai_provider(provider_draft(
                Some(declared.id.clone()),
                AiProviderKind::Anthropic,
                "https://203.0.113.7/v1",
                None,
            ))
            .await
            .expect("edited without a key, to another host");
        assert!(!edited.key_configured);

        let stored = backend
            .declared_providers()
            .await
            .expect("listed")
            .into_iter()
            .find(|config| config.id.to_string() == declared.id)
            .expect("still declared");
        assert_eq!(stored.secret_ref, None);
    });
}

#[test]
fn an_edit_on_the_same_endpoint_keeps_the_key() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime for the executor");
    let backend = Backend::open_temporary().expect("temporary backend");
    runtime.block_on(async {
        let declared = backend
            .save_ai_provider(provider_draft(
                None,
                AiProviderKind::Anthropic,
                "https://198.51.100.1/v1",
                Some("not-a-real-key"),
            ))
            .await
            .expect("declared");

        let edited = backend
            .save_ai_provider(provider_draft(
                Some(declared.id.clone()),
                AiProviderKind::Anthropic,
                "https://198.51.100.1:443/v1/",
                None,
            ))
            .await
            .expect("edited without a key, to the same endpoint");
        assert!(edited.key_configured);

        let before = backend
            .declared_providers()
            .await
            .expect("listed")
            .into_iter()
            .find(|config| config.id.to_string() == declared.id)
            .expect("still declared")
            .secret_ref;
        assert!(before.is_some());
    });
}

#[test]
fn models_after_a_host_change_read_no_key() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime for the executor");
    let backend = Backend::open_temporary().expect("temporary backend");
    runtime.block_on(async {
        let declared = backend
            .save_ai_provider(provider_draft(
                None,
                AiProviderKind::Anthropic,
                "https://198.51.100.1/v1",
                Some("not-a-real-key"),
            ))
            .await
            .expect("declared");
        backend
            .save_ai_provider(provider_draft(
                Some(declared.id.clone()),
                AiProviderKind::Anthropic,
                "https://203.0.113.7/v1",
                None,
            ))
            .await
            .expect("edited without a key, to another host");

        let error = backend
            .provider_models(&declared.id)
            .await
            .expect_err("no key follows a changed host");
        assert!(
            error.message.contains("An API key is needed"),
            "unexpected message: {}",
            error.message
        );
    });
}

#[test]
fn two_declarations_of_the_same_draft_stay_two() {
    let first = compose_provider(&draft(None), None);
    let second = compose_provider(&draft(None), None);
    assert_ne!(first.id, second.id);
}

#[test]
fn a_new_declaration_with_credentials_in_the_endpoint_is_refused_before_the_keyring() {
    let mut typed = draft(None);
    typed.base_url = "https://alice:hunter2@api.example.com/v1".to_owned();
    let error = compose_provider(&typed, None)
        .validate()
        .expect_err("credentials in the authority are refused");
    assert!(!error.to_string().contains("hunter2"));
}

fn agent_draft(id: Option<String>, command: &str) -> AgentDraft {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "label": "Codex",
        "command": command,
    }))
    .expect("valid draft")
}

#[test]
fn a_replacement_rewrites_the_agent_in_place_and_never_revives_a_removed_one() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime for the executor");
    let backend = Backend::open_temporary().expect("temporary backend");
    runtime.block_on(async {
        let declared = backend
            .save_external_agent(agent_draft(None, "/old/codex-acp"))
            .await
            .expect("decision answered")
            .expect("declared");

        backend
            .save_external_agent(agent_draft(Some(declared.id.clone()), "/new/codex-acp"))
            .await
            .expect("replaced");
        let agents = backend.declared_agents().await.expect("listed");
        assert_eq!(agents.len(), 1, "one declaration, not two");
        assert_eq!(agents[0].id.to_string(), declared.id);
        assert_eq!(agents[0].command, "/new/codex-acp");

        backend
            .remove_external_agent(&declared.id)
            .await
            .expect("removed");
        assert!(
            backend
                .save_external_agent(agent_draft(Some(declared.id.clone()), "/new/codex-acp"))
                .await
                .is_err()
        );
        assert!(backend.declared_agents().await.expect("listed").is_empty());
    });
}
