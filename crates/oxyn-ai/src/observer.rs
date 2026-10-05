//! What a conversation shows **while** it unfolds.
//!
//! # The defect this module closes
//!
//! [`AgentRuntime::run`](crate::runtime::AgentRuntime::run) runs several turns
//! and several tool calls before returning an [`AgentOutcome`]. An interface
//! wired to that single value has nothing to display in the meantime: an
//! hourglass, then a wall of text. Yet "an agent working silently for eight
//! turns is indistinguishable from a stuck agent"
//! ([UX-SPEC](../../../docs/UX-SPEC.md)) — and the user who cannot tell the
//! two apart kills the process.
//!
//! An [`AgentObserver`] is therefore notified at every moment something
//! becomes visible: a turn starts, a fragment of answer arrives, a command
//! leaves, a report comes back, the conversation ends.
//!
//! # The observer cannot become an input channel
//!
//! It is the property that matters. [`AgentObserver::observe`] returns
//! **nothing**, and an [`AgentEvent`] borrows the conversation at no point:
//! there is no path by which what an observer saw — a server's whole message,
//! for instance — could reach the next turn's prompt. What goes into a prompt
//! always goes through
//! [`ContextBuilder::build`](crate::context::ContextBuilder::build) and
//! `ToolOutcome::from_dispatch`, never through here (I-04).
//!
//! # What goes to the user is not what goes to the model
//!
//! The user has the right to read their server's whole message: it is their
//! database. The model does not — the connection's tier decides, and under
//! `Metadata` the message does not leave (see [`crate::failure`]). The observer
//! therefore receives the **whole facts**, in a [`DispatchOutcome`], and not
//! the filtered text that went to the provider.
//!
//! The two can differ, and UX-SPEC requires the panel to say so: "hiding the
//! gap would pass a poorly informed answer off as a wrong answer". The gap is
//! therefore carried by the event itself
//! ([`AgentEvent::CommandReported::withheld`]), observed and not re-deduced — a
//! caller that replayed the filter's rule would diverge from it the day it
//! changes.
//!
//! # What an observer is not allowed to do
//!
//! Block. It is called from the asynchronous loop, between two stream
//! fragments; an implementation that waits freezes the conversation, and with
//! it the cancellation the user is trying to click. The signature enforces it
//! as much as a type can: [`observe`](AgentObserver::observe) is not `async`,
//! so nothing is awaited in it; it takes `&self`, so nothing is write-locked
//! in it by construction; and [`AgentEvent`] is borrowed, so keeping it means
//! copying it. What an implementation does right is push into a channel and
//! return.

use oxyn_core::ConnectionId;

use crate::call::CallId;
use crate::error::AiError;
use crate::runtime::{AgentOutcome, DispatchOutcome};

/// Who watches a conversation unfold.
///
/// Implemented by the interface, which pushes every event towards its panel.
/// The silent implementation is provided for `()`: a caller that observes
/// nothing passes `&()`, and there is therefore no second `run` method to
/// maintain — a duplicate API is a path that ends up less audited than the
/// other.
///
/// # The implementation's contract
///
/// 1. **return immediately**: no I/O, no waiting, no contended lock. The method
///    is not `async` precisely so that no `await` can slip into it; what
///    remains possible — opening a file, locking a mutex held elsewhere —
///    freezes the conversation and the cancellation with it (I-05);
/// 2. **decide nothing**: an event is a notification, not a negotiation. The
///    method returns nothing, deliberately;
/// 3. **do not log an event whole**: it carries database content, including a
///    server's whole message (I-03).
pub trait AgentObserver: Send + Sync {
    /// Signals that something has just become visible.
    ///
    /// Called from the conversation's loop, including between two fragments of
    /// a stream. See the trait's contract.
    fn observe(&self, event: AgentEvent<'_>);
}

/// The silent observer.
///
/// What a caller with no interface to feed passes — the tests, a future
/// command-line use. The compiler erases the calls.
impl AgentObserver for () {
    fn observe(&self, _event: AgentEvent<'_>) {}
}

/// Tokens consumed, as the provider declares them.
///
/// `None` means "not declared", never zero: a cache whose use we do not know
/// is not an unused cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenUsage {
    /// Billed input tokens.
    pub input: u32,
    /// Produced tokens.
    pub output: u32,
    /// Input tokens read back from the provider's cache.
    pub cache_read: Option<u32>,
    /// Input tokens written to the provider's cache.
    pub cache_write: Option<u32>,
    /// Reasoning tokens, when the provider distinguishes them. `None` means
    /// "not declared", never "zero".
    pub reasoning: Option<u32>,
}

/// A step of an external agent's plan.
///
/// Without an identifier: in v1 of the protocol, an entry's only identity is
/// its position. Two successive sends can change its text as well as its
/// state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanStep<'a> {
    /// What the step says, as the agent writes it.
    pub content: &'a str,
    /// The importance the agent gives it.
    pub priority: PlanPriority,
    /// Where it stands.
    pub status: PlanStatus,
}

/// A step's importance, as the protocol names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanPriority {
    /// High.
    High,
    /// Medium.
    Medium,
    /// Low.
    Low,
}

/// A step's progress. Three states, and not one more: the protocol has no
/// abandoned step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    /// Not started yet.
    Pending,
    /// In progress.
    InProgress,
    /// Done.
    Completed,
}

/// Where an external agent's tool stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExternalToolStatus {
    /// Announced, not launched yet: arguments in progress or permission
    /// awaited.
    Pending,
    /// In progress.
    Running,
    /// Done.
    Completed,
    /// Failed — or refused by Oxyn.
    Failed,
}

/// What has just become visible in a conversation.
///
/// Borrowed rather than owned: a text fragment arrives by the dozens per
/// second, and copying it for an observer that may want none of it would be a
/// cost imposed on all. An implementation that must keep an event copies what
/// it needs.
///
/// The variants carry the **whole facts**, in the terms of whoever executed —
/// not the filtered text that went to the model. It is the opposite of a
/// prompt, deliberately: see the module header.
///
/// `#[non_exhaustive]`: a variant will be added, and an interface that does
/// not know it must keep compiling rather than forcing everything to be
/// reread.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum AgentEvent<'a> {
    /// A turn starts.
    ///
    /// `turn` counts from 1. `max_turns` comes with it so that the panel can
    /// say "3 / 8" without knowing the agent's declaration — and so that the
    /// ceiling, when it falls, does not surprise.
    TurnStarted {
        /// The number of the starting turn, from 1.
        turn: usize,
        /// This agent's turn ceiling.
        max_turns: usize,
    },

    /// A fragment of the model's answer has just arrived.
    ///
    /// It is what lets the answer write itself as the stream flows, and the
    /// cancellation remain offered during all that time rather than only
    /// between two turns (UX-SPEC).
    ///
    /// The text can copy database content: logging it amounts to logging that
    /// content.
    TextDelta {
        /// The fragment, as the provider emitted it. Neither split on words,
        /// nor punctuated: it is up to the display to concatenate it.
        text: &'a str,
    },

    /// A tool call has just been translated into a
    /// [`Command`](oxyn_core::Command) and leaves for the bus.
    ///
    /// Emitted **before** execution, hence before
    /// [`CommandReported`](Self::CommandReported): UX-SPEC requires every
    /// command requested by the agent to be shown before its result.
    CommandSubmitted {
        /// Which call this is: the same identity the sink receives, and the
        /// report carries. Calls may overlap; this is what keeps them apart.
        call: CallId,
        /// The tool's name, as the model requested it.
        tool: &'a str,
        /// The command's stable name, that of the audit log. The same string
        /// as the one that will appear in the history: it is what lets the two
        /// be matched.
        command: &'static str,
        /// The targeted connection, read on the command itself and not on the
        /// conversation's scope. The two should coincide; displaying it from
        /// the command is what would make a gap visible.
        connection: Option<ConnectionId>,
        /// Can the command modify data? Known before the result, hence
        /// displayable before it.
        mutating: bool,
    },

    /// A command's execution report came back.
    CommandReported {
        /// The call reported, as [`Self::CommandSubmitted`] announced it.
        call: CallId,
        /// The tool's name, as the model requested it.
        tool: &'a str,
        /// What happened, **whole**: it is what the user has the right to
        /// read, server message included. It is not what the model received.
        outcome: &'a DispatchOutcome,
        /// Is what the model received poorer than the above?
        ///
        /// `true` when the connection's tier withheld something — the server's
        /// message, under `Local` and `Metadata`. The panel must say so: a
        /// poorly informed answer would otherwise pass for a wrong answer
        /// (UX-SPEC).
        ///
        /// Observed by comparing the two texts, never re-deduced from the
        /// filter's rule.
        withheld: bool,
    },

    /// A tool call was refused at translation, before any bus.
    ///
    /// Invented tool name, tool outside the allowlist, malformed arguments:
    /// nothing was submitted, nothing ran. The model is informed and can
    /// correct itself at the next turn; the user must see it, otherwise a turn
    /// seems to have been lost.
    CallRejected {
        /// The tool's name, as the model requested it.
        tool: &'a str,
        /// The refusal. Its message is in English — it also goes to the model
        /// — and copies neither an argument nor a result (see
        /// [`crate::error`]).
        error: &'a AiError,
    },

    /// A fragment of the model's reasoning.
    ///
    /// Shown apart from the answer, folded by default: it is a draft, not an
    /// opinion. It never reaches the next prompt by this path.
    ThinkingDelta {
        /// The fragment, raw.
        text: &'a str,
    },

    /// The provider reasoned but shows none of it (encrypted or redacted
    /// block). Saying there was hidden reasoning is better than keeping quiet:
    /// otherwise, the waiting time has no explanation.
    ThinkingRedacted,

    /// The model starts writing a tool call.
    ///
    /// Emitted before the arguments are complete: nothing is translated yet,
    /// nothing is submitted.
    ToolCallDrafted {
        /// The call's position in the turn, key of the following fragments.
        index: u32,
        /// The requested tool's name.
        tool: &'a str,
    },

    /// A fragment of the arguments of a call being written.
    ToolArgumentsDelta {
        /// The call's position in the turn.
        index: u32,
        /// A piece of JSON, not necessarily valid on its own.
        fragment: &'a str,
    },

    /// The consumption declared by the provider for a turn.
    Usage(TokenUsage),

    /// An external agent reports a tool **of its own** — read, search, think.
    ///
    /// It is not an Oxyn command: nothing went through the bus. Only the kind
    /// is carried, not the title, which the agent composes and which can copy a
    /// path of the machine.
    /// The plan the agent announces, **whole**.
    ///
    /// The protocol sends back the complete list at every send, and the client
    /// **replaces**: there is neither delta nor entry identifier. Merging with
    /// the previous plan is the mistake one writes spontaneously, and it
    /// produces a plan that grows at every turn without anything failing.
    Plan {
        /// The steps, in the order given by the agent.
        steps: &'a [PlanStep<'a>],
    },

    ExternalToolCall {
        /// The call's identifier in the agent's session.
        id: &'a str,
        /// The tool's kind, in one stable word. Absent from an update that does
        /// not repeat it.
        kind: Option<&'static str>,
        /// Where the call stands.
        status: ExternalToolStatus,
    },

    /// The settings an external agent declares: its modes, and the options it
    /// lets be chosen. **The whole state**, to replace. Sent at the start of a
    /// question, so that it starts from the settings in force; their changes,
    /// during a question or between two, are followed on
    /// `ExternalSession::settings`.
    AgentSettings(&'a crate::external::settings::AgentSettings),

    /// The occupancy of an external agent's context window, and its cumulative
    /// cost when the agent declares it.
    ContextWindow {
        /// Tokens currently in the context.
        used: u64,
        /// Size of the window.
        size: u64,
        /// Amount and ISO 4217 currency, as the agent gives them.
        cost: Option<(f64, &'a str)>,
    },

    /// An external agent asked to act on the machine, and Oxyn refused.
    ///
    /// There is no "granted on request" variant: Oxyn does not offer what it
    /// has no means to show (ADR-0026).
    PermissionRefused {
        /// The kind of action requested.
        kind: &'static str,
        /// The reason, also returned to the agent.
        reason: &'static str,
    },

    /// The conversation is over.
    ///
    /// Answered, cancelled, or turn ceiling reached — the latter stated with
    /// its number of turns, because it is neither a success nor a failure.
    ///
    /// **Not** emitted when the conversation stops on an error:
    /// [`run`](crate::runtime::AgentRuntime::run) returns it to its caller,
    /// which shows it itself. Announcing it here too would make a second path,
    /// with two possible displays for a single incident.
    Finished {
        /// How it ended.
        outcome: &'a AgentOutcome,
    },
}
