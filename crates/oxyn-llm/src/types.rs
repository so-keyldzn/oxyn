//! The vocabulary of an exchange with a model.
//!
//! These types are **provider-independent**: they are what `oxyn-ai`
//! handles, and it is each implementation of
//! [`LlmProvider`](crate::provider::LlmProvider) that translates them to its
//! protocol. They therefore carry no trace of OpenAI, Anthropic or Gemini.
//!
//! # What is masked in `Debug`, and why
//!
//! [`ChatMessage`] and [`ChatRequest`] mask the **content** in their `Debug`.
//! An outgoing message carries the context `oxyn-ai` assembled: at the
//! `Sampled` tier of [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md),
//! these are real rows from the user's database. A
//! `tracing::debug!("{req:?}")` would write them into a log file, on disk, in
//! clear — exactly the failure I-03 describes.
//!
//! [`ChatEvent`], conversely, derives a complete `Debug`: it is the model's
//! output, and it is precisely what needs to be seen when a stream misbehaves.
//! Whoever logs a stream of events must know the model can copy into it what
//! it was given.
//!
//! # The content of a message is not an instruction
//!
//! A table name, a column comment or a cell value can imitate an instruction.
//! They are **data**, including once in a prompt
//! ([`AI-PROVIDERS`](../../../docs/AI-PROVIDERS.md)). This crate only carries;
//! it is `oxyn-ai` that frames untrusted content, and the `PolicyGate` that
//! prevents any model output from executing (I-07).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::reasoning::{ReasoningBlock, ReasoningEffort};

/// Who speaks, and why a stream stopped.
///
/// Defined in [`oxyn_core::ai`] and re-exported here: they are **persisted**
/// with the conversation, and persistence must not depend on an HTTP client to
/// read them. A single definition in the repository, like
/// [`ProviderId`](crate::provider::ProviderId).
///
/// The translation from a protocol string, however, stays at each provider:
/// the core knows no protocol.
pub use oxyn_core::ai::{Role, StopReason};

/// A conversation turn.
///
/// The `Debug` is written by hand: it shows the role and the size, never the
/// text. See the module note.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Who is speaking.
    pub role: Role,
    /// The text. Empty is legitimate for an assistant turn that only calls
    /// tools.
    pub content: String,
    /// Tools the model asked to call in this turn. Always empty outside
    /// [`Role::Assistant`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Identifier of the call this message answers. Mandatory for
    /// [`Role::Tool`], absent everywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Reasoning blocks produced by the model during this turn.
    ///
    /// Always empty outside [`Role::Assistant`]. They are kept **as they are**
    /// and sent back at the next turn: that is what the protocols require when
    /// a reasoning turn precedes a tool call, and a rebuilt block makes the
    /// request refused ([`crate::reasoning`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning: Vec<ReasoningBlock>,
    /// Does this message end a **stable** prefix of the conversation?
    ///
    /// It is a caching hint, not an order: a provider that can reuse a prefix
    /// sets its marker here, the others ignore it. The natural target is the
    /// context assembled by `oxyn-ai`, which does not change from one turn to
    /// the next whereas the user's question does.
    ///
    /// Marking a message that **changes** at every turn is not an error, it is
    /// simply useless: the prefix will never be found again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub cache_breakpoint: bool,
}

/// Serialization predicate: omits a false flag.
///
/// `bool::then` does not fit here — `skip_serializing_if` wants a named
/// function taking a reference.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "signature imposed by serde's skip_serializing_if"
)]
const fn is_false(value: &bool) -> bool {
    !*value
}

impl ChatMessage {
    /// Builds a message of a given role.
    #[must_use]
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning: Vec::new(),
            cache_breakpoint: false,
        }
    }

    /// Framing instruction.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    /// User turn.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }

    /// The model's turn.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }

    /// Result of a tool, attached to the call that requested it.
    ///
    /// The identifier comes from [`ToolCall::id`]: without it, the model cannot
    /// link the response to its request when it started several.
    #[must_use]
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            reasoning: Vec::new(),
            cache_breakpoint: false,
        }
    }

    /// Attaches tool calls to an assistant turn.
    #[must_use]
    pub fn with_tool_calls(mut self, calls: Vec<ToolCall>) -> Self {
        self.tool_calls = calls;
        self
    }

    /// Attaches the reasoning blocks of an assistant turn.
    ///
    /// To pass **as they were received**, in order: it is the condition for
    /// the next turn to be accepted ([`crate::reasoning`]).
    #[must_use]
    pub fn with_reasoning(mut self, blocks: Vec<ReasoningBlock>) -> Self {
        self.reasoning = blocks;
        self
    }

    /// Marks this message as the end of a stable prefix.
    ///
    /// See [`ChatMessage::cache_breakpoint`].
    #[must_use]
    pub const fn cached(mut self) -> Self {
        self.cache_breakpoint = true;
        self
    }
}

impl fmt::Debug for ChatMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatMessage")
            .field("role", &self.role)
            .field("content", &Masked(self.content.len()))
            .field("tool_calls", &self.tool_calls.len())
            .field("tool_call_id", &self.tool_call_id)
            // Counted and not rendered: a reasoning block repeats the context
            // given to the model, and this message comes out of a conversation
            // whose content is masked just above.
            .field("reasoning", &self.reasoning.len())
            .field("cache_breakpoint", &self.cache_breakpoint)
            .finish()
    }
}

/// Marker of a masked field, rendered `<masked, N bytes>`.
struct Masked(usize);

impl fmt::Debug for Masked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<masked, {} bytes>", self.0)
    }
}

/// A tool offered to the model.
///
/// `parameters` is a JSON schema. This crate does not validate it: it carries
/// it. It is `oxyn-ai` that builds it — from the bus's `Command`s, and from
/// nothing else (I-01).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Name of the tool, as the model will have to call it.
    pub name: String,
    /// What the tool does, in one sentence meant for the model.
    pub description: String,
    /// JSON schema of the expected arguments.
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    /// Declares a tool.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// A tool call requested by the model.
///
/// **It is not an action.** It is a proposal: it becomes a `Command` carrying
/// `Actor::Agent` and goes through the `PolicyGate` before any effect (I-07).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Identifier given by the provider, to copy into the response.
    pub id: String,
    /// Name of the requested tool.
    pub name: String,
    /// Arguments, already parsed. The protocols carry them as a JSON string;
    /// decoding happens at the boundary, not at the caller's.
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Builds a tool call.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

/// What a model is asked.
///
/// The `Debug` is written by hand: messages are counted in it, not rendered.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Identifier of the model at the provider (`gpt-4o-mini`, `llama3.2`…).
    pub model: String,
    /// The conversation, in order.
    pub messages: Vec<ChatMessage>,
    /// Tools offered for this turn. Empty = no tool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    /// Temperature, when the caller wants to set it. `None` leaves the
    /// provider's default — which is not the same everywhere, and is not guessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Ceiling of produced tokens. `None` leaves the provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// The caller's preference for incremental rendering.
    ///
    /// [`LlmProvider::stream`](crate::provider::LlmProvider::stream) is today
    /// the only call path and always streams: this flag records the intent
    /// for a non-streamed path, should one appear. It disables nothing.
    #[serde(default = "default_true")]
    pub stream: bool,
    /// How much work the model is asked for. `None` leaves the provider's
    /// default, which is not the same everywhere.
    ///
    /// A provider that does not know this setting **omits** it; a model that
    /// explicitly refuses it produces an
    /// [`LlmError::Unsupported`](crate::error::LlmError::Unsupported). What
    /// never happens is that it goes out blindly: several endpoints reject the
    /// whole request on an unknown field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Ceiling of tokens the model can spend thinking.
    ///
    /// Distinct from [`max_tokens`](Self::max_tokens), which bounds **all** the
    /// production — thinking included at providers that bill thinking as
    /// output. A budget above the global ceiling is a contradiction the
    /// provider reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_budget_tokens: Option<u32>,
    /// Are the tool definitions a stable prefix?
    ///
    /// Same nature as [`ChatMessage::cache_breakpoint`]: a hint, not an order.
    /// Oxyn's tools come from the command bus and do not change from one turn
    /// to the next, which makes them an obvious target.
    #[serde(default, skip_serializing_if = "is_false")]
    pub cache_tools: bool,
}

/// Default value of [`ChatRequest::stream`] at deserialization.
const fn default_true() -> bool {
    true
}

impl ChatRequest {
    /// Builds a request on a model and a conversation.
    #[must_use]
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            stream: true,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            cache_tools: false,
        }
    }

    /// Offers tools to the model.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = tools;
        self
    }

    /// Sets the temperature.
    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Sets the ceiling on produced tokens.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Sets the reasoning effort.
    #[must_use]
    pub const fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
        self
    }

    /// Sets the thinking budget, in tokens.
    #[must_use]
    pub const fn with_reasoning_budget_tokens(mut self, tokens: u32) -> Self {
        self.reasoning_budget_tokens = Some(tokens);
        self
    }

    /// Declares the tool definitions as a stable prefix.
    #[must_use]
    pub const fn with_cached_tools(mut self) -> Self {
        self.cache_tools = true;
        self
    }

    /// Does the caller ask for reasoning, in one way or another?
    ///
    /// Used by providers that must refuse rather than omit: an effort requested
    /// and silently ignored bills a response that is not the one asked for.
    #[must_use]
    pub const fn wants_reasoning(&self) -> bool {
        self.reasoning_effort.is_some() || self.reasoning_budget_tokens.is_some()
    }

    /// Total number of content bytes sent.
    ///
    /// Used by the context size guards, until a real token count exists. It is
    /// **not** a token estimate: the bytes/tokens ratio depends on the model's
    /// tokenizer.
    #[must_use]
    pub fn content_bytes(&self) -> usize {
        self.messages.iter().map(|m| m.content.len()).sum()
    }
}

impl fmt::Debug for ChatRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatRequest")
            .field("model", &self.model)
            .field("messages", &self.messages.len())
            .field("content", &Masked(self.content_bytes()))
            .field(
                "tools",
                &self.tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
            )
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .field("stream", &self.stream)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("reasoning_budget_tokens", &self.reasoning_budget_tokens)
            .field("cache_tools", &self.cache_tools)
            .finish()
    }
}

/// What comes up from a generation stream.
///
/// The enumeration is `#[non_exhaustive]`: the protocols gain event types
/// (reasoning, citations, memory) and a caller that ignores a new one stays
/// correct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ChatEvent {
    /// Text fragment produced by the model.
    TextDelta(String),
    /// A tool call starts. Emitted once per call, as soon as the provider has
    /// given its identifier and its name.
    ToolCallStarted {
        /// Position of the call in the turn, as the provider numbers it. It is
        /// the reassembly key of the fragments.
        index: u32,
        /// Identifier to copy into [`ChatMessage::tool_result`].
        id: String,
        /// Name of the requested tool.
        name: String,
    },
    /// Fragment of the arguments of a tool call, to concatenate.
    ToolCallDelta {
        /// Position of the call concerned.
        index: u32,
        /// Piece of the arguments' JSON string, raw.
        arguments: String,
    },
    /// A tool call is complete and its arguments are parsed.
    ToolCallComplete(ToolCall),
    /// Fragment of **written-out** reasoning, to concatenate.
    ///
    /// Only arrives if the provider agrees to show the reasoning. An encrypted
    /// block produces no fragment: there is nothing to show.
    ReasoningDelta {
        /// Position of the block in the turn. Two reasoning blocks can follow
        /// each other around a tool call.
        index: u32,
        /// Raw text chunk.
        text: String,
    },
    /// A reasoning block is complete.
    ///
    /// To **keep as is** and put back into the assistant turn
    /// ([`ChatMessage::reasoning`]): without it, the next turn is refused by
    /// the providers that sign their blocks.
    ReasoningComplete {
        /// Position of the block in the turn.
        index: u32,
        /// The block, to carry without modifying it.
        block: ReasoningBlock,
    },
    /// Fragment of a refusal by the model, to concatenate.
    ///
    /// A refusal is not an error: the request succeeded, the model answered
    /// that it would not answer. Distinguishing it from a
    /// [`TextDelta`](Self::TextDelta) lets the interface not present it as an
    /// answer.
    RefusalDelta(String),
    /// Usage declared by the provider.
    ///
    /// The last four fields are `Option` and not `0`: "not declared" and "zero"
    /// are two different facts, and displaying "0 tokens read from cache"
    /// where the provider said nothing would suggest a cache that does not
    /// work.
    Usage {
        /// Input tokens billed, outside the cache.
        prompt_tokens: u32,
        /// Tokens produced.
        completion_tokens: u32,
        /// Tokens **written** to the prefix cache.
        cache_write_tokens: Option<u32>,
        /// Tokens **read** from the prefix cache. They are not in
        /// `prompt_tokens`: the input total is the sum of the three.
        cache_read_tokens: Option<u32>,
        /// Tokens spent thinking, when the provider isolates them.
        reasoning_tokens: Option<u32>,
    },
    /// End of the stream. Emitted **exactly once**, last.
    Done {
        /// Why the stream stops.
        stop_reason: StopReason,
    },
    /// Non-fatal or fatal incident reported in the stream.
    ///
    /// A stream can carry an error after having already produced text: that is
    /// why it is an event and not a return value.
    Error(String),
}

impl ChatEvent {
    /// Does this event end the stream?
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. })
    }
}

/// What is known of a model's capability.
///
/// Three states and not a `bool`, because most OpenAI-compatible endpoints
/// list their models **without** saying what they can do. Answering `false`
/// would hide an available feature; answering `true`, offer it then fail.
/// `Unknown` is shown in the interface — "nothing is simulated, nothing is
/// greyed out without a reason"
/// ([`ARCHITECTURE` §4.2](../../../docs/ARCHITECTURE.md)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Support {
    /// The provider announces it.
    Yes,
    /// The provider announces the opposite.
    No,
    /// The provider says nothing. **This is the default.**
    #[default]
    Unknown,
}

impl Support {
    /// Translates a known boolean.
    #[must_use]
    pub const fn known(value: bool) -> Self {
        if value { Self::Yes } else { Self::No }
    }

    /// Is the capability announced present?
    ///
    /// `Unknown` answers `false`: we do not promise what we do not know.
    #[must_use]
    pub const fn is_yes(&self) -> bool {
        matches!(self, Self::Yes)
    }

    /// Is the capability announced absent?
    ///
    /// `Unknown` answers `false`: we do not hide what we do not know.
    #[must_use]
    pub const fn is_no(&self) -> bool {
        matches!(self, Self::No)
    }

    /// Stable name, for display and audit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "oui",
            Self::No => "non",
            Self::Unknown => "inconnu",
        }
    }
}

impl fmt::Display for Support {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Price of a model, as the **provider** declares it.
///
/// No value of this type is hard-coded in Oxyn: a price copied from memory is
/// a plausible and wrong value, invisible at compile time as in tests (I-12).
/// This field is only filled when the provider's response carries it —
/// OpenRouter is today the only one to do so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// Cost of a million input tokens.
    pub input_per_million: f64,
    /// Cost of a million produced tokens.
    pub output_per_million: f64,
    /// Currency, as the provider documents it.
    pub currency: String,
}

impl Cost {
    /// Builds a price from values coming from the provider.
    #[must_use]
    pub fn new(
        input_per_million: f64,
        output_per_million: f64,
        currency: impl Into<String>,
    ) -> Self {
        Self {
            input_per_million,
            output_per_million,
            currency: currency.into(),
        }
    }
}

/// What is known of a model offered by a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Identifier to put in [`ChatRequest::model`].
    pub id: String,
    /// Showable name. Failing a provided name, it is the identifier.
    pub display_name: String,
    /// Size of the context window in tokens, when the provider declares it.
    /// `None` means "not declared" and **never** "unlimited".
    pub context_window: Option<u32>,
    /// Does the model accept tools?
    pub supports_tools: Support,
    /// Does the model accept incremental streaming?
    pub supports_streaming: Support,
    /// Can the model reason — effort, budget, or both?
    ///
    /// `Unknown` is the common case: most endpoints list their models without
    /// declaring anything. It is what decides whether a
    /// [`ChatRequest::reasoning_effort`] is omitted or refused.
    pub supports_reasoning: Support,
    /// Effort levels the model accepts, when the provider publishes them.
    /// Empty means "not declared", never "none".
    ///
    /// Ordered and without duplicates: it is a list shown to the user, and it
    /// must not reorder from one opening to the next.
    pub reasoning_efforts: Vec<ReasoningEffort>,
    /// Price declared by the provider, if it publishes one.
    pub cost: Option<Cost>,
}

impl ModelInfo {
    /// Builds a minimal record: an identifier, and nothing asserted.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            display_name: id.clone(),
            id,
            context_window: None,
            supports_tools: Support::Unknown,
            supports_streaming: Support::Unknown,
            supports_reasoning: Support::Unknown,
            reasoning_efforts: Vec::new(),
            cost: None,
        }
    }

    /// Gives a displayable name distinct from the identifier.
    #[must_use]
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = name.into();
        self
    }

    /// Declares the size of the context window.
    #[must_use]
    pub fn with_context_window(mut self, tokens: u32) -> Self {
        self.context_window = Some(tokens);
        self
    }

    /// Declares support for tools.
    #[must_use]
    pub fn with_tool_support(mut self, support: Support) -> Self {
        self.supports_tools = support;
        self
    }

    /// Declares support for streaming.
    #[must_use]
    pub fn with_streaming_support(mut self, support: Support) -> Self {
        self.supports_streaming = support;
        self
    }

    /// Declares support for reasoning.
    #[must_use]
    pub fn with_reasoning_support(mut self, support: Support) -> Self {
        self.supports_reasoning = support;
        self
    }

    /// Declares the accepted effort levels.
    ///
    /// The list is sorted and deduplicated: it is shown as is.
    #[must_use]
    pub fn with_reasoning_efforts(mut self, mut efforts: Vec<ReasoningEffort>) -> Self {
        efforts.sort_unstable();
        efforts.dedup();
        self.reasoning_efforts = efforts;
        self
    }

    /// Declares the price published by the provider.
    #[must_use]
    pub fn with_cost(mut self, cost: Cost) -> Self {
        self.cost = Some(cost);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_debug_does_not_show_its_content() {
        // The targeted failure: `tracing::debug!("{msg:?}")` writes a row of
        // the customer database into a log file (I-03).
        let msg = ChatMessage::user("client 4711, IBAN FR76 3000 6000 0112 3456 7890 189");
        let rendered = format!("{msg:?}");
        assert!(!rendered.contains("FR76"), "{rendered}");
        assert!(!rendered.contains("4711"), "{rendered}");
        assert!(rendered.contains("masked"), "{rendered}");
        assert!(
            rendered.contains("User"),
            "the role remains useful for diagnosis"
        );
    }

    #[test]
    fn a_request_debug_does_not_show_the_messages() {
        let req = ChatRequest::new(
            "gpt-4o-mini",
            vec![
                ChatMessage::system("you are a SQL assistant"),
                ChatMessage::user("SELECT * FROM patients WHERE hiv_status = true"),
            ],
        )
        .with_tools(vec![ToolSpec::new(
            "execute",
            "run a query",
            serde_json::json!({"type": "object"}),
        )]);

        let rendered = format!("{req:?}");
        assert!(!rendered.contains("hiv_status"), "{rendered}");
        assert!(!rendered.contains("patients"), "{rendered}");
        assert!(
            rendered.contains("gpt-4o-mini"),
            "the model remains visible"
        );
        assert!(rendered.contains("execute"), "tool names remain visible");
    }

    #[test]
    fn a_tool_message_carries_the_call_identifier() {
        let msg = ChatMessage::tool_result("call_42", "3 rows");
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_42"));
    }

    #[test]
    fn an_unknown_capability_neither_promises_nor_hides() {
        let unknown = Support::default();
        assert_eq!(unknown, Support::Unknown);
        assert!(!unknown.is_yes());
        assert!(!unknown.is_no());
        assert!(Support::known(true).is_yes());
        assert!(Support::known(false).is_no());
    }

    #[test]
    fn a_model_record_asserts_nothing_by_default() {
        let info = ModelInfo::new("llama3.2");
        assert_eq!(info.display_name, "llama3.2");
        assert_eq!(info.context_window, None);
        assert_eq!(info.supports_tools, Support::Unknown);
        assert_eq!(info.cost, None);
    }

    #[test]
    fn only_done_ends_a_stream() {
        assert!(
            ChatEvent::Done {
                stop_reason: StopReason::EndTurn
            }
            .is_terminal()
        );
        assert!(!ChatEvent::TextDelta("a".to_owned()).is_terminal());
        assert!(
            !ChatEvent::Error("boum".to_owned()).is_terminal(),
            "an error can precede other events"
        );
    }

    #[test]
    fn a_serialized_request_reads_back() {
        let req = ChatRequest::new("m", vec![ChatMessage::user("hello")]).with_max_tokens(64);
        let json = serde_json::to_string(&req).expect("serialization");
        let reread_request: ChatRequest = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(reread_request, req);
    }

    #[test]
    fn a_new_request_asks_for_neither_effort_nor_budget() {
        // The default must remain "what the provider usually does": imposing
        // an effort would bill a reasoning nobody asked for.
        let req = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        assert_eq!(req.reasoning_effort, None);
        assert_eq!(req.reasoning_budget_tokens, None);
        assert!(!req.wants_reasoning());
        assert!(!req.cache_tools);
    }

    #[test]
    fn reasoning_settings_survive_a_round_trip() {
        let req = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_reasoning_effort(ReasoningEffort::XHigh)
            .with_reasoning_budget_tokens(8192)
            .with_cached_tools();
        assert!(req.wants_reasoning());

        let json = serde_json::to_value(&req).expect("serialization");
        assert_eq!(json["reasoning_effort"], "xhigh");
        assert_eq!(json["reasoning_budget_tokens"], 8192);
        assert_eq!(json["cache_tools"], true);

        let reread_request: ChatRequest = serde_json::from_value(json).expect("deserialization");
        assert_eq!(reread_request, req);
    }

    #[test]
    fn a_missing_setting_does_not_go_on_the_wire() {
        // Several endpoints reject the whole request on an unknown field: a
        // `null` is not an omission.
        let req = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        let json = serde_json::to_value(&req).expect("serialization");
        assert!(json.get("reasoning_effort").is_none(), "{json}");
        assert!(json.get("reasoning_budget_tokens").is_none(), "{json}");
        assert!(json.get("cache_tools").is_none(), "{json}");
    }

    #[test]
    fn a_reasoning_block_attaches_to_the_assistant_turn() {
        let blocks = vec![
            ReasoningBlock::summarized("I count", Some("sig".to_owned())),
            ReasoningBlock::redacted("chiffre"),
        ];
        let msg = ChatMessage::assistant("42").with_reasoning(blocks.clone());
        assert_eq!(msg.reasoning, blocks);

        // The turn serializes and reads back identically: that is the
        // condition for the next turn to be accepted.
        let json = serde_json::to_string(&msg).expect("serialization");
        let reread_message: ChatMessage = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(reread_message.reasoning, blocks);
    }

    #[test]
    fn a_message_debug_does_not_show_the_reasoning() {
        // A reasoning block repeats what was given to the model — at the
        // `Sampled` tier, database rows (I-03).
        let msg = ChatMessage::assistant("ok").with_reasoning(vec![ReasoningBlock::summarized(
            "the patients table has an hiv_status column",
            None,
        )]);
        let rendered = format!("{msg:?}");
        assert!(!rendered.contains("hiv_status"), "{rendered}");
        assert!(rendered.contains("reasoning"), "{rendered}");
    }

    #[test]
    fn a_message_marked_stable_stays_so_after_serialization() {
        let msg = ChatMessage::system("schema context").cached();
        assert!(msg.cache_breakpoint);
        let json = serde_json::to_value(&msg).expect("serialization");
        assert_eq!(json["cache_breakpoint"], true);

        let ordinary = ChatMessage::user("and the duplicates?");
        let json = serde_json::to_value(&ordinary).expect("serialization");
        assert!(
            json.get("cache_breakpoint").is_none(),
            "a false flag does not go out: {json}"
        );
    }

    #[test]
    fn a_model_record_asserts_nothing_about_reasoning_by_default() {
        let info = ModelInfo::new("llama3.2");
        assert_eq!(info.supports_reasoning, Support::Unknown);
        assert!(info.reasoning_efforts.is_empty());

        let declared = ModelInfo::new("m")
            .with_reasoning_support(Support::Yes)
            .with_reasoning_efforts(vec![
                ReasoningEffort::High,
                ReasoningEffort::Low,
                ReasoningEffort::High,
            ]);
        assert_eq!(
            declared.reasoning_efforts,
            vec![ReasoningEffort::Low, ReasoningEffort::High],
            "the list is sorted and deduplicated: it is shown as is"
        );
    }

    #[test]
    fn an_undeclared_usage_is_distinct_from_zero() {
        let event = ChatEvent::Usage {
            prompt_tokens: 10,
            completion_tokens: 2,
            cache_write_tokens: None,
            cache_read_tokens: Some(0),
            reasoning_tokens: None,
        };
        let ChatEvent::Usage {
            cache_write_tokens,
            cache_read_tokens,
            ..
        } = event
        else {
            panic!("unexpected variant");
        };
        assert_eq!(cache_write_tokens, None, "the provider said nothing");
        assert_eq!(cache_read_tokens, Some(0), "the provider said zero");
    }
}
