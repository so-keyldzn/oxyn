//! The structures that really travel on the wire, OpenAI-compatible side.
//!
//! They are separate from the domain types ([`crate::types`]) because they do
//! not follow the same constraints: on the wire, the arguments of a tool call
//! are a JSON **string** and not an object, and an empty content is written
//! `null`. Merging the two sets of types would push these protocol quirks up
//! into `oxyn-ai`.
//!
//! All response structures are **tolerant**: each field has a default, none is
//! `deny_unknown_fields`. A server returns whatever it wants (I-09), and half
//! of the "OpenAI-compatible" endpoints are only approximately so.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::{ChatRequest, Cost, ModelInfo, Support, ToolCall};

/// Currency in which OpenRouter publishes its prices.
///
/// The API response does not carry the currency; only the provider's
/// documentation gives it.
///
/// Source: docs/RESEARCH-NOTES.md, "OpenRouter provider".
const OPENROUTER_CURRENCY: &str = "USD";

// ─────────────────────────────────────────────────────────────────────────────
// Request
// ─────────────────────────────────────────────────────────────────────────────

/// Corps de `POST /chat/completions`.
#[derive(Debug, Serialize)]
pub(crate) struct ChatCompletionRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    /// Reasoning effort, **only** when the caller asks for one and the provider
    /// is known to understand it.
    ///
    /// Omitted everywhere else: a strict local server rejects the whole request
    /// on an unknown field, and that is exactly the kind of regression that
    /// only shows once at the user's.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
}

/// Streaming options. Only OpenAI and its gateways understand them; local
/// servers ignore them, hence the flag on the provider side.
#[derive(Debug, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize)]
struct WireMessage {
    role: &'static str,
    /// `null` and not `""`: an assistant turn that only calls tools has no
    /// content, and some servers reject the empty string.
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct WireToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunctionCall,
}

#[derive(Debug, Serialize)]
struct WireFunctionCall {
    name: String,
    /// JSON string, not an object: the protocol wants it so.
    arguments: String,
}

#[derive(Debug, Serialize)]
struct WireTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunctionDef,
}

#[derive(Debug, Serialize)]
struct WireFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

impl ChatCompletionRequest {
    /// Translates a domain request into the HTTP body.
    ///
    /// `stream` is forced to `true`: the only call path of this crate is
    /// streamed. See the note of [`ChatRequest::stream`].
    ///
    /// `reasoning` says whether the provider understands `reasoning_effort`.
    /// False for a generic endpoint: the field is then **omitted**, never sent
    /// blindly.
    pub(crate) fn from_request(
        request: &ChatRequest,
        include_usage: bool,
        reasoning: bool,
    ) -> Self {
        let messages = request
            .messages
            .iter()
            .map(|message| {
                let tool_calls: Vec<WireToolCall> = message
                    .tool_calls
                    .iter()
                    .map(|call| WireToolCall {
                        id: call.id.clone(),
                        kind: "function",
                        function: WireFunctionCall {
                            name: call.name.clone(),
                            arguments: arguments_to_string(&call.arguments),
                        },
                    })
                    .collect();
                let content = if message.content.is_empty() && !tool_calls.is_empty() {
                    None
                } else {
                    Some(message.content.clone())
                };
                WireMessage {
                    role: message.role.as_str(),
                    content,
                    tool_calls,
                    tool_call_id: message.tool_call_id.clone(),
                }
            })
            .collect();

        let tools = request
            .tools
            .iter()
            .map(|tool| WireTool {
                kind: "function",
                function: WireFunctionDef {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.parameters.clone(),
                },
            })
            .collect();

        Self {
            model: request.model.clone(),
            messages,
            tools,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: true,
            stream_options: include_usage.then_some(StreamOptions {
                include_usage: true,
            }),
            // The requested effort, and only if this endpoint understands it.
            reasoning_effort: request
                .reasoning_effort
                .filter(|_| reasoning)
                .map(|effort| effort.as_str()),
        }
    }
}

/// Serializes tool arguments into the string expected by the protocol.
///
/// A serialization failure of an already built [`serde_json::Value`] is
/// impossible in practice; the fallback to `{}` avoids introducing a `Result`
/// in the whole building path for a case that does not happen — and it is
/// better than an `expect` on a reachable path (I-09).
fn arguments_to_string(arguments: &serde_json::Value) -> String {
    if arguments.is_null() {
        return "{}".to_owned();
    }
    serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_owned())
}

// ─────────────────────────────────────────────────────────────────────────────
// Response stream
// ─────────────────────────────────────────────────────────────────────────────

/// A `data:` frame of a completion stream.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ChatChunk {
    #[serde(default)]
    pub(crate) choices: Vec<ChunkChoice>,
    #[serde(default)]
    pub(crate) usage: Option<WireUsage>,
    /// Some gateways (OpenRouter) slip an error into the stream rather than
    /// breaking the connection.
    #[serde(default)]
    pub(crate) error: Option<WireError>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ChunkChoice {
    #[serde(default)]
    pub(crate) delta: Delta,
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Delta {
    #[serde(default)]
    pub(crate) content: Option<String>,
    /// The model's refusal. A field distinct from `content` in this protocol,
    /// and that is a good thing: a refusal is not an answer.
    #[serde(default)]
    pub(crate) refusal: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Vec<DeltaToolCall>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DeltaToolCall {
    /// Order number of the call in the turn. It is **the** reassembly key: the
    /// fragments of one call arrive interleaved with those of the others.
    #[serde(default)]
    pub(crate) index: u32,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) function: Option<DeltaFunction>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DeltaFunction {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) arguments: Option<String>,
}

/// Declared usage.
///
/// The fields are optional `i64`s and not `u32`s: a server can send `-1` for
/// "unknown", and a strict deserialization would then make the whole frame
/// fail.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<i64>,
    #[serde(default)]
    completion_tokens: Option<i64>,
    /// Detail of the input. Absent at local servers.
    #[serde(default)]
    prompt_tokens_details: Option<TokenDetails>,
    /// Detail of the output. Absent at local servers.
    #[serde(default)]
    completion_tokens_details: Option<TokenDetails>,
}

/// Detail of a token count.
///
/// A single type for input and output: both objects carry only one field of
/// interest to us, and they do not overlap. Duplicating the definition would
/// only add a place to get it wrong.
///
/// `Debug` is written by hand: it is this repository's rule for a type that
/// crosses the network boundary, and it holds even when the structure only
/// carries counters — the exception is what makes the rule inapplicable.
#[derive(Default, Deserialize)]
struct TokenDetails {
    /// Input tokens served from the cache.
    #[serde(default)]
    cached_tokens: Option<i64>,
    /// Output tokens spent reasoning.
    #[serde(default)]
    reasoning_tokens: Option<i64>,
}

impl fmt::Debug for TokenDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenDetails")
            .field("cached_tokens", &self.cached_tokens)
            .field("reasoning_tokens", &self.reasoning_tokens)
            .finish()
    }
}

impl WireUsage {
    /// Input tokens, brought back into the realm of the possible.
    pub(crate) fn prompt(&self) -> u32 {
        clamp_tokens(self.prompt_tokens)
    }

    /// Produced tokens, brought back into the realm of the possible.
    pub(crate) fn completion(&self) -> u32 {
        clamp_tokens(self.completion_tokens)
    }

    /// Tokens read from the cache, **only if the server declares it**.
    ///
    /// `None` and not `0`: most compatible endpoints have no cache and say
    /// nothing. Displaying "0 tokens read from cache" would suggest a cache
    /// that does not work.
    pub(crate) fn cache_read(&self) -> Option<u32> {
        let raw = self.prompt_tokens_details.as_ref()?.cached_tokens?;
        Some(clamp_tokens(Some(raw)))
    }

    /// Reasoning tokens, same rule.
    pub(crate) fn reasoning(&self) -> Option<u32> {
        let raw = self.completion_tokens_details.as_ref()?.reasoning_tokens?;
        Some(clamp_tokens(Some(raw)))
    }
}

/// Brings a token count coming from the network into a `u32`.
///
/// Missing or negative counts as `0` — "not declared" — and an outsized value
/// saturates rather than overflowing silently (`as` is forbidden).
fn clamp_tokens(raw: Option<i64>) -> u32 {
    let value = raw.unwrap_or(0).max(0);
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Error carried in the stream.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireError {
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) code: Option<serde_json::Value>,
}

impl WireError {
    /// Showable message, never returning an empty string.
    pub(crate) fn describe(&self) -> String {
        match (&self.message, &self.code) {
            (Some(message), Some(code)) if !message.is_empty() => {
                format!("{message} (code {code})")
            }
            (Some(message), _) if !message.is_empty() => message.clone(),
            (_, Some(code)) => format!("code {code}"),
            _ => "no details given".to_owned(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Model list
// ─────────────────────────────────────────────────────────────────────────────

/// Corps de `GET /models`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ModelsResponse {
    /// Deliberately untyped: a malformed entry must not make the whole list
    /// fail. Each element is parsed separately by [`parse_models`].
    #[serde(default, deserialize_with = "crate::http::bounded_entries")]
    pub(crate) data: Vec<serde_json::Value>,
}

/// An entry of the model list.
#[derive(Debug, Deserialize)]
pub(crate) struct WireModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    /// Declared by OpenRouter; absent at OpenAI, Ollama and LM Studio.
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    top_provider: Option<TopProvider>,
    #[serde(default)]
    pricing: Option<WirePricing>,
    /// List of accepted parameters, at gateways that publish it.
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct TopProvider {
    #[serde(default)]
    context_length: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct WirePricing {
    /// Price **per token**, as a decimal string.
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

impl From<WireModel> for ModelInfo {
    fn from(raw: WireModel) -> Self {
        let context = raw
            .context_length
            .or_else(|| raw.top_provider.as_ref().and_then(|t| t.context_length));

        // The provider says nothing about tools in most cases: `Unknown` is
        // then the only honest answer.
        let tools = match &raw.supported_parameters {
            Some(parameters) => Support::known(parameters.iter().any(|p| p == "tools")),
            None => Support::Unknown,
        };
        // Same rule for reasoning: the gateway that publishes the list of its
        // parameters includes `reasoning_effort` in it when the model accepts
        // it.
        let reasoning = match &raw.supported_parameters {
            Some(parameters) => Support::known(
                parameters
                    .iter()
                    .any(|p| p == "reasoning_effort" || p == "reasoning"),
            ),
            None => Support::Unknown,
        };

        let mut info = Self::new(raw.id);
        if let Some(display_name) = raw.name {
            info = info.with_display_name(display_name);
        }
        if let Some(window) = context {
            info = info.with_context_window(window);
        }
        info = info
            .with_tool_support(tools)
            .with_reasoning_support(reasoning);
        if let Some(cost) = raw.pricing.and_then(|p| p.into_cost()) {
            info = info.with_cost(cost);
        }
        info
    }
}

impl WirePricing {
    /// Converts a per-token price into a per-million price.
    ///
    /// Returns `None` as soon as one of the two prices is missing, unreadable,
    /// or negative. OpenRouter is known to publish `-1` on a variably priced
    /// model, but the documentation consulted does not confirm this value
    /// (source: docs/RESEARCH-NOTES.md, "OpenRouter provider", 2026-09-24);
    /// rejecting any negative remains prudent in both cases, since displaying
    /// `-1,000,000` would be worse than displaying nothing.
    fn into_cost(self) -> Option<Cost> {
        let entry = parse_price(self.prompt.as_deref())?;
        let output = parse_price(self.completion.as_deref())?;
        Some(Cost::new(
            entry * 1_000_000.0,
            output * 1_000_000.0,
            OPENROUTER_CURRENCY,
        ))
    }
}

/// Parses a per-token price. Rejects the missing, the unreadable and the negative.
fn parse_price(raw: Option<&str>) -> Option<f64> {
    let value: f64 = raw?.trim().parse().ok()?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// Parses the model list, entry by entry.
///
/// An unreadable entry is **ignored**, not fatal: an endpoint that publishes
/// an exotic model must not make the twenty others invisible.
pub(crate) fn parse_models(response: ModelsResponse) -> Vec<ModelInfo> {
    let mut infos = Vec::with_capacity(response.data.len());
    let mut ignored = 0_usize;
    for entry in response.data {
        match serde_json::from_value::<WireModel>(entry) {
            Ok(raw) => infos.push(ModelInfo::from(raw)),
            Err(_) => ignored += 1,
        }
    }
    if ignored > 0 {
        // The entry's content is not logged: we do not know what a third-party
        // endpoint puts in it.
        tracing::debug!(ignored, "unreadable entries in the model list");
    }
    infos
}

/// Rebuilds a complete tool call from its fragments.
///
/// # Errors
/// Returns the parse error message — **without** the arguments string, which
/// is a model output and can copy what it was given.
pub(crate) fn build_tool_call(
    id: String,
    name: String,
    arguments: &str,
) -> Result<ToolCall, String> {
    let raw = arguments.trim();
    if raw.is_empty() {
        // A tool without parameters: the model sometimes sends nothing at all.
        return Ok(ToolCall::new(id, name, serde_json::json!({})));
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => Ok(ToolCall::new(id, name, value)),
        Err(err) => Err(format!(
            "cannot read the arguments of tool `{name}`: {} (line {}, column {})",
            classify_label(&err),
            err.line(),
            err.column()
        )),
    }
}

/// Labels the nature of a JSON parse error, without copying the data.
///
/// It is the only detail of a parse defect we accept to show: the faulty text
/// is a model output or a third party's response, and it can copy what was
/// sent.
pub(crate) fn classify_label(err: &serde_json::Error) -> &'static str {
    match err.classify() {
        serde_json::error::Category::Io => "input-output error",
        serde_json::error::Category::Syntax => "invalid JSON syntax",
        serde_json::error::Category::Data => "unexpected data type",
        serde_json::error::Category::Eof => "truncated JSON",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, ToolSpec};

    fn serialise(request: &ChatCompletionRequest) -> serde_json::Value {
        serde_json::to_value(request).expect("request serialization")
    }

    #[test]
    fn a_minimal_request_carries_only_what_is_needed() {
        let req = ChatRequest::new("llama3.2", vec![ChatMessage::user("hello")]);
        let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));

        assert_eq!(body["model"], "llama3.2");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hello");
        assert!(
            body.get("tools").is_none(),
            "an empty tools array must not go out: some servers refuse it"
        );
        assert!(body.get("temperature").is_none());
        assert!(body.get("max_tokens").is_none());
        assert!(
            body.get("stream_options").is_none(),
            "local servers do not know stream_options"
        );
    }

    #[test]
    fn the_usage_request_is_explicit() {
        let req = ChatRequest::new("gpt-4o-mini", vec![ChatMessage::user("a")]);
        let body = serialise(&ChatCompletionRequest::from_request(&req, true, false));
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn tools_go_out_in_function_format() {
        let req =
            ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![ToolSpec::new(
                "execute_query",
                "run a query",
                serde_json::json!({"type": "object", "properties": {}}),
            )]);
        let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "execute_query");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn call_arguments_go_out_as_a_json_string() {
        // The protocol detail that gets missed: `arguments` is a string.
        let call = ToolCall::new("call_1", "execute", serde_json::json!({"sql": "SELECT 1"}));
        let req = ChatRequest::new(
            "m",
            vec![ChatMessage::assistant("").with_tool_calls(vec![call])],
        );
        let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));
        let arguments = &body["messages"][0]["tool_calls"][0]["function"]["arguments"];
        assert!(arguments.is_string(), "{arguments}");
        assert_eq!(arguments.as_str(), Some(r#"{"sql":"SELECT 1"}"#));
    }

    #[test]
    fn an_assistant_turn_without_text_sends_no_empty_content() {
        let call = ToolCall::new("call_1", "execute", serde_json::json!({}));
        let req = ChatRequest::new(
            "m",
            vec![ChatMessage::assistant("").with_tool_calls(vec![call])],
        );
        let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));
        assert!(body["messages"][0].get("content").is_none());
    }

    #[test]
    fn a_tool_message_carries_its_identifier() {
        let req = ChatRequest::new("m", vec![ChatMessage::tool_result("call_1", "42 rows")]);
        let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));
        assert_eq!(body["messages"][0]["role"], "tool");
        assert_eq!(body["messages"][0]["tool_call_id"], "call_1");
        assert_eq!(body["messages"][0]["content"], "42 rows");
    }

    #[test]
    fn a_negative_or_missing_usage_counts_as_zero() {
        let usage: WireUsage =
            serde_json::from_str(r#"{"prompt_tokens": -1}"#).expect("tolerant deserialization");
        assert_eq!(usage.prompt(), 0);
        assert_eq!(usage.completion(), 0);
    }

    #[test]
    fn an_empty_frame_deserializes() {
        // The first fragment of several servers is `{"choices":[{"delta":{}}]}`.
        let chunk: ChatChunk =
            serde_json::from_str(r#"{"choices":[{"delta":{}}]}"#).expect("frame tolerated");
        assert_eq!(chunk.choices.len(), 1);
        assert!(chunk.choices[0].delta.content.is_none());
    }

    #[test]
    fn unknown_fields_do_not_fail_a_frame() {
        let chunk: ChatChunk = serde_json::from_str(
            r#"{"id":"x","object":"chat.completion.chunk","system_fingerprint":"fp","choices":[]}"#,
        )
        .expect("unknown fields are ignored");
        assert!(chunk.choices.is_empty());
    }

    #[test]
    fn a_variable_price_is_not_displayed() {
        let variable = WirePricing {
            prompt: Some("-1".to_owned()),
            completion: Some("-1".to_owned()),
        };
        assert!(variable.into_cost().is_none());

        let unreadable = WirePricing {
            prompt: Some("gratuit".to_owned()),
            completion: Some("0".to_owned()),
        };
        assert!(unreadable.into_cost().is_none());
    }

    #[test]
    fn a_per_token_price_becomes_a_per_million_price() {
        let price = WirePricing {
            prompt: Some("0.0000005".to_owned()),
            completion: Some("0.0000015".to_owned()),
        }
        .into_cost()
        .expect("readable price");
        assert!((price.input_per_million - 0.5).abs() < 1e-9, "{price:?}");
        assert!((price.output_per_million - 1.5).abs() < 1e-9, "{price:?}");
    }

    #[test]
    fn an_unreadable_model_entry_does_not_lose_the_others() {
        let response: ModelsResponse =
            serde_json::from_str(r#"{"data":[{"id":"bon"},{"pas_d_id":true},{"id":"aussi-bon"}]}"#)
                .expect("list tolerated");
        let infos = parse_models(response);
        let ids: Vec<&str> = infos.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["bon", "aussi-bon"]);
    }

    #[test]
    fn a_model_without_metadata_asserts_nothing() {
        let response: ModelsResponse =
            serde_json::from_str(r#"{"data":[{"id":"llama3.2"}]}"#).expect("list tolerated");
        let infos = parse_models(response);
        assert_eq!(infos[0].context_window, None);
        assert_eq!(infos[0].supports_tools, Support::Unknown);
        assert_eq!(infos[0].display_name, "llama3.2");
    }

    #[test]
    fn a_model_that_declares_its_parameters_is_believed() {
        let response: ModelsResponse = serde_json::from_str(
            r#"{"data":[
                {"id":"a","supported_parameters":["tools","temperature"],"context_length":128000},
                {"id":"b","supported_parameters":["temperature"]}
            ]}"#,
        )
        .expect("list tolerated");
        let infos = parse_models(response);
        assert_eq!(infos[0].supports_tools, Support::Yes);
        assert_eq!(infos[0].context_window, Some(128_000));
        assert_eq!(infos[1].supports_tools, Support::No);
    }

    #[test]
    fn the_main_provider_window_serves_as_fallback() {
        let response: ModelsResponse =
            serde_json::from_str(r#"{"data":[{"id":"a","top_provider":{"context_length":8192}}]}"#)
                .expect("list tolerated");
        assert_eq!(parse_models(response)[0].context_window, Some(8192));
    }

    #[test]
    fn missing_arguments_count_as_an_empty_object() {
        let call = build_tool_call("c1".to_owned(), "ping".to_owned(), "  ")
            .expect("a tool without parameters is legitimate");
        assert_eq!(call.arguments, serde_json::json!({}));
    }

    #[test]
    fn truncated_arguments_produce_an_error_without_copying_them() {
        let error = build_tool_call(
            "c1".to_owned(),
            "execute".to_owned(),
            r#"{"sql": "SELECT secret FROM"#,
        )
        .expect_err("truncated JSON");
        assert!(error.contains("execute"), "{error}");
        assert!(
            !error.contains("secret"),
            "the model output must not be copied: {error}"
        );
    }

    #[test]
    fn an_error_in_the_stream_describes_itself() {
        let err = WireError {
            message: Some("rate limited".to_owned()),
            code: Some(serde_json::json!(429)),
        };
        assert_eq!(err.describe(), "rate limited (code 429)");
        assert_eq!(WireError::default().describe(), "no details given");
    }
}
