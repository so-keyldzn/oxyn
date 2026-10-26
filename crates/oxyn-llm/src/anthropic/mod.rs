//! The Anthropic provider.
//!
//! # Why not an OpenAI-compatible adapter
//!
//! The `/v1/messages` protocol differs on four points that a thin translation
//! layer cannot make up for:
//!
//! 1. the system instruction is a **top-level field**, not a message;
//! 2. the content of a message is a **list of typed blocks**, not a string;
//! 3. a tool result is a `tool_result` block in a message of role `user`, not
//!    a `tool` role;
//! 4. the SSE stream is **named** (`content_block_delta`, `message_delta`…)
//!    and uses no end sentinel.
//!
//! An adapter that claimed to cover both protocols would be wrong on tool
//! calls, that is precisely where Oxyn needs them
//! ([`ARCHITECTURE` §7.5](../../../../docs/ARCHITECTURE.md)).
//!
//! # Every external fact here is dated and sourced
//!
//! Path, headers, API version, event names, field names, stop reason values,
//! status mapping: checked on **2026-09-16** in the official documentation and
//! recorded in [`RESEARCH-NOTES`](../../../../docs/RESEARCH-NOTES.md) §
//! "Anthropic provider" (I-12).
//!
//! # What is not done, and why
//!
//! * **No retry.** A transient error is reported as such and the caller
//!   decides (I-13). The documentation describes resuming an interrupted
//!   stream; it asks to send the partial response back to the model, which is
//!   a product decision, not a transport one.
//! * **No global timeout on the request.** A long generation is normal; a
//!   global timeout would kill it in the middle. Only the *connection* is
//!   bounded.
//! * **No server-side tool.** Oxyn offers none: its tools are bus `Command`s
//!   and nothing else (I-01).

mod decode;
mod wire;

use std::fmt;
use std::pin::pin;

use async_trait::async_trait;
use futures::future::{Either, select};
use futures::stream::{BoxStream, StreamExt};
use oxyn_core::{CancelToken, OxynError, Result};
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Client, RequestBuilder, Url};
use serde_json::Value;

use crate::error::LlmError;
use crate::http;
use crate::provider::{self, LlmProvider, ProviderId};
use crate::reach;
use crate::secret::ApiKey;
use crate::stream::{describe_stream_error, events_stream};
use crate::types::{ChatEvent, ChatRequest, ModelInfo};

/// Endpoint of Anthropic's API.
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// Path of the conversation endpoint.
pub const MESSAGES_PATH: &str = "v1/messages";

/// Path of the token counting endpoint.
pub const COUNT_TOKENS_PATH: &str = "v1/messages/count_tokens";

/// Path of the model list endpoint.
pub const MODELS_PATH: &str = "v1/models";

/// Header carrying the key. This protocol does not use `Authorization`.
pub const API_KEY_HEADER: &str = "x-api-key";

/// Value of the `anthropic-version` header.
///
/// The API refuses the versions it does not know. This value is the one the
/// documentation gives as an example on each of its pages, the most recent
/// included: it is a **contract** version, not a release date, and it does
/// not follow model releases.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Maximum number of models requested per page.
///
/// The documentation bounds this parameter to 1000 and defaults it to 20.
/// Twenty is not enough — the catalog has more —, and the maximum value avoids
/// a pagination that would bring nothing here.
const MODELS_PAGE_SIZE: u32 = 1000;

/// Maximum number of pages walked when listing models.
///
/// A hard bound rather than trust in `has_more`: a server that always
/// answered "there are more" would make the call loop forever.
const MAX_MODEL_PAGES: usize = 10;

/// Ceiling of produced tokens, when the caller sets none.
///
/// It is not an external value but an **Oxyn choice**: `/v1/messages` requires
/// `max_tokens`, and something has to be answered. The value is deliberately
/// modest — a cut response reports itself
/// ([`StopReason::is_truncated`](crate::types::StopReason::is_truncated)), an
/// invoice cannot be taken back.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Anthropic provider.
///
/// `Debug` written by hand: the key does not appear in it (I-03).
pub struct AnthropicProvider {
    base_url: Url,
    api_key: ApiKey,
    version: String,
    default_max_tokens: u32,
    client: Client,
}

impl AnthropicProvider {
    /// Builds the provider on the public endpoint.
    ///
    /// This unclassified transport performs no DNS measurement. For a
    /// privacy-gated request, use [`crate::build_provider`] with its measured reach.
    ///
    /// # Errors
    /// Unreadable base URL, or HTTP client impossible to build.
    pub fn new(api_key: impl Into<ApiKey>) -> Result<Self> {
        Self::with_base_url(api_key, ANTHROPIC_BASE_URL)
    }

    /// Builds the provider on a chosen endpoint (proxy, corporate gateway).
    ///
    /// # Errors
    /// Unreadable base URL, or HTTP client impossible to build.
    pub fn with_base_url(api_key: impl Into<ApiKey>, base_url: &str) -> Result<Self> {
        Ok(Self::with_base_url_and_reach(
            api_key,
            base_url,
            crate::Reach::Unresolved,
        )?)
    }

    pub(crate) fn with_base_url_and_reach(
        api_key: impl Into<ApiKey>,
        base_url: &str,
        reach: crate::Reach,
    ) -> std::result::Result<Self, LlmError> {
        let id = ProviderId::anthropic();
        let parsed = Url::parse(base_url).map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot parse the base URL: {err}"),
        })?;
        let client = http::client(&id, &parsed, reach)?;
        Ok(Self {
            base_url: provider::normalize_base_url(parsed),
            api_key: api_key.into(),
            version: ANTHROPIC_VERSION.to_owned(),
            default_max_tokens: DEFAULT_MAX_TOKENS,
            client,
        })
    }

    /// Sets the value of the `anthropic-version` header.
    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Sets the token ceiling used when the request carries none.
    #[must_use]
    pub const fn with_default_max_tokens(mut self, max_tokens: u32) -> Self {
        self.default_max_tokens = max_tokens;
        self
    }

    /// The headers every request must carry, **apart from** the key.
    ///
    /// Returned separately: the key is only set at sending time, and never
    /// copied elsewhere.
    #[must_use]
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("anthropic-version", self.version.clone()),
            ("content-type", "application/json".to_owned()),
        ]
    }

    /// Builds the body of `POST /v1/messages`.
    ///
    /// # Errors
    /// Request without a model, or without any message apart from the system
    /// instruction — the API refuses them, and failing here avoids a useless
    /// round trip.
    pub fn wire_request(&self, request: &ChatRequest) -> Result<Value> {
        self.build_body(request, true)
    }

    /// Builds a body, streamed or not.
    fn build_body(&self, request: &ChatRequest, stream: bool) -> Result<Value> {
        let invalid = |detail: &str| LlmError::Config {
            provider: ProviderId::anthropic(),
            detail: detail.to_owned(),
        };
        if request.model.trim().is_empty() {
            return Err(invalid("no model requested").into());
        }

        // Refuse rather than send a conversation from which a reasoning block
        // would have disappeared: the signature would no longer hold.
        let body = wire::build_request(request, self.default_max_tokens, stream).map_err(|_| {
            LlmError::Unsupported {
                provider: ProviderId::anthropic(),
                capability: "replaying this kind of reasoning block".to_owned(),
            }
        })?;
        let empty = body
            .get("messages")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty);
        if empty {
            return Err(invalid("no message to send").into());
        }
        Ok(body)
    }

    /// Prepares an HTTP request: URL, protocol headers, then the key.
    ///
    /// The key is only set here, and the header is marked sensitive: the HTTP
    /// stack will not render it in its traces (I-03). The error message never
    /// copies the faulty value.
    ///
    /// # Errors
    /// Key not representable in an HTTP header — a key pasted from a terminal
    /// often carries a line break.
    fn authorize(
        &self,
        mut builder: RequestBuilder,
    ) -> std::result::Result<RequestBuilder, LlmError> {
        for (header_name, value) in self.headers() {
            builder = builder.header(header_name, value);
        }
        if self.api_key.is_blank() {
            return Err(LlmError::MissingApiKey {
                provider: ProviderId::anthropic(),
            });
        }
        let mut key =
            HeaderValue::from_str(self.api_key.expose()).map_err(|_| LlmError::Config {
                provider: ProviderId::anthropic(),
                detail: "the API key contains a character that is not valid in an HTTP header"
                    .to_owned(),
            })?;
        key.set_sensitive(true);
        Ok(builder.header(HeaderName::from_static(API_KEY_HEADER), key))
    }

    /// Prepares the `POST /v1/messages` of a generation.
    ///
    /// # Errors
    /// URL that cannot be assembled, or key unusable in a header.
    fn prepared_request(&self) -> Result<RequestBuilder> {
        Ok(self.authorize(self.client.post(self.messages_url()?))?)
    }

    /// Assembles a relative path onto the base URL.
    fn join(&self, path: &str) -> std::result::Result<Url, LlmError> {
        self.base_url.join(path).map_err(|err| LlmError::Config {
            provider: ProviderId::anthropic(),
            detail: format!("cannot append path `{path}` to the base URL: {err}"),
        })
    }

    /// URL de `/v1/messages`.
    fn messages_url(&self) -> Result<Url> {
        Ok(self.join(MESSAGES_PATH)?)
    }

    /// Classifies a transport error, without ever copying the key.
    fn transport(&self, err: &reqwest::Error) -> LlmError {
        // The idle bound also covers the wait for the headers.
        LlmError::from_transport(
            ProviderId::anthropic(),
            err,
            Some(crate::http::IDLE_TIMEOUT),
        )
    }

    /// Turns a failure response into an error, body scrubbed.
    ///
    /// The status decides the retry; the body only serves display, and it is
    /// truncated and rid of the key by [`LlmError::from_response`] — except
    /// for the documented context overflow, which becomes
    /// [`LlmError::ContextTooLong`]. It is read on that scrubbed body: a body
    /// cut past the display bound no longer parses, and stays an ordinary
    /// refusal.
    async fn failure(&self, response: reqwest::Response, cancel: Option<&CancelToken>) -> LlmError {
        match http::failure(
            &ProviderId::anthropic(),
            response,
            Some(&self.api_key),
            cancel,
        )
        .await
        {
            LlmError::Http {
                provider,
                status,
                message,
            } if wire::is_context_overflow(status, &message) => {
                LlmError::ContextTooLong { provider, message }
            }
            other => other,
        }
    }

    /// Sends a request and returns its response, yielding to cancellation.
    ///
    /// The sending itself must yield: an endpoint that does not answer would
    /// otherwise leave the user in front of a "Cancel" button with no effect.
    async fn send(
        &self,
        builder: RequestBuilder,
        cancel: &CancelToken,
    ) -> Result<reqwest::Response> {
        // `std::pin::pin!` and not `futures::pin_mut!`: the standard library's
        // pinning introduces no `unsafe` block in this crate, where it is
        // refused.
        let send = pin!(builder.send());
        let pending = pin!(cancel.cancelled());
        let response = match select(pending, send).await {
            Either::Left(((), _)) => return Err(OxynError::Cancelled),
            Either::Right((result, _)) => result.map_err(|err| self.transport(&err))?,
        };
        if !response.status().is_success() {
            // Under the same token as the sending: an error body that never
            // ends must not make "Cancel" ineffective.
            return Err(self.failure(response, Some(cancel)).await.into());
        }
        Ok(response)
    }

    /// Reads a JSON body under a bound, classifying a decoding defect.
    ///
    /// Without a token: the calls that use it receive no cancellation from the
    /// trait. Size and timeout remain bounded.
    async fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
        subject: &str,
    ) -> Result<T> {
        Ok(http::read_json(&ProviderId::anthropic(), response, subject, None).await?)
    }

    /// The model list, every page walked under a bound, failures typed.
    ///
    /// Typed rather than projected onto [`oxyn_core::OxynError`]: listing models
    /// tells a refused key (`401`) from a key without the right (`403`), which
    /// the projection merges.
    pub(crate) async fn fetch_models(&self) -> std::result::Result<Vec<ModelInfo>, LlmError> {
        let mut infos = Vec::new();
        let mut after: Option<String> = None;

        for _ in 0..MAX_MODEL_PAGES {
            let mut url = self.join(MODELS_PATH)?;
            url.query_pairs_mut()
                .append_pair("limit", &MODELS_PAGE_SIZE.to_string());
            if let Some(cursor) = &after {
                url.query_pairs_mut().append_pair("after_id", cursor);
            }

            let request = self.authorize(self.client.get(url))?;
            // No cancellation here: listing models is a short request, and the
            // trait passes no token.
            let response = request.send().await.map_err(|err| self.transport(&err))?;
            if !response.status().is_success() {
                return Err(self.failure(response, None).await);
            }

            let raw: wire::ModelsResponse =
                http::read_json(&ProviderId::anthropic(), response, "model list", None).await?;
            let more = raw.has_more;
            let last = raw.last_id.clone();
            infos.extend(wire::parse_models(raw));
            // Cumulative bound: each page is bounded, not their sum.
            if infos.len() > http::MAX_MODELS {
                return Err(http::too_many_models(&ProviderId::anthropic()));
            }

            // `last_id` missing while there would be more: stop rather than ask
            // for the same page again forever.
            match (more, last) {
                (true, Some(cursor)) => after = Some(cursor),
                _ => break,
            }
        }

        Ok(infos)
    }
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &reach::redacted(&self.base_url))
            .field("api_key", &"<present>")
            .field("version", &self.version)
            .field("default_max_tokens", &self.default_max_tokens)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> ProviderId {
        ProviderId::anthropic()
    }

    fn endpoint(&self) -> Option<&Url> {
        Some(&self.base_url)
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.fetch_models().await?)
    }

    async fn count_tokens(&self, request: &ChatRequest) -> Result<Option<u32>> {
        // `stream` is refused by this endpoint: the body is therefore built
        // without it, and without `max_tokens` which it does not expect either.
        let body = self.build_body(request, false)?;
        let url = self.join(COUNT_TOKENS_PATH)?;
        let http_request = self.authorize(self.client.post(url))?.json(&body);

        let response = http_request
            .send()
            .await
            .map_err(|err| self.transport(&err))?;
        if !response.status().is_success() {
            return Err(self.failure(response, None).await.into());
        }
        let raw: wire::CountResponse = self.read_json(response, "token count").await?;
        Ok(Some(raw.count()))
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        // The request is built and validated **before** any network call: a
        // malformed request must report itself as such rather than come back
        // as a `400` a few hundred milliseconds later.
        let body = self.wire_request(&request)?;
        let http_request = self.prepared_request()?.json(&body);
        let response = self.send(http_request, cancel).await?;

        let bytes = response
            .bytes_stream()
            .map(|result| result.map_err(|err| describe_stream_error(&err)));
        Ok(events_stream(
            Box::pin(bytes),
            decode::MessageDecoder::new(),
            cancel.clone(),
            Some(self.api_key.clone()),
        ))
    }
}

#[cfg(test)]
mod tests;
