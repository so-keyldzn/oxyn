//! One statement, one HTTP request, one bounded answer.
//!
//! The token travels in the `Authorization` header, marked sensitive so that
//! `reqwest`'s own `Debug` does not print it, and never in the URL
//! ([I-03](../../../CLAUDE.md#i-03)). TLS is `rustls` on `aws-lc-rs` with the
//! system's trust store, always verified: there is no way to turn it off
//! ([ADR-0052](../../../docs/adr/0052-verified-tls-outside-local.md)).
//!
//! # The answer is read under a budget
//!
//! Hrana over HTTP sends the whole result in one JSON document, so the
//! driver cannot read it row by row. What it can do is refuse to hold more
//! than [`MAX_RESPONSE_BYTES`]: the body is read chunk by chunk, and the
//! request is dropped the moment the budget is crossed
//! ([I-06](../../../CLAUDE.md#i-06)). A `Content-Length` over the budget is
//! refused before the first byte is read.
//!
//! # A stop is a dropped request
//!
//! Hrana over HTTP has no call that interrupts a running statement, and Turso
//! documents none. Dropping the request closes the connection; whether the
//! server then stops the statement is its own business, and the driver does
//! not declare `SERVER_SIDE_CANCEL`. What it does promise is the verdict: a
//! read that was stopped is `Cancelled`, and a statement that may write is
//! `OutcomeUnknown`, never `Cancelled`
//! ([DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md#4-it-distinguishes-three-families-of-errors-and-classifies-them)).

use std::time::Duration;

use oxyn_core::{CancelToken, DriverId, OxynError, Result, StatementIntent};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{Client, Response, StatusCode};

use crate::error::{self, MAX_MESSAGE_BYTES, RESULT_TOO_LARGE};
use crate::options::ConnectSpec;
use crate::wire::{self, Answer, StmtResult};

/// The most a single answer may weigh, as bytes of JSON.
///
/// The parsed rows weigh several times the text: about 3× for ordinary cells,
/// up to 9× for a body of empty rows (a 24-byte `Vec` header for 3 bytes of
/// JSON), so at the limit the order is 100 to 300 MiB, for one statement the
/// user asked for. Raised by ADR-0069 only if Turso offers a streamed result.
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// How long opening the TCP and TLS connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The bound on a statement that names none. The executor's deadline is the
/// usual one; this one keeps a driver used on its own from waiting for ever.
const DEFAULT_BOUND: Duration = Duration::from_secs(300);

/// A Turso HTTP endpoint, with the token that talks to it.
#[derive(Debug, Clone)]
pub struct Http {
    client: Client,
    spec: ConnectSpec,
    max_response: usize,
}

impl Http {
    /// The client of a connection.
    ///
    /// `no_proxy`: a proxy chosen by the system would carry the token and the
    /// user's data through a host nobody configured for this connection. No
    /// redirect is followed: `reqwest` would resend the `Authorization`
    /// header to wherever it points.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the TLS stack cannot be built.
    pub fn new(spec: ConnectSpec) -> Result<Self> {
        // Outside the tests, `https` is not a setting.
        #[cfg(test)]
        let https_only = !spec.is_plaintext();
        #[cfg(not(test))]
        let https_only = true;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_keepalive(Duration::from_secs(30))
            .https_only(https_only)
            .build()
            .map_err(|_| OxynError::Config("the HTTP client cannot be built".to_owned()))?;
        Ok(Self {
            client,
            spec,
            max_response: MAX_RESPONSE_BYTES,
        })
    }

    /// The same client with another response budget, for the tests.
    #[cfg(test)]
    pub(crate) fn with_max_response(mut self, bytes: usize) -> Self {
        self.max_response = bytes;
        self
    }

    /// The connection parameters.
    #[must_use]
    pub const fn spec(&self) -> &ConnectSpec {
        &self.spec
    }

    /// Runs the pipeline `body` and returns the statement's result.
    ///
    /// `bound` is the statement's own deadline; `cancel` stops it at any
    /// point, which drops the request.
    ///
    /// # Errors
    /// [`OxynError::Cancelled`] if `cancel` fired before the request left, or
    /// during a statement that cannot have written; [`OxynError::OutcomeUnknown`]
    /// if it fired during one that may have; [`OxynError::Timeout`] past
    /// `bound`; otherwise the failure classified by [`crate::error`].
    pub(crate) async fn run(
        &self,
        driver: &DriverId,
        intent: StatementIntent,
        body: Vec<u8>,
        bound: Option<Duration>,
        cancel: &CancelToken,
    ) -> Result<StmtResult> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let bound = bound.unwrap_or(DEFAULT_BOUND);
        let exchange = tokio::time::timeout(bound, self.exchange(driver, intent, body));
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(abandoned(intent)),
            outcome = exchange => outcome.unwrap_or(Err(OxynError::Timeout { after: bound })),
        }
    }

    async fn exchange(
        &self,
        driver: &DriverId,
        intent: StatementIntent,
        body: Vec<u8>,
    ) -> Result<StmtResult> {
        let token = self.spec.token();
        let url = self
            .spec
            .endpoint()
            .join(wire::PIPELINE_PATH)
            .map_err(|_| OxynError::Internal("the pipeline URL cannot be joined".to_owned()))?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| OxynError::Config("the database token cannot be sent".to_owned()))?;
        authorization.set_sensitive(true);

        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, authorization)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|failure| error::transport(intent, failure, token))?;

        if response.status() != StatusCode::OK {
            let code = response.status().as_u16();
            // An error body is read to a bound: it may be a gateway's page.
            // A body that breaks here changes nothing: the status says enough.
            let body = read_body(response, MAX_MESSAGE_BYTES)
                .await
                .map(|(body, _)| body)
                .unwrap_or_default();
            let fault = wire::parse_http_fault(&body);
            return Err(error::status(driver, intent, code, fault.as_ref(), token));
        }
        if response
            .content_length()
            .is_some_and(|length| length > u64::try_from(self.max_response).unwrap_or(u64::MAX))
        {
            return Err(OxynError::Query(RESULT_TOO_LARGE.to_owned()));
        }
        let (body, overflowed) = read_body(response, self.max_response)
            .await
            .map_err(|failure| error::transport(intent, failure, token))?;
        if overflowed {
            return Err(OxynError::Query(RESULT_TOO_LARGE.to_owned()));
        }
        match wire::parse_answer(&body) {
            Ok(Answer::Result(result)) => Ok(result),
            Ok(Answer::Fault(fault)) => Err(error::statement(driver, intent, &fault, token)),
            Err(wire::Malformed) => Err(error::malformed(driver, intent)),
        }
    }
}

/// The verdict of a stop, which only a read can claim to have undone nothing.
fn abandoned(intent: StatementIntent) -> OxynError {
    if intent.is_read_only() {
        OxynError::Cancelled
    } else {
        OxynError::OutcomeUnknown(
            "the statement was stopped on Oxyn's side, but Turso has no way to interrupt a \
             running statement: it may have been applied; check the data before running it again"
                .to_owned(),
        )
    }
}

/// Reads at most `limit` bytes of a body. The flag says the body had more.
///
/// # Errors
/// The transport error that broke the body in flight: a connection reset 20 MB
/// into a read is a failed exchange, not a short document.
async fn read_body(
    mut response: Response,
    limit: usize,
) -> std::result::Result<(Vec<u8>, bool), reqwest::Error> {
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if out.len().saturating_add(chunk.len()) > limit {
            let room = limit.saturating_sub(out.len());
            out.extend_from_slice(chunk.get(..room).unwrap_or(&[]));
            return Ok((out, true));
        }
        out.extend_from_slice(&chunk);
    }
    Ok((out, false))
}
