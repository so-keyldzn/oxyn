//! The error boundary of model providers.
//!
//! [`LlmError`] is the enumeration of this boundary, in the sense of
//! [`.claude/rules/rust.md`](../../../.claude/rules/rust.md): the caller must
//! be able to tell "the network gave out" from "the key is refused" without
//! parsing a string.
//!
//! It converts into [`OxynError`] at the crate's exit, because the scheduler
//! and the interface only know one error type. The conversion **preserves the
//! family** ([`ErrorClass`]): it is the family, not the message, that decides
//! a retry.
//!
//! Two rules govern this module:
//!
//! 1. **No response body arrives raw in an error.** It is truncated and rid of
//!    any literal occurrence of the key (I-03): some providers copy the
//!    received key into their message.
//! 2. **Only what never went out is transient.** The boundary is in the type,
//!    not in the message:
//!
//!    | Variant | What happened | Family |
//!    |---|---|---|
//!    | [`LlmError::Transport`] | the connection was not established — resolution, refusal, TLS handshake, **connection timeout**: nothing went out | transient |
//!    | [`LlmError::ResponseTimeout`] | the request went out, the **response timeout** expired | ambiguous |
//!    | [`LlmError::ConnectionLost`] | the request went out, the connection gave out before the response | ambiguous |
//!
//!    A provider billed per token may have produced — and billed — the
//!    response that was not received: replaying pays twice (I-13).
//!
//!    The two ambiguous variants **stay ambiguous in the domain**:
//!    [`LlmError::ResponseTimeout`] becomes [`OxynError::Timeout`], with the
//!    timeout actually configured; [`LlmError::ConnectionLost`] becomes
//!    [`OxynError::OutcomeUnknown`].
//!
//!    The response timeout is the idle bound of the HTTP client
//!    (`http::IDLE_TIMEOUT`): it bounds the wait for the headers, then each
//!    silence between two chunks, never the whole generation. A caller that
//!    passes no timeout gets `ConnectionLost`, ambiguous as well, rather than
//!    a `ResponseTimeout` with an invented duration. A silence **during** the
//!    stream is not an error of this type: like any cut, it ends the stream
//!    with `StopReason::Interrupted`.
//! 3. **No message from the network stack enters a transport error.**
//!    reqwest's message repeats the URL, which can carry an internal host or a
//!    sensitive parameter (I-03). The text describes the fact.

use std::time::Duration;

use oxyn_core::{ErrorClass, OxynError};

use crate::provider::ProviderId;
use crate::secret::{ApiKey, redact_key};

/// Maximum length of a response body quoted in an error message.
///
/// A provider can answer an HTML page of several kilobytes — that of a captive
/// portal or of a corporate proxy, typically. Copying it whole into an error
/// message fills the log and helps no one.
const MAX_MESSAGE_LEN: usize = 512;

/// Failure of an exchange with a model provider.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LlmError {
    /// The connection to the provider was not established: **nothing went
    /// out**. Transient family.
    ///
    /// An exceeded connection timeout is here, and only it: a timeout exceeded
    /// after sending is [`ResponseTimeout`](Self::ResponseTimeout). A cut during
    /// an already open stream is not an error at all: it ends the stream with
    /// `StopReason::Interrupted`.
    #[error("cannot reach provider `{provider}`: {detail}")]
    Transport {
        /// Target provider.
        provider: ProviderId,
        /// What the transport layer reports, without a response body.
        detail: String,
    },

    /// The request went out, and the response timeout expired. **Ambiguous**
    /// family: the provider may have processed, and billed, the request.
    #[error(
        "provider `{provider}` did not answer within {after:?}; the request was sent and may have been processed"
    )]
    ResponseTimeout {
        /// Target provider.
        provider: ProviderId,
        /// The response timeout **configured** on the client, never a measured
        /// or reconstructed duration.
        after: Duration,
    },

    /// The request went out, and the connection gave out before the response.
    /// **Ambiguous** family, for the same reason.
    #[error(
        "lost the connection to provider `{provider}` after sending the request, which may have been processed: {detail}"
    )]
    ConnectionLost {
        /// Target provider.
        provider: ProviderId,
        /// The observed fact, in a fixed sentence: never the network stack's
        /// message, which repeats the URL.
        detail: &'static str,
    },

    /// The provider answered, with a failure code.
    #[error("provider `{provider}` returned HTTP {status}: {message}")]
    Http {
        /// Target provider.
        provider: ProviderId,
        /// HTTP status code as received.
        status: u16,
        /// Body of the response, truncated and scrubbed of the key.
        message: String,
    },

    /// The provider refused the request as longer than the model's context
    /// window, and said so in a form it documents. Permanent family.
    ///
    /// Recognized per provider, from its own failure shape: a provider that
    /// documents no such signal never produces it, and its refusal stays an
    /// [`Http`](Self::Http) error like any other (RESEARCH-NOTES, « Context
    /// window overflow »).
    #[error(
        "provider `{provider}` refused the request as longer than the model's context window: {message}"
    )]
    ContextTooLong {
        /// Target provider.
        provider: ProviderId,
        /// Body of the response, truncated and scrubbed of the key.
        message: String,
    },

    /// The response arrived but cannot be read: malformed JSON, truncated SSE
    /// event, outsized line.
    #[error("unreadable response from provider `{provider}`: {detail}")]
    Decode {
        /// Target provider.
        provider: ProviderId,
        /// Nature of the decoding defect, never the faulty data.
        detail: String,
    },

    /// The provider requires a key and received none.
    ///
    /// Distinct from a `401`: here the fault is local, and the message must
    /// send the user to the configuration rather than to the provider.
    #[error("no API key configured for provider `{provider}`")]
    MissingApiKey {
        /// Target provider.
        provider: ProviderId,
    },

    /// The provider's configuration is invalid: unreadable base URL,
    /// deployment name containing a path separator, model missing from the
    /// request.
    #[error("invalid configuration for provider `{provider}`: {detail}")]
    Config {
        /// Target provider.
        provider: ProviderId,
        /// What is missing or malformed.
        detail: String,
    },

    /// The provider cannot do what it is asked.
    ///
    /// "Not being able to do something is an acceptable answer; letting people
    /// believe otherwise is not."
    #[error("provider `{provider}` does not support {capability}")]
    Unsupported {
        /// Target provider.
        provider: ProviderId,
        /// What is not available.
        capability: String,
    },

    /// The protocol is declared in Oxyn, but this exchange is not written
    /// there yet.
    ///
    /// Distinct from [`Unsupported`](Self::Unsupported), which says the
    /// **provider** cannot do it: here it is Oxyn that cannot yet, and the
    /// nuance is what keeps a user from looking for the defect at their
    /// provider.
    ///
    /// It exists so that an unfinished path **refuses** instead of panicking: a
    /// `todo!()` on a public trait method is a guaranteed panic the day someone
    /// plugs in the provider ([I-09](../../../CLAUDE.md#i-09)).
    #[error("Oxyn does not implement this exchange for provider `{provider}` yet: {operation}")]
    NotImplemented {
        /// Target provider.
        provider: ProviderId,
        /// The missing exchange, named from the caller's point of view.
        operation: String,
    },

    /// The exchange was interrupted on request, through the [`CancelToken`].
    ///
    /// [`CancelToken`]: oxyn_core::CancelToken
    #[error("model exchange cancelled")]
    Cancelled,
}

impl LlmError {
    /// Builds an HTTP error from a status and a raw body.
    ///
    /// The body is **truncated** to `MAX_MESSAGE_LEN` — named and not linked:
    /// the constant is private, and a reader of the public API could not follow
    /// it. It is also scrubbed of any literal occurrence of the key. It is the
    /// only constructor to use for a failure response: calling the variant
    /// directly bypasses the scrubbing.
    #[must_use]
    pub fn from_response(
        provider: ProviderId,
        status: u16,
        body: &str,
        key: Option<&ApiKey>,
    ) -> Self {
        Self::Http {
            provider,
            status,
            message: sanitize(body, key),
        }
    }

    /// Builds the error of a refused redirection.
    ///
    /// No redirection is followed (see `http`): the message says what to fix
    /// rather than copying `Location`, which can name an internal host. The
    /// `3xx` status falls into the permanent family: retrying would redirect
    /// again, it is the configuration that must change.
    #[must_use]
    pub fn redirect_refused(provider: ProviderId, status: u16) -> Self {
        Self::Http {
            provider,
            status,
            message: "the provider answered with a redirect; redirects are not followed — \
                      set the base URL to the final address"
                .to_owned(),
        }
    }

    /// Family of the error, in the sense of `DRIVER-CONTRACT` §4.
    ///
    /// The status table is the only retry rule of this crate, and it is
    /// tested:
    ///
    /// | Status | Family | Why |
    /// |---|---|---|
    /// | `408`, `429`, `500`, `502`, `503`, `529` | transient | overload or passing incident |
    /// | `504` | **ambiguous** | processing had started: the response may have been produced and billed |
    /// | `401`, `403`, `404`, other `4xx` | permanent | reconfigure, not retry |
    /// | other `5xx` | permanent | the provider refused, it did not falter |
    ///
    /// Sources and dates in RESEARCH-NOTES § "Replaying a `500`, `502` or
    /// `504`": `500` is documented as retryable by Anthropic and OpenAI; `502`
    /// is so nowhere and keeps its classification **without verification**;
    /// `504` is, at Anthropic, a timeout exceeded "while processing".
    ///
    /// `529` is not a standard status: Anthropic uses it for a passing overload
    /// of its service (`overloaded_error`), checked on 2026-09-16. Without this
    /// line it fell into "other `5xx`", hence permanent — and the interface
    /// offered "reconfigure" where "retry" is the only useful action.
    ///
    /// Outside HTTP statuses: see the table of transport errors at the head of
    /// the module.
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Transport { .. } => ErrorClass::Transient,
            Self::ResponseTimeout { .. } | Self::ConnectionLost { .. } => ErrorClass::Ambiguous,
            Self::Http { status, .. } => match status {
                408 | 429 | 500 | 502 | 503 | 529 => ErrorClass::Transient,
                504 => ErrorClass::Ambiguous,
                _ => ErrorClass::Permanent,
            },
            Self::ContextTooLong { .. }
            | Self::Decode { .. }
            | Self::MissingApiKey { .. }
            | Self::Config { .. }
            | Self::Unsupported { .. }
            // Permanent, and that is the point: retrying will not write the
            // missing code. The message must send to another provider.
            | Self::NotImplemented { .. }
            | Self::Cancelled => ErrorClass::Permanent,
        }
    }

    /// Can the exchange be replayed as is?
    ///
    /// Only the transient family answers `true` — and this crate never replays
    /// on its own: the retry policy belongs to the caller, which alone knows
    /// whether the user is still waiting.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.class().is_retryable()
    }

    /// Classifies an HTTP stack error raised **before** the response headers
    /// were received.
    ///
    /// `response_timeout` is the response timeout configured on the client
    /// that produced the error, `None` if there is none.
    ///
    /// The order of the tests is the rule: reqwest marks a connection timeout
    /// both `is_connect()` and `is_timeout()` — the timeout is set in the
    /// connector, which hyper-util classifies `Connect` (checked in the sources
    /// of reqwest 0.13.4, `src/connect.rs`). Testing the timeout first would
    /// classify as ambiguous what never went out.
    ///
    /// Everything that is neither a missed connection nor a request impossible
    /// to build is ambiguous: **when in doubt, the request went out**.
    pub(crate) fn from_transport(
        provider: ProviderId,
        err: &reqwest::Error,
        response_timeout: Option<Duration>,
    ) -> Self {
        if err.is_connect() {
            let detail = if err.is_timeout() {
                "connection timed out"
            } else {
                "connection refused or host unreachable"
            };
            return Self::Transport {
                provider,
                detail: detail.to_owned(),
            };
        }
        if err.is_builder() {
            return Self::Transport {
                provider,
                detail: "the request could not be built".to_owned(),
            };
        }
        if err.is_timeout() {
            return match response_timeout {
                Some(after) => Self::ResponseTimeout { provider, after },
                None => Self::ConnectionLost {
                    provider,
                    detail: "timed out waiting for the response",
                },
            };
        }
        let detail = if err.is_body() || err.is_decode() {
            "the provider interrupted the response"
        } else {
            "the connection closed before the response arrived"
        };
        Self::ConnectionLost { provider, detail }
    }
}

/// Labels the nature of a JSON parse error, **without** copying the faulty
/// data.
///
/// It is the only detail of a parse defect we accept to show: the faulty text
/// is a model output or a third party's response, and it can copy what was
/// sent (I-03). Shared by the providers: they all parse JSON coming from the
/// network, and a second table of labels would diverge.
pub(crate) fn classify_json_error(err: &serde_json::Error) -> &'static str {
    match err.classify() {
        serde_json::error::Category::Io => "I/O error",
        serde_json::error::Category::Syntax => "invalid JSON syntax",
        serde_json::error::Category::Data => "unexpected data type",
        serde_json::error::Category::Eof => "truncated JSON",
    }
}

/// Truncates and scrubs a response body before it becomes a message.
///
/// The truncation respects character boundaries: cutting in the middle of a
/// UTF-8 code unit would panic, and the body comes from the network (I-09).
pub(crate) fn sanitize(body: &str, key: Option<&ApiKey>) -> String {
    let scrubbed = redact_key(body.trim(), key);
    if scrubbed.len() <= MAX_MESSAGE_LEN {
        return scrubbed;
    }
    let mut end = MAX_MESSAGE_LEN;
    while end > 0 && !scrubbed.is_char_boundary(end) {
        end -= 1;
    }
    // `get` and not `[..end]`: the body comes from the network, and no slice
    // indexing must survive on this path (I-09).
    let mut truncated = scrubbed.get(..end).unwrap_or_default().to_owned();
    truncated.push_str(" […]");
    truncated
}

impl From<LlmError> for OxynError {
    /// Projects the provider error onto the domain's vocabulary.
    ///
    /// The projection is chosen to **keep the family**: `429` and `503` become
    /// [`OxynError::Connection`], the only transient variant the domain has,
    /// rather than [`OxynError::Query`] which would make them final. The
    /// "connection impossible" label is then a little broad, and that is the
    /// price: it is the retry decision that must stay right, not the wording.
    fn from(err: LlmError) -> Self {
        match err {
            LlmError::Cancelled => Self::Cancelled,
            LlmError::Transport { .. } => Self::Connection(err.to_string()),
            // Ambiguous on both sides of the boundary: see the module note.
            LlmError::ResponseTimeout { after, .. } => Self::Timeout { after },
            LlmError::ConnectionLost { .. } => Self::OutcomeUnknown(err.to_string()),
            LlmError::MissingApiKey { .. } => Self::Authentication(err.to_string()),
            LlmError::Config { .. } => Self::Config(err.to_string()),
            LlmError::ContextTooLong { .. } => Self::ContextTooLong(err.to_string()),
            LlmError::Decode { .. } => Self::Serialization(err.to_string()),
            LlmError::Unsupported { capability, .. } => Self::NotSupported { capability },
            // `NotSupported` and not `Internal`: for the caller, the fact is
            // the same — the capability is not there —, and the message already
            // says where the limit is.
            LlmError::NotImplemented { .. } => Self::NotSupported {
                capability: err.to_string(),
            },
            LlmError::Http { status, .. } => match status {
                401 | 403 => Self::Authentication(err.to_string()),
                // Read on the family and not on the status: the `class` table
                // remains the only rule, and the projection cannot contradict
                // it.
                _ if err.class() == ErrorClass::Ambiguous => Self::OutcomeUnknown(err.to_string()),
                _ if err.is_retryable() => Self::Connection(err.to_string()),
                _ => Self::Query(err.to_string()),
            },
        }
    }
}

#[cfg(test)]
mod tests;
