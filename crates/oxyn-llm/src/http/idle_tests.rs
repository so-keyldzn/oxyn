//! A provider that goes silent: the question ends, classified, never replayed.
//!
//! The production bound is minutes; these tests rebuild the same client with
//! a bound of a few milliseconds. What is checked is that the bound exists on
//! the client every provider uses, and what its expiry becomes.

use std::sync::Arc;
use std::time::Duration;

use oxyn_core::ErrorClass;

use super::loopback::Server;
use super::{IDLE_TIMEOUT, builder, dns};
use crate::Reach;
use crate::error::LlmError;
use crate::provider::ProviderId;
use crate::stream::describe_stream_error;

/// Bound used in place of [`IDLE_TIMEOUT`].
const SHORT: Duration = Duration::from_millis(150);

/// Safety bound of the tests: if it expires, the read is still waiting — the
/// defect these tests close.
const GUARD: Duration = Duration::from_secs(5);

/// The production builder, with the short bound.
fn short_client() -> reqwest::Client {
    builder(Reach::Local, Arc::new(dns::SystemResolver))
        .read_timeout(SHORT)
        .build()
        .expect("client")
}

#[test]
fn every_provider_client_carries_the_idle_bound() {
    let url = reqwest::Url::parse("http://127.0.0.1:1/").expect("test URL");
    let client = super::client(&ProviderId::openai(), &url, Reach::Local).expect("client");
    let rendered = format!("{client:?}");
    assert!(
        rendered.contains(&format!("read_timeout: {IDLE_TIMEOUT:?}")),
        "{rendered}"
    );
}

#[tokio::test]
async fn a_stream_that_goes_silent_ends_as_a_timeout() {
    // Headers, one frame, then a connection that stays open and says nothing:
    // a half-open socket after the laptop slept looks exactly like this.
    let server = Server::start(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\ndata: a\n\n".to_vec(),
        true,
    )
    .await;
    let mut response = short_client()
        .post(&server.origin)
        .send()
        .await
        .expect("headers arrive");

    let first = response.chunk().await.expect("the first frame arrives");
    assert_eq!(first.as_deref(), Some(&b"data: a\n\n"[..]));
    let silence = tokio::time::timeout(GUARD, response.chunk())
        .await
        .expect("the idle bound ends the read")
        .expect_err("silence is an error, not an end");
    assert_eq!(
        describe_stream_error(&silence),
        "timed out while receiving the stream"
    );
}

#[tokio::test]
async fn headers_that_never_come_end_as_an_ambiguous_timeout() {
    // The request is read, nothing is answered: it went out, and may have
    // been processed — and billed.
    let server = Server::start(Vec::new(), true).await;
    let err = tokio::time::timeout(
        GUARD,
        short_client().post(&server.origin).body("prompt").send(),
    )
    .await
    .expect("the idle bound ends the wait")
    .expect_err("no headers");

    let classified = LlmError::from_transport(ProviderId::openai(), &err, Some(SHORT));
    assert!(
        matches!(classified, LlmError::ResponseTimeout { after, .. } if after == SHORT),
        "{classified:?}"
    );
    assert_eq!(classified.class(), ErrorClass::Ambiguous);
    assert!(!classified.is_retryable(), "never replayed (I-13)");
    assert_eq!(server.hits(), 1, "the request did go out");
}
