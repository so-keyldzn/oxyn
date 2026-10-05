//! Listing from a form, against a server on the loopback. No real provider is
//! called, and no key here is real.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use oxyn_core::{AiProviderConfig, AiProviderKind, ProviderId};
use oxyn_llm::{LlmError, Reach};
use oxyn_secrets::{MemorySecretStore, SecretStore};
use oxyn_store::Store;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;

use super::{LIST_TTL, ListKey, MAX_CACHED_LISTS, ModelLists, classify, endpoint};
use crate::backend::Backend;
use crate::ipc::ai::{ListingFailure, ModelChoice, ModelListing, ModelProbe, ProviderDraft};

/// Recognizable wherever it must not be.
const TYPED_KEY: &str = "sk-typed-draft-0000";
const STORED_KEY: &str = "sk-stored-draft-1111";

/// A server answering the same bytes to every request, or nothing at all.
struct Mock {
    origin: String,
    requests: Arc<StdMutex<Vec<String>>>,
}

impl Mock {
    /// `reply` `None` holds every connection without answering.
    async fn start(reply: Option<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port is free");
        let origin = format!("http://{}", listener.local_addr().expect("bound address"));
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let reply = reply.clone();
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move {
                    let mut read = Vec::new();
                    let mut buffer = [0_u8; 4096];
                    while let Ok(n) = socket.read(&mut buffer).await {
                        if n == 0 {
                            break;
                        }
                        read.extend_from_slice(buffer.get(..n).unwrap_or_default());
                        if read.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    recorded
                        .lock()
                        .expect("test lock")
                        .push(String::from_utf8_lossy(&read).into_owned());
                    match reply {
                        Some(reply) => {
                            let _ = socket.write_all(&reply).await;
                            let _ = socket.flush().await;
                        }
                        None => tokio::time::sleep(Duration::from_secs(30)).await,
                    }
                });
            }
        });
        Self { origin, requests }
    }

    fn hits(&self) -> usize {
        self.requests.lock().expect("test lock").len()
    }

    fn seen(&self) -> String {
        self.requests.lock().expect("test lock").join("\n")
    }
}

fn json(status: u16, body: &str) -> Option<Vec<u8>> {
    Some(
        format!(
            "HTTP/1.1 {status} Status\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes(),
    )
}

const TWO_MODELS: &str =
    r#"{"object":"list","data":[{"id":"alpha"},{"id":"beta","context_window":8192}]}"#;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime for the executor")
}

fn probe(id: Option<&str>, kind: AiProviderKind, base_url: &str, key: Option<&str>) -> ModelProbe {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "kind": kind,
        "baseUrl": base_url,
        "key": key,
    }))
    .expect("valid probe")
}

fn provider_draft(
    id: Option<&str>,
    base_url: &str,
    model: &str,
    key: Option<&str>,
) -> ProviderDraft {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "kind": "openai_compatible",
        "label": "Local",
        "baseUrl": base_url,
        "model": model,
        "key": key,
    }))
    .expect("valid draft")
}

fn backend_at(path: &std::path::Path) -> Backend {
    Backend::assemble(
        Arc::new(Store::open_at(path).expect("temporary store")),
        Arc::new(MemorySecretStore::new()) as Arc<dyn SecretStore>,
    )
    .expect("temporary backend")
}

fn ids(listing: &ModelListing) -> Vec<String> {
    match listing {
        ModelListing::Ok { models, .. } => models.iter().map(|m| m.id.clone()).collect(),
        ModelListing::Failed { reason, message } => panic!("listing failed: {reason:?} {message}"),
    }
}

fn failure(listing: &ModelListing) -> (ListingFailure, String) {
    match listing {
        ModelListing::Failed { reason, message } => (*reason, message.clone()),
        ModelListing::Ok { .. } => panic!("the listing should have failed"),
    }
}

// ── Against a server ─────────────────────────────────────────────────────────

#[test]
fn a_typed_key_lists_then_the_cache_answers_until_a_refresh() {
    let runtime = runtime();
    let backend = Backend::open_temporary().expect("temporary backend");
    runtime.block_on(async {
        let server = Mock::start(json(200, TWO_MODELS)).await;
        let base = format!("{}/v1", server.origin);
        let first = backend
            .list_draft_models(
                probe(
                    None,
                    AiProviderKind::OpenAiCompatible,
                    &base,
                    Some(TYPED_KEY),
                ),
                false,
            )
            .await
            .expect("well-formed probe");
        assert_eq!(ids(&first), ["alpha", "beta"]);
        assert!(matches!(first, ModelListing::Ok { cached: false, .. }));
        let seen = server.seen();
        assert!(seen.starts_with("GET /v1/models "), "{seen}");
        assert!(seen.contains(&format!("Bearer {TYPED_KEY}")), "{seen}");

        // The same endpoint written differently is the same entry.
        let again = backend
            .list_draft_models(
                probe(
                    None,
                    AiProviderKind::OpenAiCompatible,
                    &format!("{base}/"),
                    None,
                ),
                false,
            )
            .await
            .expect("well-formed probe");
        assert!(matches!(again, ModelListing::Ok { cached: true, .. }));
        assert_eq!(server.hits(), 1);

        let refreshed = backend
            .list_draft_models(
                probe(
                    None,
                    AiProviderKind::OpenAiCompatible,
                    &base,
                    Some(TYPED_KEY),
                ),
                true,
            )
            .await
            .expect("well-formed probe");
        assert!(matches!(refreshed, ModelListing::Ok { cached: false, .. }));
        assert_eq!(server.hits(), 2);
    });
}

#[test]
fn a_bare_array_is_listed() {
    let runtime = runtime();
    let backend = Backend::open_temporary().expect("temporary backend");
    runtime.block_on(async {
        let server = Mock::start(json(200, r#"[{"id":"together/m","display_name":"M"}]"#)).await;
        let listing = backend
            .list_draft_models(
                probe(None, AiProviderKind::OpenAiCompatible, &server.origin, None),
                false,
            )
            .await
            .expect("well-formed probe");
        assert_eq!(ids(&listing), ["together/m"]);
    });
}

#[test]
fn each_refusal_reaches_the_form_as_its_reason_and_is_not_cached() {
    let runtime = runtime();
    let backend = Backend::open_temporary().expect("temporary backend");
    runtime.block_on(async {
        for (reply, key, expected) in [
            (
                json(401, r#"{"error":"bad"}"#),
                Some(TYPED_KEY),
                ListingFailure::Unauthorized,
            ),
            (
                json(401, r#"{"error":"bad"}"#),
                None,
                ListingFailure::MissingKey,
            ),
            (json(403, "{}"), Some(TYPED_KEY), ListingFailure::Forbidden),
            (
                json(404, "not found"),
                Some(TYPED_KEY),
                ListingFailure::Unsupported,
            ),
            (
                json(429, "{}"),
                Some(TYPED_KEY),
                ListingFailure::RateLimited,
            ),
            (
                json(200, "<html>a login page</html>"),
                Some(TYPED_KEY),
                ListingFailure::Malformed,
            ),
        ] {
            let server = Mock::start(reply).await;
            let base = format!("{}/v1", server.origin);
            for _ in 0..2 {
                let listing = backend
                    .list_draft_models(
                        probe(None, AiProviderKind::OpenAiCompatible, &base, key),
                        false,
                    )
                    .await
                    .expect("well-formed probe");
                let (reason, message) = failure(&listing);
                assert_eq!(reason, expected, "{message}");
                assert!(!message.contains(TYPED_KEY), "{message}");
                assert!(!message.contains(&server.origin), "{message}");
            }
            assert_eq!(server.hits(), 2, "a failure is never cached");
        }
    });
}

#[test]
fn a_silent_server_is_a_timeout() {
    let runtime = runtime();
    let backend = Backend::open_temporary().expect("temporary backend");
    runtime.block_on(async {
        let server = Mock::start(None).await;
        let listing = backend
            .list_draft_models_within(
                probe(None, AiProviderKind::OpenAiCompatible, &server.origin, None),
                false,
                Some(Duration::from_millis(300)),
            )
            .await
            .expect("well-formed probe");
        assert_eq!(failure(&listing).0, ListingFailure::Timeout);
    });
}

#[test]
fn a_server_that_is_not_running_is_unreachable() {
    let runtime = runtime();
    let backend = Backend::open_temporary().expect("temporary backend");
    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a free port");
    let listing = runtime
        .block_on(backend.list_draft_models(
            probe(
                None,
                AiProviderKind::OpenAiCompatible,
                &format!("http://{free}/v1"),
                None,
            ),
            false,
        ))
        .expect("well-formed probe");
    let (reason, message) = failure(&listing);
    assert_eq!(reason, ListingFailure::Unreachable);
    assert!(message.contains("Is it running?"), "{message}");
}

#[test]
fn an_unusable_address_is_refused_before_any_request() {
    let runtime = runtime();
    let backend = Backend::open_temporary().expect("temporary backend");
    for (kind, base, key, expected) in [
        (
            AiProviderKind::OpenAiCompatible,
            "",
            None,
            ListingFailure::InvalidEndpoint,
        ),
        (
            AiProviderKind::OpenAiCompatible,
            "localhost:11434",
            None,
            ListingFailure::InvalidEndpoint,
        ),
        (
            AiProviderKind::OpenAiCompatible,
            "ftp://models.example/v1",
            None,
            ListingFailure::InvalidEndpoint,
        ),
        (
            AiProviderKind::OpenAiCompatible,
            "https://alice:hunter2@models.example/v1",
            None,
            ListingFailure::InvalidEndpoint,
        ),
        (
            AiProviderKind::Anthropic,
            "https://192.0.2.1",
            None,
            ListingFailure::MissingKey,
        ),
    ] {
        let listing = runtime
            .block_on(backend.list_draft_models(probe(None, kind, base, key), false))
            .expect("well-formed probe");
        let (reason, message) = failure(&listing);
        assert_eq!(reason, expected, "`{base}`: {message}");
        assert!(!message.contains("hunter2"), "{message}");
    }
}

#[test]
fn a_malformed_request_is_an_error_of_the_call() {
    let runtime = runtime();
    let backend = Backend::open_temporary().expect("temporary backend");
    let bad_id = runtime.block_on(backend.list_draft_models(
        probe(
            Some("not a provider id!"),
            AiProviderKind::OpenAi,
            "https://api.example",
            None,
        ),
        false,
    ));
    assert!(bad_id.is_err());
    let long_key = "k".repeat(super::MAX_KEY_BYTES + 1);
    let long = runtime.block_on(backend.list_draft_models(
        probe(
            None,
            AiProviderKind::OpenAi,
            "https://api.example",
            Some(&long_key),
        ),
        false,
    ));
    assert!(long.is_err());
}

#[test]
fn the_stored_key_follows_its_endpoint_and_no_other() {
    let runtime = runtime();
    let directory = tempfile::tempdir().expect("temporary directory");
    let backend = backend_at(&directory.path().join("workspace.sqlite"));
    runtime.block_on(async {
        let stored = Mock::start(json(200, TWO_MODELS)).await;
        let other = Mock::start(json(200, TWO_MODELS)).await;
        let declared = backend
            .save_ai_provider(provider_draft(
                None,
                &format!("{}/v1", stored.origin),
                "alpha",
                Some(STORED_KEY),
            ))
            .await
            .expect("declared");

        let same = backend
            .list_draft_models(
                probe(
                    Some(&declared.id),
                    AiProviderKind::OpenAiCompatible,
                    "",
                    None,
                ),
                true,
            )
            .await
            .expect("well-formed probe");
        assert_eq!(ids(&same), ["alpha", "beta"]);
        assert!(
            stored.seen().contains(&format!("Bearer {STORED_KEY}")),
            "{}",
            stored.seen()
        );

        let moved = backend
            .list_draft_models(
                probe(
                    Some(&declared.id),
                    AiProviderKind::OpenAiCompatible,
                    &format!("{}/v1", other.origin),
                    None,
                ),
                true,
            )
            .await
            .expect("well-formed probe");
        assert_eq!(ids(&moved), ["alpha", "beta"]);
        let seen = other.seen();
        assert!(
            !seen.contains(STORED_KEY),
            "the stored key followed a changed endpoint: {seen}"
        );
        assert!(
            !seen.to_ascii_lowercase().contains("authorization:"),
            "{seen}"
        );
    });
}

#[test]
fn a_model_chosen_from_the_list_survives_a_reload() {
    let runtime = runtime();
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("workspace.sqlite");
    let chosen = runtime.block_on(async {
        let server = Mock::start(json(200, TWO_MODELS)).await;
        let base = format!("{}/v1", server.origin);
        let backend = backend_at(&path);
        let listing = backend
            .list_draft_models(
                probe(None, AiProviderKind::OpenAiCompatible, &base, None),
                false,
            )
            .await
            .expect("well-formed probe");
        let chosen = ids(&listing).pop().expect("a model to choose");
        let declared = backend
            .save_ai_provider(provider_draft(None, &base, &chosen, None))
            .await
            .expect("declared with the chosen model");
        // The declared provider lists through the same path and cache.
        let models: Vec<ModelChoice> = backend.provider_models(&declared.id).await.expect("listed");
        assert_eq!(models.len(), 2);
        assert_eq!(server.hits(), 1, "the declared listing reused the draft's");
        chosen
    });

    let reloaded = backend_at(&path);
    let stored = runtime
        .block_on(reloaded.declared_providers())
        .expect("listed")
        .into_iter()
        .next()
        .expect("one declaration");
    assert_eq!(stored.model, chosen);
}

// ── The cache ────────────────────────────────────────────────────────────────

fn key(kind: AiProviderKind, raw: &str) -> ListKey {
    ListKey::of(kind, &Url::parse(raw).expect("a URL"))
}

fn one_model() -> Vec<ModelChoice> {
    vec![ModelChoice {
        id: "m".to_owned(),
        display_name: "m".to_owned(),
        context_window: None,
        cost: None,
        reasoning_efforts: Vec::new(),
    }]
}

#[test]
fn an_entry_expires_after_its_ttl() {
    let lists = ModelLists::default();
    let at = Instant::now();
    let entry = key(AiProviderKind::OpenAi, "https://api.example/v1");
    lists.put(entry.clone(), one_model(), 1, at);
    assert!(
        lists
            .get(&entry, at + LIST_TTL - Duration::from_secs(1))
            .is_some()
    );
    assert!(lists.get(&entry, at + LIST_TTL).is_none());
}

#[test]
fn entries_are_kept_apart_by_kind_and_endpoint() {
    let lists = ModelLists::default();
    let at = Instant::now();
    lists.put(
        key(AiProviderKind::OpenAi, "https://api.example/v1"),
        one_model(),
        1,
        at,
    );
    for other in [
        key(AiProviderKind::OpenAiCompatible, "https://api.example/v1"),
        key(AiProviderKind::OpenAi, "https://api.example/v2"),
        key(AiProviderKind::OpenAi, "https://other.example/v1"),
        key(AiProviderKind::OpenAi, "https://api.example:8443/v1"),
    ] {
        assert!(lists.get(&other, at).is_none());
    }
}

#[test]
fn a_key_never_holds_credentials_or_query() {
    let entry = key(
        AiProviderKind::OpenAi,
        "https://bob:hunter2@api.example/v1/?token=abc#f",
    );
    assert_eq!(entry.endpoint, "https://api.example:443/v1");
    assert!(key(AiProviderKind::OpenAi, "https://api.example:443/v1") == entry);
}

#[test]
fn the_cache_is_bounded() {
    let lists = ModelLists::default();
    let at = Instant::now();
    for n in 0..=MAX_CACHED_LISTS {
        let later = at + Duration::from_millis(u64::try_from(n).expect("small"));
        lists.put(
            key(AiProviderKind::OpenAi, &format!("https://h{n}.example/v1")),
            one_model(),
            1,
            later,
        );
    }
    assert_eq!(lists.entries.lock().len(), MAX_CACHED_LISTS);
    let newest = at + Duration::from_secs(1);
    assert!(
        lists
            .get(
                &key(AiProviderKind::OpenAi, "https://h0.example/v1"),
                newest
            )
            .is_none()
    );
    assert!(
        lists
            .get(
                &key(AiProviderKind::OpenAi, "https://h1.example/v1"),
                newest
            )
            .is_some()
    );
}

// ── Classification ───────────────────────────────────────────────────────────

#[test]
fn hints_name_the_version_segment_without_rewriting_it() {
    let http = |status| LlmError::from_response(ProviderId::openai(), status, "", None);
    let url = Url::parse("http://localhost:11434").expect("URL");
    let (reason, message) = classify(
        &http(404),
        AiProviderKind::OpenAiCompatible,
        &url,
        Reach::Local,
        false,
    );
    assert_eq!(reason, ListingFailure::Unsupported);
    assert!(message.contains("/v1"), "{message}");

    let anthropic = Url::parse("https://api.anthropic.com/v1").expect("URL");
    let (_, message) = classify(
        &http(404),
        AiProviderKind::Anthropic,
        &anthropic,
        Reach::Remote,
        true,
    );
    assert!(message.contains("stops before /v1"), "{message}");

    let (reason, _) = classify(
        &http(503),
        AiProviderKind::OpenAi,
        &anthropic,
        Reach::Remote,
        true,
    );
    assert_eq!(reason, ListingFailure::Unreachable);
    let (reason, _) = classify(
        &http(308),
        AiProviderKind::OpenAi,
        &anthropic,
        Reach::Remote,
        true,
    );
    assert_eq!(reason, ListingFailure::InvalidEndpoint);
}

#[test]
fn an_endpoint_check_never_quotes_the_address() {
    let config = |raw: &str| {
        AiProviderConfig::new(
            ProviderId::for_new_declaration(AiProviderKind::OpenAi),
            AiProviderKind::OpenAi,
            "probe",
            raw,
            "probe",
        )
    };
    for raw in [
        "https://u:secret-pass@h.example/v1",
        "not a url secret-pass",
    ] {
        let message = endpoint(&config(raw)).expect_err("refused");
        assert!(!message.contains("secret-pass"), "{message}");
    }
    assert!(endpoint(&config("http://[::1]:1234/v1")).is_ok());
}
