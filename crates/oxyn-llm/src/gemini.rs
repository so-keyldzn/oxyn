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
//! Listing the models is sent (`gemini/models.rs`). Building the request is written
//! and tested. Sending and decoding the stream explicitly **refuse** ([`LlmError::NotImplemented`]): an unfinished path
//! that panics kills the application the day someone configures this provider
//! ([I-09](../../../CLAUDE.md#i-09)), whereas a refusal is read, displayed and
//! worked around by switching provider.

mod models;

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
    /// This unclassified transport performs no DNS measurement. For a
    /// privacy-gated request, use [`crate::build_provider`] with its measured reach.
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
        let id = ProviderId::gemini();
        let parsed = Url::parse(base_url).map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot parse the base URL: {err}"),
        })?;
        let client = http::client(&id, &parsed, reach)?;
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
        Ok(self.fetch_models().await?)
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
mod tests;
