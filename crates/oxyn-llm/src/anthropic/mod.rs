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
        let id = ProviderId::anthropic();
        let parsed = Url::parse(base_url).map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot parse the base URL: {err}"),
        })?;
        let client = http::client(&id)?;
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
        // No response timeout is configured (see `CONNECT_TIMEOUT`).
        LlmError::from_transport(ProviderId::anthropic(), err, None)
    }

    /// Turns a failure response into an error, body scrubbed.
    ///
    /// The status decides the retry; the body only serves display, and it is
    /// truncated and rid of the key by [`LlmError::from_response`].
    async fn failure(&self, response: reqwest::Response, cancel: Option<&CancelToken>) -> LlmError {
        http::failure(
            &ProviderId::anthropic(),
            response,
            Some(&self.api_key),
            cancel,
        )
        .await
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
                return Err(self.failure(response, None).await.into());
            }

            let raw: wire::ModelsResponse = self.read_json(response, "model list").await?;
            let more = raw.has_more;
            let last = raw.last_id.clone();
            infos.extend(wire::parse_models(raw));
            // Cumulative bound: each page is bounded, not their sum.
            if infos.len() > http::MAX_MODELS {
                return Err(http::too_many_models(&ProviderId::anthropic()).into());
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
mod tests {
    use oxyn_core::OxynError;
    use serde_json::json;

    use super::*;
    use crate::reasoning::{ReasoningBlock, ReasoningEffort};
    use crate::types::{ChatMessage, ToolCall, ToolSpec};

    fn provider() -> AnthropicProvider {
        AnthropicProvider::new("sk-ant-test").expect("construction")
    }

    fn body(request: &ChatRequest) -> Value {
        provider().wire_request(request).expect("valid request")
    }

    // ── System instruction, blocks, alternation ────────────────────────────

    #[test]
    fn the_system_instruction_leaves_the_messages() {
        // The point that distinguishes this protocol: `system` is a field, not
        // a message.
        let request = ChatRequest::new(
            "claude-model",
            vec![
                ChatMessage::system("you are a SQL assistant"),
                ChatMessage::user("hello"),
            ],
        );
        let body = body(&request);

        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["text"], "you are a SQL assistant");
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn several_system_instructions_are_concatenated_not_lost() {
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::system("rule 1"),
                ChatMessage::system("rule 2"),
                ChatMessage::user("go"),
            ],
        );
        assert_eq!(body(&request)["system"][0]["text"], "rule 1\n\nrule 2");
    }

    #[test]
    fn the_content_is_a_list_of_blocks() {
        let request = ChatRequest::new("m", vec![ChatMessage::user("hello")]);
        let body = body(&request);
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
    }

    #[test]
    fn a_tool_call_becomes_a_tool_use_block() {
        let call = ToolCall::new("call_1", "execute", json!({"sql": "SELECT 1"}));
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("count the rows"),
                ChatMessage::assistant("").with_tool_calls(vec![call]),
            ],
        );
        let block = body(&request)["messages"][1]["content"][0].clone();
        assert_eq!(block["type"], "tool_use");
        assert_eq!(block["id"], "call_1");
        assert_eq!(block["name"], "execute");
        assert_eq!(
            block["input"],
            json!({"sql": "SELECT 1"}),
            "the input is an object, not a string: this protocol differs from OpenAI"
        );
    }

    #[test]
    fn a_tool_result_becomes_a_user_block() {
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("count"),
                ChatMessage::assistant("").with_tool_calls(vec![ToolCall::new(
                    "call_1",
                    "execute",
                    json!({}),
                )]),
                ChatMessage::tool_result("call_1", "42"),
            ],
        );
        let last = body(&request)["messages"][2].clone();
        assert_eq!(last["role"], "user", "this protocol has no `tool` role");
        assert_eq!(last["content"][0]["type"], "tool_result");
        assert_eq!(last["content"][0]["tool_use_id"], "call_1");
        assert_eq!(last["content"][0]["content"], "42");
    }

    #[test]
    fn two_consecutive_messages_of_the_same_role_are_merged() {
        // The API requires alternation; two successive tool results are the
        // common case when the model asked for several.
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("a"),
                ChatMessage::tool_result("call_1", "r1"),
                ChatMessage::tool_result("call_2", "r2"),
            ],
        );
        let body = body(&request);
        let messages = body["messages"].as_array().expect("array");
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert_eq!(messages[0]["content"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn tools_use_input_schema() {
        let request =
            ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![ToolSpec::new(
                "lister",
                "list the tables",
                json!({"type": "object"}),
            )]);
        let body = body(&request);
        assert_eq!(body["tools"][0]["name"], "lister");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert!(
            body["tools"][0].get("parameters").is_none(),
            "`parameters` is OpenAI's name, not this one's"
        );
    }

    #[test]
    fn the_token_ceiling_is_always_present() {
        // `max_tokens` is mandatory in this protocol.
        let without_limit = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        assert_eq!(
            body(&without_limit)["max_tokens"],
            json!(DEFAULT_MAX_TOKENS)
        );

        let with_limit = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_max_tokens(128);
        assert_eq!(body(&with_limit)["max_tokens"], json!(128));
    }

    #[test]
    fn a_request_without_model_or_message_is_refused() {
        let f = provider();
        assert!(
            f.wire_request(&ChatRequest::new("", vec![ChatMessage::user("a")]))
                .is_err()
        );
        assert!(
            f.wire_request(&ChatRequest::new("m", vec![ChatMessage::system("seule")]))
                .is_err(),
            "a system instruction alone does not make a conversation"
        );
    }

    // ── Reasoning ──────────────────────────────────────────────────────────

    #[test]
    fn effort_goes_through_output_config_and_leaves_the_thinking_mode_alone() {
        // Sending a thinking mode that was not requested would make the request
        // fail on models that do not know it.
        let request = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_reasoning_effort(ReasoningEffort::XHigh);
        let body = body(&request);
        assert_eq!(body["output_config"]["effort"], "xhigh");
        assert!(body.get("thinking").is_none(), "{body}");
    }

    #[test]
    fn a_thinking_budget_requires_the_explicit_mode() {
        let request =
            ChatRequest::new("m", vec![ChatMessage::user("a")]).with_reasoning_budget_tokens(8192);
        let body = body(&request);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 8192);
        assert_eq!(
            body["thinking"]["display"], "summarized",
            "without it the reasoning comes back empty"
        );
    }

    #[test]
    fn both_settings_coexist() {
        let request = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_reasoning_effort(ReasoningEffort::Low)
            .with_reasoning_budget_tokens(1024);
        let body = body(&request);
        assert_eq!(body["output_config"]["effort"], "low");
        assert_eq!(body["thinking"]["budget_tokens"], 1024);
    }

    #[test]
    fn a_request_without_reasoning_request_carries_no_trace_of_it() {
        let body = body(&ChatRequest::new("m", vec![ChatMessage::user("a")]));
        assert!(body.get("thinking").is_none(), "{body}");
        assert!(body.get("output_config").is_none(), "{body}");
    }

    #[test]
    fn reasoning_blocks_go_back_first_and_intact() {
        // The API checks their signature: reordering them, editing them or
        // losing one makes the request refused.
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("count"),
                ChatMessage::assistant("here it is")
                    .with_reasoning(vec![
                        ReasoningBlock::summarized("I am thinking", Some("SIG".to_owned())),
                        ReasoningBlock::redacted("CHIFFRE"),
                    ])
                    .with_tool_calls(vec![ToolCall::new("c1", "execute", json!({}))]),
            ],
        );
        let content = body(&request)["messages"][1]["content"].clone();
        let blocks = content.as_array().expect("array");

        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[0]["thinking"], "I am thinking");
        assert_eq!(blocks[0]["signature"], "SIG");
        assert_eq!(blocks[1]["type"], "redacted_thinking");
        assert_eq!(blocks[1]["data"], "CHIFFRE");
        assert_eq!(blocks[2]["type"], "text", "reasoning comes first");
        assert_eq!(blocks[3]["type"], "tool_use");
    }

    #[test]
    fn a_reasoning_block_without_signature_does_not_carry_the_field() {
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("a"),
                ChatMessage::assistant("b")
                    .with_reasoning(vec![ReasoningBlock::summarized("t", None)]),
            ],
        );
        let block = body(&request)["messages"][1]["content"][0].clone();
        assert!(
            block.get("signature").is_none(),
            "an empty field is not an absence: {block}"
        );
    }

    // ── Cache de prompt ────────────────────────────────────────────────────

    #[test]
    fn a_marked_message_carries_the_cache_marker() {
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::system("the full schema").cached(),
                ChatMessage::user("and the duplicates?"),
            ],
        );
        let body = body(&request);
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert!(
            body["messages"][0]["content"][0]
                .get("cache_control")
                .is_none(),
            "the question changes at every turn: marking it would be useless"
        );
    }

    #[test]
    fn the_marker_is_set_on_the_last_block_of_the_message() {
        // A marker closes a prefix; setting it at the head would cache nothing.
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::assistant("text")
                    .with_tool_calls(vec![ToolCall::new("c1", "execute", json!({}))])
                    .cached(),
                ChatMessage::user("suite"),
            ],
        );
        let content = body(&request)["messages"][0]["content"].clone();
        let blocks = content.as_array().expect("array");
        assert!(blocks[0].get("cache_control").is_none(), "{blocks:?}");
        assert_eq!(
            blocks[blocks.len() - 1]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn marked_tools_carry_the_marker_on_the_last_one() {
        let request = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_tools(vec![
                ToolSpec::new("one", "d1", json!({})),
                ToolSpec::new("two", "d2", json!({})),
            ])
            .with_cached_tools();
        let body = body(&request);
        assert!(body["tools"][0].get("cache_control").is_none());
        assert_eq!(body["tools"][1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn the_number_of_markers_is_bounded() {
        // Beyond the bound, the API refuses the whole request: Oxyn stops
        // before rather than let a `400` reach the user.
        let mut messages = vec![ChatMessage::system("contexte").cached()];
        for number in 0..10 {
            messages.push(ChatMessage::user(format!("question {number}")).cached());
            messages.push(ChatMessage::assistant(format!("answer {number}")).cached());
        }
        let request = ChatRequest::new("m", messages)
            .with_tools(vec![ToolSpec::new("t", "d", json!({}))])
            .with_cached_tools();

        let markers = count_markers(&body(&request));
        assert!(
            markers <= wire::MAX_CACHE_BREAKPOINTS,
            "{markers} markers set"
        );
    }

    /// Counts the `cache_control`s present anywhere in the body.
    fn count_markers(value: &Value) -> usize {
        match value {
            Value::Object(object) => {
                let here = usize::from(object.contains_key("cache_control"));
                here + object.values().map(count_markers).sum::<usize>()
            }
            Value::Array(items) => items.iter().map(count_markers).sum(),
            _ => 0,
        }
    }

    // ── Token counting ─────────────────────────────────────────────────────

    #[test]
    fn the_counting_body_carries_neither_stream_nor_ceiling() {
        // The counting endpoint refuses `stream`.
        let request = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        let body = provider()
            .build_body(&request, false)
            .expect("valid request");
        assert!(body.get("stream").is_none(), "{body}");
        assert!(body.get("max_tokens").is_none(), "{body}");
        assert_eq!(body["model"], "m");
    }

    // ── Privacy and headers ────────────────────────────────────────────────

    #[test]
    fn debug_does_not_show_the_key() {
        let rendered = format!("{:?}", AnthropicProvider::new("sk-ant-THIS").expect("ok"));
        assert!(!rendered.contains("THIS"), "{rendered}");
        assert!(rendered.contains("<present>"), "{rendered}");
    }

    #[test]
    fn a_key_with_a_line_break_is_refused_without_being_displayed() {
        // A key pasted from a terminal often carries a `\n`.
        let f = AnthropicProvider::new("sk-ant-with\nbreak").expect("construction");
        let err = f.prepared_request().expect_err("invalid header");
        assert!(!err.to_string().contains("sk-ant-with"), "{err}");
    }

    #[test]
    fn a_blank_key_is_refused_locally() {
        // A `401` would suggest an account problem whereas the configuration is
        // simply incomplete.
        let f = AnthropicProvider::new("   ").expect("construction");
        let err = f.prepared_request().expect_err("blank key");
        assert!(matches!(err, OxynError::Authentication(_)), "{err}");
    }

    #[test]
    fn a_prepared_request_builds_with_a_valid_key() {
        assert!(provider().prepared_request().is_ok());
    }

    #[test]
    fn urls_are_assembled() {
        let f = provider();
        assert_eq!(
            f.messages_url().expect("URL").as_str(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            f.join(COUNT_TOKENS_PATH).expect("URL").as_str(),
            "https://api.anthropic.com/v1/messages/count_tokens"
        );
        assert_eq!(
            f.join(MODELS_PATH).expect("URL").as_str(),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn a_base_url_without_final_slash_keeps_its_path() {
        // The `Url::join` trap: without a final `/`, the last segment is
        // replaced — a corporate gateway often serves under a prefix.
        let f = AnthropicProvider::with_base_url("sk-ant-test", "https://passerelle.example/api")
            .expect("construction");
        assert_eq!(
            f.messages_url().expect("URL").as_str(),
            "https://passerelle.example/api/v1/messages"
        );
    }

    #[test]
    fn the_headers_carry_the_version() {
        let f = provider().with_version("2099-01-01");
        let headers = f.headers();
        assert!(
            headers
                .iter()
                .any(|(header_name, value)| *header_name == "anthropic-version"
                    && value == "2099-01-01"),
            "{headers:?}"
        );
        assert!(
            headers
                .iter()
                .all(|(header_name, _)| *header_name != API_KEY_HEADER),
            "the key does not go through there: {headers:?}"
        );
    }

    #[test]
    fn an_unreadable_base_url_is_a_configuration_error() {
        let err =
            AnthropicProvider::with_base_url("sk-ant-test", "not a url").expect_err("invalid URL");
        assert!(matches!(err, OxynError::Config(_)), "{err}");
    }

    // ── Cancellation before sending───────────────────────────────────────────

    #[test]
    fn an_already_cancelled_token_short_circuits_the_call() {
        let f = provider();
        let token = CancelToken::new();
        token.cancel();
        let request = ChatRequest::new("claude-model", vec![ChatMessage::user("hello")]);
        let issue = futures::executor::block_on(f.stream(request, &token));
        let err = match issue {
            Ok(_) => panic!("cancelled beforehand"),
            Err(err) => err,
        };
        assert!(err.is_cancelled(), "{err}");
    }

    // ── Failure statuses ───────────────────────────────────────────────────

    #[test]
    fn failure_statuses_carry_their_class() {
        // The class decides a retry, and it is the caller that decides — never
        // this crate (I-13). The values come from the error table checked on
        // 2026-09-16.
        let key = ApiKey::new("sk-ant-test");
        let cases = [
            (
                401,
                r#"{"type":"error","error":{"type":"authentication_error"}}"#,
                false,
            ),
            (
                413,
                r#"{"type":"error","error":{"type":"request_too_large"}}"#,
                false,
            ),
            (
                429,
                r#"{"type":"error","error":{"type":"rate_limit_error"}}"#,
                true,
            ),
            (
                500,
                r#"{"type":"error","error":{"type":"api_error"}}"#,
                true,
            ),
            (
                529,
                r#"{"type":"error","error":{"type":"overloaded_error"}}"#,
                true,
            ),
        ];
        for (statut, body, retryable) in cases {
            let err = LlmError::from_response(ProviderId::anthropic(), statut, body, Some(&key));
            assert_eq!(err.is_retryable(), retryable, "HTTP {statut} : {err}");
        }
    }

    #[test]
    fn an_authentication_refusal_never_copies_the_key() {
        // This provider sometimes copies the received key into its message.
        let key = ApiKey::new("sk-ant-api03-TRES-SECRET");
        let err = LlmError::from_response(
            ProviderId::anthropic(),
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key: sk-ant-api03-TRES-SECRET"}}"#,
            Some(&key),
        );
        let rendered = err.to_string();
        assert!(!rendered.contains("TRES-SECRET"), "{rendered}");
        assert!(rendered.contains(crate::secret::REDACTED), "{rendered}");

        let projected: OxynError = err.into();
        assert!(matches!(projected, OxynError::Authentication(_)));
        assert!(!projected.to_string().contains("TRES-SECRET"));
    }

    // ── Exact shape of the request ─────────────────────────────────────────

    #[test]
    fn a_complete_request_has_the_expected_shape() {
        // A deliberately readable snapshot rather than a file on the side: what
        // matters is that a change of shape shows **in review**, and a JSON
        // value comparison says exactly what moved.
        let request = ChatRequest::new(
            "claude-model",
            vec![
                ChatMessage::system("you are a SQL assistant").cached(),
                ChatMessage::user("count the customers"),
            ],
        )
        .with_max_tokens(256)
        .with_tools(vec![ToolSpec::new(
            "execute_query",
            "run a query",
            json!({"type": "object", "properties": {"sql": {"type": "string"}}}),
        )])
        .with_cached_tools()
        .with_reasoning_effort(ReasoningEffort::Medium);

        assert_eq!(
            body(&request),
            json!({
                "model": "claude-model",
                "tools": [{
                    "name": "execute_query",
                    "description": "run a query",
                    "input_schema": {
                        "type": "object",
                        "properties": {"sql": {"type": "string"}}
                    },
                    "cache_control": {"type": "ephemeral"}
                }],
                "system": [{
                    "type": "text",
                    "text": "you are a SQL assistant",
                    "cache_control": {"type": "ephemeral"}
                }],
                "messages": [{
                    "role": "user",
                    "content": [{"type": "text", "text": "count the customers"}]
                }],
                "stream": true,
                "max_tokens": 256,
                "output_config": {"effort": "medium"}
            })
        );
    }

    #[test]
    fn a_local_connection_cannot_reach_this_provider() {
        // ADR-0006: `Local` promises that nothing leaves the machine. This
        // provider is remote by construction — there is no Anthropic
        // deployment on the loopback —, so the combination is refused. The
        // test resolves no name: it relies on the fact that the tier, itself,
        // does not depend on resolution.
        assert!(
            !oxyn_core::PrivacyTier::Local.allows_remote_provider(),
            "a tier that let a remote provider through would promise nothing anymore"
        );
        for niveau in [
            oxyn_core::PrivacyTier::Metadata,
            oxyn_core::PrivacyTier::Sampled,
        ] {
            assert!(niveau.allows_remote_provider(), "{niveau:?}");
        }

        // And the default endpoint is not a literal loopback address: nothing
        // here can pass itself off as local.
        let f = provider();
        let point = f.endpoint().expect("this provider has a URL");
        assert_ne!(
            crate::reach::literal_reach(point),
            Some(crate::reach::Reach::Local),
            "{point}"
        );
    }

    #[test]
    fn an_invalid_request_reports_itself_before_any_network_call() {
        let f = provider();
        let request = ChatRequest::new("  ", vec![ChatMessage::user("hello")]);
        let issue = futures::executor::block_on(f.stream(request, &CancelToken::new()));
        let err = match issue {
            Ok(_) => panic!("empty model"),
            Err(err) => err,
        };
        assert!(matches!(&err, OxynError::Config(_)), "{err}");
    }
}
