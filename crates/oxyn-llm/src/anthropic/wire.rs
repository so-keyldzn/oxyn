//! The structures that really travel on the wire, Anthropic side.
//!
//! Everything written here is **checked and dated** in
//! [`RESEARCH-NOTES`](../../../docs/RESEARCH-NOTES.md) § "Anthropic provider"
//! (I-12): paths, headers, event names, field names and enumeration values.
//! None is written from memory — a plausible and wrong value shows up neither
//! at compile time, nor in tests, nor in review.
//!
//! As on the OpenAI-compatible side, all **response** structures are
//! tolerant: each field has a default, none is `deny_unknown_fields`. The
//! documentation explicitly announces that new event types can appear, and a
//! server returns whatever it wants anyway (I-09).

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::reasoning::{ReasoningBlock, ReasoningEffort};
use crate::types::{ChatMessage, ChatRequest, ModelInfo, Role, Support};

/// Maximum number of cache markers accepted in a request.
///
/// Beyond it, the API refuses the whole request. Oxyn therefore stops
/// **before** the limit rather than let a `400` reach the user for a setting
/// they are not aware of having set.
pub(crate) const MAX_CACHE_BREAKPOINTS: usize = 4;

// ─────────────────────────────────────────────────────────────────────────────
// Request
// ─────────────────────────────────────────────────────────────────────────────

/// Counter of cache markers, bounded.
///
/// Passed along during construction: it is the only way to respect the global
/// bound while the markers come from three places (tools, system instruction,
/// messages).
struct CacheBudget(usize);

impl CacheBudget {
    const fn new() -> Self {
        Self(MAX_CACHE_BREAKPOINTS)
    }

    /// Consumes a marker if one is left.
    fn take(&mut self) -> bool {
        if self.0 == 0 {
            return false;
        }
        self.0 -= 1;
        true
    }
}

/// The cache marker, as it goes on the wire.
fn cache_control() -> Value {
    json!({ "type": "ephemeral" })
}

/// Sets the cache marker on a block, if the budget allows.
fn mark_cached(bloc: &mut Value, budget: &mut CacheBudget) {
    if let Some(objet) = bloc.as_object_mut()
        && budget.take()
    {
        objet.insert("cache_control".to_owned(), cache_control());
    }
}

/// Un bloc de texte.
fn text_block(texte: &str) -> Value {
    json!({ "type": "text", "text": texte })
}

/// Gathers the system instructions into a single top-level field.
///
/// Several `System` messages are concatenated rather than lost: it is the only
/// recourse for a protocol that accepts only one. The instruction goes out as
/// a **list of blocks** and not as a string, because a cache marker can only
/// be set on a block.
fn system_prompt(messages: &[ChatMessage], budget: &mut CacheBudget) -> Option<Value> {
    let morceaux: Vec<&ChatMessage> = messages
        .iter()
        .filter(|m| m.role == Role::System && !m.content.is_empty())
        .collect();
    if morceaux.is_empty() {
        return None;
    }
    let cachable = morceaux.iter().any(|m| m.cache_breakpoint);
    let texte: Vec<&str> = morceaux.iter().map(|m| m.content.as_str()).collect();
    let mut bloc = text_block(&texte.join("\n\n"));
    if cachable {
        mark_cached(&mut bloc, budget);
    }
    Some(Value::Array(vec![bloc]))
}

/// Projects the conversation onto the protocol's list of messages.
///
/// Three translations to make, and they are the reason this module exists:
///
/// * a tool result becomes a `tool_result` block in a message of role `user`;
/// * the reasoning blocks of an assistant turn are placed **at the head** of
///   its content, in the order received and without being modified;
/// * two consecutive messages of the same role are **merged** — the API
///   requires alternation, and two successive tool results are the common
///   case when the model asked for several.
fn conversation(
    messages: &[ChatMessage],
    budget: &mut CacheBudget,
) -> Result<Vec<Value>, UnrenderableReasoning> {
    let mut sorties: Vec<(&'static str, Vec<Value>)> = Vec::new();

    for message in messages {
        let (role, mut blocs) = match message.role {
            Role::System => continue,
            Role::User => ("user", vec![text_block(&message.content)]),
            Role::Tool => {
                let identifiant = message.tool_call_id.clone().unwrap_or_default();
                (
                    "user",
                    vec![json!({
                        "type": "tool_result",
                        "tool_use_id": identifiant,
                        "content": message.content,
                    })],
                )
            }
            Role::Assistant => {
                let mut blocs = Vec::new();
                // The reasoning blocks first, as they were received.
                // Reordering them, editing one or losing one makes the request
                // refused: the API checks their signature.
                for bloc in &message.reasoning {
                    blocs.push(reasoning_block(bloc)?);
                }
                if !message.content.is_empty() {
                    blocs.push(text_block(&message.content));
                }
                for appel in &message.tool_calls {
                    blocs.push(json!({
                        "type": "tool_use",
                        "id": appel.id,
                        "name": appel.name,
                        "input": appel.arguments,
                    }));
                }
                if blocs.is_empty() {
                    continue;
                }
                ("assistant", blocs)
            }
        };

        // The marker is set on the **last** block of the message: it closes a
        // prefix, it does not open it.
        if message.cache_breakpoint
            && let Some(dernier) = blocs.last_mut()
        {
            mark_cached(dernier, budget);
        }

        // `last()` then `last_mut()` in two steps: a `match` on `last_mut()`
        // would keep the mutable borrow alive in the arm that pushes, and the
        // borrow checker refuses it.
        if sorties
            .last()
            .is_some_and(|(precedent, _)| *precedent == role)
        {
            if let Some((_, accumules)) = sorties.last_mut() {
                accumules.extend(blocs);
            }
        } else {
            sorties.push((role, blocs));
        }
    }

    Ok(sorties
        .into_iter()
        .map(|(role, blocs)| json!({ "role": role, "content": blocs }))
        .collect())
}

/// A reasoning block this protocol cannot send back.
///
/// [`ReasoningBlock`] is defined in `oxyn-core` and `#[non_exhaustive]`: a
/// variant can appear there without this module knowing it. Omitting it would
/// make the request refused for an invalid signature — or, worse, accepted
/// with a truncated reasoning. So it is refused before sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnrenderableReasoning;

/// Renders a reasoning block in the protocol's format, **without modifying it**.
fn reasoning_block(bloc: &ReasoningBlock) -> Result<Value, UnrenderableReasoning> {
    match bloc {
        ReasoningBlock::Summarized { text, signature } => {
            let mut objet = Map::new();
            objet.insert("type".to_owned(), json!("thinking"));
            objet.insert("thinking".to_owned(), json!(text));
            if let Some(signature) = signature {
                objet.insert("signature".to_owned(), json!(signature));
            }
            Ok(Value::Object(objet))
        }
        ReasoningBlock::Redacted { data } => Ok(json!({
            "type": "redacted_thinking",
            "data": data,
        })),
        _ => Err(UnrenderableReasoning),
    }
}

/// Projette les outils offerts, marqueur de cache compris.
fn tools(request: &ChatRequest, budget: &mut CacheBudget) -> Option<Value> {
    if request.tools.is_empty() {
        return None;
    }
    let mut outils: Vec<Value> = request
        .tools
        .iter()
        .map(|outil| {
            json!({
                "name": outil.name,
                "description": outil.description,
                // `input_schema` and not `parameters`: that is the field's
                // name in this protocol.
                "input_schema": outil.parameters,
            })
        })
        .collect();
    // The marker is set on the **last** tool: it covers all the definitions
    // that precede it.
    if request.cache_tools
        && let Some(dernier) = outils.last_mut()
    {
        mark_cached(dernier, budget);
    }
    Some(Value::Array(outils))
}

/// Builds the body of `POST /v1/messages`.
///
/// `stream` governs the field of the same name: streaming for a generation,
/// its absence for a token count — the counting endpoint refuses `stream`.
///
/// Fails only if the conversation carries a reasoning block this protocol
/// cannot send back ([`UnrenderableReasoning`]).
pub(crate) fn build_request(
    request: &ChatRequest,
    default_max_tokens: u32,
    stream: bool,
) -> Result<Value, UnrenderableReasoning> {
    let mut budget = CacheBudget::new();
    let mut corps = Map::new();

    corps.insert("model".to_owned(), json!(request.model));
    // The order in which the budget is applied follows that of the prefix:
    // tools first, then the system instruction, then the messages. It is the
    // order in which the provider assembles the prompt, hence the one where a
    // marker has a chance to be useful.
    if let Some(outils) = tools(request, &mut budget) {
        corps.insert("tools".to_owned(), outils);
    }
    if let Some(consigne) = system_prompt(&request.messages, &mut budget) {
        corps.insert("system".to_owned(), consigne);
    }
    corps.insert(
        "messages".to_owned(),
        Value::Array(conversation(&request.messages, &mut budget)?),
    );

    if stream {
        corps.insert("stream".to_owned(), json!(true));
        corps.insert(
            "max_tokens".to_owned(),
            json!(request.max_tokens.unwrap_or(default_max_tokens)),
        );
    }
    if let Some(temperature) = request.temperature {
        corps.insert("temperature".to_owned(), json!(temperature));
    }

    // A thinking budget requires the explicit mode; effort alone goes through
    // `output_config` and lets the model decide whether it thinks. Sending a
    // thinking mode that was not requested would make the request fail on
    // models that do not know it.
    if let Some(budget_jetons) = request.reasoning_budget_tokens {
        corps.insert(
            "thinking".to_owned(),
            json!({
                "type": "enabled",
                "budget_tokens": budget_jetons,
                // Without it the reasoning comes back empty: the default of
                // several models is not to return it.
                "display": "summarized",
            }),
        );
    }
    if let Some(effort) = request.reasoning_effort {
        corps.insert(
            "output_config".to_owned(),
            json!({ "effort": effort.as_str() }),
        );
    }

    Ok(Value::Object(corps))
}

// ─────────────────────────────────────────────────────────────────────────────
// Stream events
// ─────────────────────────────────────────────────────────────────────────────

/// The `type` field of a frame, when it carries one.
///
/// The name of the SSE event and the `type` field of its payload are redundant
/// in this protocol. We read the **field**, not the event name: a frame
/// reassembled by a proxy can lose its name, never its payload.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Envelope {
    #[serde(default)]
    pub(crate) r#type: Option<String>,
    #[serde(default)]
    pub(crate) index: Option<u32>,
    #[serde(default)]
    pub(crate) message: Option<MessageStart>,
    #[serde(default)]
    pub(crate) content_block: Option<ContentBlock>,
    #[serde(default)]
    pub(crate) delta: Option<Delta>,
    #[serde(default)]
    pub(crate) usage: Option<WireUsage>,
    #[serde(default)]
    pub(crate) error: Option<WireError>,
}

/// La charge de `message_start`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct MessageStart {
    #[serde(default)]
    pub(crate) usage: Option<WireUsage>,
}

/// Le bloc ouvert par `content_block_start`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ContentBlock {
    #[serde(default)]
    pub(crate) r#type: Option<String>,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Present on an already complete reasoning block.
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    #[serde(default)]
    pub(crate) signature: Option<String>,
    /// Payload of an encrypted reasoning block.
    #[serde(default)]
    pub(crate) data: Option<String>,
}

/// The payload of a `content_block_delta` or of a `message_delta`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Delta {
    #[serde(default)]
    pub(crate) r#type: Option<String>,
    /// `text_delta`.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// `input_json_delta`: fragment of the arguments' JSON string.
    #[serde(default)]
    pub(crate) partial_json: Option<String>,
    /// `thinking_delta`.
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    /// `signature_delta`.
    #[serde(default)]
    pub(crate) signature: Option<String>,
    /// Carried by `message_delta`.
    #[serde(default)]
    pub(crate) stop_reason: Option<String>,
}

/// Declared usage.
///
/// The fields are optional `i64`s and not `u32`s: a server can send a negative
/// value, and a strict deserialization would then make the whole frame fail
/// (I-09).
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireUsage {
    #[serde(default)]
    input_tokens: Option<i64>,
    #[serde(default)]
    output_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<i64>,
    #[serde(default)]
    cache_read_input_tokens: Option<i64>,
}

impl WireUsage {
    /// Input tokens outside the cache.
    pub(crate) fn input(&self) -> u32 {
        clamp_tokens(self.input_tokens)
    }

    /// Jetons produits.
    pub(crate) fn output(&self) -> u32 {
        clamp_tokens(self.output_tokens)
    }

    /// Tokens written to the cache, when the provider declares it.
    pub(crate) fn cache_write(&self) -> Option<u32> {
        self.cache_creation_input_tokens
            .map(|brut| clamp_tokens(Some(brut)))
    }

    /// Tokens read from the cache, when the provider declares it.
    pub(crate) fn cache_read(&self) -> Option<u32> {
        self.cache_read_input_tokens
            .map(|brut| clamp_tokens(Some(brut)))
    }

    /// Does the frame carry usable information?
    ///
    /// `message_start` announces a partial usage; emitting it as is would make
    /// a counter flicker that has no meaning before the end.
    pub(crate) fn is_empty(&self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cache_creation_input_tokens.is_none()
            && self.cache_read_input_tokens.is_none()
    }
}

/// Brings a token count coming from the network into a `u32`.
///
/// Missing or negative counts as `0` — "not declared" — and an outsized value
/// saturates rather than overflowing silently (`as` is forbidden).
fn clamp_tokens(brut: Option<i64>) -> u32 {
    let valeur = brut.unwrap_or(0).max(0);
    u32::try_from(valeur).unwrap_or(u32::MAX)
}

/// Error carried in the stream, or returned by a failure status.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireError {
    #[serde(default)]
    pub(crate) r#type: Option<String>,
    #[serde(default)]
    pub(crate) message: Option<String>,
}

impl WireError {
    /// Showable message, never returning an empty string.
    pub(crate) fn describe(&self) -> String {
        match (&self.r#type, &self.message) {
            (Some(genre), Some(message)) if !message.is_empty() => format!("{message} ({genre})"),
            (_, Some(message)) if !message.is_empty() => message.clone(),
            (Some(genre), _) => genre.clone(),
            _ => "no details given".to_owned(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Model list
// ─────────────────────────────────────────────────────────────────────────────

/// Corps de `GET /v1/models`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ModelsResponse {
    /// Deliberately untyped: a malformed entry must not make the whole list
    /// fail.
    #[serde(default, deserialize_with = "crate::http::bounded_entries")]
    pub(crate) data: Vec<Value>,
    #[serde(default)]
    pub(crate) has_more: bool,
    #[serde(default)]
    pub(crate) last_id: Option<String>,
}

/// An entry of the model list.
#[derive(Debug, Deserialize)]
pub(crate) struct WireModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    /// Context window, in tokens.
    #[serde(default)]
    max_input_tokens: Option<u32>,
    #[serde(default)]
    capabilities: Option<Capabilities>,
}

/// What the model declares it can do.
#[derive(Debug, Default, Deserialize)]
struct Capabilities {
    #[serde(default)]
    thinking: Option<ThinkingCapability>,
    #[serde(default)]
    effort: Option<EffortCapability>,
}

/// Declaration of the thinking capability.
#[derive(Debug, Default, Deserialize)]
struct ThinkingCapability {
    #[serde(default)]
    supported: Option<bool>,
}

/// Declaration of the effort capability, level by level.
#[derive(Debug, Default, Deserialize)]
struct EffortCapability {
    #[serde(default)]
    supported: Option<bool>,
    #[serde(default)]
    low: Option<Supported>,
    #[serde(default)]
    medium: Option<Supported>,
    #[serde(default)]
    high: Option<Supported>,
    #[serde(default)]
    xhigh: Option<Supported>,
    #[serde(default)]
    max: Option<Supported>,
}

/// The `{"supported": bool}` flag repeated everywhere in this response.
#[derive(Debug, Default, Deserialize)]
struct Supported {
    #[serde(default)]
    supported: bool,
}

impl EffortCapability {
    /// The levels actually accepted, in the order of the scale.
    fn levels(&self) -> Vec<ReasoningEffort> {
        [
            (self.low.as_ref(), ReasoningEffort::Low),
            (self.medium.as_ref(), ReasoningEffort::Medium),
            (self.high.as_ref(), ReasoningEffort::High),
            (self.xhigh.as_ref(), ReasoningEffort::XHigh),
            (self.max.as_ref(), ReasoningEffort::Max),
        ]
        .into_iter()
        .filter_map(|(declare, niveau)| declare?.supported.then_some(niveau))
        .collect()
    }
}

impl From<WireModel> for ModelInfo {
    fn from(brut: WireModel) -> Self {
        let capacites = brut.capabilities.unwrap_or_default();

        // Reasoning is declared through two independent routes: a model that
        // accepts effort can reason, even if it declares no thinking mode.
        // Either one is enough.
        let reflexion = capacites
            .thinking
            .as_ref()
            .and_then(|t| t.supported)
            .unwrap_or(false);
        let effort = capacites
            .effort
            .as_ref()
            .and_then(|e| e.supported)
            .unwrap_or(false);
        let raisonnement = if capacites.thinking.is_some() || capacites.effort.is_some() {
            Support::known(reflexion || effort)
        } else {
            Support::Unknown
        };

        let mut fiche = Self::new(brut.id);
        if let Some(nom) = brut.display_name {
            fiche = fiche.with_display_name(nom);
        }
        if let Some(fenetre) = brut.max_input_tokens
            && fenetre > 0
        {
            fiche = fiche.with_context_window(fenetre);
        }
        fiche = fiche
            .with_reasoning_support(raisonnement)
            // The endpoint says nothing about these two. `Unknown` is the only
            // honest answer: every model of this provider accepts them in
            // practice, but "in practice" is not a declaration (I-12).
            .with_tool_support(Support::Unknown)
            .with_streaming_support(Support::Unknown);
        if let Some(niveaux) = capacites.effort.as_ref().map(EffortCapability::levels)
            && !niveaux.is_empty()
        {
            fiche = fiche.with_reasoning_efforts(niveaux);
        }
        // `cost` stays `None`: this endpoint publishes no price, and a price
        // copied from memory is a plausible and wrong value (I-12).
        fiche
    }
}

/// Parses the model list, entry by entry.
///
/// An unreadable entry is **ignored**, not fatal: an exotic model must not
/// make the twenty others invisible.
pub(crate) fn parse_models(reponse: ModelsResponse) -> Vec<ModelInfo> {
    let mut fiches = Vec::with_capacity(reponse.data.len());
    let mut ignorees = 0_usize;
    for entree in reponse.data {
        match serde_json::from_value::<WireModel>(entree) {
            Ok(brut) => fiches.push(ModelInfo::from(brut)),
            Err(_) => ignorees += 1,
        }
    }
    if ignorees > 0 {
        // The entry's content is not logged.
        tracing::debug!(ignorees, "unreadable entries in the model list");
    }
    fiches
}

// ─────────────────────────────────────────────────────────────────────────────
// Comptage de jetons
// ─────────────────────────────────────────────────────────────────────────────

/// Body of `POST /v1/messages/count_tokens`.
///
/// The type is not called `TokenCount`: this repository refuses a derived
/// `Debug` on a type whose name evokes a secret, and the check is on the name.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct CountResponse {
    #[serde(default)]
    input_tokens: Option<i64>,
}

impl CountResponse {
    /// The count, brought back into the realm of the possible.
    pub(crate) fn count(&self) -> u32 {
        clamp_tokens(self.input_tokens)
    }
}
