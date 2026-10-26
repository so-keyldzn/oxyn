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

use crate::types::{ChatEvent, ChatRequest, Cost, ModelInfo, Support, ToolCall};

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
    /// Order number of the call in the turn: the fragments of one call arrive
    /// interleaved with those of the others. Optional, and not defaulted to
    /// `0`: a gateway that omits it would see all its calls merged into the
    /// first. The decoder pairs it with `id` (see its module note).
    #[serde(default)]
    pub(crate) index: Option<u32>,
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
///
/// Two shapes are read: OpenAI's `{"data": [...]}`, and the bare array
/// Together AI returns (RESEARCH-NOTES, "Model listing", 2026-10-04). An object
/// without `data` is refused, not read as an empty list: a native endpoint
/// (`/api/tags`, `/api/v1/models`) answering at this address would otherwise
/// show « no model » where the truth is « wrong address ».
#[derive(Debug, Default)]
pub(crate) struct ModelsResponse {
    /// Deliberately untyped: a malformed entry must not make the whole list
    /// fail. Each element is parsed separately by [`parse_models`].
    pub(crate) data: Vec<serde_json::Value>,
}

/// The `data` member, bounded like a bare array.
#[derive(Deserialize)]
struct Entries(#[serde(deserialize_with = "crate::http::bounded_entries")] Vec<serde_json::Value>);

impl<'de> Deserialize<'de> for ModelsResponse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Body;
        impl<'de> serde::de::Visitor<'de> for Body {
            type Value = ModelsResponse;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a model list: an object with `data`, or an array")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                seq: A,
            ) -> Result<Self::Value, A::Error> {
                Ok(ModelsResponse {
                    data: crate::http::bounded_seq(seq)?,
                })
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut data = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "data" && data.is_none() {
                        data = Some(map.next_value::<Entries>()?.0);
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                data.map(|data| ModelsResponse { data })
                    .ok_or_else(|| serde::de::Error::missing_field("data"))
            }
        }
        deserializer.deserialize_any(Body)
    }
}

/// An entry of the model list.
///
/// Every field but `id` is optional and read leniently: each provider of this
/// family names its metadata differently (RESEARCH-NOTES, "Model listing",
/// 2026-10-04), and a value out of range loses the field, never the model.
#[derive(Debug, Deserialize)]
pub(crate) struct WireModel {
    id: String,
    /// OpenRouter, DeepSeek.
    #[serde(default, deserialize_with = "lenient_string")]
    name: Option<String>,
    /// Together AI.
    #[serde(default, deserialize_with = "lenient_string")]
    display_name: Option<String>,
    /// OpenRouter, Together AI, xAI.
    #[serde(default, deserialize_with = "lenient_u32")]
    context_length: Option<u32>,
    /// Groq, DeepSeek.
    #[serde(default, deserialize_with = "lenient_u32")]
    context_window: Option<u32>,
    /// Mistral.
    #[serde(default, deserialize_with = "lenient_u32")]
    max_context_length: Option<u32>,
    /// Together AI's kind of model: `chat`, `language`, `code`, `image`,
    /// `embedding`, `moderation`, `rerank`.
    #[serde(default, rename = "type", deserialize_with = "lenient_string")]
    kind: Option<String>,
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
    #[serde(default, deserialize_with = "lenient_u32")]
    context_length: Option<u32>,
}

/// A count that does not fit, or is not a count, is not declared.
fn lenient_u32<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_u64().and_then(|n| u32::try_from(n).ok()))
}

/// A label that is not a string is not declared.
fn lenient_string<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(text) => Ok(Some(text)),
        _ => Ok(None),
    }
}

/// Kinds Together AI documents that no chat request can use.
const NOT_CHAT_KINDS: [&str; 4] = ["image", "embedding", "moderation", "rerank"];

impl WireModel {
    /// Can a chat request use this model, as far as the provider says?
    fn serves_chat(&self) -> bool {
        self.kind
            .as_deref()
            .is_none_or(|kind| !NOT_CHAT_KINDS.contains(&kind))
    }
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
            .or(raw.context_window)
            .or(raw.max_context_length)
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
        if let Some(display_name) = raw.name.or(raw.display_name) {
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
            Ok(raw) if raw.serves_chat() => infos.push(ModelInfo::from(raw)),
            Ok(_) => {}
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
/// Unreadable arguments do not lose the call: it becomes
/// [`ChatEvent::ToolCallInvalid`](crate::types::ChatEvent::ToolCallInvalid),
/// which the caller answers with a rejection, and the turn's other calls stand.
pub(crate) fn tool_call_event(id: String, name: String, arguments: &str) -> ChatEvent {
    let raw = arguments.trim();
    if raw.is_empty() {
        // A tool without parameters: the model sometimes sends nothing at all.
        return ChatEvent::ToolCallComplete(ToolCall::new(id, name, serde_json::json!({})));
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => ChatEvent::ToolCallComplete(ToolCall::new(id, name, value)),
        // The arguments string is not copied: it is a model output, and can
        // copy what the model was given.
        Err(err) => ChatEvent::ToolCallInvalid {
            id,
            name,
            detail: format!(
                "the arguments are not valid JSON: {} (line {}, column {})",
                classify_label(&err),
                err.line(),
                err.column()
            ),
        },
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
mod tests;
