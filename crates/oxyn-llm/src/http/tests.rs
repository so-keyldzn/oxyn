//! The transport, tested on the loopback against the three families.
//!
//! No real key: the sentinel below is valid nowhere, and looking for it in
//! what goes out is enough to prove it is not there.

use oxyn_core::{CancelToken, OxynError};

use super::loopback::{self, Server};
use crate::anthropic::AnthropicProvider;
use crate::gemini::GeminiProvider;
use crate::openai_compatible::OpenAiCompatibleProvider;
use crate::provider::{LlmProvider, ProviderId};
use crate::secret::ApiKey;
use crate::types::{ChatMessage, ChatRequest};

/// Fake key, recognizable wherever it must not be.
const SENTINEL: &str = "sk-sentinel-5f0c1e9a-must-not-leak";

/// What the conversation sends: we check it does not go elsewhere.
const PROMPT: &str = "prompt-sentinel-customers-table";

fn question() -> ChatRequest {
    ChatRequest::new("model", vec![ChatMessage::user(PROMPT)])
}

/// A response the target would return if it were contacted.
fn accepted() -> Vec<u8> {
    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        .to_vec()
}

/// The failure response, which the stream does not open.
fn refused(issue: oxyn_core::Result<impl Sized>) -> OxynError {
    match issue {
        Ok(_) => panic!("a redirect must not open a stream"),
        Err(err) => err,
    }
}

/// What every refused redirection must hold, whatever the provider.
fn assert_refused(err: &OxynError, origin: &Server, target: &Server) {
    let rendu = format!("{err} {err:?}");
    assert!(
        rendu.contains("redirect"),
        "the message names the cause: {rendu}"
    );
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

/// For each status that keeps method and body.
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

/// Gemini does not send a generation yet; its client is nevertheless the one
/// that will go out the day it does, and it is the one tested.
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
        assert_eq!(
            response.status().as_u16(),
            status,
            "the redirect is not followed"
        );
        assert_eq!(target.hits(), 0, "{}", target.seen());
        assert!(
            origin
                .seen()
                .to_ascii_lowercase()
                .contains("x-goog-api-key:")
        );
    }
}

// ── #8: an error streamed after a `200` does not copy the key ─────────────

/// A `200` stream whose only frame is an error quoting the key.
fn streamed_error(frame: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{frame}",
        frame.len()
    )
    .into_bytes()
}

/// Plays a stream to the end and returns all its events.
async fn drained(provider: &dyn LlmProvider) -> Vec<crate::types::ChatEvent> {
    use futures::StreamExt;
    let flux = provider
        .stream(question(), &CancelToken::new())
        .await
        .expect("a 200 opens the stream");
    flux.collect().await
}

/// No event, neither by its text nor by its `Debug`, quotes the key; and the
/// error is indeed there, otherwise the test would prove nothing.
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

// ── #12: a non-stream body is read under a bound ───────────────────────────

use std::time::Duration;

use super::{
    MAX_ERROR_BODY_BYTES, MAX_JSON_BODY_BYTES, Read, failure, failure_within, read_json_within,
    read_limited,
};
use crate::error::LlmError;

/// Safety bound of the tests: if it expires, the reading is waiting for a
/// body that will never come — the defect these tests close. No test waits
/// for it to succeed.
const GUARD: Duration = Duration::from_secs(5);

/// Headers of status `status`, then `body`, on a connection the server does
/// not close.
fn open_body(status: u16, length: Option<usize>, body: &[u8]) -> Vec<u8> {
    let mut reply = format!("HTTP/1.1 {status} Status\r\n").into_bytes();
    if let Some(length) = length {
        reply.extend_from_slice(format!("content-length: {length}\r\n").as_bytes());
    }
    reply.extend_from_slice(b"\r\n");
    reply.extend_from_slice(body);
    reply
}

/// The headers are received: it is the reading of the body that is tested.
async fn headers_of(server: &Server) -> reqwest::Response {
    super::client(&ProviderId::openai())
        .expect("client")
        .get(&server.origin)
        .send()
        .await
        .expect("headers arrive")
}

#[tokio::test]
async fn cancel_ends_the_read_of_an_error_body_that_never_finishes() {
    let server = Server::start(open_body(500, Some(1_000_000), b"partial diagnostic"), true).await;
    let response = headers_of(&server).await;
    let cancel = CancelToken::new();
    let annule = cancel.clone();

    let (err, ()) = tokio::time::timeout(
        GUARD,
        futures::future::join(
            failure(&ProviderId::openai(), response, None, Some(&cancel)),
            async move {
                tokio::task::yield_now().await;
                annule.cancel();
            },
        ),
    )
    .await
    .expect("cancelling ends the read of an open body");
    assert!(matches!(err, LlmError::Cancelled), "{err:?}");
}

#[tokio::test]
async fn an_error_body_past_its_delay_keeps_the_status_and_what_arrived() {
    let server = Server::start(open_body(503, Some(1_000_000), b"overloaded"), true).await;
    let response = headers_of(&server).await;

    let err = tokio::time::timeout(
        GUARD,
        failure_within(
            &ProviderId::openai(),
            response,
            None,
            None,
            Duration::from_millis(50),
        ),
    )
    .await
    .expect("the delay bounds the read");
    assert!(
        matches!(&err, LlmError::Http { status: 503, message, .. } if message.contains("overloaded")),
        "{err:?}"
    );
    assert!(err.is_retryable(), "the status still decides the class");
}

#[tokio::test]
async fn an_error_body_is_never_held_past_its_bound() {
    // Four mebibytes announced by nothing, then a connection that stays open:
    // reading to the end would wait forever.
    let big = vec![b'x'; 4 * 1024 * 1024];
    let server = Server::start(open_body(500, None, &big), true).await;
    let response = headers_of(&server).await;

    let lu = tokio::time::timeout(
        GUARD,
        read_limited(response, MAX_ERROR_BODY_BYTES, GUARD, None),
    )
    .await
    .expect("the bound stops the read");
    match lu {
        Read::Overflow(corps) => assert_eq!(corps.len(), MAX_ERROR_BODY_BYTES),
        _ => panic!("the bound is reported as such"),
    }
}

#[tokio::test]
async fn an_error_body_announced_long_is_read_up_to_its_bound_only() {
    let big = vec![b'y'; 64 * 1024];
    let server = Server::start(open_body(500, Some(1 << 30), &big), true).await;
    let response = headers_of(&server).await;

    let lu = tokio::time::timeout(
        GUARD,
        read_limited(response, MAX_ERROR_BODY_BYTES, GUARD, None),
    )
    .await
    .expect("the bound stops the read");
    assert!(matches!(lu, Read::Overflow(corps) if corps.len() == MAX_ERROR_BODY_BYTES));
}

#[tokio::test]
async fn a_key_cut_by_the_bound_does_not_leave_its_start_behind() {
    // The server copies the key, and the connection stops in the middle.
    let partial = format!("rejected key {}", SENTINEL.get(..12).unwrap_or_default());
    let server = Server::start(open_body(401, Some(10_000), partial.as_bytes()), true).await;
    let response = headers_of(&server).await;
    let cle = ApiKey::new(SENTINEL);

    let err = tokio::time::timeout(
        GUARD,
        failure_within(
            &ProviderId::openai(),
            response,
            Some(&cle),
            None,
            Duration::from_millis(50),
        ),
    )
    .await
    .expect("the delay bounds the read");
    let rendu = format!("{err} {err:?}");
    assert!(
        !rendu.contains(SENTINEL.get(..8).unwrap_or_default()),
        "{rendu}"
    );
    assert!(matches!(err, LlmError::Http { status: 401, .. }), "{rendu}");
}

#[tokio::test]
async fn a_model_list_announced_too_large_is_refused_by_name() {
    let reply = open_body(200, Some(MAX_JSON_BODY_BYTES + 1), b"{\"data\":[");
    let server = Server::start(reply, true).await;
    let provider =
        OpenAiCompatibleProvider::new(ProviderId::openai(), &format!("{}/v1", server.origin))
            .expect("provider");

    let err = refused(
        tokio::time::timeout(GUARD, provider.models())
            .await
            .expect("refused without reading"),
    );
    let rendu = err.to_string();
    assert!(rendu.contains("model list"), "{rendu}");
    assert!(rendu.contains(&MAX_JSON_BODY_BYTES.to_string()), "{rendu}");
}

#[tokio::test]
async fn a_model_list_without_length_is_refused_at_the_first_byte_too_many() {
    let mut liste = b"{\"data\":[".to_vec();
    liste.extend(std::iter::repeat_n(b' ', 4096));
    let server = Server::start(open_body(200, None, &liste), true).await;
    let response = headers_of(&server).await;

    let err = tokio::time::timeout(
        GUARD,
        read_json_within::<serde_json::Value>(
            &ProviderId::anthropic(),
            response,
            "model list",
            None,
            1024,
        ),
    )
    .await
    .expect("the bound stops the read")
    .expect_err("too large");
    assert!(
        matches!(&err, LlmError::Decode { detail, .. } if detail.contains("model list is larger than 1024 bytes")),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_anthropic_model_list_announced_too_large_is_refused_by_name() {
    let reply = open_body(200, Some(MAX_JSON_BODY_BYTES + 1), b"{");
    let server = Server::start(reply, true).await;
    let provider = AnthropicProvider::with_base_url(SENTINEL, &server.origin).expect("provider");

    let err = refused(
        tokio::time::timeout(GUARD, provider.models())
            .await
            .expect("refused without reading"),
    );
    assert!(err.to_string().contains("model list"), "{err}");
}

#[tokio::test]
async fn a_model_list_with_too_many_entries_is_refused_by_name() {
    // Minimal entries: the byte bound does not stop them.
    let mut liste = String::from("{\"data\":[");
    liste.push_str(&vec!["{}"; super::MAX_MODELS + 1].join(","));
    liste.push_str("]}");
    let server = Server::start(open_body(200, Some(liste.len()), liste.as_bytes()), false).await;
    let provider =
        OpenAiCompatibleProvider::new(ProviderId::openai(), &format!("{}/v1", server.origin))
            .expect("provider");

    let err = refused(provider.models().await);
    let rendu = err.to_string();
    assert!(rendu.contains(&super::MAX_MODELS.to_string()), "{rendu}");
    assert!(rendu.contains("model list"), "{rendu}");
}
