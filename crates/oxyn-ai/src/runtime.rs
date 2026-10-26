//! An agent's loop: model → tool calls → command bus → feeding back.
//!
//! # The runtime has no privileged path
//!
//! Every tool call becomes a [`Command`] carrying `Actor::Agent` and goes into
//! the [`CommandSink`] supplied by the caller — implemented by `oxyn-exec`,
//! which reclassifies the text then submits it to the `PolicyGate` (I-01, I-07,
//! ADR-0004). This module knows no driver, no database session, no policy: so
//! it cannot bypass them. That is what makes an instruction hidden in a column
//! comment produce, at worst, a visible approval request, never an execution.
//!
//! The `PolicyGate` is not called here, and that is not an oversight: doing so
//! would make a second place that decides, and two places that decide make one
//! place that will forget.
//!
//! # Context can only come in through the door
//!
//! [`AgentSession::new`] requires an [`AgentContext`], which is only built by
//! [`ContextBuilder::build`](crate::context::ContextBuilder::build). I-04's
//! single gateway is therefore a type constraint, not a review convention:
//! there is no constructor that accepts a ready-made prompt.
//!
//! What **comes back** from a tool call has the same constraint, and it is
//! easier to miss: a server error message quotes database content. A
//! [`CommandSink`] therefore returns a [`DispatchOutcome`] — raw facts — that
//! only `ToolOutcome::from_dispatch` turns into prompt text, under the
//! session's tier. See [`crate::failure`].
//!
//! # What the loop bounds
//!
//! * **turns** — [`AgentSpec::max_turns`], capped by
//!   [`MAX_TURNS_CEILING`](crate::spec::MAX_TURNS_CEILING). An agent looping
//!   on a remote provider is a bill the user discovers after the fact;
//! * **cancellation** — the [`CancelToken`] is re-read before each turn,
//!   between tool calls and while reading the stream. The tool calls of a
//!   cancelled turn are **dropped**: running a command after the user pressed
//!   Escape would be precisely what they just refused;
//! * **output** — nothing is executed by this module, including what "only
//!   reads".
//!
//! # The conversation can be watched while it unfolds
//!
//! [`AgentRuntime::run`] returns an [`AgentOutcome`] at the end, but notifies
//! an [`AgentObserver`] at every moment something becomes visible: a turn, a
//! text fragment, a submitted command, its report, the end. A caller without
//! an interface passes `&()`.
//!
//! The observer receives the **whole facts** — what the user has the right to
//! read from their own database — whereas the prompt receives only what the
//! tier lets out. Two recipients, and a single filter:
//! `ToolOutcome::from_dispatch`. See [`crate::observer`].

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use oxyn_catalog::CatalogHandle;
use oxyn_core::{
    Actor, AgentSessionId, CancelToken, Command, Decision, ErrorClass, ExecStats, OxynError,
    QueryLanguage,
};
use oxyn_llm::reasoning::ReasoningBlock;
use oxyn_llm::{ChatEvent, ChatMessage, ChatRequest, LlmProvider, Reach, StopReason, ToolCall};

use crate::call::{CallHandle, CallId};
use crate::context::{AgentContext, ContextBuilder, ContextPolicy, RowSample, estimate_tokens};
use crate::error::AiError;
use crate::failure::FailureReport;
use crate::observer::{AgentEvent, AgentObserver, TokenUsage};
use crate::privacy::{self, PrivacyTier};
use crate::spec::AgentSpec;
use crate::tools::{MAX_SAMPLE_ROWS, SampleAsk, ToolRegistry, ToolRequest, ToolScope};
use crate::untrusted;

/// What running a command produced, **before** the privacy tier applies.
///
/// This is what a [`CommandSink`] returns: facts, in the terms of whoever
/// executed. This type **never** joins a prompt as is — it must first go
/// through `ToolOutcome::from_dispatch`, which is the only way and which
/// requires a [`PrivacyTier`]. A sink therefore has nothing to know about
/// privacy, and nothing to get wrong about it.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub enum DispatchOutcome {
    /// The command was executed.
    Completed {
        /// What to tell the model about it: volume, truncation.
        summary: String,
    },
    /// The user approved a sample requested by the agent, and it was read.
    /// `sample` carries **only** the ticked columns: the read
    /// (`PreviewRelation`) projects them, the server returns only them, and
    /// the sink copies them before returning this variant.
    ///
    /// Real row values. What the model receives of them is rendered by
    /// `ContextBuilder::build`, under the connection's tier, in
    /// `ToolOutcome::from_dispatch` — the same function as for a sample
    /// pinned by the user ([I-04](../../../CLAUDE.md#i-04)).
    Sampled {
        /// The connection's cache, so that the rendering names the relation
        /// the way the context names it.
        catalog: CatalogHandle,
        /// The approved rows.
        sample: RowSample,
        /// What the sink does when the sample **actually** leaves: record the
        /// release, announce it. Called by the loop once the rendering is
        /// known and kept, never before — see [`SampleReceipt`].
        receipt: SampleReceipt,
    },
    /// The local catalog was read: the cache handle, never a rendering.
    ///
    /// What the model learns from it is rendered by `ContextBuilder::build`,
    /// under the connection's tier, in `ToolOutcome::from_dispatch`.
    CatalogRead {
        /// The connection's cache.
        catalog: CatalogHandle,
    },
    /// The command awaits the user's approval. **Nothing ran.**
    AwaitingApproval {
        /// The reason as the `PolicyGate` worded it.
        reason: String,
    },
    /// The command is denied. No confirmation will unblock it.
    Denied {
        /// The reason as the `PolicyGate` worded it.
        reason: String,
    },
    /// Execution failed.
    Failed {
        /// The error's family, as the driver classified it. Passed on, not
        /// re-derived: a caller that parsed the message would silently break
        /// the day the message changes.
        class: ErrorClass,
        /// The server's message, whole. It may quote a row value: that is
        /// precisely why it cannot join a prompt without going through
        /// `ToolOutcome::from_dispatch`.
        message: String,
    },
}

/// What a sink does with a read sample, once it leaves for good.
///
/// The sink reads the rows; the rendering happens afterwards, in
/// `ToolOutcome::from_dispatch`, which can still drop them — budget exceeded,
/// tier lowered. Recording the release in the sink, before that rendering,
/// recorded and announced as sent a sample the model did not receive. The
/// sink therefore hands this gesture to the loop, which performs it only once
/// the rendering is kept, **before** returning the text: if recording fails,
/// nothing leaves.
///
/// A boundary: `oxyn-ai` decides when, `oxyn-desktop` knows where to record.
#[async_trait]
pub trait SampleRelease: Send + Sync {
    /// Records the sample's release and announces it. Returns `false` if
    /// recording failed: the sample does not leave.
    ///
    /// Called at most once by the loop; an implementation that receives a
    /// second call records nothing more.
    async fn release(&self) -> bool;
}

/// The handle of a [`SampleRelease`], carried by [`DispatchOutcome::Sampled`].
///
/// Two receipts are equal if they designate **the same** gesture. Dropped
/// without being released — sample discarded, question closed in the
/// meantime —, it records nothing: that is the intended effect.
#[derive(Clone)]
pub struct SampleReceipt(Arc<dyn SampleRelease>);

impl SampleReceipt {
    /// Wraps the sink's gesture.
    #[must_use]
    pub fn new(release: Arc<dyn SampleRelease>) -> Self {
        Self(release)
    }

    async fn release(&self) -> bool {
        self.0.release().await
    }
}

impl PartialEq for SampleReceipt {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for SampleReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SampleReceipt").finish_non_exhaustive()
    }
}

/// Told to the model, and shown, when recording a sample failed.
const SAMPLE_UNRECORDED: &str =
    "Oxyn could not record this sample in its audit trail, so nothing was sent";

/// `Debug` written by hand, and I-03's corollary is what requires it.
///
/// [`Failed`](DispatchOutcome::Failed) carries the server's message **whole**
/// — that is the whole point of this type, and the user has the right to read
/// it. But a derived `Debug` would make it copyable by a
/// `tracing::debug!("{outcome:?}")` added six months later to diagnose a panel
/// that shows nothing, and `Key (email)=(dupont@example.com)` would go to disk
/// in clear. That is exactly the leak mode I-03's verifiable corollary names:
/// the fault shows neither at compile time, nor in tests, nor in review.
///
/// What remains: the error's class and the message's **length**. Enough to
/// diagnose "the message is empty" or "the message is 4 KB", never enough to
/// read a row value.
impl fmt::Debug for DispatchOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed { summary } => f
                .debug_struct("Completed")
                .field("summary", summary)
                .finish(),
            Self::AwaitingApproval { reason } => f
                .debug_struct("AwaitingApproval")
                .field("reason", reason)
                .finish(),
            Self::CatalogRead { .. } => f.debug_struct("CatalogRead").finish_non_exhaustive(),
            // `RowSample` already masks its values; only the count comes out.
            Self::Sampled { sample, .. } => {
                f.debug_struct("Sampled").field("sample", sample).finish()
            }
            Self::Denied { reason } => f.debug_struct("Denied").field("reason", reason).finish(),
            Self::Failed { class, message } => f
                .debug_struct("Failed")
                .field("class", class)
                .field("message_bytes", &message.len())
                .finish(),
        }
    }
}

impl DispatchOutcome {
    /// Summarizes a successful execution from its statistics.
    #[must_use]
    pub fn completed(stats: &ExecStats) -> Self {
        let mut summary = format!("{} rows, {} batches", stats.rows, stats.batches);
        if stats.truncated {
            summary.push_str(" (truncated at the row limit; this is not the whole result)");
        }
        Self::Completed { summary }
    }

    /// Translates a `PolicyGate` decision.
    ///
    /// Returns `None` for [`Decision::Allow`]: there is then nothing to tell
    /// the model until the command has produced a result.
    ///
    /// The [`Preview`](oxyn_core::Preview) an approval request carries is
    /// **not** taken over: it is meant for the user, who must see the exact
    /// SQL and the connection's name before deciding. The model wrote the
    /// statement itself and has nothing to learn from the connection's name —
    /// sending it would only be one more leak.
    #[must_use]
    pub fn from_decision(decision: &Decision) -> Option<Self> {
        match decision {
            Decision::Allow => None,
            Decision::RequireApproval { reason, .. } => Some(Self::AwaitingApproval {
                reason: reason.clone(),
            }),
            Decision::Deny { reason } => Some(Self::Denied {
                reason: reason.clone(),
            }),
        }
    }

    /// Translates a domain error, carrying its class over rather than letting
    /// it be inferred from a message.
    #[must_use]
    pub fn failed(error: &OxynError) -> Self {
        Self::Failed {
            class: error.class(),
            message: error.to_string(),
        }
    }
}

/// What running a command produced, in the terms the model is allowed to
/// know.
///
/// Deliberately poor: the model learns what happened, not the rows. Results
/// live as `RecordBatch` in the result buffer (ADR-0002) and are shown to the
/// **user**; passing them through the conversation would send them to the
/// provider, which the connection's tier does not necessarily allow (I-04).
///
/// **Cannot be built outside this crate.** The [`Failed`](Self::Failed)
/// variant carries a [`FailureReport`] whose fields are private, and the only
/// path that produces one is `from_dispatch`, which requires the connection's
/// tier. The filter therefore cannot be bypassed by oversight.
///
/// `Debug` is written by hand, like those of [`AgentContext`] and
/// `AgentPrompt`: [`Described`](Self::Described) carries the rendered schema,
/// and a `tracing::debug!("{outcome:?}")` would write it to a log (I-03).
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolOutcome {
    /// The command was executed.
    Completed {
        /// What to tell the model about it: volume, duration, truncation.
        summary: String,
    },
    /// The database's structure, rendered by the gateway.
    Described {
        /// The block `ContextBuilder::build` produced — **already fenced**:
        /// it is the same text as a prompt's context.
        block: String,
    },
    /// An approved sample, rendered by the gateway.
    Sampled {
        /// The block `ContextBuilder::build` produced, **already fenced** —
        /// real row values.
        block: String,
    },
    /// The command awaits the user's approval. **Nothing ran.**
    AwaitingApproval {
        /// The reason as the `PolicyGate` worded it.
        reason: String,
    },
    /// The command is denied. No confirmation will unblock it.
    Denied {
        /// The reason as the `PolicyGate` worded it.
        reason: String,
    },
    /// Execution failed, reduced to what the tier lets out.
    Failed {
        /// The filtered failure. See [`FailureReport`].
        report: FailureReport,
    },
}

impl ToolOutcome {
    /// Applies the connection's tier to what a sink reported.
    ///
    /// **This is where the tier applies to the error path**, as
    /// [`ContextBuilder::build`](crate::context::ContextBuilder::build)
    /// applies it to the context path. `pub(crate)`: the only caller is the
    /// loop, which holds the session's tier — and the session holds the
    /// connection's, never a global setting (I-04, ADR-0006).
    ///
    /// A read catalog is rendered here, by [`ContextBuilder::build`] — the
    /// same function as a prompt's context, under the same tier, with the same
    /// budget —, in the connection's language and steered by the command's
    /// search words. There is no second rendering of the schema.
    #[must_use]
    pub(crate) fn from_dispatch(
        tier: PrivacyTier,
        outcome: DispatchOutcome,
        language: QueryLanguage,
        focus: Option<&str>,
        fence: &untrusted::Fence,
    ) -> Self {
        match outcome {
            DispatchOutcome::Completed { summary } => Self::Completed { summary },
            DispatchOutcome::CatalogRead { catalog } => {
                let cache = catalog.catalog().read();
                let context = ContextBuilder::new(&cache, tier)
                    .with_language(language)
                    .focused_on(focus.unwrap_or_default())
                    .with_fence(fence.clone())
                    .build();
                Self::Described {
                    block: context.prompt_block().to_owned(),
                }
            }
            DispatchOutcome::Sampled {
                catalog, sample, ..
            } => Self::from_sample(tier, &catalog, sample, language, fence),
            DispatchOutcome::AwaitingApproval { reason } => Self::AwaitingApproval { reason },
            DispatchOutcome::Denied { reason } => Self::Denied { reason },
            DispatchOutcome::Failed { class, message } => Self::Failed {
                report: FailureReport::redact(tier, class, &message),
            },
        }
    }

    /// Renders an approved sample through [`ContextBuilder::build`], under
    /// `tier`.
    ///
    /// No relation is described: the agent named the one it wanted, and the
    /// structure comes to it from `describe_schema`. The whole budget
    /// therefore goes to the rows, and the rendering cap is the request's.
    ///
    /// A sample the gateway drops — a tier that no longer lets values out, a
    /// budget exceeded — becomes a **refusal**: the model must not believe it
    /// received what did not leave.
    fn from_sample(
        tier: PrivacyTier,
        catalog: &CatalogHandle,
        sample: RowSample,
        language: QueryLanguage,
        fence: &untrusted::Fence,
    ) -> Self {
        let rows = usize::try_from(MAX_SAMPLE_ROWS).unwrap_or(usize::MAX);
        let context = {
            let cache = catalog.catalog().read();
            ContextBuilder::new(&cache, tier)
                .with_policy(ContextPolicy {
                    max_relations: 0,
                    max_sample_rows: rows,
                    ..ContextPolicy::default()
                })
                .with_language(language)
                .with_samples(vec![sample])
                .with_fence(fence.clone())
                .build()
        };
        if context.dropped_samples() > 0 {
            return Self::Denied {
                reason: if tier.allows_row_values() {
                    "the approved sample did not fit the context budget, so nothing was sent; \
                     ask for fewer rows or columns"
                        .to_owned()
                } else {
                    format!(
                        "this connection's privacy tier is now `{tier}`, which lets no row value \
                         leave; nothing was sent"
                    )
                },
            };
        }
        Self::Sampled {
            block: context.prompt_block().to_owned(),
        }
    }

    /// Is this text poorer than the facts it was drawn from?
    ///
    /// In other words: did the tier hold back something the user, for their
    /// part, will see? UX-SPEC asks the panel to say so — "hiding the gap
    /// would pass off an ill-informed answer as a wrong answer".
    ///
    /// The gap is **observed** by comparing the two texts, never re-derived by
    /// replaying `from_dispatch`'s rule: a copied rule diverges the day the
    /// original changes, and diverges silently.
    #[must_use]
    pub(crate) fn withholds_from(&self, facts: &DispatchOutcome) -> bool {
        match (self, facts) {
            (Self::Completed { summary }, DispatchOutcome::Completed { summary: facts }) => {
                summary != facts
            }
            // The catalog carries no row value: the tier held nothing back.
            // The budget, for its part, is stated in the block itself.
            (Self::Described { .. }, DispatchOutcome::CatalogRead { .. }) => false,
            // What was approved is what leaves: the unticked columns were read
            // by the preview, but the sink did not copy them — they are not in
            // the facts, so the tier has nothing to hold back. A dropped
            // sample becomes a refusal, and falls into the mismatched case
            // below.
            (Self::Sampled { .. }, DispatchOutcome::Sampled { .. }) => false,
            (
                Self::AwaitingApproval { reason },
                DispatchOutcome::AwaitingApproval { reason: facts },
            )
            | (Self::Denied { reason }, DispatchOutcome::Denied { reason: facts }) => {
                reason != facts
            }
            (Self::Failed { report }, DispatchOutcome::Failed { message, .. }) => {
                report.detail() != Some(message.as_str())
            }
            // Mismatched variants: `from_dispatch` keeps the variant, so this
            // case does not exist today — but both types are
            // `#[non_exhaustive]` and nothing forces a re-read here. Doubt
            // does not favor silence: we announce a gap rather than let it be
            // believed the model knew everything.
            _ => true,
        }
    }

    /// Did the command actually have an effect?
    ///
    /// `false` for a pending approval: the trap is a model assuming an
    /// `INSERT` took place and building on that assumption.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Described { .. } | Self::Sampled { .. }
        )
    }

    /// The text sent back to the model, **fenced as untrusted content**.
    ///
    /// A server error message contains database content: the name of the
    /// missing table, the value that violates a constraint. So it comes in
    /// through the same door as the rest. Fencing is **uniform** — including
    /// for reasons worded by Oxyn — because a rule without exceptions can be
    /// checked at a glance.
    ///
    /// Fencing filters nothing: it keeps the content from leaving its fence.
    /// What decides what *enters* the fence is the tier, applied upstream by
    /// `from_dispatch`.
    #[must_use]
    pub(crate) fn render(&self, fence: &untrusted::Fence) -> String {
        match self {
            // The block comes out of the gateway, which already fenced it:
            // fencing it again would neutralize its tags and make it differ
            // from a prompt's context. The status is fenced like the others.
            Self::Described { block } | Self::Sampled { block } => {
                format!("{}\n{block}", fence.fence("status: completed"))
            }
            _ => fence.fence(&self.to_string()),
        }
    }
}

impl fmt::Display for ToolOutcome {
    /// The body meant for the model, **in English**: it is a prompt, not an
    /// interface message. Not fenced — see `ToolOutcome::render`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed { summary } => write!(f, "status: completed\n{summary}"),
            Self::Described { block } | Self::Sampled { block } => {
                write!(f, "status: completed\n{block}")
            }
            Self::AwaitingApproval { reason } => write!(
                f,
                "status: awaiting_approval\n\
                 Nothing ran. The user has been asked to approve it and has not answered yet. \
                 Do not assume any effect took place.\nreason: {reason}"
            ),
            Self::Denied { reason } => write!(
                f,
                "status: denied\n\
                 This will not run, and no approval can unblock it. Do not retry it, and do \
                 not look for another way to achieve the same effect.\nreason: {reason}"
            ),
            Self::Failed { report } => write!(f, "status: failed\n{report}"),
        }
    }
}

/// See the type: only the rendered schema is masked, by its length. The other
/// variants carry only what a derived one already showed.
impl fmt::Debug for ToolOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed { summary } => f
                .debug_struct("Completed")
                .field("summary", summary)
                .finish(),
            Self::Described { block } => f
                .debug_struct("Described")
                .field("block", &format_args!("<redacted, {} bytes>", block.len()))
                .finish(),
            Self::Sampled { block } => f
                .debug_struct("Sampled")
                .field("block", &format_args!("<redacted, {} bytes>", block.len()))
                .finish(),
            Self::AwaitingApproval { reason } => f
                .debug_struct("AwaitingApproval")
                .field("reason", reason)
                .finish(),
            Self::Denied { reason } => f.debug_struct("Denied").field("reason", reason).finish(),
            Self::Failed { report } => f.debug_struct("Failed").field("report", report).finish(),
        }
    }
}

/// What the runtime hands its commands to.
///
/// Implemented by `oxyn-exec`: this is **the** boundary between the agent
/// runtime and execution. The implementation's contract:
///
/// 1. reclassify the text before any decision — the intent the command
///    carries comes from an agent, hence from a caller (ARCHITECTURE §8);
/// 2. submit to the `PolicyGate` with the `Actor` received, unchanged;
/// 3. log, including a refusal;
/// 4. propagate cancellation all the way to the server.
///
/// The method returns no `Result`: an execution failure **is** an answer to
/// give the model ([`DispatchOutcome::Failed`]), not an incident that
/// interrupts the conversation. What interrupts the conversation is what
/// comes from the provider, not from the database.
///
/// It returns a [`DispatchOutcome`] and not a [`ToolOutcome`]: the privacy
/// tier applies **afterwards**, in the loop, at the only place where it is
/// known. An implementation therefore cannot bring a server message into a
/// prompt, even by trying.
///
/// `call` is the tool call the command answers: its [`CallId`] is the one the
/// observer received in [`AgentEvent::CommandSubmitted`] and will receive in
/// [`AgentEvent::CommandReported`] — what lets a host keep each call's steps
/// on that call's row when calls overlap. A sink that leaves a request with
/// the user calls [`CallHandle::awaiting_user`] once the executor holds it.
#[async_trait]
pub trait CommandSink: Send + Sync {
    /// Submits a command and returns what happened.
    async fn dispatch(
        &self,
        call: &CallHandle,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> DispatchOutcome;

    /// Asks the user to approve a sample, then reads it.
    ///
    /// The read is `ask`'s command, submitted **after** approval, with
    /// `actor` and through the same path as [`CommandSink::dispatch`]: the
    /// `PolicyGate` still decides. The implementation's contract, on top of
    /// the trait's four rules:
    ///
    /// 1. re-read the connection's tier **now**, and refuse without showing
    ///    anything outside `Sampled`;
    /// 2. check the relation and the columns against the catalog before
    ///    showing anything;
    /// 3. read nothing before the user's decision — made by a user gesture,
    ///    never by the agent —, bound the wait in time and give it up on
    ///    cancellation;
    /// 4. copy only the ticked columns, and return
    ///    [`DispatchOutcome::Sampled`] — never the values in another variant.
    ///
    /// **The default refuses**, and that is intended: a sink that cannot show
    /// the approval screen has no path to a value. Nothing is read.
    async fn request_sample(
        &self,
        call: &CallHandle,
        actor: Actor,
        ask: SampleAsk,
        cancel: &CancelToken,
    ) -> DispatchOutcome {
        let _ = (call, actor, ask, cancel);
        DispatchOutcome::Denied {
            reason: "this destination cannot ask the user to approve a row sample; \
                     nothing was read"
                .to_owned(),
        }
    }
}

/// How a conversation ended.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AgentOutcome {
    /// The model answered without asking for a tool.
    Answered {
        /// The text produced. It may copy database content: logging it
        /// amounts to logging that content.
        text: String,
        /// Number of turns consumed.
        turns: usize,
        /// Is the answer cut (token cap, filtering)? A cut answer that does
        /// not say so looks like a wrong answer.
        truncated: bool,
        /// Why the turn stopped, as the provider said.
        ///
        /// Carried so the interface names **the right** cut: a context window
        /// overflow presented as a "token cap" sends the user to raise a
        /// setting that cannot help.
        stop: StopReason,
    },
    /// The user cancelled.
    Cancelled {
        /// Number of turns consumed before the cancellation.
        turns: usize,
    },
    /// The turn ceiling was reached without a final answer.
    ///
    /// This is not an error: it is the bound doing its job. What was executed
    /// was executed, and appears in the log.
    TurnLimit {
        /// Number of turns consumed, equal to `max_turns`.
        turns: usize,
    },
    /// The model refused to go on.
    ///
    /// Distinct from `Answered`: a refusal read as an answer would pass for an
    /// opinion about the database; and distinct from an error: nothing is
    /// broken.
    Refused {
        /// What was produced before the refusal, possibly empty.
        text: String,
        /// Number of turns consumed.
        turns: usize,
    },
    /// The provider paused the turn on its side.
    ///
    /// Resuming is **manual**: relaunching a paused turn on its own would be
    /// an expense the user did not ask for.
    Paused {
        /// What was produced before the pause.
        text: String,
        /// Number of turns consumed.
        turns: usize,
    },
}

/// A conversation with an agent.
///
/// Built only from an [`AgentContext`]: that is what makes the single gateway
/// a property of the type (I-04).
///
/// `Clone` serves an answer's versions: regenerating or editing starts again
/// from the state **before** the question, copied as is. A copy opens no door
/// — it only contains what already went through the context.
#[derive(Debug, Clone)]
pub struct AgentSession {
    id: AgentSessionId,
    tier: PrivacyTier,
    scope: ToolScope,
    fence: untrusted::Fence,
    messages: Vec<ChatMessage>,
}

impl AgentSession {
    /// Opens a conversation on an assembled context.
    ///
    /// The system message is composed in this order: the agent's prompt, the
    /// preamble that says what a fence is, then the fenced context. The
    /// preamble comes **before** the content it qualifies, because a model
    /// that reads the instruction after the data has already read the data.
    #[must_use]
    pub fn new(spec: &AgentSpec, context: &AgentContext, scope: ToolScope) -> Self {
        let system = format!(
            "{}\n\n{}\n\n{}",
            spec.system_prompt.trim(),
            context.untrusted_preamble(),
            context.prompt_block()
        );
        Self {
            id: AgentSessionId::new(),
            tier: context.tier(),
            scope,
            fence: context.fence().clone(),
            messages: vec![ChatMessage::system(system)],
        }
    }

    /// The conversation identifier. With the `AgentId`, it is the key by which
    /// the audit log ties a command to its agent.
    #[must_use]
    pub const fn id(&self) -> AgentSessionId {
        self.id
    }

    /// The tier applied to this conversation's context.
    #[must_use]
    pub const fn tier(&self) -> PrivacyTier {
        self.tier
    }

    /// This conversation's tool scope.
    #[must_use]
    pub const fn scope(&self) -> &ToolScope {
        &self.scope
    }

    /// The messages exchanged, in order.
    #[must_use]
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Estimated cost of sending this conversation again offering `tools`, in
    /// tokens.
    ///
    /// The context budget's estimate ([`estimate_tokens`]) of what the next
    /// request serializes: every message — texts, tool calls and their
    /// arguments, tool results, reasoning blocks and their signatures — and
    /// the definitions of the tools, which every request carries again and a
    /// provider counts in the window (RESEARCH-NOTES, « Context window
    /// overflow »). Read on Oxyn's own JSON form of the request: its field
    /// names and quoting stand for each protocol's framing, which this does
    /// not copy. As coarse as the estimate — a caller compares it with a
    /// model's window to decide when to start over, never to bill. Serializes
    /// the conversation once: linear in its size.
    #[must_use]
    pub fn estimated_tokens(&self, tools: &[oxyn_llm::ToolSpec]) -> usize {
        /// What a request carries again at every turn, borrowed: the estimate
        /// does not copy the conversation it measures.
        #[derive(serde::Serialize)]
        struct Resent<'a> {
            messages: &'a [ChatMessage],
            tools: &'a [oxyn_llm::ToolSpec],
        }
        // A conversation that does not serialize cannot be sent either:
        // counted as too large, it starts over rather than fail later.
        serde_json::to_string(&Resent {
            messages: &self.messages,
            tools,
        })
        .map_or(usize::MAX, |json| estimate_tokens(&json))
    }

    /// Adds a question from the user.
    ///
    /// It is text the user typed themselves: it is not fenced, and it is the
    /// only category of content that is not.
    pub fn ask(&mut self, question: impl Into<String>) {
        self.messages.push(ChatMessage::user(question));
    }

    /// Adds a question that names objects, in an already open conversation.
    ///
    /// The system message is not rewritten: the mentioned objects precede the
    /// question in the user message, rendered by [`ContextBuilder::build`] —
    /// the only path that makes an [`AgentContext`], hence under an applied
    /// tier. The text is the one an external agent receives in the same case
    /// ([`AgentPrompt::following`](crate::external::prompt::AgentPrompt::following)).
    ///
    /// # Errors
    ///
    /// [`oxyn_core::OxynError::Config`], carried by [`AiError::Core`], if
    /// `context` was rendered under a tier other than the conversation's:
    /// nothing is added, not even the question.
    pub fn ask_about(
        &mut self,
        context: &AgentContext,
        question: impl AsRef<str>,
    ) -> Result<(), AiError> {
        if context.tier() != self.tier {
            return Err(AiError::Core(oxyn_core::OxynError::Config(format!(
                "the mentioned objects were rendered under the `{}` tier, and this \
                 conversation runs under `{}`",
                context.tier(),
                self.tier
            ))));
        }
        self.messages
            .push(ChatMessage::user(context.follow_up(question.as_ref())));
        Ok(())
    }
}

/// A call the model proposed, as the turn received it.
#[derive(Debug)]
struct Proposed {
    call: ToolCall,
    /// Why its arguments could not be read, when they could not. Such a call
    /// is answered with a rejection the model can correct, and never runs.
    unreadable: Option<String>,
}

/// What a turn produced.
#[derive(Debug)]
struct Turn {
    text: String,
    calls: Vec<Proposed>,
    /// The turn's reasoning blocks, to put back in the assistant message: a
    /// provider that signs its blocks refuses the next turn if one is
    /// missing.
    reasoning: Vec<ReasoningBlock>,
    /// What the model said to refuse, if it refused.
    refusal: String,
    stop: StopReason,
    truncated: bool,
    cancelled: bool,
}

/// How the end of a turn without tool calls reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Answered,
    Refused,
    Paused,
}

impl Stop {
    fn of(reason: &StopReason) -> Self {
        match reason {
            // The provider held back the rest: it is a refusal, not a short
            // answer.
            StopReason::ContentFilter | StopReason::Refusal => Self::Refused,
            // The turn is paused, not finished: the answer is partial and the
            // user decides whether to resume. Providers say so with a variant
            // since `StopReason` has one — reading it in `Other("pause_turn")`
            // no longer saw any pause.
            StopReason::Paused => Self::Paused,
            _ => Self::Answered,
        }
    }
}

/// An agent's loop.
#[derive(Debug)]
pub struct AgentRuntime {
    spec: AgentSpec,
    provider: Arc<dyn LlmProvider>,
    reach: Reach,
    tools: ToolRegistry,
    model: String,
    /// The effort to request, already checked against the model. `None`:
    /// nothing is sent, and the provider applies its default.
    effort: Option<oxyn_llm::ReasoningEffort>,
}

impl AgentRuntime {
    /// Prepares running an agent on a provider.
    ///
    /// `reach` is the endpoint's classification, computed by the caller when
    /// the provider is registered: DNS resolution is blocking and has no
    /// business here (I-05, [`oxyn_llm::reach`]).
    ///
    /// # Errors
    /// What [`AgentSpec::validate`] refuses. Failing here rather than at the
    /// first tool call avoids paying for turns to discover that an agent is
    /// badly declared.
    pub fn new(
        spec: AgentSpec,
        provider: Arc<dyn LlmProvider>,
        reach: Reach,
        tools: ToolRegistry,
        model: impl Into<String>,
    ) -> Result<Self, AiError> {
        spec.validate(&tools)?;
        Ok(Self {
            spec,
            provider,
            reach,
            tools,
            model: model.into(),
            effort: None,
        })
    }

    /// Requests an effort level for every turn of this run.
    ///
    /// `model` is the chosen model's card, as the provider publishes it. The
    /// level must appear in it: an empty list means "not declared", and
    /// nothing undeclared is sent.
    ///
    /// `High` is checked like the others, then **omitted**: it is the default
    /// of the providers that expose this setting, and omitting it leaves the
    /// request identical to that of a user who chose nothing.
    ///
    /// # Errors
    /// [`AiError::ReasoningEffortNotOffered`] when the model does not declare
    /// this level.
    pub fn with_reasoning_effort(
        mut self,
        effort: oxyn_llm::ReasoningEffort,
        model: &oxyn_llm::ModelInfo,
    ) -> Result<Self, AiError> {
        if !model.reasoning_efforts.contains(&effort) {
            return Err(AiError::ReasoningEffortNotOffered {
                effort,
                reasoning: model.supports_reasoning,
            });
        }
        self.effort = (effort != oxyn_llm::ReasoningEffort::High).then_some(effort);
        Ok(self)
    }

    /// The agent's declaration.
    #[must_use]
    pub const fn spec(&self) -> &AgentSpec {
        &self.spec
    }

    /// Where a request to this provider goes.
    #[must_use]
    pub const fn reach(&self) -> Reach {
        self.reach
    }

    /// Is this provider usable under this tier?
    ///
    /// To be asked before offering the agent in the interface: offering then
    /// refusing is worse than not offering.
    #[must_use]
    pub const fn accepts_tier(&self, tier: PrivacyTier) -> bool {
        privacy::allows_endpoint(tier, self.reach)
    }

    /// Runs the conversation until an answer, a cancellation or the turn
    /// ceiling, reporting to `observer` as it goes.
    ///
    /// The tier is re-checked **here**, against the session's, and not at
    /// construction: the tier belongs to the connection, and one runtime
    /// instance can serve two connections with different tiers. Checking it
    /// at the only place where it is known is what makes I-04 tenable.
    ///
    /// `observer` is what makes the conversation watchable while it unfolds;
    /// a caller with nothing to display passes `&()`. There is **no**
    /// unobserved variant of this method: two ways of running a conversation
    /// would be two paths to audit where I-04 asks for one.
    ///
    /// The observer receives the whole facts; the prompt receives only what
    /// the tier lets out. See [`crate::observer`].
    ///
    /// # Errors
    /// [`AiError::RemoteProviderRefused`] if the session's tier forbids this
    /// endpoint; [`AiError::Provider`] or [`AiError::Core`] if the provider
    /// fails. A `PolicyGate` refusal, on the other hand, is not an error: it is
    /// sent back to the model and the conversation goes on. An error emits no
    /// [`AgentEvent::Finished`]: it is returned here, to the caller.
    pub async fn run(
        &self,
        session: &mut AgentSession,
        sink: &dyn CommandSink,
        observer: &dyn AgentObserver,
        cancel: &CancelToken,
    ) -> Result<AgentOutcome, AiError> {
        let outcome = self.converse(session, sink, observer, cancel).await?;
        observer.observe(AgentEvent::Finished { outcome: &outcome });
        Ok(outcome)
    }

    /// The loop itself.
    ///
    /// Separate from [`run`](Self::run) for a single reason: the end of the
    /// conversation is announced at a single place, whatever path leads there.
    /// Three `return`s and three notifications to keep consistent would make
    /// the fourth one the one that gets forgotten.
    async fn converse(
        &self,
        session: &mut AgentSession,
        sink: &dyn CommandSink,
        observer: &dyn AgentObserver,
        cancel: &CancelToken,
    ) -> Result<AgentOutcome, AiError> {
        if !self.accepts_tier(session.tier) {
            return Err(AiError::RemoteProviderRefused { tier: session.tier });
        }

        let specs = self.tools.specs_for(&self.spec.allowed_tools)?;
        let actor = Actor::agent(self.spec.id, session.id);
        let mut turns = 0usize;

        while turns < self.spec.max_turns {
            if cancel.is_cancelled() {
                return Ok(AgentOutcome::Cancelled { turns });
            }
            turns += 1;
            observer.observe(AgentEvent::TurnStarted {
                turn: turns,
                max_turns: self.spec.max_turns,
            });

            let turn = self.one_turn(session, &specs, observer, cancel).await?;
            if turn.cancelled {
                return Ok(AgentOutcome::Cancelled { turns });
            }
            if turn.calls.is_empty() {
                // Kept in the session: the next question reads it, and
                // "continue" picks up where it stopped. An empty text is not a
                // message — several providers refuse it.
                if !turn.text.is_empty() {
                    session.messages.push(
                        ChatMessage::assistant(turn.text.clone())
                            .with_reasoning(turn.reasoning.clone()),
                    );
                }
                // A refusal stated by the model counts as the stop reason:
                // some providers refuse in text and end on `EndTurn`.
                let ending = if turn.refusal.is_empty() {
                    Stop::of(&turn.stop)
                } else {
                    Stop::Refused
                };
                return Ok(match ending {
                    Stop::Refused => AgentOutcome::Refused {
                        text: if turn.refusal.is_empty() {
                            turn.text
                        } else {
                            turn.refusal
                        },
                        turns,
                    },
                    Stop::Paused => AgentOutcome::Paused {
                        text: turn.text,
                        turns,
                    },
                    Stop::Answered => AgentOutcome::Answered {
                        text: turn.text,
                        turns,
                        truncated: turn.truncated,
                        stop: turn.stop,
                    },
                });
            }

            let calls = turn.calls;
            session.messages.push(
                ChatMessage::assistant(turn.text)
                    .with_reasoning(turn.reasoning)
                    .with_tool_calls(calls.iter().map(|proposed| proposed.call.clone()).collect()),
            );

            for Proposed { call, unreadable } in calls {
                if cancel.is_cancelled() {
                    return Ok(AgentOutcome::Cancelled { turns });
                }
                let content = match unreadable {
                    Some(detail) => rejection(
                        observer,
                        &call.name,
                        &AiError::InvalidArguments {
                            name: call.name.clone(),
                            detail,
                        },
                        &session.fence,
                    ),
                    None => {
                        self.run_one_tool(&call, actor, session, sink, observer, cancel)
                            .await?
                    }
                };
                session
                    .messages
                    .push(ChatMessage::tool_result(call.id, content));
            }
        }

        Ok(AgentOutcome::TurnLimit { turns })
    }

    /// Translates a call, submits it to the bus, and returns what to tell the
    /// model.
    async fn run_one_tool(
        &self,
        call: &ToolCall,
        actor: Actor,
        session: &AgentSession,
        sink: &dyn CommandSink,
        observer: &dyn AgentObserver,
        cancel: &CancelToken,
    ) -> Result<String, AiError> {
        run_tool_call(
            &self.tools,
            &self.spec.allowed_tools,
            call,
            actor,
            &session.scope,
            session.tier,
            &session.fence,
            sink,
            observer,
            cancel,
        )
        .await
    }
}

/// The **single** path of a tool call, wherever it comes from.
///
/// The inner loop goes through it for a model call; the MCP bridge goes
/// through it for an external agent's call ([ADR-0030](../../../docs/adr/0030-outils-oxyn-exposes-a-un-agent-externe.md)).
/// Two paths would have diverged, and the one nobody reviews is the one the
/// agent would have taken ([I-01](../../../CLAUDE.md#i-01)).
#[expect(
    clippy::too_many_arguments,
    reason = "everything is imposed by the host; grouping would hide what comes from where"
)]
pub(crate) async fn run_tool_call(
    tools: &ToolRegistry,
    allowed: &[String],
    call: &ToolCall,
    actor: Actor,
    scope: &ToolScope,
    tier: PrivacyTier,
    fence: &untrusted::Fence,
    sink: &dyn CommandSink,
    observer: &dyn AgentObserver,
    cancel: &CancelToken,
) -> Result<String, AiError> {
    {
        let request = match tools.request(call, allowed, scope) {
            Ok(request) => request,
            // An invented tool name or malformed arguments can be fixed on the
            // next turn: we tell the model rather than interrupt.
            Err(err) if err.is_recoverable_by_model() => {
                return Ok(rejection(observer, &call.name, &err, fence));
            }
            Err(err) => return Err(err),
        };

        let command = request.command();
        tracing::debug!(
            tool = %call.name,
            command = command.name(),
            mutating = command.is_mutating(),
            "command submitted to the bus by an agent"
        );
        // Minted here, the one place every call passes: the observer and the
        // sink receive the same identity, whoever made the call.
        let handle = CallHandle::new(CallId::fresh());
        // Announced **before** execution: UX-SPEC asks that every command show
        // before its result, and a command announced after the fact says
        // nothing about the wait that just went by.
        observer.observe(AgentEvent::CommandSubmitted {
            call: handle.id(),
            tool: &call.name,
            command: command.name(),
            connection: command.target_connection(),
            mutating: command.is_mutating(),
        });

        // The search words are those the command carries — hence those the
        // log keeps —, not the call's: there is only one truth.
        let focus = match command {
            Command::DescribeCatalog { focus, .. } => focus.clone(),
            _ => None,
        };
        let dispatched = match request {
            // Refused here, under the tier the loop holds, **before** the user
            // is solicited: an approval asked under `Metadata` would be a
            // screen that can only refuse. The sink then re-reads the recorded
            // tier, which may have dropped since.
            ToolRequest::Sample(_) if !tier.allows_row_values() => DispatchOutcome::Denied {
                reason: format!(
                    "row samples are shared only on a connection whose privacy tier is \
                     `sampled`; this one is `{tier}`. Nothing was read or asked. Work from \
                     the structure, and do not ask again"
                ),
            },
            ToolRequest::Sample(ask) => sink.request_sample(&handle, actor, ask, cancel).await,
            ToolRequest::Dispatch(command) => sink.dispatch(&handle, actor, command, cancel).await,
        };
        // The session's tier is the connection's. It is the only place on the
        // error path where it is known, hence the only place where it can
        // apply (I-04).
        let outcome = ToolOutcome::from_dispatch(
            tier,
            dispatched.clone(),
            scope.language,
            focus.as_deref(),
            fence,
        );
        let (outcome, dispatched) = settle_sample(outcome, dispatched).await;
        // The user sees the facts; the model sees `outcome`. The gap is
        // carried by the event, observed on the two values held here — the
        // only place where they coexist.
        observer.observe(AgentEvent::CommandReported {
            call: handle.id(),
            tool: &call.name,
            outcome: &dispatched,
            withheld: outcome.withholds_from(&dispatched),
        });
        Ok(outcome.render(fence))
    }
}

/// What the model reads when its call is refused before reaching the bus.
///
/// One wording for every such refusal — an unknown tool, arguments that do not
/// fit, arguments that are not JSON at all —: the model corrects them the same
/// way, on its next turn.
fn rejection(
    observer: &dyn AgentObserver,
    tool: &str,
    err: &AiError,
    fence: &untrusted::Fence,
) -> String {
    tracing::debug!(tool = %tool, "tool call refused at translation");
    observer.observe(AgentEvent::CallRejected { tool, error: err });
    fence.fence(&format!("status: rejected\nerror: {err}"))
}

/// Releases a rendered sample, or says what actually happened.
///
/// The rendering is known: it is now, and only now, that the release is
/// recorded. A sample the rendering dropped is not released — nothing is
/// recorded, nothing is announced —, and the observer learns the refusal the
/// model reads, not "N rows sent".
async fn settle_sample(
    outcome: ToolOutcome,
    dispatched: DispatchOutcome,
) -> (ToolOutcome, DispatchOutcome) {
    let DispatchOutcome::Sampled { receipt, .. } = &dispatched else {
        return (outcome, dispatched);
    };
    match outcome {
        ToolOutcome::Sampled { .. } => {
            if receipt.release().await {
                (outcome, dispatched)
            } else {
                let reason = SAMPLE_UNRECORDED.to_owned();
                (
                    ToolOutcome::Denied {
                        reason: reason.clone(),
                    },
                    DispatchOutcome::Denied { reason },
                )
            }
        }
        ToolOutcome::Denied { reason } => (
            ToolOutcome::Denied {
                reason: reason.clone(),
            },
            DispatchOutcome::Denied { reason },
        ),
        // `from_sample` only returns those two; another variant sent nothing,
        // and releases nothing.
        other => (other, dispatched),
    }
}

/// An `io::Write` that keeps nothing: it measures a serialization.
struct ByteCount(usize);

impl std::io::Write for ByteCount {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Counts a stream event against the generation budget.
///
/// Only what the turn keeps counts: the text, the refusal, the reasoning, and
/// each complete call or reasoning block. Argument fragments count too, since
/// they are shown as the stream goes.
fn charge(
    budget: &mut oxyn_llm::GenerationBudget,
    event: &ChatEvent,
) -> Result<(), oxyn_llm::BudgetExceeded> {
    match event {
        ChatEvent::TextDelta(text) | ChatEvent::RefusalDelta(text) => budget.charge(text.len()),
        ChatEvent::ReasoningDelta { text, .. } => budget.charge(text.len()),
        ChatEvent::ToolCallDelta { arguments, .. } => budget.charge(arguments.len()),
        // The name only arrives here: without this count, a third-party
        // provider could lengthen it without limit.
        ChatEvent::ToolCallStarted { index, name, .. } => {
            budget.charge_tool_name(*index, 0, name.len())
        }
        ChatEvent::ToolCallComplete(call) => {
            // Measured without being copied: the write only counts.
            let mut counter = ByteCount(0);
            let size = serde_json::to_writer(&mut counter, &call.arguments)
                .map_or(usize::MAX, |()| counter.0);
            let index = u32::try_from(budget.tool_calls()).unwrap_or(u32::MAX);
            budget.check_tool_arguments(index, size)?;
            budget.open_tool_call()
        }
        // Its arguments were counted as fragments; it still counts as a call.
        ChatEvent::ToolCallInvalid { .. } => budget.open_tool_call(),
        ChatEvent::ReasoningComplete { block, .. } => {
            budget.open_block()?;
            match block {
                // An encrypted block arrives at once, without counted
                // fragments.
                ReasoningBlock::Redacted { data } => budget.charge(data.len()),
                _ => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

impl AgentRuntime {
    /// One round trip with the model.
    async fn one_turn(
        &self,
        session: &AgentSession,
        specs: &[oxyn_llm::ToolSpec],
        observer: &dyn AgentObserver,
        cancel: &CancelToken,
    ) -> Result<Turn, AiError> {
        let mut request = ChatRequest::new(self.model.clone(), session.messages.clone())
            .with_tools(specs.to_vec());
        if let Some(effort) = self.effort {
            request = request.with_reasoning_effort(effort);
        }
        let mut stream = self.provider.stream(request, cancel).await?;

        let mut text = String::new();
        let mut calls = Vec::new();
        let mut reasoning = Vec::new();
        let mut refusal = String::new();
        let mut stop = StopReason::Unspecified;
        let mut failure: Option<String> = None;
        // `oxyn-llm`'s providers already bound their stream; this count holds
        // for any `LlmProvider`, and this is where text and refusals
        // accumulate. Same limits, same type: two sets of caps would diverge.
        let mut budget = oxyn_llm::GenerationBudget::new();

        // No `select!` on cancellation: the dropped future could be dropped
        // after consuming bytes, leaving the decoder out of sync. The token is
        // cloned into the provider's stream anyway, which emits
        // `Done { Cancelled }` — re-reading it here only shortens the wait.
        while let Some(event) = stream.next().await {
            // Counted before being accumulated or shown. On overflow, the
            // stream is dropped — the connection closes — and the turn is a
            // cut: nothing it proposed is executed, and the provider may have
            // billed what we did not read (I-13).
            if let Err(limit) = charge(&mut budget, &event) {
                return Err(AiError::Interrupted(limit.to_string()));
            }
            match event {
                ChatEvent::TextDelta(delta) => {
                    // Notified before being accumulated: that is what makes
                    // the answer write itself as the stream goes, and keeps
                    // cancellation clickable meanwhile rather than between two
                    // turns.
                    observer.observe(AgentEvent::TextDelta { text: &delta });
                    text.push_str(&delta);
                }
                ChatEvent::ToolCallStarted { index, name, .. } => {
                    observer.observe(AgentEvent::ToolCallDrafted { index, tool: &name });
                }
                ChatEvent::ToolCallDelta { index, arguments } => {
                    // The partial JSON is shown as it arrives: that is what
                    // says, during a long call, that the model is still
                    // writing.
                    observer.observe(AgentEvent::ToolArgumentsDelta {
                        index,
                        fragment: &arguments,
                    });
                }
                ChatEvent::ReasoningDelta { text: fragment, .. } => {
                    observer.observe(AgentEvent::ThinkingDelta { text: &fragment });
                }
                ChatEvent::ReasoningComplete { block, .. } => {
                    // An encrypted block has no text: we say it took place
                    // rather than let it be believed the model did not think.
                    if matches!(block, ReasoningBlock::Redacted { .. }) {
                        observer.observe(AgentEvent::ThinkingRedacted);
                    }
                    // Kept as is: providers that sign their blocks refuse the
                    // next turn if one is missing.
                    reasoning.push(block);
                }
                // A refusal is not answer text: confusing them would pass
                // "I am not answering" off as an opinion about the database.
                ChatEvent::RefusalDelta(fragment) => refusal.push_str(&fragment),
                ChatEvent::Usage {
                    prompt_tokens,
                    completion_tokens,
                    cache_write_tokens,
                    cache_read_tokens,
                    reasoning_tokens,
                } => observer.observe(AgentEvent::Usage(TokenUsage {
                    input: prompt_tokens,
                    output: completion_tokens,
                    cache_read: cache_read_tokens,
                    cache_write: cache_write_tokens,
                    reasoning: reasoning_tokens,
                })),
                ChatEvent::ToolCallComplete(call) => calls.push(Proposed {
                    call,
                    unreadable: None,
                }),
                // Kept as a call, not as a failure of the turn: its siblings
                // still run, and the model reads why this one did not. The
                // history carries it with empty arguments — a protocol that
                // wants an object there refuses anything else, and the
                // rejection says what was wrong.
                ChatEvent::ToolCallInvalid { id, name, detail } => calls.push(Proposed {
                    call: ToolCall::new(id, name, serde_json::json!({})),
                    unreadable: Some(detail),
                }),
                // Kept, and reading goes on: the `Done` that follows says
                // whether the provider announced its failure or the stream was
                // cut. Leaving here lost `Interrupted`, and a possibly billed
                // turn became an ordinary, replayable failure (I-13).
                ChatEvent::Error(message) => failure = Some(message),
                ChatEvent::Done { stop_reason } => {
                    stop = stop_reason;
                    break;
                }
                // `ChatEvent` is `#[non_exhaustive]`: an event this version
                // does not know is not shown rather than given an invented
                // meaning.
                _ => {}
            }
            if cancel.is_cancelled() {
                break;
            }
        }

        let cancelled = cancel.is_cancelled() || stop == StopReason::Cancelled;
        if !cancelled {
            if stop.is_ambiguous() {
                return Err(AiError::Interrupted(failure.unwrap_or_else(|| {
                    "the stream ended without the provider closing the turn".to_owned()
                })));
            }
            // A failure announced by the provider: there is no doubt about
            // what happened on its side, and the turn can be asked again.
            if let Some(message) = failure {
                return Err(AiError::Provider(message));
            }
        }
        Ok(Turn {
            text,
            // A cancelled turn executes nothing: the user just asked precisely
            // for it to stop.
            calls: if cancelled { Vec::new() } else { calls },
            reasoning,
            refusal,
            truncated: stop.is_truncated(),
            stop,
            cancelled,
        })
    }
}

#[cfg(test)]
mod tests;
