//! Listing models on the loopback: what goes out, and how each failure is
//! typed. No real provider is called.

use oxyn_core::AiProviderKind;

use super::loopback::Server;
use crate::error::LlmError;
use crate::provider::list_models;
use crate::reach::Reach;
use crate::secret::ApiKey;

/// Fake key, recognizable wherever it must not be.
const SENTINEL: &str = "sk-sentinel-models-must-not-leak";

/// A complete JSON response.
fn json(status: u16, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} Status\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

async fn listed(
    kind: AiProviderKind,
    base: &str,
    key: Option<&str>,
) -> Result<Vec<String>, LlmError> {
    list_models(kind, base, key.map(ApiKey::new), Reach::Local)
        .await
        .map(|models| models.into_iter().map(|model| model.id).collect())
}

#[tokio::test]
async fn an_openai_compatible_list_goes_to_the_base_path_with_the_key() {
    let server = Server::start(
        json(200, r#"{"object":"list","data":[{"id":"m1"},{"id":"m2"}]}"#),
        false,
    )
    .await;
    let ids = listed(
        AiProviderKind::OpenAiCompatible,
        &format!("{}/v1", server.origin),
        Some(SENTINEL),
    )
    .await
    .expect("listed");
    assert_eq!(ids, ["m1", "m2"]);
    let seen = server.seen();
    assert!(seen.starts_with("GET /v1/models "), "{seen}");
    assert!(seen.contains(&format!("Bearer {SENTINEL}")), "{seen}");
}

#[tokio::test]
async fn a_keyless_local_endpoint_is_listed_without_authorization() {
    let server = Server::start(json(200, r#"[{"id":"local"}]"#), false).await;
    let ids = listed(AiProviderKind::OpenAiCompatible, &server.origin, None)
        .await
        .expect("listed");
    assert_eq!(ids, ["local"]);
    let seen = server.seen().to_ascii_lowercase();
    assert!(seen.starts_with("get /models "), "{seen}");
    assert!(!seen.contains("authorization:"), "{seen}");
}

#[tokio::test]
async fn refusals_keep_their_status_and_never_the_key() {
    for status in [401_u16, 403, 404, 405, 429, 500] {
        let body = format!(r#"{{"error":"bad key {SENTINEL}"}}"#);
        let server = Server::start(json(status, &body), false).await;
        let err = listed(
            AiProviderKind::OpenAi,
            &format!("{}/v1", server.origin),
            Some(SENTINEL),
        )
        .await
        .expect_err("refused");
        assert!(
            matches!(&err, LlmError::Http { status: s, .. } if *s == status),
            "{err}"
        );
        assert!(!err.to_string().contains(SENTINEL), "{err}");
    }
}

#[tokio::test]
async fn an_unreadable_body_is_a_decoding_error() {
    for body in ["<html>not json</html>", r#"{"models":[]}"#] {
        let server = Server::start(json(200, body), false).await;
        let err = listed(AiProviderKind::OpenAiCompatible, &server.origin, None)
            .await
            .expect_err("malformed");
        assert!(matches!(err, LlmError::Decode { .. }), "{err}");
    }
}

#[tokio::test]
async fn a_closed_port_is_a_transport_error() {
    // A port bound then released: nothing listens on it any more.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let free = listener.local_addr().expect("address");
    drop(listener);
    let err = listed(
        AiProviderKind::OpenAiCompatible,
        &format!("http://{free}/v1"),
        None,
    )
    .await
    .expect_err("nothing listens");
    assert!(matches!(err, LlmError::Transport { .. }), "{err}");
}

#[tokio::test]
async fn a_remote_family_without_a_key_does_not_go_out() {
    let server = Server::start(json(200, "[]"), false).await;
    for kind in [
        AiProviderKind::OpenAi,
        AiProviderKind::Anthropic,
        AiProviderKind::Gemini,
    ] {
        let err = listed(kind, &server.origin, None)
            .await
            .expect_err("no key");
        assert!(matches!(err, LlmError::MissingApiKey { .. }), "{err}");
    }
    assert_eq!(server.hits(), 0, "{}", server.seen());
}

#[tokio::test]
async fn gemini_lists_through_its_header_and_follows_a_page_token() {
    let server = Server::start(
        json(
            200,
            r#"{"models":[{"name":"models/gemini-x","displayName":"Gemini X","supportedGenerationMethods":["generateContent"]}],"nextPageToken":""}"#,
        ),
        false,
    )
    .await;
    let ids = listed(AiProviderKind::Gemini, &server.origin, Some(SENTINEL))
        .await
        .expect("listed");
    assert_eq!(ids, ["gemini-x"]);
    let seen = server.seen();
    assert!(
        seen.starts_with("GET /v1beta/models?pageSize=1000 "),
        "{seen}"
    );
    assert!(
        seen.to_ascii_lowercase().contains("x-goog-api-key:"),
        "{seen}"
    );
    assert!(
        !seen.contains("key="),
        "the key never goes in the query: {seen}"
    );
}

#[tokio::test]
async fn a_token_handed_out_forever_ends_at_the_page_bound() {
    // Every page names a next one; the walk stops at its bound rather than
    // looping on the server's word.
    let server = Server::start(
        json(
            200,
            r#"{"models":[{"name":"models/m"}],"nextPageToken":"again"}"#,
        ),
        false,
    )
    .await;
    let ids = listed(AiProviderKind::Gemini, &server.origin, Some(SENTINEL))
        .await
        .expect("listed");
    // The first page, then the « again » page; the repeated token stops it.
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_eq!(server.hits(), 2);
}
