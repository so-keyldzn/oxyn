//! The HTTP transport shared by the three provider families.
//!
//! A single place builds the client and reads a failure response: these are
//! the two moves where an omission does not show, and three copies always end
//! up diverging.
//!
//! # No redirection is followed
//!
//! A provider's reach ([`Reach`](crate::Reach)) is measured on the host of its
//! base URL, before sending, and it is that reach that the connection's tier
//! allows or refuses (I-04). A redirection followed by the HTTP stack would
//! send the body — the prompt, hence the database context — to another origin
//! **without a new check**, and the `ai_egress` trace would keep the reach of
//! the first one. A `307` or a `308` keeps the method and the body: that is
//! exactly what RFC 9110 asks of them.
//!
//! The key would follow too: the HTTP stack removes `Authorization` from one
//! origin to another, but does not know `x-api-key`, `api-key` or
//! `x-goog-api-key`, which would go out as they are (I-03).
//!
//! Revalidating each hop would have required a DNS resolution in the
//! redirection policy, which is synchronous, and a second classification to
//! keep consistent with the first. Refusing is simpler and fits in one
//! sentence: a `3xx` becomes an error that asks to point the base URL at the
//! final address.
//!
//! # No body is read without a bound
//!
//! Outside a stream, a body is read whole before being parsed: an error body
//! for its diagnostic, a list of models, a count. Each is read **in chunks**,
//! under three bounds at once — a size, checked on the announced length before
//! any reading then at each chunk; a timeout; and the cancellation token when
//! the caller holds one. A provider that sends a failure status then a body
//! that never ends therefore holds neither memory nor the "Cancel" button.

use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::future::{Either, select};
use oxyn_core::CancelToken;
use reqwest::{Client, ClientBuilder, Response, Url, redirect};
use serde::Deserializer;
use serde::de::{DeserializeOwned, SeqAccess, Visitor};
use serde_json::Value;

use crate::error::{LlmError, classify_json_error};
use crate::provider::ProviderId;
use crate::reach::{Reach, literal_reach};
use crate::secret::ApiKey;

mod dns;

/// Timeout for establishing the TCP and TLS connection.
///
/// Bounds **only** the connection setup: a generation can last minutes, and
/// bounding it globally would cut long responses.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest silence accepted from a provider once the request went out: until
/// the response headers, then between two body chunks.
///
/// There is still no total bound — a long answer that keeps arriving is never
/// cut. This one ends what never arrives: a half-open connection after the
/// laptop slept or the Wi-Fi changed, where no reset ever comes, and the
/// question would otherwise wait forever.
///
/// No provider documents how often it sends something while the model thinks
/// silently: Anthropic's `ping` comes "in any number", llama.cpp sends `:`
/// comments, OpenAI documents nothing
/// ([RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md#idle-streams--checked-on-2026-10-04)).
/// The value is therefore Oxyn's, chosen long — a reasoning model thinking
/// for minutes must not be cut —, since "Stop" stays the fast way out
/// ([AI-PROVIDERS](../../../docs/AI-PROVIDERS.md#a-silent-provider)). Its
/// expiry is an ambiguous failure: the request went out and may have been
/// billed, it is never replayed (I-13).
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// `User-Agent` header sent to every provider.
const USER_AGENT: &str = concat!("oxyn/", env!("CARGO_PKG_VERSION"));

/// Builds a provider's HTTP client.
///
/// It is the **only** client constructor of this crate: a provider that built
/// another one would get back the default redirection policy.
///
/// # Errors
/// [`LlmError::Config`] if the TLS stack does not initialize or a local
/// measurement is paired with a literal non-loopback address.
pub(crate) fn client(id: &ProviderId, url: &Url, reach: Reach) -> Result<Client, LlmError> {
    // Literal addresses bypass reqwest's resolver entirely.
    if reach == Reach::Local && literal_reach(url) == Some(Reach::Remote) {
        return Err(LlmError::Config {
            provider: id.clone(),
            detail: "a local provider cannot connect to a non-loopback address".to_owned(),
        });
    }
    builder(reach, Arc::new(dns::SystemResolver))
        .build()
        .map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot build the HTTP client: {err}"),
        })
}

fn builder(reach: Reach, resolver: Arc<dyn reqwest::dns::Resolve>) -> ClientBuilder {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        // Reset by every chunk received, `ping` frames included: it bounds a
        // silence, never a generation.
        .read_timeout(IDLE_TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(redirect::Policy::none())
        // A proxy would change the recipient without changing the recorded reach.
        .no_proxy()
        .dns_resolver(Arc::new(dns::ReachResolver { reach, resolver }))
}

/// Turns a failure response into an error, body scrubbed.
///
/// The status decides the retry; the body only serves display. A redirection
/// has no useful body: its message says what to fix, and does not copy the
/// `Location` header, which can name an internal host.
///
/// The body is read under bounds ([`MAX_ERROR_BODY_BYTES`],
/// [`ERROR_BODY_TIMEOUT`], `cancel`). Truncated, expired or broken, it does
/// not hide the status: the error keeps its class, with the diagnostic
/// received so far. Only cancellation changes the outcome, into
/// [`LlmError::Cancelled`]: the user asked for it to stop, not for a diagnostic.
pub(crate) async fn failure(
    id: &ProviderId,
    response: Response,
    key: Option<&ApiKey>,
    cancel: Option<&CancelToken>,
) -> LlmError {
    failure_within(id, response, key, cancel, ERROR_BODY_TIMEOUT).await
}

/// [`failure`], under a chosen timeout. Separate so that the tests exercise
/// the timeout without waiting ten seconds for it.
async fn failure_within(
    id: &ProviderId,
    response: Response,
    key: Option<&ApiKey>,
    cancel: Option<&CancelToken>,
    within: Duration,
) -> LlmError {
    let statut = response.status();
    if statut.is_redirection() {
        return LlmError::redirect_refused(id.clone(), statut.as_u16());
    }
    let body = match read_limited(response, MAX_ERROR_BODY_BYTES, within, cancel).await {
        Read::Cancelled => return LlmError::Cancelled,
        Read::Complete(body) => body,
        Read::Overflow(mut body) | Read::TimedOut(mut body) | Read::Broken(mut body) => {
            // A cut body can be cut in the middle of a copy of the key: its
            // beginning would escape the scrubbing, which only looks for the
            // whole key. Removing as many bytes as the key has closes this
            // case, at the cost of a few bytes of an already incomplete diagnostic.
            let margin = key.map_or(0, ApiKey::len);
            body.truncate(body.len().saturating_sub(margin));
            body
        }
    };
    let body = String::from_utf8_lossy(&body);
    LlmError::from_response(id.clone(), statut.as_u16(), &body, key)
}

/// Reads and parses a non-stream JSON body, under [`MAX_JSON_BODY_BYTES`] and
/// [`JSON_BODY_TIMEOUT`].
///
/// `subject` names what is being read ("model list"), so that the refusal says
/// what exceeded the limit. A body too large is **refused**, never truncated
/// then parsed: a cut list of models would read as a shorter complete list.
///
/// # Errors
/// [`LlmError::Decode`] for a body too large or unreadable,
/// [`LlmError::ConnectionLost`] for an expired or broken body — the request
/// went out —, [`LlmError::Cancelled`] on cancellation.
pub(crate) async fn read_json<T: DeserializeOwned>(
    id: &ProviderId,
    response: Response,
    subject: &str,
    cancel: Option<&CancelToken>,
) -> Result<T, LlmError> {
    read_json_within(id, response, subject, cancel, MAX_JSON_BODY_BYTES).await
}

/// [`read_json`], under a chosen size. Separate for the tests.
async fn read_json_within<T: DeserializeOwned>(
    id: &ProviderId,
    response: Response,
    subject: &str,
    cancel: Option<&CancelToken>,
    limit: usize,
) -> Result<T, LlmError> {
    let too_large = || LlmError::Decode {
        provider: id.clone(),
        detail: format!("the {subject} is larger than {limit} bytes; Oxyn refuses to read it"),
    };
    // An announced length beyond the limit is enough: nothing is read.
    if response
        .content_length()
        .is_some_and(|announced| usize::try_from(announced).map_or(true, |n| n > limit))
    {
        return Err(too_large());
    }
    let body = match read_limited(response, limit, JSON_BODY_TIMEOUT, cancel).await {
        Read::Complete(body) => body,
        Read::Overflow(_) => return Err(too_large()),
        Read::TimedOut(_) => {
            return Err(LlmError::ConnectionLost {
                provider: id.clone(),
                detail: "the response body did not arrive in time",
            });
        }
        Read::Broken(_) => {
            return Err(LlmError::ConnectionLost {
                provider: id.clone(),
                detail: "the provider interrupted the response",
            });
        }
        Read::Cancelled => return Err(LlmError::Cancelled),
    };
    serde_json::from_slice(&body).map_err(|err| {
        if err.to_string().starts_with(TOO_MANY_ENTRIES) {
            return too_many_models(id);
        }
        LlmError::Decode {
            provider: id.clone(),
            detail: format!("cannot read the {subject} ({})", classify_json_error(&err)),
        }
    })
}

/// Bytes read, at most, from a failure response body.
///
/// The displayed message keeps only its beginning (`error::MAX_MESSAGE_LEN`);
/// reading beyond would only fill memory. Sixteen kibibytes leave a proxy's
/// error page room to reach its useful text.
const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;

/// Time left for the body of a failure response, headers received.
///
/// The status is already known, and it is what classifies the error: waiting
/// longer for a diagnostic would not change the decision, only the time spent
/// in front of a "Cancel" button.
const ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(10);

/// Bytes read, at most, from a non-stream JSON response: model list, token
/// count.
///
/// A model list is metadata — names and a few capabilities per model. Sixteen
/// mebibytes are far above what a model picker can present; beyond that, the
/// response is refused rather than read.
pub(crate) const MAX_JSON_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Entries of a model list, at most — per response, and in total when the
/// list is paginated.
///
/// The byte bound is not enough: a minimal entry is about ten bytes, and each
/// becomes a JSON value then a much heavier record. Five thousand models is
/// already more than a picker can present; beyond that, the list is refused.
pub(crate) const MAX_MODELS: usize = 5000;

/// Prefix of the refusal raised during parsing, recognized by [`read_json`]
/// to turn it into an error that names the limit. It is ours: never server
/// data.
const TOO_MANY_ENTRIES: &str = "oxyn: too many entries";

/// Deserializes a list, refusing its entry number [`MAX_MODELS`] + 1
/// **before** allocating it.
///
/// # Errors
/// A deserialization error beyond [`MAX_MODELS`] entries.
pub(crate) fn bounded_entries<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Value>, D::Error> {
    struct Bounded;
    impl<'de> Visitor<'de> for Bounded {
        type Value = Vec<Value>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "a list of at most {MAX_MODELS} entries")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = seq.next_element::<Value>()? {
                if entries.len() >= MAX_MODELS {
                    return Err(serde::de::Error::custom(TOO_MANY_ENTRIES));
                }
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_seq(Bounded)
}

/// The explicit refusal of a model list that is too long.
pub(crate) fn too_many_models(id: &ProviderId) -> LlmError {
    LlmError::Decode {
        provider: id.clone(),
        detail: format!(
            "the model list has more than {MAX_MODELS} entries; Oxyn refuses to read it"
        ),
    }
}

/// Time left for the body of a JSON response, headers received.
///
/// These requests are short and the trait passes them no cancellation token:
/// without a bound, a server that does not finish its body would hold the call
/// indefinitely.
const JSON_BODY_TIMEOUT: Duration = Duration::from_secs(30);

/// What a bounded read obtained.
enum Read {
    /// The whole body, under the limit.
    Complete(Vec<u8>),
    /// The limit is reached: what came before, never more.
    Overflow(Vec<u8>),
    /// The timeout expired: what had arrived.
    TimedOut(Vec<u8>),
    /// The transport broke: what had arrived.
    Broken(Vec<u8>),
    /// The caller cancelled.
    Cancelled,
}

/// Outcome of waiting for a chunk.
enum Step {
    Cancelled,
    TimedOut,
    Chunk(reqwest::Result<Option<Bytes>>),
}

/// Reads a body without ever holding more than `limit` bytes, yielding to
/// cancellation and to the timeout.
///
/// The reading stops at the first byte too many, whatever the announced
/// length; the buffer is never reserved beyond the limit. Refusing a body on
/// its announcement alone is the caller's business: an error diagnostic
/// announced as long is still read up to the limit.
async fn read_limited(
    mut response: Response,
    limit: usize,
    within: Duration,
    cancel: Option<&CancelToken>,
) -> Read {
    let announced = response
        .content_length()
        .and_then(|n| usize::try_from(n).ok());
    let mut read = Vec::with_capacity(announced.unwrap_or(0).min(limit));
    let mut deadline = pin!(tokio::time::sleep(within));
    // A fresh token, never cancelled, when the caller has none: a single loop,
    // instead of two that would diverge.
    let never = CancelToken::new();
    let cancel = cancel.unwrap_or(&never);
    loop {
        if cancel.is_cancelled() {
            return Read::Cancelled;
        }
        let step = {
            let pending = pin!(cancel.cancelled());
            let chunk = pin!(response.chunk());
            match select(pending, select(deadline.as_mut(), chunk)).await {
                Either::Left(((), _)) => Step::Cancelled,
                Either::Right((Either::Left(((), _)), _)) => Step::TimedOut,
                Either::Right((Either::Right((chunk, _)), _)) => Step::Chunk(chunk),
            }
        };
        match step {
            Step::Cancelled => return Read::Cancelled,
            Step::TimedOut => return Read::TimedOut(read),
            Step::Chunk(Err(_)) => return Read::Broken(read),
            Step::Chunk(Ok(None)) => return Read::Complete(read),
            Step::Chunk(Ok(Some(chunk))) => {
                let place = limit.saturating_sub(read.len());
                if chunk.len() > place {
                    read.extend_from_slice(chunk.get(..place).unwrap_or_default());
                    return Read::Overflow(read);
                }
                read.extend_from_slice(&chunk);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod loopback;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod reach_tests;

#[cfg(test)]
mod idle_tests;
