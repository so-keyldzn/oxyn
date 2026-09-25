//! Conversations as they cross to the webview.
//!
//! # A conversation is a tree
//!
//! Each question-and-answer is a node. A follow-up is a child of the answer it
//! follows; regenerating or editing a question adds a **sibling** — another
//! version of that exchange — and the versions stay navigable (‹ 2/3 ›). What
//! the panel shows is one path through the tree, chosen per parent
//! ([`Selection`]).
//!
//! # One ordered stream per node
//!
//! [`AiEvent`] is what a node hears, in the order things happened; the channel
//! carries an [`AiUpdate`] naming the node. Exactly one of
//! [`AiEvent::Finished`] and [`AiEvent::Failed`] ends a run; a tool call is
//! announced ([`AiEvent::ToolCall`]) before its report
//! ([`AiEvent::ToolReported`]) because that is when it happened
//! ([UX-SPEC](../../../../../docs/UX-SPEC.md#le-panneau-montre-ce-qui-se-passe-y-compris-quand-rien-narrive)).

use std::fmt;

use oxyn_ai::external::session::{AuthKind, AuthOption};
use oxyn_ai::external::settings;
use oxyn_ai::{AgentContext, AgentOutcome};
use oxyn_core::ai::StopReason;
use oxyn_core::{Environment, ErrorClass, PrivacyTier, Provenance};
use serde::{Deserialize, Serialize};

use super::{DestinationChoice, ProviderReach};

/// A question for the assistant.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskRequest {
    pub connection: String,
    pub session: String,
    /// `None` starts a new conversation.
    #[serde(default)]
    pub thread: Option<String>,
    /// The node this question follows; `None` for the first exchange. An edit
    /// or a regeneration passes the parent of the node it revises.
    #[serde(default)]
    pub parent: Option<u32>,
    pub question: String,
    pub destination: DestinationChoice,
    /// A row sample the user approved for **this** question, if any.
    #[serde(default)]
    pub sample: Option<SampleApproval>,
    /// The objects the user named with `@`, in the order they were typed.
    /// Imposed on the context; they carry no row value.
    #[serde(default)]
    pub mentions: Vec<Mention>,
}

/// An object the user named with `@` in a question.
///
/// An address, never text to reparse: the label in the question is for the
/// reader, this is what the backend checks against the catalog. Naming is not
/// sending values — a row sample is a separate approval.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Mention {
    /// A table, view or collection — and one of its fields for `table.field`.
    Relation {
        address: crate::ipc::CatalogAddress,
        field: Option<String>,
    },
    /// A query saved in the workspace library, by its document id.
    SavedQuery { document: String },
}

/// A mention as the thread shows it: a chip, named from the catalog.
///
/// `label` is the object's name as the catalog — or the library, for a saved
/// query — gave it: a server-controlled string, rendered as text. `missing`
/// says the object is gone, which is claimed only on evidence: a listing that
/// was read and no longer holds it. An object whose schema was never read here
/// is not called missing.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionView {
    /// `table`, `view`, `collection`, `column`, `savedQuery` or `other`.
    pub kind: &'static str,
    pub label: String,
    pub mention: Mention,
    pub missing: bool,
}

// Object names: counted, like everything a conversation quotes.
impl fmt::Debug for MentionView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MentionView")
            .field("kind", &self.kind)
            .field("missing", &self.missing)
            .finish_non_exhaustive()
    }
}

// A field name and a document id: nothing a log needs.
impl fmt::Debug for Mention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Relation { field, .. } => f
                .debug_struct("Relation")
                .field("field", &field.is_some())
                .finish_non_exhaustive(),
            Self::SavedQuery { .. } => f.debug_struct("SavedQuery").finish_non_exhaustive(),
        }
    }
}

/// What the user ticked on the sample screen, and the grant it answers.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SampleApproval {
    /// The token the backend issued with the offer. Spent on presentation.
    pub request: String,
    /// The relation the offer named; checked, never trusted.
    pub source: crate::ipc::CatalogAddress,
    /// The ticked column names.
    pub columns: Vec<String>,
}

// A token and column names: none of it belongs in a log.
impl fmt::Debug for SampleApproval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SampleApproval")
            .field("columns", &self.columns.len())
            .finish_non_exhaustive()
    }
}

/// A row sample offered for approval — never a value.
///
/// Answers `ai_request_sample`, or rides on [`AiEvent::SampleRequested`] when
/// an agent asks; drawn by `AssistantSampleApproval` either way.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleRequest {
    /// The grant's token, or the agent's request: what the approval sends
    /// back. Never rendered.
    pub id: String,
    /// Who asked, by name, when an agent did; `None` when the user pinned the
    /// relation. The screen says it: a request the user did not start must
    /// never look like one they did.
    pub requested_by: Option<String>,
    /// The relation, written to be read.
    pub source: String,
    /// The address the approval sends back, unchanged.
    pub address: crate::ipc::CatalogAddress,
    /// How many rows would leave: the backend's bound, not an estimate.
    pub rows: u32,
    /// The columns offered, as the catalog reports them.
    pub fields: Vec<crate::ipc::RelationField>,
    /// Who would receive the sample, by name.
    pub destination: String,
    pub reach: ProviderReach,
}

impl fmt::Debug for SampleRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SampleRequest")
            .field("fields", &self.fields.len())
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

// The question may quote row values the user pasted: never in a log.
impl fmt::Debug for AskRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AskRequest")
            .field("thread", &self.thread)
            .field("parent", &self.parent)
            .field("question_bytes", &self.question.len())
            .field("destination", &self.destination)
            .field("sample", &self.sample.is_some())
            .field("mentions", &self.mentions.len())
            .finish_non_exhaustive()
    }
}

/// Where a question landed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskStarted {
    pub thread: String,
    pub node: u32,
}

/// A conversation in the history list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub title: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    /// Exchanges, all versions counted.
    pub exchanges: usize,
    pub running: bool,
}

/// What the launch removed from the workspace's assistant history, and by
/// which rule: the policy's own numbers, so the panel never restates them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrunedHistory {
    /// Conversations deleted, whole.
    pub conversations: u32,
    pub max_conversations: u32,
    pub max_age_days: Option<u32>,
    pub max_bytes: u64,
}

impl PrunedHistory {
    /// `None` when the prune removed nothing: there is nothing to say.
    #[must_use]
    pub fn of(
        report: oxyn_store::PruneReport,
        policy: oxyn_store::RetentionPolicy,
    ) -> Option<Self> {
        (!report.is_empty()).then_some(Self {
            conversations: report.conversations,
            max_conversations: policy.max_conversations,
            max_age_days: policy.max_age_days,
            max_bytes: policy.max_bytes,
        })
    }
}

/// A conversation whose connection was deleted: listed, never reopened or
/// changed from here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanThreadSummary {
    pub id: String,
    pub title: String,
    /// The connection's name when the thread was written; `None` if it was
    /// written without one.
    pub connection_name: Option<String>,
    pub updated_at_ms: u64,
    /// Exchanges, all versions counted.
    pub exchanges: usize,
}

/// Which version of an exchange is shown under a parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub parent: Option<u32>,
    pub node: u32,
}

/// One exchange, with what it streamed so far.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub id: u32,
    pub parent: Option<u32>,
    pub question: String,
    /// What the question named with `@`, in the order typed.
    pub mentions: Vec<MentionView>,
    pub events: Vec<AiEvent>,
}

/// A whole conversation, for opening it or taking it back after a reload.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadView {
    pub id: String,
    pub title: String,
    pub nodes: Vec<NodeView>,
    pub selections: Vec<Selection>,
    /// The node being answered, if any.
    pub running: Option<u32>,
}

/// What the channel carries: an event, and the node it belongs to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUpdate {
    pub node: u32,
    pub event: AiEvent,
}

/// Who answers, as the panel header says it — permanently, next to the tier.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    /// `provider` or `agent`.
    pub kind: &'static str,
    pub label: String,
    /// `None` for an external agent: Oxyn cannot know its model.
    pub model: Option<String>,
    pub reach: ProviderReach,
    /// What an external agent says it is, once started.
    pub agent_version: Option<String>,
}

/// What the tier let into the prompt, counted — never listed.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSummary {
    pub relations: usize,
    pub omitted_relations: usize,
    pub dropped_samples: usize,
    pub estimated_tokens: usize,
    /// Mentions nothing was sent for: unknown to the local catalog, a saved
    /// query that is gone or belongs to another connection, or past the cap.
    pub ignored_mentions: usize,
    /// Mentions named in the prompt but not described: over the budget.
    pub omitted_mentions: usize,
}

impl ContextSummary {
    /// `ignored_before` counts the mentions the host dropped before the gate —
    /// a saved query it could not read —, added to those the gate ignored.
    pub fn of(context: &AgentContext, ignored_before: usize) -> Self {
        Self {
            relations: context.relations().len(),
            omitted_relations: context.omitted_relations(),
            dropped_samples: context.dropped_samples(),
            estimated_tokens: context.estimated_tokens(),
            ignored_mentions: context.ignored_mentions().saturating_add(ignored_before),
            omitted_mentions: context.omitted_mentions(),
        }
    }
}

/// What became of a tool call.
///
/// `Denied` (the policy refused, nothing can unblock it) and `Cancelled` (the
/// conversation was stopped while it ran) are distinct on purpose; a user's
/// own refusal of an approval is a third fact the front records after `decide`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Completed,
    AwaitingApproval,
    Denied,
    Failed,
    Cancelled,
}

/// Where an external agent's own tool stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentToolStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl From<oxyn_ai::ExternalToolStatus> for AgentToolStatus {
    fn from(status: oxyn_ai::ExternalToolStatus) -> Self {
        match status {
            oxyn_ai::ExternalToolStatus::Running => Self::Running,
            oxyn_ai::ExternalToolStatus::Completed => Self::Completed,
            oxyn_ai::ExternalToolStatus::Failed => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Ending {
    /// `truncated`: the answer is not whole — the panel offers to continue.
    /// `cut` says what cut it, so the panel names the right limit.
    #[serde(rename_all = "camelCase")]
    Answered {
        turns: usize,
        truncated: bool,
        cut: Option<Cut>,
    },
    #[serde(rename_all = "camelCase")]
    Cancelled { turns: usize },
    /// The turn ceiling: neither a success nor a failure.
    #[serde(rename_all = "camelCase")]
    TurnLimit { turns: usize },
    /// The model declined to go on. Not an answer, not a failure.
    #[serde(rename_all = "camelCase")]
    Refused { turns: usize },
    /// The provider paused the turn on its side; continuing is the user's call.
    #[serde(rename_all = "camelCase")]
    Paused { turns: usize },
    /// An external agent reached its own request ceiling for the turn.
    AgentLimit,
    /// An ending this build cannot name. Never read as an answer.
    Unknown,
}

/// What cut an answer short, when it was.
///
/// Named rather than folded into « the token limit »: raising the token ceiling
/// does nothing for an answer that filled the context window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Cut {
    /// The provider's ceiling on generated tokens.
    TokenLimit,
    /// The model's context window: the input must shrink, not the ceiling rise.
    ContextWindow,
    /// The provider announced a failure after it began answering.
    ProviderError,
    /// Another reason this build does not name.
    Unknown,
}

impl Cut {
    fn of(stop: &StopReason) -> Self {
        match stop {
            StopReason::MaxTokens => Self::TokenLimit,
            StopReason::ContextWindowExceeded => Self::ContextWindow,
            StopReason::ProviderError => Self::ProviderError,
            _ => Self::Unknown,
        }
    }
}

impl Ending {
    pub fn of(outcome: &AgentOutcome) -> Self {
        match outcome {
            AgentOutcome::Answered {
                turns,
                truncated,
                stop,
                ..
            } => Self::Answered {
                turns: *turns,
                truncated: *truncated,
                cut: truncated.then(|| Cut::of(stop)),
            },
            AgentOutcome::Cancelled { turns } => Self::Cancelled { turns: *turns },
            AgentOutcome::TurnLimit { turns } => Self::TurnLimit { turns: *turns },
            AgentOutcome::Refused { turns, .. } => Self::Refused { turns: *turns },
            AgentOutcome::Paused { turns, .. } => Self::Paused { turns: *turns },
            _ => Self::Unknown,
        }
    }
}

/// Why a run broke off, as data: the front decides what it offers from this,
/// never from the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureCategory {
    /// The tier or the declaration forbids it: asking again changes nothing.
    Refused,
    /// The model endpoint failed. Asking again repeats the model call only.
    Provider,
    /// Oxyn could not prepare the conversation (catalog, keyring).
    Setup,
    /// The agent's program is not where its declaration says.
    AgentNotFound,
    /// The agent wants its user signed in.
    AgentSignIn,
    /// The agent speaks another protocol version.
    AgentIncompatible,
    /// The agent's process stopped.
    AgentExited,
    /// The agent did not finish starting in time, and Oxyn stopped it.
    AgentTimedOut,
    /// The agent answered with an error.
    Agent,
}

/// How to sign in with an agent, as the panel offers it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInHelp {
    pub agent: String,
    pub methods: Vec<SignInMethod>,
    /// The documented terminal command for a known agent, when the agent
    /// itself offers nothing Oxyn can run.
    pub terminal_command: Option<String>,
}

/// One way to sign in.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// `agent`: Oxyn asks the agent to do it. `terminal`: the user runs
    /// `command` in a terminal; Oxyn does not.
    pub kind: &'static str,
    pub command: Option<String>,
}

impl SignInMethod {
    /// `launch` is the declared command line, for a terminal method.
    pub fn of(option: &AuthOption, launch: &[String]) -> Self {
        let (kind, command) = match &option.kind {
            AuthKind::Terminal { args } => (
                "terminal",
                Some(
                    launch
                        .iter()
                        .chain(args)
                        .map(|word| shell_quote(word))
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
            ),
            _ => ("agent", None),
        };
        Self {
            id: option.id.clone(),
            name: option.name.clone(),
            description: option.description.clone(),
            kind,
            command,
        }
    }
}

/// A word as a POSIX shell reads it back unchanged.
///
/// The command is shown to be copied into a terminal: an unquoted space or `$`
/// would run something else than what is displayed.
pub fn shell_quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// Why the assistant starts over without the earlier exchanges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryReset {
    /// The connection's tier changed: what was said under the other tier does
    /// not go out under this one (I-04).
    TierChanged,
    /// The external agent was started again, and remembers nothing.
    AgentRestarted,
    /// Another destination answers: the earlier answers were not its own.
    DestinationChanged,
    /// An approved row sample serves one question: the exchange that used it
    /// is not remembered, and it does not remember the ones before (ADR-0006).
    SampleNotKept,
    /// The exchange this one follows was read back from the workspace. What a
    /// model was told is not written — the store keeps no tool arguments and
    /// no results — so the conversation starts again from a fresh context.
    Restarted,
}

/// An amount and its ISO 4217 currency, as the agent gives them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Money {
    pub amount: f64,
    pub currency: String,
}

/// One step of an agent's plan, as the protocol gives it.
///
/// No identifier: in v1 an entry's identity is its position, and two sends may
/// change both its text and its state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub content: String,
    pub priority: PlanPriority,
    pub status: PlanStatus,
}

impl PlanEntry {
    pub(crate) fn of(step: &oxyn_ai::PlanStep<'_>) -> Self {
        Self {
            content: step.content.to_owned(),
            priority: match step.priority {
                oxyn_ai::PlanPriority::High => PlanPriority::High,
                oxyn_ai::PlanPriority::Medium => PlanPriority::Medium,
                oxyn_ai::PlanPriority::Low => PlanPriority::Low,
            },
            status: match step.status {
                oxyn_ai::PlanStatus::Pending => PlanStatus::Pending,
                oxyn_ai::PlanStatus::InProgress => PlanStatus::InProgress,
                oxyn_ai::PlanStatus::Completed => PlanStatus::Completed,
            },
        }
    }
}

/// How much the agent says a step matters.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanPriority {
    High,
    Medium,
    Low,
}

/// Where a step stands. Three states; the protocol has no abandoned step.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

/// An external agent's modes and options, **in full**, as it last declared
/// them. One shape and one conversion for the `agentSettings` event and for
/// the answer to a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettingsView {
    pub modes: Vec<AgentChoice>,
    pub current_mode: Option<String>,
    pub options: Vec<AgentOption>,
}

impl AgentSettingsView {
    pub(crate) fn of(declared: &settings::AgentSettings) -> Self {
        Self {
            modes: declared.modes.iter().map(AgentChoice::of).collect(),
            current_mode: declared.current_mode.clone(),
            options: declared.options.iter().map(AgentOption::of).collect(),
        }
    }
}

/// A mode, or one value an option may take.
///
/// `id` is what goes back to the agent, `name` what the user reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChoice {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

impl AgentChoice {
    fn of(choice: &settings::AgentChoice) -> Self {
        Self {
            id: choice.id.clone(),
            name: choice.name.clone(),
            description: choice.description.clone(),
        }
    }
}

/// One option an external agent lets the user set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// `mode`, `model`, `modelConfig`, `thoughtLevel` or `other`. A selector
    /// reads its own word and nothing else.
    pub category: &'static str,
    pub value: AgentOptionValue,
}

impl AgentOption {
    fn of(option: &settings::AgentOption) -> Self {
        Self {
            id: option.id.clone(),
            name: option.name.clone(),
            description: option.description.clone(),
            category: match option.category {
                settings::OptionCategory::Mode => "mode",
                settings::OptionCategory::Model => "model",
                settings::OptionCategory::ModelConfig => "modelConfig",
                settings::OptionCategory::ThoughtLevel => "thoughtLevel",
                // Never read as a known category (see `OptionCategory::Other`).
                _ => "other",
            },
            value: match &option.value {
                settings::OptionValue::Select { current, choices } => AgentOptionValue::Select {
                    current: current.clone(),
                    choices: choices.iter().map(AgentChoice::of).collect(),
                },
                settings::OptionValue::Boolean(on) => AgentOptionValue::Boolean { on: *on },
            },
        }
    }
}

/// An option's value, in the agent's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentOptionValue {
    #[serde(rename_all = "camelCase")]
    Select {
        current: String,
        choices: Vec<AgentChoice>,
    },
    #[serde(rename_all = "camelCase")]
    Boolean { on: bool },
}

/// A change the user asks of the conversation's agent: `{mode}` or
/// `{option, value}`. Identifiers are the agent's, checked by the backend
/// against what the agent last declared — never trusted from the front.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum AgentSettingChange {
    Mode {
        mode: String,
    },
    Option {
        option: String,
        value: AgentSettingValue,
    },
}

/// A choice identifier, or on and off.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AgentSettingValue {
    Choice(String),
    Boolean(bool),
}

impl AgentSettingChange {
    pub(crate) fn into_change(self) -> settings::SettingChange {
        match self {
            Self::Mode { mode } => settings::SettingChange::Mode { id: mode },
            Self::Option { option, value } => settings::SettingChange::Option {
                id: option,
                value: match value {
                    AgentSettingValue::Choice(choice) => settings::SettingValue::Choice(choice),
                    AgentSettingValue::Boolean(on) => settings::SettingValue::Boolean(on),
                },
            },
        }
    }
}

/// What became of a settings change.
///
/// `sent` carries the settings **as the agent answered**, not as asked: its
/// answer to `set_config_option` is its whole option list, and a mode is
/// taken on its confirmation only if it still declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentSettingAnswer {
    #[serde(rename_all = "camelCase")]
    Sent { settings: AgentSettingsView },
    #[serde(rename_all = "camelCase")]
    Refused {
        /// `questionInProgress`, `unknownMode`, `unknownOption`,
        /// `unknownValue` or `byAgent`.
        reason: &'static str,
        /// For `byAgent`: the agent's error code — never its words.
        code: Option<i32>,
    },
}

impl AgentSettingAnswer {
    pub(crate) const fn refused(refusal: &settings::SettingRefused) -> Self {
        let (reason, code) = match refusal {
            settings::SettingRefused::QuestionInProgress => ("questionInProgress", None),
            settings::SettingRefused::UnknownMode => ("unknownMode", None),
            settings::SettingRefused::UnknownOption => ("unknownOption", None),
            settings::SettingRefused::UnknownValue => ("unknownValue", None),
            settings::SettingRefused::ByAgent { code } => ("byAgent", Some(*code)),
        };
        Self::Refused { reason, code }
    }
}

/// One step of a conversation, in the order it happened.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AiEvent {
    /// The question that starts this run.
    #[serde(rename_all = "camelCase")]
    Question {
        text: String,
        /// What the question named with `@`: the thread draws them as chips.
        mentions: Vec<MentionView>,
    },
    /// The conversation is assembled. Sent before the first turn.
    #[serde(rename_all = "camelCase")]
    Started {
        destination: Destination,
        tier: PrivacyTier,
        /// `None` when the question left alone — a follow-up in an external
        /// agent's session, which already holds the structure.
        context: Option<ContextSummary>,
        /// What will sign a proposal opened from this answer
        /// ([ADR-0023](../../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
        /// `None` for an external agent, which `Provenance` cannot describe yet.
        provenance: Option<Provenance>,
    },
    /// The earlier exchanges are not part of this run.
    #[serde(rename_all = "camelCase")]
    MemoryReset { reason: MemoryReset },
    /// Oxyn reads from the server the structure the assistant needs and its
    /// catalog does not hold yet — metadata only, through the bus
    /// ([ADR-0036](../../../../../docs/adr/0036-l-assistant-complete-le-catalogue.md)).
    /// Sent before the first read; not sent when nothing was missing.
    CatalogReading,
    /// That reading is over. Counts only: no object name reaches the panel
    /// through it.
    #[serde(rename_all = "camelCase")]
    CatalogRead {
        /// Levels listed: the server, a catalog's schemas, a schema's relations.
        listed: usize,
        /// Relations whose fields, indexes and keys were read.
        described: usize,
        /// Reads that failed or were refused: their level stays unread.
        failed: usize,
        /// Relations the assistant sees by name only.
        not_loaded: usize,
        /// Schemas whose relations were never listed.
        unlisted: usize,
        /// `deadline` or `cancelled` when the reading stopped before its end.
        stopped: Option<&'static str>,
    },
    /// This conversation is not being written to the workspace: the question
    /// still leaves, and the panel says so once.
    NotSaved,
    /// Reopened from the workspace, and older exchanges of this conversation
    /// were not loaded: the panel holds a bounded number of them.
    OlderNotLoaded,
    /// Reopened from the workspace: this exchange used an approved sample, so
    /// its answer was never written.
    AnswerNotKept,
    /// A tool call as the workspace kept it — what the user read of it.
    ///
    /// Apart from `toolCall`, and not folded into it: a stored call has no
    /// environment, no connection and no « writes » flag, because the store
    /// has no column for them. Folding it in would mean inventing three facts
    /// per call, and « does not write » is the one no reader should invent.
    #[serde(rename_all = "camelCase")]
    RestoredCall {
        tool: String,
        /// The report as it was shown.
        summary: String,
        statement: Option<String>,
        status: ToolStatus,
        error_class: Option<&'static str>,
        /// The call returned rows the panel showed, and the workspace kept
        /// none of them: the panel says so rather than drawing an empty grid.
        rows_not_kept: bool,
    },
    /// The user approved a row sample for this question. Its size only:
    /// neither a value nor a column name is kept in a conversation.
    #[serde(rename_all = "camelCase")]
    SampleApproved { rows: u32, columns: u32 },
    /// An agent asked for a row sample, through `call`: the approval screen
    /// opens, and the call waits for `ai_answer_sample`. Nothing is read yet.
    #[serde(rename_all = "camelCase")]
    SampleRequested { call: u32, request: SampleRequest },
    /// The agent's request was answered — or expired, or was withdrawn with
    /// its call — and the screen closes. `approved` only when columns were
    /// ticked; the rows read, if any, are said by `sampleApproved`.
    #[serde(rename_all = "camelCase")]
    SampleAnswered { request: String, approved: bool },
    #[serde(rename_all = "camelCase")]
    TurnStarted { turn: usize, max_turns: usize },
    /// A fragment of the model's reasoning: shown apart, folded.
    #[serde(rename_all = "camelCase")]
    ThinkingDelta { text: String },
    /// The model reasoned, and the provider shows none of it.
    ThinkingRedacted,
    /// The reasoning is over, measured here so a reload keeps the duration.
    #[serde(rename_all = "camelCase")]
    ThinkingEnded { elapsed_ms: u64 },
    /// A fragment of the answer: to append, not to show alone.
    #[serde(rename_all = "camelCase")]
    TextDelta { text: String },
    /// The model is writing a tool call; nothing is translated nor submitted.
    #[serde(rename_all = "camelCase")]
    ToolDraft { index: u32, tool: String },
    /// A fragment of a drafted call's arguments: partial JSON, to append.
    #[serde(rename_all = "camelCase")]
    ToolArguments { index: u32, fragment: String },
    /// A command leaves for the bus. Shown before its report.
    #[serde(rename_all = "camelCase")]
    ToolCall {
        /// Correlates the call with its approval and report, within a run.
        call: u32,
        tool: String,
        /// The audit-log name of the command.
        command: String,
        /// The exact statement, when the command carries one.
        statement: Option<String>,
        /// The connection **name**, never its identifier.
        connection: String,
        environment: Option<Environment>,
        mutating: bool,
    },
    /// The policy held the call back: the user decides, through `decide`.
    #[serde(rename_all = "camelCase")]
    ApprovalRequested {
        call: u32,
        /// The pending command's id, to pass to `decide`. Never rendered.
        approval: String,
        reason: String,
        /// The statement as the executor will run it, reclassified.
        statement: String,
        connection: String,
        environment: Option<Environment>,
        /// `agent`: approving it does not make it the user's command.
        actor: &'static str,
        /// When the request expires unanswered, in milliseconds since the
        /// epoch: the bound of the wait, for the card to state.
        expires_at_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    ToolReported {
        call: u32,
        status: ToolStatus,
        /// The server's words, whole: this is what the user reads.
        detail: String,
        error_class: Option<&'static str>,
        /// The model received less than `detail`, under the connection's tier.
        withheld: bool,
        /// Rows produced or affected, when the command completed and measured
        /// them. Structured, never parsed from `detail`.
        rows: Option<u64>,
        /// The result the executor retained, to read by pages with
        /// `open_retained_result` and `read_result_page` on the conversation's
        /// connection — the grid of a console, for the user alone.
        ///
        /// `None` when the command left no result, or targeted a connection
        /// other than the conversation's. Nothing of it reaches the model: the
        /// runtime's `DispatchOutcome` has no such field (ADR-0030 §4).
        result: Option<String>,
    },
    /// A call refused before the bus: nothing ran.
    #[serde(rename_all = "camelCase")]
    CallRejected { tool: String, error: String },
    /// The agent's plan, **in full**.
    ///
    /// Replaces the previous one; the protocol sends no delta and gives entries
    /// no identifier, so their only identity is their position.
    #[serde(rename_all = "camelCase")]
    Plan { entries: Vec<PlanEntry> },
    /// An external agent's modes and options, **in full**, as it last declared
    /// them. Sent when a question starts and whenever the agent changes them;
    /// replaces the previous one.
    AgentSettings(AgentSettingsView),
    /// An external agent's own tool — not an Oxyn command. Kind only: the
    /// agent's title may quote a path of the machine.
    #[serde(rename_all = "camelCase")]
    AgentTool {
        id: String,
        /// The kind of tool, in one stable word.
        tool: Option<&'static str>,
        status: AgentToolStatus,
    },
    /// An external agent asked to act on the machine; Oxyn refused.
    #[serde(rename_all = "camelCase")]
    PermissionRefused {
        /// The kind of action asked for.
        action: &'static str,
        reason: &'static str,
    },
    /// Tokens a provider declared for one turn. `None` is « not declared ».
    #[serde(rename_all = "camelCase")]
    Usage {
        input: u32,
        output: u32,
        cache_read: Option<u32>,
        cache_write: Option<u32>,
    },
    /// An external agent's context window, and its cumulative cost if given.
    #[serde(rename_all = "camelCase")]
    ContextWindow {
        used: u64,
        size: u64,
        cost: Option<Money>,
    },
    #[serde(rename_all = "camelCase")]
    Finished { ending: Ending },
    #[serde(rename_all = "camelCase")]
    Failed {
        message: String,
        category: FailureCategory,
        /// Whether « ask again » may be offered. Decided by the backend from
        /// the error's class and from whether a write ran — never by the front
        /// from the category or the words (I-13).
        retryable: bool,
        /// For `agentSignIn`: how to sign in.
        sign_in: Option<SignInHelp>,
        /// For `agentNotFound`: where a program of that name was found instead.
        found_elsewhere: Option<String>,
        /// For `agentExited`: what the process said on its way out, when it
        /// said it in time.
        exit: Option<super::AgentExit>,
    },
}

/// The driver's error family, as a word the front can switch on.
pub const fn error_class(class: ErrorClass) -> &'static str {
    match class {
        ErrorClass::Transient => "transient",
        ErrorClass::Permanent => "permanent",
        ErrorClass::Ambiguous => "ambiguous",
        // Not announced as retryable: inviting a retry on something unclassified
        // is how a duplicate row appears (I-13).
        _ => "unknown",
    }
}

impl AiEvent {
    pub(crate) fn agent_settings(declared: &settings::AgentSettings) -> Self {
        Self::AgentSettings(AgentSettingsView::of(declared))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settings_change_is_read_in_its_two_shapes_and_nothing_else() {
        let read = |json: &str| {
            serde_json::from_str::<AgentSettingChange>(json).map(AgentSettingChange::into_change)
        };
        assert_eq!(
            read(r#"{"mode":"plan"}"#).ok(),
            Some(settings::SettingChange::Mode {
                id: "plan".to_owned()
            })
        );
        assert_eq!(
            read(r#"{"option":"effort","value":"low"}"#).ok(),
            Some(settings::SettingChange::Option {
                id: "effort".to_owned(),
                value: settings::SettingValue::Choice("low".to_owned()),
            })
        );
        assert_eq!(
            read(r#"{"option":"fast","value":true}"#).ok(),
            Some(settings::SettingChange::Option {
                id: "fast".to_owned(),
                value: settings::SettingValue::Boolean(true),
            })
        );
        // Neither both at once, nor a value of another type.
        assert!(read(r#"{"mode":"plan","option":"effort","value":"low"}"#).is_err());
        assert!(read(r#"{"option":"effort","value":3}"#).is_err());
        assert!(read(r#"{"option":"effort"}"#).is_err());
    }

    #[test]
    fn a_refused_change_says_why_and_carries_no_words_of_the_agent() {
        assert_eq!(
            serde_json::to_string(&AgentSettingAnswer::refused(
                &settings::SettingRefused::ByAgent { code: -32000 }
            ))
            .expect("serializable"),
            r#"{"type":"refused","reason":"byAgent","code":-32000}"#
        );
        assert_eq!(
            serde_json::to_string(&AgentSettingAnswer::Sent {
                settings: AgentSettingsView::of(&settings::AgentSettings {
                    current_mode: Some("plan".to_owned()),
                    ..Default::default()
                })
            })
            .expect("serializable"),
            r#"{"type":"sent","settings":{"modes":[],"currentMode":"plan","options":[]}}"#
        );
    }

    #[test]
    fn an_agents_settings_cross_as_the_front_reads_them() {
        let declared = settings::AgentSettings {
            modes: vec![settings::AgentChoice {
                id: "plan".to_owned(),
                name: "Plan".to_owned(),
                description: None,
            }],
            current_mode: Some("plan".to_owned()),
            modes_locked: false,
            options: vec![
                settings::AgentOption {
                    id: "effort".to_owned(),
                    name: "Effort".to_owned(),
                    description: None,
                    category: settings::OptionCategory::ThoughtLevel,
                    value: settings::OptionValue::Select {
                        current: "high".to_owned(),
                        choices: vec![settings::AgentChoice {
                            id: "high".to_owned(),
                            name: "High".to_owned(),
                            description: Some("Slower".to_owned()),
                        }],
                    },
                },
                settings::AgentOption {
                    id: "fast".to_owned(),
                    name: "Fast".to_owned(),
                    description: None,
                    category: settings::OptionCategory::Other,
                    value: settings::OptionValue::Boolean(false),
                },
            ],
        };
        let json =
            serde_json::to_string(&AiEvent::agent_settings(&declared)).expect("serializable");
        assert_eq!(
            json,
            r#"{"kind":"agentSettings","modes":[{"id":"plan","name":"Plan","description":null}],"currentMode":"plan","options":[{"id":"effort","name":"Effort","description":null,"category":"thoughtLevel","value":{"type":"select","current":"high","choices":[{"id":"high","name":"High","description":"Slower"}]}},{"id":"fast","name":"Fast","description":null,"category":"other","value":{"type":"boolean","on":false}}]}"#
        );
    }

    #[test]
    fn events_are_discriminated_by_kind() {
        let json = serde_json::to_string(&AiUpdate {
            node: 3,
            event: AiEvent::Finished {
                ending: Ending::Answered {
                    turns: 2,
                    truncated: true,
                    cut: Some(Cut::ContextWindow),
                },
            },
        })
        .expect("serializable");
        assert_eq!(
            json,
            r#"{"node":3,"event":{"kind":"finished","ending":{"type":"answered","turns":2,"truncated":true,"cut":"contextWindow"}}}"#
        );
    }

    #[test]
    fn a_terminal_sign_in_is_shown_as_a_command_that_reads_back_the_same() {
        let option = AuthOption {
            id: "claude-ai-login".into(),
            name: "Claude Subscription".into(),
            description: None,
            kind: AuthKind::Terminal {
                args: vec!["--cli".into(), "auth".into(), "login".into()],
            },
        };
        let launch = vec![
            "/Users/some one/.local/bin/npx".to_owned(),
            "-y".to_owned(),
            "@agentclientprotocol/claude-agent-acp@0.78.0".to_owned(),
        ];
        let method = SignInMethod::of(&option, &launch);
        assert_eq!(method.kind, "terminal");
        assert_eq!(
            method.command.as_deref(),
            Some(
                "'/Users/some one/.local/bin/npx' -y @agentclientprotocol/claude-agent-acp@0.78.0 --cli auth login"
            )
        );
        assert_eq!(shell_quote("it's $HOME"), r"'it'\''s $HOME'");
    }
}
