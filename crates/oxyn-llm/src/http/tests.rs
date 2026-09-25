//! Le transport, éprouvé sur la boucle locale contre les trois familles.
//!
//! Aucune clé réelle : la sentinelle ci-dessous n'est valable nulle part, et
//! la chercher dans ce qui sort suffit à prouver qu'elle n'y est pas.

use oxyn_core::{CancelToken, OxynError};

use super::loopback::{self, Server};
use crate::anthropic::AnthropicProvider;
use crate::gemini::GeminiProvider;
use crate::openai_compatible::OpenAiCompatibleProvider;
use crate::provider::{LlmProvider, ProviderId};
use crate::secret::ApiKey;
use crate::types::{ChatMessage, ChatRequest};

/// Clé factice, reconnaissable partout où elle ne doit pas être.
const SENTINEL: &str = "sk-sentinel-5f0c1e9a-must-not-leak";

/// Ce que la conversation envoie : on vérifie qu'il ne part pas ailleurs.
const PROMPT: &str = "prompt-sentinel-customers-table";

fn question() -> ChatRequest {
    ChatRequest::new("model", vec![ChatMessage::user(PROMPT)])
}

/// Une réponse que la cible rendrait si on la contactait.
fn accepted() -> Vec<u8> {
    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        .to_vec()
}

/// La réponse d'échec, que le flux n'ouvre pas.
fn refused(issue: oxyn_core::Result<impl Sized>) -> OxynError {
    match issue {
        Ok(_) => panic!("a redirect must not open a stream"),
        Err(err) => err,
    }
}

/// Ce que toute redirection refusée doit tenir, quel que soit le fournisseur.
fn assert_refused(err: &OxynError, origin: &Server, target: &Server) {
    let rendu = format!("{err} {err:?}");
    assert!(rendu.contains("redirect"), "the message names the cause: {rendu}");
    assert!(!rendu.contains(SENTINEL), "{rendu}");
    assert!(
        !rendu.contains(&target.origin),
        "the Location header is not repeated: {rendu}"
    );
    assert!(!err.is_retryable(), "a redirect is fixed by configuration");
    assert_eq!(origin.hits(), 1, "the configured origin was asked once");
    assert!(origin.seen().contains(PROMPT), "the request did reach it");
    assert_eq!(
        target.hits(),
        0,
        "neither the body nor the key may reach another origin: {}",
        target.seen()
    );
}

/// Pour chaque statut qui conserve méthode et corps.
const PRESERVING: [u16; 2] = [307, 308];

#[tokio::test]
async fn openai_compatible_does_not_follow_a_redirect_with_its_bearer_key() {
    for status in PRESERVING {
        let target = Server::start(accepted(), false).await;
        let origin = Server::start(
            loopback::redirect(status, &format!("{}/v1/chat/completions", target.origin)),
            false,
        )
        .await;
        let provider =
            OpenAiCompatibleProvider::new(ProviderId::openai(), &format!("{}/v1", origin.origin))
                .expect("provider")
                .with_api_key(ApiKey::new(SENTINEL));

        let err = refused(provider.stream(question(), &CancelToken::new()).await);
        assert_refused(&err, &origin, &target);
    }
}

#[tokio::test]
async fn azure_does_not_follow_a_redirect_with_its_api_key_header() {
    for status in PRESERVING {
        let target = Server::start(accepted(), false).await;
        let origin = Server::start(loopback::redirect(status, &target.origin), false).await;
        let provider = OpenAiCompatibleProvider::azure(&origin.origin, "deployment", SENTINEL)
            .expect("provider");

        let err = refused(provider.stream(question(), &CancelToken::new()).await);
        assert_refused(&err, &origin, &target);
        assert!(
            origin.seen().to_ascii_lowercase().contains("api-key:"),
            "the proprietary header was sent to the configured origin only"
        );
    }
}

#[tokio::test]
async fn anthropic_does_not_follow_a_redirect_with_its_x_api_key() {
    for status in PRESERVING {
        let target = Server::start(accepted(), false).await;
        let origin = Server::start(
            loopback::redirect(status, &format!("{}/v1/messages", target.origin)),
            false,
        )
        .await;
        let provider =
            AnthropicProvider::with_base_url(SENTINEL, &origin.origin).expect("provider");

        let err = refused(provider.stream(question(), &CancelToken::new()).await);
        assert_refused(&err, &origin, &target);
        assert!(
            origin.seen().to_ascii_lowercase().contains("x-api-key:"),
            "the proprietary header was sent to the configured origin only"
        );
    }
}

#[tokio::test]
async fn a_model_listing_does_not_follow_a_redirect_either() {
    let target = Server::start(accepted(), false).await;
    let origin = Server::start(loopback::redirect(308, &target.origin), false).await;
    let provider =
        OpenAiCompatibleProvider::new(ProviderId::openai(), &format!("{}/v1", origin.origin))
            .expect("provider")
            .with_api_key(ApiKey::new(SENTINEL));

    let err = refused(provider.models().await);
    assert!(err.to_string().contains("redirect"), "{err}");
    assert_eq!(target.hits(), 0, "{}", target.seen());
}

/// Gemini n'envoie pas encore de génération ; son client est pourtant celui
/// qui partira le jour où il le fera, et c'est lui qu'on éprouve.
#[tokio::test]
async fn gemini_client_does_not_follow_a_redirect_with_its_x_goog_api_key() {
    for status in PRESERVING {
        let target = Server::start(accepted(), false).await;
        let origin = Server::start(loopback::redirect(status, &target.origin), false).await;
        let provider = GeminiProvider::with_base_url(SENTINEL, &origin.origin).expect("provider");

        let response = provider
            .prepared_request("gemini-model")
            .expect("request")
            .body(PROMPT)
            .send()
            .await
            .expect("the origin answers");
        assert_eq!(response.status().as_u16(), status, "the redirect is not followed");
        assert_eq!(target.hits(), 0, "{}", target.seen());
        assert!(
            origin
                .seen()
                .to_ascii_lowercase()
                .contains("x-goog-api-key:")
        );
    }
}

// ── #8 : une erreur diffusée après un `200` ne recopie pas la clé ─────────

/// Un flux `200` dont l'unique trame est une erreur citant la clé.
fn streamed_error(frame: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{frame}",
        frame.len()
    )
    .into_bytes()
}

/// Joue un flux jusqu'au bout et rend tous ses événements.
async fn drained(
    provider: &dyn LlmProvider,
) -> Vec<crate::types::ChatEvent> {
    use futures::StreamExt;
    let flux = provider
        .stream(question(), &CancelToken::new())
        .await
        .expect("a 200 opens the stream");
    flux.collect().await
}

/// Aucun événement, ni par son texte ni par son `Debug`, ne cite la clé ; et
/// l'erreur est bien là, sinon le test ne prouverait rien.
fn assert_redacted(events: &[crate::types::ChatEvent]) {
    let rendu = format!("{events:?}");
    assert!(!rendu.contains(SENTINEL), "{rendu}");
    let erreur = events
        .iter()
        .find_map(|event| match event {
            crate::types::ChatEvent::Error(message) => Some(message.as_str()),
            _ => None,
        })
        .expect("the streamed error is reported");
    assert!(erreur.contains("<redacted API key>"), "{erreur}");
    assert!(
        matches!(
            events.last(),
            Some(crate::types::ChatEvent::Done {
                stop_reason: crate::types::StopReason::ProviderError
            })
        ),
        "{rendu}"
    );
}

#[tokio::test]
async fn an_openai_compatible_streamed_error_does_not_repeat_the_key() {
    let frame = format!(
        "data: {{\"error\":{{\"message\":\"gateway rejected key {SENTINEL}\",\"type\":\"invalid_request_error\"}}}}\n\n"
    );
    let server = Server::start(streamed_error(&frame), false).await;
    let provider =
        OpenAiCompatibleProvider::new(ProviderId::openrouter(), &format!("{}/v1", server.origin))
            .expect("provider")
            .with_api_key(ApiKey::new(SENTINEL));

    assert_redacted(&drained(&provider).await);
}

#[tokio::test]
async fn an_anthropic_streamed_error_does_not_repeat_the_key() {
    let frame = format!(
        "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"overloaded_error\",\"message\":\"proxy saw x-api-key {SENTINEL}\"}}}}\n\n"
    );
    let server = Server::start(streamed_error(&frame), false).await;
    let provider = AnthropicProvider::with_base_url(SENTINEL, &server.origin).expect("provider");

    assert_redacted(&drained(&provider).await);
}
