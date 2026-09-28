//! The Gemini provider — complete request structure, sending in phase 2.
//!
//! # Why not an OpenAI-compatible adapter
//!
//! Google does publish a compatibility endpoint, but the native protocol
//! differs on points that touch exactly what Oxyn needs:
//!
//! 1. the **model is in the path** of the URL, not in the body;
//! 2. the turns are called `contents`, and the assistant's role is `model`;
//! 3. a tool call is a `functionCall` designated by **name**, without an
//!    identifier: matching a response to its request requires finding the name
//!    again from the previous turn;
//! 4. the tool response must be an **object**, never a bare string.
//!
//! Going through the compatibility layer would lose point 3, that is multiple
//! tool calls — the case that matters
//! ([`ARCHITECTURE` §7.5](../../../docs/ARCHITECTURE.md)).
//!
//! # What this module already does
//!
//! Building the request is written and tested. Sending and decoding the stream
//! explicitly **refuse** ([`LlmError::NotImplemented`]): an unfinished path
//! that panics kills the application the day someone configures this provider
//! ([I-09](../../../CLAUDE.md#i-09)), whereas a refusal is read, displayed and
//! worked around by switching provider.

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxyn_core::{CancelToken, Result};
use reqwest::header::HeaderValue;
use reqwest::{Client, Url};
use serde_json::{Map, Value, json};

use crate::error::LlmError;
use crate::http;
use crate::provider::{self, LlmProvider, ProviderId};
use crate::reach;
use crate::secret::ApiKey;
use crate::types::{ChatEvent, ChatMessage, ChatRequest, ModelInfo, Role};

/// Endpoint of the Gemini API.
///
/// Source: docs/RESEARCH-NOTES.md, "Gemini provider".
pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";

/// API version in the path.
///
/// `v1beta` rather than `v1`: the streamGenerateContent reference only
/// documents the path under `/v1beta/…`, and it remains the default used by
/// the official SDKs. Source: docs/RESEARCH-NOTES.md, "Gemini provider".
pub const GEMINI_API_VERSION: &str = "v1beta";

/// Header carrying the key.
///
/// Source: docs/RESEARCH-NOTES.md, "Gemini provider".
pub const API_KEY_HEADER: &str = "x-goog-api-key";

/// Gemini provider.
///
/// `Debug` written by hand: the key does not appear in it (I-03).
pub struct GeminiProvider {
    base_url: Url,
    api_key: ApiKey,
    api_version: String,
    client: Client,
}

impl GeminiProvider {
    /// Builds the provider on the public endpoint.
    ///
    /// # Errors
    /// Unreadable base URL, or HTTP client impossible to build.
    pub fn new(api_key: impl Into<ApiKey>) -> Result<Self> {
        Self::with_base_url(api_key, GEMINI_BASE_URL)
    }

    /// Builds the provider on a chosen endpoint.
    ///
    /// # Errors
    /// Unreadable base URL, or HTTP client impossible to build.
    pub fn with_base_url(api_key: impl Into<ApiKey>, base_url: &str) -> Result<Self> {
        let id = ProviderId::gemini();
        let parsed = Url::parse(base_url).map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot parse the base URL: {err}"),
        })?;
        let client = http::client(&id)?;
        Ok(Self {
            base_url: provider::normalize_base_url(parsed),
            api_key: api_key.into(),
            api_version: GEMINI_API_VERSION.to_owned(),
            client,
        })
    }

    /// Sets the API version used in the path.
    #[must_use]
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }

    /// Streaming generation URL for a given model.
    ///
    /// The model name is **validated** before entering the path: it is a
    /// received identifier, and Oxyn never concatenates one without checking
    /// it (I-10). A name containing `/` or `?` would rewrite the request.
    ///
    /// # Errors
    /// Model name empty or containing a character outside `[A-Za-z0-9._-]`, or
    /// path that cannot be assembled.
    pub fn stream_url(&self, model: &str) -> Result<Url> {
        let invalid = |detail: &str| LlmError::Config {
            provider: ProviderId::gemini(),
            detail: detail.to_owned(),
        };
        let model = model.trim();
        if model.is_empty() {
            return Err(invalid("no model requested").into());
        }
        if !model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(
                invalid("the model name accepts only A-Z, a-z, 0-9, `-`, `_` and `.`").into(),
            );
        }

        let path = format!("{}/models/{model}:streamGenerateContent", self.api_version);
        let mut url = self
            .base_url
            .join(&path)
            .map_err(|err| invalid(&format!("cannot build the request path: {err}")))?;
        // `alt=sse`: without it, the API returns a whole JSON array rather than
        // a stream — which would amount to waiting for the end before
        // displaying anything, and that is precisely what Oxyn refuses.
        url.query_pairs_mut().append_pair("alt", "sse");
        Ok(url)
    }

    /// Builds the body of the generation request.
    ///
    /// # Errors
    /// Request without any turn apart from the system instruction.
    pub fn wire_request(&self, request: &ChatRequest) -> Result<Value> {
        let contents = contents(&request.messages);
        if contents.is_empty() {
            return Err(LlmError::Config {
                provider: ProviderId::gemini(),
                detail: "no message to send".to_owned(),
            }
            .into());
        }

        let mut body = Map::new();
        body.insert("contents".to_owned(), Value::Array(contents));

        if let Some(instruction) = system_instruction(&request.messages) {
            body.insert(
                "systemInstruction".to_owned(),
                json!({ "parts": [{ "text": instruction }] }),
            );
        }

        if !request.tools.is_empty() {
            let declarations: Vec<Value> = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    })
                })
                .collect();
            body.insert(
                "tools".to_owned(),
                json!([{ "functionDeclarations": declarations }]),
            );
        }

        let mut config = Map::new();
        if let Some(temperature) = request.temperature {
            config.insert("temperature".to_owned(), json!(temperature));
        }
        if let Some(max_tokens) = request.max_tokens {
            // `maxOutputTokens`, not `max_tokens`: the name differs.
            config.insert("maxOutputTokens".to_owned(), json!(max_tokens));
        }
        if !config.is_empty() {
            body.insert("generationConfig".to_owned(), Value::Object(config));
        }

        Ok(Value::Object(body))
    }

    /// Prepares the HTTP request: model URL, then the key.
    ///
    /// The key is only set here — never in the URL, where it would end up in
    /// the provider's access logs — and the header is marked sensitive: the
    /// HTTP stack will not render it in its traces (I-03).
    ///
    /// # Errors
    /// Refused model name, or key not representable in an HTTP header — a key
    /// pasted from a terminal often carries a line break.
    pub(crate) fn prepared_request(&self, model: &str) -> Result<reqwest::RequestBuilder> {
        let url = self.stream_url(model)?;
        let mut key =
            HeaderValue::from_str(self.api_key.expose()).map_err(|_| LlmError::Config {
                provider: ProviderId::gemini(),
                detail: "the API key contains a character that is not valid in an HTTP header"
                    .to_owned(),
            })?;
        key.set_sensitive(true);
        Ok(self
            .client
            .post(url)
            .header("content-type", "application/json")
            .header(API_KEY_HEADER, key))
    }
}

/// Gathers the system instructions.
fn system_instruction(messages: &[ChatMessage]) -> Option<String> {
    let chunks: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == Role::System && !m.content.is_empty())
        .map(|m| m.content.as_str())
        .collect();
    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join("\n\n"))
    }
}

/// Projects the conversation onto the protocol's `contents`.
///
/// The tricky point is the tool response: this protocol designates it by
/// **function name**, whereas the domain only carries a call identifier. The
/// name is therefore found again in the previous assistant turns. Without it,
/// the response would be attached at random as soon as the model asks for two
/// tools.
fn contents(messages: &[ChatMessage]) -> Vec<Value> {
    let mut names_by_call: HashMap<&str, &str> = HashMap::new();
    let mut outputs: Vec<(&'static str, Vec<Value>)> = Vec::new();

    for message in messages {
        let (role, parts) = match message.role {
            Role::System => continue,
            Role::User => ("user", vec![json!({ "text": message.content })]),
            Role::Assistant => {
                let mut parts = Vec::new();
                if !message.content.is_empty() {
                    parts.push(json!({ "text": message.content }));
                }
                for call in &message.tool_calls {
                    names_by_call.insert(call.id.as_str(), call.name.as_str());
                    parts.push(json!({
                        "functionCall": { "name": call.name, "args": call.arguments }
                    }));
                }
                if parts.is_empty() {
                    continue;
                }
                ("model", parts)
            }
            Role::Tool => {
                let identifier = message.tool_call_id.as_deref().unwrap_or_default();
                let tool_name = names_by_call.get(identifier).copied().unwrap_or(identifier);
                (
                    "user",
                    vec![json!({
                        "functionResponse": {
                            "name": tool_name,
                            // The response must be an object: a bare string
                            // is refused by the API.
                            "response": { "result": message.content },
                        }
                    })],
                )
            }
        };

        // `last()` then `last_mut()` in two steps: a `match` on `last_mut()`
        // would keep the mutable borrow alive in the arm that pushes, and the
        // borrow checker refuses it.
        if outputs
            .last()
            .is_some_and(|(previous, _)| *previous == role)
        {
            if let Some((_, accumulated)) = outputs.last_mut() {
                accumulated.extend(parts);
            }
        } else {
            outputs.push((role, parts));
        }
    }

    outputs
        .into_iter()
        .map(|(role, parts)| json!({ "role": role, "parts": parts }))
        .collect()
}

impl fmt::Debug for GeminiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiProvider")
            .field("base_url", &reach::redacted(&self.base_url))
            .field("api_key", &"<present>")
            .field("api_version", &self.api_version)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::gemini()
    }

    fn endpoint(&self) -> Option<&Url> {
        Some(&self.base_url)
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        // Neither the path nor the schema of the list is checked against the
        // registry (I-12): a list written from memory would be plausible and wrong.
        Err(LlmError::NotImplemented {
            provider: ProviderId::gemini(),
            operation: "listing models (endpoint path and response shape still unverified)"
                .to_owned(),
        }
        .into())
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>> {
        // The request is built and validated before the refusal: a malformed
        // request must report itself as such rather than be hidden by the
        // absence of sending.
        let body = self.wire_request(&request)?;
        let _request = self.prepared_request(&request.model)?.json(&body);
        // What is missing is sending and decoding Gemini's SSE frames
        // (`candidates[].content.parts`, `functionCall`, `usageMetadata`,
        // `finishReason`).
        Err(LlmError::NotImplemented {
            provider: ProviderId::gemini(),
            operation: "streaming a generation (streamGenerateContent)".to_owned(),
        }
        .into())
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::OxynError;

    use super::*;
    use crate::types::{ToolCall, ToolSpec};

    fn provider() -> GeminiProvider {
        GeminiProvider::new("test-key").expect("construction")
    }

    #[test]
    fn an_unimplemented_exchange_refuses_instead_of_panicking() {
        // I-09: a panic kills the application. A provider that is configured
        // but whose protocol is not written must give a message.
        let f = provider();
        let list = futures::executor::block_on(f.models()).expect_err("not implemented yet");
        assert!(matches!(&list, OxynError::NotSupported { .. }), "{list}");
        assert!(!list.is_retryable(), "retrying will not write the code");

        let request = ChatRequest::new("gemini-2.0-flash", vec![ChatMessage::user("hello")]);
        let flux = futures::executor::block_on(f.stream(request, &CancelToken::new()))
            .err()
            .expect("not implemented yet");
        let rendered = flux.to_string();
        assert!(rendered.contains("gemini"), "{rendered}");
        assert!(
            rendered.contains("Oxyn does not implement"),
            "the message must point at Oxyn, not the provider: {rendered}"
        );
    }

    #[test]
    fn an_invalid_request_reports_itself_before_the_refusal() {
        // The order matters: otherwise the refusal would hide a defect of the
        // request, and the validation would stop being tested until phase 2.
        let f = provider();
        let request = ChatRequest::new("  ", vec![ChatMessage::user("hello")]);
        let err = futures::executor::block_on(f.stream(request, &CancelToken::new()))
            .err()
            .expect("empty model");
        assert!(matches!(&err, OxynError::Config(_)), "{err}");
    }

    #[test]
    fn the_model_is_in_the_path_with_streaming_requested() {
        let url = provider()
            .stream_url("gemini-2.0-flash")
            .expect("valid URL");
        assert_eq!(
            url.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn a_model_name_that_would_rewrite_the_request_is_refused() {
        // I-10: a received identifier is not concatenated without checking.
        let f = provider();
        for malformed in ["", "../other", "model?key=stolen", "model#x", "mo del"] {
            assert!(
                f.stream_url(malformed).is_err(),
                "`{malformed}` should have been refused"
            );
        }
    }

    #[test]
    fn the_system_instruction_is_a_separate_field() {
        let request = ChatRequest::new(
            "gemini-2.0-flash",
            vec![
                ChatMessage::system("you are a SQL assistant"),
                ChatMessage::user("hello"),
            ],
        );
        let body = provider().wire_request(&request).expect("valid request");
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "you are a SQL assistant"
        );
        assert_eq!(body["contents"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn the_assistant_role_is_called_model() {
        let request = ChatRequest::new(
            "m",
            vec![ChatMessage::user("a"), ChatMessage::assistant("b")],
        );
        let body = provider().wire_request(&request).expect("valid request");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][1]["role"], "model");
    }

    #[test]
    fn a_tool_response_finds_the_function_name_again() {
        // The point that rules out the adapter: this protocol designates by name.
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("count"),
                ChatMessage::assistant("").with_tool_calls(vec![ToolCall::new(
                    "call_1",
                    "execute_query",
                    json!({"sql": "SELECT 1"}),
                )]),
                ChatMessage::tool_result("call_1", "42"),
            ],
        );
        let body = provider().wire_request(&request).expect("valid request");
        let response = &body["contents"][2]["parts"][0]["functionResponse"];
        assert_eq!(response["name"], "execute_query");
        assert_eq!(response["response"]["result"], "42");
    }

    #[test]
    fn two_tool_responses_do_not_mix() {
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("do both"),
                ChatMessage::assistant("").with_tool_calls(vec![
                    ToolCall::new("c1", "lire", json!({})),
                    ToolCall::new("c2", "count_rows", json!({})),
                ]),
                ChatMessage::tool_result("c2", "two"),
                ChatMessage::tool_result("c1", "one"),
            ],
        );
        let body = provider().wire_request(&request).expect("valid request");
        let parts = body["contents"][2]["parts"]
            .as_array()
            .expect("both responses are merged into one turn");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["functionResponse"]["name"], "count_rows");
        assert_eq!(parts[1]["functionResponse"]["name"], "lire");
    }

    #[test]
    fn a_tool_call_becomes_a_function_call() {
        let request = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("a"),
                ChatMessage::assistant("I am looking").with_tool_calls(vec![ToolCall::new(
                    "c1",
                    "lister",
                    json!({"schema": "public"}),
                )]),
            ],
        );
        let body = provider().wire_request(&request).expect("valid request");
        let parts = body["contents"][1]["parts"].as_array().expect("parts");
        assert_eq!(parts[0]["text"], "I am looking");
        assert_eq!(parts[1]["functionCall"]["name"], "lister");
        assert_eq!(parts[1]["functionCall"]["args"]["schema"], "public");
    }

    #[test]
    fn tools_are_grouped_into_declarations() {
        let request = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![
            ToolSpec::new("lister", "liste", json!({"type": "object"})),
            ToolSpec::new("count_rows", "count", json!({"type": "object"})),
        ]);
        let body = provider().wire_request(&request).expect("valid request");
        let declarations = body["tools"][0]["functionDeclarations"]
            .as_array()
            .expect("a single group of declarations");
        assert_eq!(declarations.len(), 2);
        assert_eq!(declarations[0]["name"], "lister");
    }

    #[test]
    fn the_token_ceiling_carries_the_protocol_name() {
        let request = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_max_tokens(256);
        let body = provider().wire_request(&request).expect("valid request");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], json!(256));
        assert!(body["generationConfig"].get("max_tokens").is_none());
    }

    #[test]
    fn a_request_without_a_useful_turn_is_refused() {
        let request = ChatRequest::new("m", vec![ChatMessage::system("seule")]);
        assert!(provider().wire_request(&request).is_err());
    }

    #[test]
    fn debug_does_not_show_the_key() {
        let rendered = format!("{:?}", GeminiProvider::new("THISMUSTNOTLEAK").expect("ok"));
        assert!(!rendered.contains("THISMUSTNOTLEAK"), "{rendered}");
    }

    #[test]
    fn a_key_with_a_line_break_is_refused_without_being_displayed() {
        // A key pasted from a terminal often carries a `\n`.
        let f = GeminiProvider::new("key-with\nbreak").expect("construction");
        let err = f
            .prepared_request("gemini-2.0-flash")
            .expect_err("invalid header");
        assert!(!err.to_string().contains("key-with"), "{err}");
    }

    #[test]
    fn a_prepared_request_does_not_put_the_key_in_the_url() {
        // A key as a query parameter ends up in the access logs.
        let f = provider();
        let url = f.stream_url("gemini-2.0-flash").expect("valid URL");
        assert!(!url.as_str().contains("test-key"), "{url}");
        assert!(f.prepared_request("gemini-2.0-flash").is_ok());
    }
}
