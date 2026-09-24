//! Running a question: a provider conversation, or an external agent's.
//!
//! # What this module owns, and what it does not
//!
//! It owns **no** execution path: every command an agent submits goes through
//! [`ExecutorSink::for_agent`], the scheduler the user's own commands take,
//! and the `PolicyGate` decides — never this file, never the webview
//! ([I-01](../../../../../CLAUDE.md#i-01), [I-07](../../../../../CLAUDE.md#i-07)).
//!
//! It does not own the privacy tier either. The tier is read on the
//! connection **when the question is asked**, enters the prompt only through
//! `ContextBuilder` (or `AgentPrompt` for an external agent), and the runtime
//! re-checks it against a reach it measured itself
//! ([I-04](../../../../../CLAUDE.md#i-04)). A remembered session built under
//! another tier is not reused ([`super::threads`]).

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use oxyn_ai::external::locate::SearchPath;
use oxyn_ai::external::mcp::{TierSource, ToolService, ToolTurns};
use oxyn_ai::external::prompt::AgentPrompt;
use oxyn_ai::external::session::{AgentReady, ExternalError, ExternalSession, ToolBridge};
use oxyn_ai::external::turn::TurnEnd;
use oxyn_ai::tools::SampleAsk;
use oxyn_ai::{
    AgentEvent, AgentObserver, AgentOutcome, AgentRuntime, AgentSession, AiError, CommandSink,
    ContextBuilder, DispatchOutcome, ToolRegistry, ToolScope, sql_agent,
};
use oxyn_core::ai::MAX_PROVIDER_MODEL_BYTES;
use oxyn_core::{
    Actor, AgentId, AgentSessionId, AiProviderConfig, CancelToken, Capabilities, Command,
    ConnectionConfig, ConnectionId, Environment, ErrorClass, ExternalAgentConfig, PrivacyTier,
    Provenance, QueryLanguage, SessionId,
};
use oxyn_exec::{DispatchReport, Executor, ExecutorSink};
use oxyn_llm::Reach;
use oxyn_store::{EgressReach, EgressRecord};
use tauri::ipc::Channel;

use super::catalog_fill::{CatalogFill, Filled, Want};
use super::persistence;
use super::samples::{self, Grant, Offer, Presented, Recipient, RecipientKind, SampleRefused};
use super::threads::{AgentLink, LinkedAgent, Memory, Scope, Thread, WithdrawOnRelease};
use super::{Backend, check_endpoint};
use crate::backend::Inner;
use crate::ipc::ai::{
    AgentExit, AgentSettingAnswer, AgentSettingChange, AgentSettingsView, AgentStart, AiEvent,
    AiUpdate, AskRequest, AskStarted, ContextSummary, Cut, Destination, DestinationChoice, Ending,
    FailureCategory, MemoryReset, Money, PlanEntry, SignInHelp, SignInMethod, ThreadSummary,
    ThreadView, ToolStatus, error_class, preset_of, preset_sign_in,
};
use crate::ipc::ai::{SampleApproval, SampleRequest};
use crate::ipc::{CatalogAddress, IpcError, RelationField};
use sampling::Sampling;

/// The longest question accepted.
///
/// Product bound: a question is typed or pasted by a person, and a pasted dump
/// of several megabytes would go to a provider under the user's key.
const MAX_QUESTION_BYTES: usize = 32 * 1024;

/// Said when the session cannot run SQL: the built-in agent only writes SQL
/// ([ADR-0003](../../../../../docs/adr/0003-driver-capabilities.md)).
const NO_SQL: &str = "This session does not support SQL, and the assistant only writes SQL.";

/// Copies what a run observes into its node.
///
/// It filters nothing: the tier was applied upstream, where it is known, and
/// `withheld` says when the model received less than what passes here.
pub(super) struct Observer {
    thread: Arc<Thread>,
    node: u32,
}

impl fmt::Debug for Observer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Observer")
            .field("node", &self.node)
            .finish()
    }
}

impl AgentObserver for Observer {
    fn observe(&self, event: AgentEvent<'_>) {
        let emit = |event| self.thread.emit(self.node, event);
        match event {
            AgentEvent::TurnStarted { turn, max_turns } => {
                emit(AiEvent::TurnStarted { turn, max_turns });
            }
            AgentEvent::TextDelta { text } => emit(AiEvent::TextDelta {
                text: text.to_owned(),
            }),
            AgentEvent::ThinkingDelta { text } => emit(AiEvent::ThinkingDelta {
                text: text.to_owned(),
            }),
            AgentEvent::ThinkingRedacted => emit(AiEvent::ThinkingRedacted),
            AgentEvent::ToolCallDrafted { index, tool } => emit(AiEvent::ToolDraft {
                index,
                tool: tool.to_owned(),
            }),
            AgentEvent::ToolArgumentsDelta { index, fragment } => emit(AiEvent::ToolArguments {
                index,
                fragment: fragment.to_owned(),
            }),
            AgentEvent::Usage(usage) => emit(AiEvent::Usage {
                input: usage.input,
                output: usage.output,
                cache_read: usage.cache_read,
                cache_write: usage.cache_write,
            }),
            // Announced by the sink, which holds the statement.
            AgentEvent::CommandSubmitted {
                tool,
                command,
                connection,
                mutating,
            } => self.thread.open_call(tool, command, connection, mutating),
            AgentEvent::CommandReported {
                outcome, withheld, ..
            } => report_call(&self.thread, self.node, outcome, withheld),
            AgentEvent::CallRejected { tool, error } => emit(AiEvent::CallRejected {
                tool: tool.to_owned(),
                error: error.to_string(),
            }),
            // The whole plan, replacing the previous one: the protocol sends no
            // delta, and merging would grow the plan at every turn.
            AgentEvent::Plan { steps } => emit(AiEvent::Plan {
                entries: steps.iter().map(PlanEntry::of).collect(),
            }),
            AgentEvent::AgentSettings(declared) => emit(AiEvent::agent_settings(declared)),
            AgentEvent::ExternalToolCall { id, kind, status } => emit(AiEvent::AgentTool {
                id: id.to_owned(),
                tool: kind,
                status: status.into(),
            }),
            AgentEvent::PermissionRefused { kind, reason } => {
                emit(AiEvent::PermissionRefused {
                    action: kind,
                    reason,
                });
            }
            AgentEvent::ContextWindow { used, size, cost } => emit(AiEvent::ContextWindow {
                used,
                size,
                cost: cost.map(|(amount, currency)| Money {
                    amount,
                    currency: currency.to_owned(),
                }),
            }),
            AgentEvent::Finished { outcome } => emit(AiEvent::Finished {
                ending: Ending::of(outcome),
            }),
            // `AgentEvent` is `#[non_exhaustive]`: a step not shown, never one
            // invented.
            _ => {}
        }
    }
}

/// An external agent's refusal of a machine action arrives twice — as
/// `PermissionRefused` and, for older callers, as `CallRejected`. This panel
/// shows the first only.
struct AgentObserverFilter(Observer);

impl AgentObserver for AgentObserverFilter {
    fn observe(&self, event: AgentEvent<'_>) {
        if !matches!(event, AgentEvent::CallRejected { .. }) {
            self.0.observe(event);
        }
    }
}

/// The connection **name** a command targets — never its identifier.
fn name_target(
    connection: Option<ConnectionId>,
    scope: Option<&Scope>,
) -> (String, Option<Environment>) {
    match (connection, scope) {
        (Some(target), Some(scope)) if target == scope.connection => {
            (scope.name.clone(), Some(scope.environment))
        }
        (Some(_), _) => ("a connection outside this conversation".to_owned(), None),
        (None, _) => ("no connection".to_owned(), None),
    }
}

/// « 1 row », « 5 rows ».
fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Announces the open call with its statement. Returns its id and target.
fn announce_call(
    thread: &Thread,
    node: u32,
    statement: Option<String>,
) -> (u32, String, Option<Environment>) {
    let call = thread.announce();
    let (target, environment) = name_target(call.connection, thread.scope().as_ref());
    thread.emit(
        node,
        AiEvent::ToolCall {
            call: call.id,
            tool: call.tool,
            command: call.command.to_owned(),
            statement,
            connection: target.clone(),
            environment,
            mutating: call.mutating,
        },
    );
    (call.id, target, environment)
}

fn report_call(thread: &Thread, node: u32, outcome: &DispatchOutcome, withheld: bool) {
    let Some(current) = thread.take_call() else {
        return;
    };
    if !current.announced {
        let (connection, environment) = name_target(current.connection, thread.scope().as_ref());
        thread.emit(
            node,
            AiEvent::ToolCall {
                call: current.id,
                tool: current.tool.clone(),
                command: current.command.to_owned(),
                statement: None,
                connection,
                environment,
                mutating: current.mutating,
            },
        );
    }
    // Addressed by the front with the conversation's own connection: a result
    // of another one would be read under the wrong owner, so it is not shown.
    let own_connection = matches!(
        (current.connection, thread.scope()),
        (Some(target), Some(scope)) if target == scope.connection
    );
    let (status, detail, class) = match outcome {
        DispatchOutcome::Completed { summary } => (ToolStatus::Completed, summary.clone(), None),
        // What the model was told is its rendering, under the tier; the panel
        // says what happened, not what the catalog holds.
        DispatchOutcome::CatalogRead { .. } => (
            ToolStatus::Completed,
            "read the structure from Oxyn's local catalog".to_owned(),
            None,
        ),
        // Counts only, like `sampleApproved`: neither a value nor a column
        // name is kept in a conversation.
        DispatchOutcome::Sampled { sample, .. } => (
            ToolStatus::Completed,
            format!(
                "sent {} of {} you approved",
                counted(sample.rows.len(), "row"),
                counted(sample.columns.len(), "column")
            ),
            None,
        ),
        DispatchOutcome::AwaitingApproval { reason } => {
            (ToolStatus::AwaitingApproval, reason.clone(), None)
        }
        DispatchOutcome::Denied { reason } => (ToolStatus::Denied, reason.clone(), None),
        DispatchOutcome::Failed { class, message } => (
            if current.cancelled {
                ToolStatus::Cancelled
            } else {
                ToolStatus::Failed
            },
            message.clone(),
            Some(error_class(*class)),
        ),
        // Never read as a success: the user would act on an effect that may
        // not have happened.
        _ => (
            ToolStatus::Failed,
            "This Oxyn build cannot interpret the report of this command. Do not assume it ran."
                .to_owned(),
            None,
        ),
    };
    thread.emit(
        node,
        AiEvent::ToolReported {
            call: current.id,
            status,
            detail,
            error_class: class,
            withheld,
            // Only a completed command has a count; a failure's partial effect
            // is unknown, and saying 0 would be a claim.
            rows: matches!(status, ToolStatus::Completed)
                .then_some(current.rows)
                .flatten(),
            result: (matches!(status, ToolStatus::Completed) && own_connection)
                .then_some(current.result)
                .flatten()
                .map(|result| result.to_string()),
        },
    );
}

/// The connection's tier, read from the store at each tool call.
///
/// The same record `ai_ask` reads before a question, so a call and a question
/// never disagree on the tier. Off the async workers: the store is SQLite.
pub(super) struct StoredTier {
    pub(super) executor: Arc<Executor>,
    pub(super) connection: ConnectionId,
}

#[async_trait]
impl TierSource for StoredTier {
    async fn current(&self) -> Option<PrivacyTier> {
        let executor = Arc::clone(&self.executor);
        let connection = self.connection;
        tokio::task::spawn_blocking(move || executor.store().connections().get(connection))
            .await
            .ok()?
            .ok()?
            .map(|config| config.privacy_tier)
    }
}

/// Forgets a result the moment its guard drops.
struct ForgetResult {
    executor: Arc<Executor>,
    result: oxyn_core::ResultId,
}

impl Drop for ForgetResult {
    fn drop(&mut self) {
        self.executor.forget_result(self.result);
    }
}

/// The sink a conversation reaches the scheduler through.
///
/// Built on [`ExecutorSink::for_agent`], which refuses any actor but this
/// agent — `Actor::Human` included. What it adds is visibility only: the exact
/// statement before it runs, and, when the policy holds it back, what the user
/// needs to decide. It decides nothing.
pub(super) struct AgentSink {
    sink: ExecutorSink,
    executor: Arc<Executor>,
    thread: Arc<Thread>,
    node: u32,
    question: QuestionOpen,
    /// What an agent's request for a sample needs to reach the user. `None`
    /// refuses every request: without a screen, there is no path to a value.
    sampling: Option<Sampling>,
}

/// Whether the question a sink reports into still waits for its answer.
///
/// An external agent's call can come back after its question ended — the
/// prompt returned while a call was at the executor. A request for approval
/// born then would show under an answer marked finished, where nobody looks,
/// and stay approvable. Closing takes the same lock as showing a request, so a
/// request is either shown while the question is open or withdrawn.
#[derive(Clone)]
pub(super) struct QuestionOpen(Arc<parking_lot::Mutex<bool>>);

impl QuestionOpen {
    pub(super) fn new() -> Self {
        Self(Arc::new(parking_lot::Mutex::new(true)))
    }

    pub(super) fn close(&self) {
        *self.0.lock() = false;
    }
}

/// Closes the question on every way out of it, early returns included.
struct CloseQuestion(QuestionOpen);

impl Drop for CloseQuestion {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// Said to the agent when its request was withdrawn with the question.
const REQUEST_WITHDRAWN: &str = "the question this call answered has ended, so its request for \
     approval was withdrawn; nothing ran";

impl fmt::Debug for AgentSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentSink").finish_non_exhaustive()
    }
}

#[async_trait]
impl CommandSink for AgentSink {
    async fn dispatch(
        &self,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> DispatchOutcome {
        let statement = match &command {
            Command::Execute { request, .. } => Some(request.text.clone()),
            _ => None,
        };
        let (call, connection, environment) =
            announce_call(&self.thread, self.node, statement.clone());
        let mutating = command.is_mutating();
        // `describe_schema` reads the cache as it stands: what it would
        // describe and the cache does not hold is read first, as the agent's
        // own reads — its tool call is at their origin (ADR-0036).
        if let Command::DescribeCatalog {
            connection: target,
            focus,
        } = &command
        {
            self.complete_catalog(
                actor,
                *target,
                Want::Search(focus.as_deref().unwrap_or_default()),
                cancel,
            )
            .await;
        }
        let relist = match &command {
            Command::RefreshCatalog { connection } => Some(*connection),
            _ => None,
        };
        let mut report = self.sink.dispatch(actor, command, cancel).await;
        // `refresh_catalog` reads the server level again, then the relations
        // of each schema — within the same bounds — so that what it refreshes
        // is what a question needs, not only the first level.
        if let (Some(target), DispatchReport::Completed { summary, .. }) = (relist, &mut report) {
            let filled = self
                .complete_catalog(actor, target, Want::Relist, cancel)
                .await;
            summary.push_str(&format!(
                "; {} lists of schemas and their objects were read again",
                filled.listed
            ));
            if filled.unlisted > 0 || filled.stopped.is_some() || filled.failed > 0 {
                summary.push_str(&format!(
                    "; {} schemas are still not listed, and {} reads failed",
                    filled.unlisted, filled.failed
                ));
            }
        }
        // Held until the request is shown: `close` waits for it, or the request
        // sees the question closed and is withdrawn.
        let open = self.question.0.lock();
        if let DispatchReport::AwaitingApproval { command, .. } = &report
            && !*open
        {
            // Withdrawn by the executor itself, as a refusal it already knows:
            // hiding it would leave it approvable.
            let _withdrawn = self.executor.reject(*command);
            return DispatchOutcome::Denied {
                reason: REQUEST_WITHDRAWN.to_owned(),
            };
        }
        // A refused write ran nothing; any other answer — held for approval,
        // completed, failed, even cancelled — may leave an effect behind.
        if mutating && !matches!(report, DispatchReport::Denied { .. }) {
            self.thread.note_write();
        }
        match &report {
            DispatchReport::AwaitingApproval { command, reason } => {
                // The reclassified statement, as the executor will run it if
                // the user agrees.
                let held = self
                    .executor
                    .approvals()
                    .pending()
                    .into_iter()
                    .find(|pending| pending.id == *command)
                    .and_then(|pending| pending.preview)
                    .map(|preview| preview.statement);
                self.thread.emit(
                    self.node,
                    AiEvent::ApprovalRequested {
                        call,
                        approval: command.to_string(),
                        reason: reason.clone(),
                        statement: held.or(statement).unwrap_or_default(),
                        connection,
                        environment,
                        actor: "agent",
                    },
                );
            }
            // The result goes to the thread, for the panel; `translate` below
            // drops it, so the model never learns it exists.
            DispatchReport::Completed {
                stats: Some(stats),
                result,
                ..
            } => self.thread.record_rows(call, stats.rows, *result),
            DispatchReport::Failed { .. } if cancel.is_cancelled() => {
                self.thread.mark_cancelled(call);
            }
            _ => {}
        }
        drop(open);
        translate(report)
    }

    /// The user decides, on the approval screen; then the read goes through
    /// [`CommandSink::dispatch`]'s own sink. See [`sampling`].
    async fn request_sample(
        &self,
        actor: Actor,
        ask: SampleAsk,
        cancel: &CancelToken,
    ) -> DispatchOutcome {
        self.sample(actor, ask, cancel).await
    }
}

impl AgentSink {
    /// Reads what an agent's tool call needs and the catalog does not hold,
    /// as that agent, showing the step on its question.
    async fn complete_catalog(
        &self,
        actor: Actor,
        connection: ConnectionId,
        want: Want<'_>,
        cancel: &CancelToken,
    ) -> Filled {
        let Some(catalog) = self.executor.catalog(connection) else {
            return Filled::default();
        };
        // The agent-bound sink: any actor but this agent is refused there.
        let fill = CatalogFill {
            sink: &self.sink,
            actor,
            connection,
            catalog,
        };
        let emit = |event| self.thread.emit(self.node, event);
        fill.run(want, cancel, &emit).await
    }
}

/// A report in the runtime's terms: a correspondence of variants, no judgment.
fn translate(report: DispatchReport) -> DispatchOutcome {
    match report {
        DispatchReport::Completed { summary, .. } => DispatchOutcome::Completed { summary },
        DispatchReport::CatalogRead { catalog, .. } => DispatchOutcome::CatalogRead { catalog },
        DispatchReport::AwaitingApproval { reason, .. } => {
            DispatchOutcome::AwaitingApproval { reason }
        }
        DispatchReport::Denied { reason, .. } => DispatchOutcome::Denied { reason },
        DispatchReport::Failed { class, error, .. } => DispatchOutcome::Failed {
            class,
            message: error,
        },
        // A refusal is the only rendering that does not lie: a success would
        // let the model assume an effect, a retryable failure would invite a
        // replay of a command that may have run (I-13).
        other => {
            tracing::error!(
                command = %other.command(),
                "execution report not translated: the desktop agent sink is behind oxyn-exec"
            );
            DispatchOutcome::Denied {
                reason: "this Oxyn build cannot interpret the result of that command; \
                         do not assume it ran, and do not retry it"
                    .to_owned(),
            }
        }
    }
}

/// The destination as the store records it: the declaration answering this
/// exchange, named even when it is later removed.
fn stored_destination(resolved: &Resolved) -> oxyn_store::conversations::Destination {
    match resolved {
        Resolved::Provider { config, model, .. } => {
            oxyn_store::conversations::Destination::provider(
                config.id.clone(),
                config.label.clone(),
                model.clone(),
            )
        }
        Resolved::Agent(agent) => oxyn_store::conversations::Destination::external_agent(
            agent.id.clone(),
            agent.label.clone(),
        ),
    }
}

/// Who answers, once resolved against what is declared.
enum Resolved {
    Provider {
        config: AiProviderConfig,
        model: String,
        effort: Option<oxyn_llm::ReasoningEffort>,
    },
    Agent(ExternalAgentConfig),
}

/// Why a run could not go on, as the panel shows it.
pub(super) struct Failure {
    message: String,
    category: FailureCategory,
    /// The error's class, when the failure carries one. Transmitted, never
    /// deduced from the message.
    class: Option<ErrorClass>,
    /// Boxed, like `exit`: rare, and every `Result` of this module carries a
    /// `Failure`.
    sign_in: Option<Box<SignInHelp>>,
    found_elsewhere: Option<String>,
    /// What an agent's process said as it died, when it did.
    exit: Option<Box<AgentExit>>,
    /// The agent's session can no longer answer — dead, stopped for its
    /// start, or out of its confinement. The conversation lets it go, so
    /// asking again starts a new one, as the failure says.
    ends_agent: bool,
}

/// Said after a failure that followed a write, so nobody reads the failure as
/// « nothing happened ».
const MAY_HAVE_APPLIED: &str = "A command from this answer may already have been applied: check \
     the data before asking again.";

impl Failure {
    fn new(message: impl Into<String>, category: FailureCategory) -> Self {
        Self {
            message: message.into(),
            category,
            class: None,
            sign_in: None,
            found_elsewhere: None,
            exit: None,
            ends_agent: false,
        }
    }

    /// Whether offering the same question again is safe — decided here, where
    /// the class and the run's writes are known, and never in the webview from
    /// a category or a message (front.md, [I-13](../../../../../CLAUDE.md#i-13)).
    fn retryable(&self, wrote: bool) -> bool {
        if wrote || self.class == Some(ErrorClass::Ambiguous) {
            return false;
        }
        !matches!(
            self.category,
            FailureCategory::Refused | FailureCategory::AgentIncompatible
        )
    }

    /// `wrote`: a command that may change data reached the executor during
    /// the run.
    fn into_event(self, wrote: bool) -> AiEvent {
        let retryable = self.retryable(wrote);
        AiEvent::Failed {
            message: if wrote {
                format!("{} {MAY_HAVE_APPLIED}", self.message)
            } else {
                self.message
            },
            category: self.category,
            retryable,
            sign_in: self.sign_in.map(|help| *help),
            found_elsewhere: self.found_elsewhere,
            exit: self.exit.map(|exit| *exit),
        }
    }

    /// As the panel's start of an agent reports it: the same words, the same
    /// help, and nothing to ask again — a new start is a click.
    fn into_start(self) -> AgentStart {
        AgentStart::Failed {
            message: self.message,
            category: self.category,
            sign_in: self.sign_in.map(|help| *help),
            found_elsewhere: self.found_elsewhere,
            exit: self.exit.map(|exit| *exit),
        }
    }
}

impl Backend {
    /// Asks the assistant a question, and returns at once with where it landed.
    ///
    /// What follows streams on `channel`, each event tagged with its node: the
    /// question, `started`, turns, reasoning, text, tool calls before their
    /// reports, and exactly one `finished` or `failed`.
    ///
    /// Refused here, before anything starts: an empty question, a session
    /// without SQL, an unknown declaration, an external agent on a `Local`
    /// connection (refused **before launch**, ADR-0026), a run already going on
    /// in this conversation.
    pub async fn ai_ask(
        &self,
        request: AskRequest,
        channel: Channel<AiUpdate>,
    ) -> Result<AskStarted, IpcError> {
        // Taken out before anything can refuse the question: a grant presented
        // is spent, whether the question starts or not.
        let grant = request
            .sample
            .as_ref()
            .map(|approval| self.inner.ai.samples.take(&approval.request));
        let connection: ConnectionId = request
            .connection
            .parse()
            .map_err(|error| IpcError::invalid(format!("invalid connection: {error}")))?;
        let session: SessionId = request
            .session
            .parse()
            .map_err(|error| IpcError::invalid(format!("invalid session: {error}")))?;
        let question = request.question.trim().to_owned();
        if question.is_empty() {
            return Err(IpcError::invalid("Ask a question first"));
        }
        if question.len() > MAX_QUESTION_BYTES {
            return Err(IpcError::invalid(format!(
                "A question is limited to {} KiB",
                MAX_QUESTION_BYTES / 1024
            )));
        }
        // Their shape only: what they name is checked against the catalog by
        // the gate, when the context is built.
        let mentioned = super::mentions::parse(&request.mentions)?;

        // Read now, not when the panel opened: a tier changed since then must
        // govern this question (I-04).
        let config = self.read_config(connection).await?;
        let capabilities = self
            .inner
            .executor
            .sessions()
            .get(session)
            .ok_or_else(|| IpcError::invalid("This session is no longer open"))?
            .capabilities();
        if !capabilities.contains(Capabilities::SQL) {
            return Err(IpcError::invalid(NO_SQL));
        }

        let resolved = match self
            .resolve_destination(&request.destination, config.privacy_tier)
            .await
        {
            Ok(resolved) => resolved,
            Err(refused) => {
                // A question refused here — a connection now local-only, a
                // declaration gone — says the agents kept for this connection
                // were launched under something that no longer holds.
                self.inner.ai.release_agents(connection);
                return Err(refused);
            }
        };
        let thread = self
            .inner
            .ai
            .thread_for(connection, request.thread.as_deref())?;
        // The question as it was asked: a grant for a new conversation names
        // none, whatever id the conversation gets now.
        let asked_in = request.thread.clone();
        let approval = request.sample.clone();
        // Read before the node opens: the question is shown and written with
        // its chips, named from the catalog and the library. Local reads only
        // — the cache, one store read per saved query.
        let named =
            super::mentions::read(&self.inner, connection, mentioned, &CancelToken::new()).await;
        let (node, cancel) = thread.begin(
            request.parent,
            &question,
            channel,
            Scope {
                connection,
                name: config.name.clone(),
                environment: config.environment,
            },
        )?;
        thread.name_mentions(node, named.views.clone());
        thread.emit(
            node,
            AiEvent::Question {
                text: question.clone(),
                mentions: named.views.clone(),
            },
        );
        let started = AskStarted {
            thread: thread.id(),
            node,
        };
        // Written before the run starts, so an approved sample's egress entry
        // can name the conversation it left for. A failure is said once per
        // thread and holds nothing back: a transcript is a comfort, not a
        // barrier.
        if !self
            .save_question(&thread, &config, &resolved, request.parent, node, &question)
            .await
            && thread.warn_unsaved()
        {
            thread.emit(node, AiEvent::NotSaved);
        }

        let inner = Arc::clone(&self.inner);
        tauri::async_runtime::spawn(async move {
            let run = Run {
                inner: &inner,
                thread: &thread,
                node,
                parent: request.parent,
                connection: &config,
                cancel: &cancel,
                mentions: &named,
            };
            let result = async {
                // Checked before anything else, whatever the destination: a
                // refusal reads nothing.
                let sample = match (&approval, grant) {
                    (Some(approval), Some(grant)) => {
                        let recipient = match &resolved {
                            Resolved::Provider { config, model, .. } => Recipient::provider(
                                config.id.clone(),
                                model.clone(),
                                // Classified again: an address edited since the
                                // offer must not inherit its approval.
                                classify(&config.base_url).await?,
                            ),
                            Resolved::Agent(agent) => Recipient::agent(agent),
                        };
                        let tiers = StoredTier {
                            executor: Arc::clone(&inner.executor),
                            connection,
                        };
                        Some(
                            run.take_sample(
                                session,
                                approval,
                                grant,
                                asked_in.as_deref(),
                                &recipient,
                                &tiers,
                            )
                            .await?,
                        )
                    }
                    _ => None,
                };
                match resolved {
                    Resolved::Provider {
                        config: provider,
                        model,
                        effort,
                    } => {
                        run.converse(session, provider, model, effort, sample, question)
                            .await
                    }
                    Resolved::Agent(agent) => {
                        inner.ai.release_agents_except(connection, thread.id);
                        run.ask_agent(session, &agent, &question, sample).await
                    }
                }
            }
            .await;
            let wrote = thread.wrote();
            let failed = result
                .as_ref()
                .err()
                .map(|failure| (failure.category, failure.retryable(wrote)));
            if let Err(failure) = result {
                thread.emit(node, failure.into_event(wrote));
            }
            // After the last event of the run: what is written is what the
            // panel showed.
            persistence::save_answer(&inner.executor, &thread, node, config.privacy_tier, failed)
                .await;
            thread.finish(node);
        });
        Ok(started)
    }

    /// Opens the thread in the store if needed, then writes the question.
    ///
    /// Answers whether the exchange is now written: a thread whose header or
    /// question could not be written keeps answering, and says so once.
    async fn save_question(
        &self,
        thread: &Arc<Thread>,
        config: &ConnectionConfig,
        resolved: &Resolved,
        parent: Option<u32>,
        node: u32,
        question: &str,
    ) -> bool {
        // A question whose parent was never written would change branch: the
        // exchange is left out rather than re-parented.
        let parent_stored = match parent {
            Some(parent) => match thread.stored_node(parent) {
                Some(stored) => Some(stored),
                None => return false,
            },
            None => None,
        };
        let destination = stored_destination(resolved);
        let record = persistence::question_of(
            parent_stored,
            config.privacy_tier,
            question,
            destination.clone(),
        )
        .with_mentions(super::mentions::stored(&thread.mentions_of(node)));
        let executor = Arc::clone(&self.inner.executor);
        let id = thread.id;
        let header = thread.conversation().is_none().then(|| {
            let mut header = oxyn_store::conversations::Conversation::new(
                executor.workspace(),
                destination,
                thread.title(),
            )
            .on_connection(config.id, config.name.clone());
            // The window's thread and the store's row are one conversation.
            header.id = id;
            header
        });
        let written = tokio::task::spawn_blocking(move || {
            let conversations = executor.store().conversations();
            if let Some(header) = header {
                conversations.save(&header)?;
            }
            let stored = conversations.append_exchange(id, &record)?;
            // The branch a reopened conversation shows is the one being
            // written: without it, the thread could not be read back.
            if let Some(node) = stored {
                conversations.select(id, Some(node))?;
            }
            Ok::<_, oxyn_store::StoreError>(stored)
        })
        .await;
        match written {
            Ok(Ok(Some(stored))) => {
                thread.remember_conversation();
                thread.map_node(node, stored);
                true
            }
            // `None` is a thread deleted elsewhere, an error is a disk that
            // refused: neither stops the question.
            _ => false,
        }
    }

    /// Stops what a conversation is running, if anything.
    pub fn ai_cancel(&self, connection: ConnectionId, thread: &str) -> Result<bool, IpcError> {
        Ok(self.inner.ai.find(connection, thread)?.cancel())
    }

    /// The conversations of a connection, most recent first.
    #[must_use]
    pub async fn ai_threads(&self, connection: ConnectionId) -> Vec<ThreadSummary> {
        let live = self.inner.ai.list(connection);
        let executor = Arc::clone(&self.inner.executor);
        let saved =
            tokio::task::spawn_blocking(move || persistence::history(&executor, connection))
                .await
                .unwrap_or_default();
        // The window's own threads win: they know what is running, and a
        // thread open here is the same conversation as its row.
        let mut all = live;
        let known: std::collections::HashSet<String> =
            all.iter().map(|summary| summary.id.clone()).collect();
        all.extend(
            saved
                .into_iter()
                .filter(|summary| !known.contains(&summary.id)),
        );
        all.sort_by_key(|summary| std::cmp::Reverse(summary.updated_at_ms));
        all
    }

    /// A whole conversation, and `channel` as its live stream from now on.
    ///
    /// A read: nothing is asked again. This is how the panel takes a
    /// conversation back after the webview reloaded.
    pub async fn ai_open_thread(
        &self,
        connection: ConnectionId,
        thread: &str,
        channel: Channel<AiUpdate>,
    ) -> Result<ThreadView, IpcError> {
        if let Ok(live) = self.inner.ai.find(connection, thread) {
            return Ok(live.view(Some(channel)));
        }
        // Not in this window: read back from the workspace, and shown without
        // a memory — nothing read from disk goes to a model.
        let restored = self.load_thread(connection, thread).await?;
        Ok(self
            .inner
            .ai
            .adopt(connection, restored)
            .view(Some(channel)))
    }

    /// One conversation of the workspace, as far back as the panel holds.
    async fn load_thread(
        &self,
        connection: ConnectionId,
        thread: &str,
    ) -> Result<persistence::Restored, IpcError> {
        let id: oxyn_core::ConversationId = thread
            .parse()
            .map_err(|_| IpcError::invalid("This conversation does not exist"))?;
        let executor = Arc::clone(&self.inner.executor);
        tokio::task::spawn_blocking(move || persistence::load(&executor, connection, id))
            .await
            .map_err(|_| IpcError::invalid("Reading this conversation failed"))?
    }

    pub async fn ai_rename_thread(
        &self,
        connection: ConnectionId,
        thread: &str,
        title: &str,
    ) -> Result<(), IpcError> {
        let found = self.inner.ai.find(connection, thread)?;
        found.rename(title)?;
        let executor = Arc::clone(&self.inner.executor);
        let (id, title) = (found.id, found.title());
        // The title the store keeps is the one the panel shows: bounded there.
        let _written = tokio::task::spawn_blocking(move || {
            executor.store().conversations().rename(id, &title)
        })
        .await;
        Ok(())
    }

    pub async fn ai_delete_thread(
        &self,
        connection: ConnectionId,
        thread: &str,
    ) -> Result<(), IpcError> {
        let id: oxyn_core::ConversationId = thread
            .parse()
            .map_err(|_| IpcError::invalid("This conversation does not exist"))?;
        // The rows its calls left are released with it: nothing can show them
        // once the conversation is gone.
        let results = self
            .inner
            .ai
            .find(connection, thread)
            .map(|live| live.results())
            .unwrap_or_default();
        // A conversation open in this window is closed first; one that is only
        // on disk is deleted all the same.
        let _live = self.inner.ai.delete(connection, thread);
        let executor = Arc::clone(&self.inner.executor);
        // `ai_egress` is untouched: what left this machine outlives the
        // conversation that sent it (SECURITY).
        let _written = tokio::task::spawn_blocking(move || {
            // Forgetting may delete spill files: off the async workers.
            for result in results {
                executor.forget_result(result);
            }
            executor.store().conversations().delete(id)
        })
        .await;
        Ok(())
    }

    /// Shows another version of an exchange.
    pub async fn ai_select_version(
        &self,
        connection: ConnectionId,
        thread: &str,
        node: u32,
    ) -> Result<(), IpcError> {
        let found = self.inner.ai.find(connection, thread)?;
        found.select(node)?;
        let (Some(id), Some(stored)) = (found.conversation(), found.stored_node(node)) else {
            return Ok(());
        };
        let executor = Arc::clone(&self.inner.executor);
        let _written = tokio::task::spawn_blocking(move || {
            executor.store().conversations().select(id, Some(stored))
        })
        .await;
        Ok(())
    }

    /// Asks the external agent of a conversation to run one of its sign-in
    /// methods. The agent does it — usually in a browser; Oxyn sees no token.
    pub async fn ai_authenticate(
        &self,
        connection: ConnectionId,
        thread: Option<&str>,
        method: &str,
    ) -> Result<(), IpcError> {
        let session = self.agent_to_ask(connection, thread)?;
        session
            .authenticate(method)
            .await
            .map_err(|error| IpcError::invalid(error.to_string()))
    }

    /// Asks the conversation's agent to switch mode or set an option.
    ///
    /// A refusal — here or by the agent — is an answer, not an error: the
    /// selector says why. An accepted change answers with the settings as the
    /// agent declared them in reply.
    ///
    /// # Errors
    /// No such conversation, no agent running, or the agent gone.
    pub async fn ai_set_agent_setting(
        &self,
        connection: ConnectionId,
        thread: Option<&str>,
        change: AgentSettingChange,
    ) -> Result<AgentSettingAnswer, IpcError> {
        let session = self.agent_to_ask(connection, thread)?;
        match session.change_setting(change.into_change()).await {
            Ok(settings) => Ok(AgentSettingAnswer::Sent {
                settings: AgentSettingsView::of(&settings),
            }),
            Err(ExternalError::Setting(refusal)) => Ok(AgentSettingAnswer::refused(&refusal)),
            Err(error) => Err(IpcError::invalid(error.to_string())),
        }
    }

    /// The agent a sign-in or a settings change is for: the one the panel
    /// started ahead of the next question, when there is one — it is the one
    /// that will answer —, else the conversation's own.
    ///
    /// The panel only starts one when the conversation's own would not answer
    /// next, and starting from a conversation whose agent would releases it
    /// (`ai_start_agent`): the two never compete.
    fn agent_to_ask(
        &self,
        connection: ConnectionId,
        thread: Option<&str>,
    ) -> Result<Arc<ExternalSession>, IpcError> {
        if let Some(waiting) = self.inner.ai.waiting_session(connection) {
            return Ok(waiting);
        }
        let live = match thread {
            Some(thread) => self.inner.ai.find(connection, thread)?.live_agent(),
            None => None,
        };
        live.ok_or_else(|| {
            IpcError::invalid("The agent is no longer running. Ask your question again.")
        })
    }

    /// Offers a row sample of `address` for the next question of a
    /// conversation — never a value, only what would be read and where it
    /// would go.
    ///
    /// Offered under `Sampled` only: under any other tier the offer does not
    /// exist. Offered for a built-in provider and for an external agent alike
    /// — the sample enters either prompt through the `ContextBuilder`
    /// ([ADR-0034](../../../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
    /// The grant it issues is checked again, tier first, when the question
    /// presents it.
    ///
    /// # Errors
    /// Another tier, an unknown destination, a relation absent from the
    /// catalog.
    pub async fn ai_request_sample(
        &self,
        connection: ConnectionId,
        thread: Option<String>,
        parent: Option<u32>,
        address: CatalogAddress,
        destination: DestinationChoice,
    ) -> Result<SampleRequest, IpcError> {
        let config = self.read_config(connection).await?;
        if config.privacy_tier != PrivacyTier::Sampled {
            return Err(IpcError::invalid(
                "Row samples are offered only on a connection whose privacy tier is Sampled. \
                 Nothing was read.",
            ));
        }
        let (recipient, label) = match self
            .resolve_destination(&destination, config.privacy_tier)
            .await?
        {
            Resolved::Provider {
                config: provider,
                model,
                ..
            } => {
                let reach = classify(&provider.base_url)
                    .await
                    .map_err(|failure| IpcError::invalid(failure.message))?;
                (
                    Recipient::provider(provider.id.clone(), model, reach),
                    provider.label,
                )
            }
            Resolved::Agent(agent) => (Recipient::agent(&agent), agent.label),
        };
        let reach = recipient.reach;
        let path = address.to_path()?;
        let relation = self
            .inner
            .executor
            .catalog(connection)
            .and_then(|catalog| catalog.read().relation(&path).cloned())
            .ok_or_else(|| {
                IpcError::invalid(
                    "This relation is not in the catalog yet: open it in the explorer first.",
                )
            })?;
        // TODO(2026-12-31, column classification declared by a driver): leave
        // out the columns a driver classifies as secret; owned by ia-tauri.
        // None declares one yet, and no filter pretends to: the screen names
        // every column, and the user unticks.
        let fields: Vec<RelationField> = relation.fields.iter().map(RelationField::from).collect();
        let offered = fields.iter().map(|field| field.name.clone()).collect();
        let id = self.inner.ai.samples.issue(Offer {
            connection,
            thread,
            parent,
            source: path.clone(),
            offered,
            recipient,
        });
        Ok(SampleRequest {
            id,
            // The user pinned it: nobody else asked.
            requested_by: None,
            source: path.to_string(),
            address: CatalogAddress::of(&path),
            rows: samples::MAX_SAMPLE_ROWS,
            fields,
            destination: label,
            reach: reach.into(),
        })
    }

    /// Withdraws a sample offer the user declined: its grant can no longer be
    /// presented.
    pub fn ai_withdraw_sample(&self, connection: ConnectionId, request: &str) {
        self.inner.ai.samples.withdraw(connection, request);
    }

    /// The user's answer to an agent's request for a sample: the ticked
    /// columns, or `None` to decline. The call waiting on it reads the rows —
    /// or says « declined » — itself.
    ///
    /// The only way an agent's request is approved: this is a Tauri command,
    /// the user's gesture, and nothing an agent sends reaches it.
    ///
    /// # Errors
    /// The request is unknown, answered, expired or of another connection;
    /// or the columns were not offered — which declines it.
    pub fn ai_answer_sample(
        &self,
        connection: ConnectionId,
        request: &str,
        columns: Option<&[String]>,
    ) -> Result<(), IpcError> {
        self.inner
            .ai
            .asks
            .answer(connection, request, columns)
            .map_err(|refused| IpcError::invalid(refused.to_string()))
    }

    /// Closes a connection's conversations. To call when the connection closes.
    pub fn close_ai_conversation(&self, connection: ConnectionId) {
        self.inner.ai.forget(connection);
    }

    async fn resolve_destination(
        &self,
        choice: &DestinationChoice,
        tier: PrivacyTier,
    ) -> Result<Resolved, IpcError> {
        match choice {
            DestinationChoice::Provider { id, model, effort } => {
                let config = self
                    .declared_providers()
                    .await?
                    .into_iter()
                    .find(|config| config.id.as_str() == id)
                    .ok_or_else(|| IpcError::invalid("This provider is no longer declared"))?;
                let model = match model.as_deref().map(str::trim) {
                    Some(model) if !model.is_empty() => {
                        if model.len() > MAX_PROVIDER_MODEL_BYTES {
                            return Err(IpcError::invalid("A model name is limited to 128 bytes"));
                        }
                        model.to_owned()
                    }
                    _ => config.model.clone(),
                };
                Ok(Resolved::Provider {
                    config,
                    model,
                    effort: *effort,
                })
            }
            DestinationChoice::Agent { id } => {
                let agent = self
                    .declared_agents()
                    .await?
                    .into_iter()
                    .find(|agent| agent.id.as_str() == id)
                    .ok_or_else(|| IpcError::invalid("This agent is no longer declared"))?;
                if !oxyn_ai::privacy::allows_external_agent(tier, &agent) {
                    return Err(IpcError::invalid(
                        "This connection is local-only, and Oxyn cannot see where an external \
                         agent sends its prompts. The agent was not started.",
                    ));
                }
                Ok(Resolved::Agent(agent))
            }
        }
    }
}

/// Said when the read itself failed; the journal keeps the error.
const SAMPLE_READ_FAILED: &str = "Reading the approved sample failed on the server, so nothing was sent. The command \
     journal has the error.";

/// A sample read and admitted, with what it was admitted under.
pub(super) struct ApprovedSample {
    rows: oxyn_ai::context::RowSample,
    /// The reach the recipient was classified at when the grant was checked.
    reach: Reach,
    /// The tier read back after the read: `Sampled`.
    tier: PrivacyTier,
    /// What will have left, written just before it does.
    egress: EgressRecord,
}

/// The reach as the audit stores it: every variant named, so a new one does
/// not fall silently into another.
const fn egress_reach(reach: Reach) -> EgressReach {
    match reach {
        Reach::Local => EgressReach::Local,
        Reach::Remote => EgressReach::Remote,
        Reach::Unresolved => EgressReach::Unresolved,
    }
}

/// Appends to `ai_egress`, off the async workers: the store is SQLite. Answers
/// whether it was written — never the store's words, nor the record: a name
/// can be the data.
async fn append_egress(executor: &Arc<Executor>, record: EgressRecord) -> bool {
    let executor = Arc::clone(executor);
    let written =
        tokio::task::spawn_blocking(move || executor.store().egress().append(&record)).await;
    matches!(written, Ok(Ok(_)))
}

/// Classifies a provider's address, off the async workers: it resolves a name.
async fn classify(base_url: &str) -> Result<Reach, Failure> {
    let base_url = base_url.to_owned();
    tokio::task::spawn_blocking(move || oxyn_llm::endpoint_reach(&base_url))
        .await
        .map_err(|error| setup(format!("classifying the endpoint: {error}")))
}

/// One run, and what it needs.
struct Run<'a> {
    inner: &'a Inner,
    thread: &'a Arc<Thread>,
    node: u32,
    parent: Option<u32>,
    connection: &'a ConnectionConfig,
    cancel: &'a CancelToken,
    /// What the user named with `@`, for whichever destination answers.
    mentions: &'a super::mentions::Named,
}

impl Run<'_> {
    fn emit(&self, event: AiEvent) {
        self.thread.emit(self.node, event);
    }

    /// Reads what the context of this question needs and the catalog does not
    /// hold yet, within the bounds of [`super::catalog_fill`], showing the step.
    ///
    /// As [`Actor::Human`]: the question is the user's gesture, and what it
    /// makes Oxyn read is decided by Oxyn — the question's words and its
    /// mentions select, as they select for the context — never by a model.
    /// The reads are the tree's expansions, through the same bus and gate
    /// ([ADR-0036](../../../../../docs/adr/0036-l-assistant-complete-le-catalogue.md)).
    async fn complete_catalog(&self, want: Want<'_>) {
        let Some(catalog) = self.inner.executor.catalog(self.connection.id) else {
            return;
        };
        let sink = ExecutorSink::new(Arc::clone(&self.inner.executor));
        let fill = CatalogFill {
            sink: &sink,
            actor: Actor::Human,
            connection: self.connection.id,
            catalog,
        };
        let emit = |event| self.emit(event);
        fill.run(want, self.cancel, &emit).await;
    }

    /// What the context of a question needs: the whole selection for a
    /// session that starts, its mentions alone for one that follows.
    fn want<'q>(&'q self, question: &'q str, follows: bool) -> Want<'q> {
        if follows {
            Want::Mentions(&self.mentions.mentions)
        } else {
            Want::Question {
                focus: question,
                mentions: &self.mentions.mentions,
            }
        }
    }

    /// Completes the catalog, then assembles the provider conversation from it.
    ///
    /// The completion follows [`Self::prepare_dialogue`]'s own choice: a
    /// remembered session under the same tier receives its mentions only, so
    /// only they are read.
    async fn prepare(
        &self,
        agent: &oxyn_ai::AgentSpec,
        session: SessionId,
        question: &str,
        sample: Option<oxyn_ai::context::RowSample>,
        tier: PrivacyTier,
    ) -> Result<(AgentSession, Option<oxyn_ai::AgentContext>), Failure> {
        let follows = sample.is_none()
            && self
                .thread
                .memory_from(self.parent)
                .is_some_and(|memory| memory.tier == tier);
        self.complete_catalog(self.want(question, follows)).await;
        self.prepare_dialogue(agent, session, question, sample, tier)
    }

    fn observer(&self) -> Observer {
        Observer {
            thread: Arc::clone(self.thread),
            node: self.node,
        }
    }

    /// Checks the question's sample grant, already taken out, and reads what
    /// it admits.
    ///
    /// Every check of [`samples::consume`] passes before the read, and the
    /// tier it checks is read from `tiers` **now**, not when the sample was
    /// offered — then **again after the read**: a tier lowered while the
    /// preview ran sends nothing, and the tier re-read is the one the prompt
    /// is built under. The read is an ordinary preview on the bus, as the
    /// user: the audit shows what was really read, and only the ticked columns
    /// are copied out of it.
    async fn take_sample(
        &self,
        session: SessionId,
        approval: &SampleApproval,
        grant: Result<Grant, SampleRefused>,
        asked_in: Option<&str>,
        recipient: &Recipient,
        tiers: &dyn TierSource,
    ) -> Result<ApprovedSample, Failure> {
        let refused =
            |refusal: SampleRefused| Failure::new(refusal.to_string(), FailureCategory::Refused);
        let Ok(source) = approval.source.to_path() else {
            // Not an address the offer could have named.
            return Err(refused(SampleRefused::OtherSource));
        };
        let presented = Presented {
            connection: self.connection.id,
            thread: asked_in,
            parent: self.parent,
            source: &source,
            ticked: &approval.columns,
            recipient,
        };
        let (path, columns) =
            samples::consume(grant, &presented, tiers.current().await).map_err(refused)?;
        // The audit's key: what left is recorded as read by this preview.
        let preview = oxyn_core::CommandId::new();
        let rows = self.read_sample(preview, session, &path, &columns).await?;
        let Some(tier @ PrivacyTier::Sampled) = tiers.current().await else {
            return Err(refused(SampleRefused::TierLowered));
        };
        let mut egress = EgressRecord::new(
            self.connection.id,
            path.to_string(),
            columns.clone(),
            u32::try_from(rows.len()).unwrap_or(u32::MAX),
            recipient.provider.clone(),
            egress_reach(recipient.reach),
        )
        .read_by(preview);
        // An agent chooses its model itself: none is recorded rather than one
        // invented.
        if recipient.kind == RecipientKind::Provider {
            egress = egress.with_model(&recipient.model);
        }
        // The audit names the conversation when there is one: a thread whose
        // header could not be written still sends, and still records.
        if let (Some(conversation), Some(node)) = (
            self.thread.conversation(),
            self.thread.stored_node(self.node),
        ) {
            egress = egress.in_conversation(conversation, Some(node));
        }
        Ok(ApprovedSample {
            rows: oxyn_ai::context::RowSample::new(path, columns, rows),
            reach: recipient.reach,
            tier,
            egress,
        })
    }

    /// Appends to `ai_egress` before a sample leaves.
    async fn record_egress(&self, record: EgressRecord) -> Result<(), Failure> {
        if !append_egress(&self.inner.executor, record).await {
            return Err(Failure::new(
                "Oxyn could not record this sample in its audit trail, so nothing was sent.",
                FailureCategory::Refused,
            ));
        }
        Ok(())
    }

    async fn read_sample(
        &self,
        preview: oxyn_core::CommandId,
        session: SessionId,
        path: &oxyn_catalog::CatalogPath,
        columns: &[String],
    ) -> Result<Vec<Vec<oxyn_core::ScalarValue>>, Failure> {
        let Some(relation) = path.relation() else {
            return Err(Failure::new(
                SampleRefused::OtherSource.to_string(),
                FailureCategory::Refused,
            ));
        };
        let executor = &self.inner.executor;
        let command = Command::PreviewRelation {
            connection: self.connection.id,
            session,
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: relation.to_owned(),
            limit: samples::MAX_SAMPLE_ROWS,
            shape: oxyn_core::PreviewShape::default(),
        };
        match executor
            .dispatch_as(preview, Actor::Human, command, self.cancel)
            .await
        {
            Ok(oxyn_exec::Outcome::Executed { result, buffer, .. }) => {
                // Forgotten once copied: a read held for a grid nobody opens
                // would keep the rows in memory until sixteen newer results
                // push it out.
                let _forget = ForgetResult {
                    executor: Arc::clone(executor),
                    result,
                };
                let columns = columns.to_vec();
                let cancel = self.cancel.clone();
                tokio::task::spawn_blocking(move || {
                    samples::copy_rows(&buffer, &columns, samples::MAX_SAMPLE_ROWS, &cancel)
                })
                .await
                .map_err(|_| setup("copying the approved sample failed; nothing was sent"))?
                .ok_or_else(|| {
                    Failure::new(
                        "A column approved for the sample is no longer in this relation; \
                             nothing was sent. Approve the sample again.",
                        FailureCategory::Refused,
                    )
                })
            }
            Ok(oxyn_exec::Outcome::NeedsApproval { command, .. }) => {
                // Not left waiting: this read belongs to the sample the user
                // already decided on, and a second prompt for it would be one
                // clicked by reflex.
                executor.reject(command);
                Err(Failure::new(
                    "Reading the sample needs an approval of its own on this connection; \
                     nothing was read or sent.",
                    FailureCategory::Refused,
                ))
            }
            Ok(oxyn_exec::Outcome::Denied { reason, .. }) => Err(Failure::new(
                format!("Reading the sample was refused: {reason}. Nothing was sent."),
                FailureCategory::Refused,
            )),
            Ok(_) => Err(setup(
                "reading the sample returned no rows; nothing was sent",
            )),
            // Not the driver's words: a server error can quote the cell it
            // refused, and this message is shown and kept (I-03).
            Err(_) => Err(Failure::new(SAMPLE_READ_FAILED, FailureCategory::Refused)),
        }
    }

    /// The provider conversation this question continues — the remembered one
    /// — or a fresh one built by the `ContextBuilder`.
    ///
    /// A question with an approved sample always starts fresh, and its
    /// exchange is marked as leaving no memory **before** anything else: the
    /// sample enters a prompt through the `ContextBuilder` only, a remembered
    /// session would skip it, and a session remembered after it would carry it
    /// into the next question. What the conversation keeps of it is its size.
    ///
    /// The question is added here, with the objects the user mentioned: in a
    /// fresh session they lead its context; in a remembered one — whose system
    /// message is not rewritten — they precede the question, rendered by the
    /// same `ContextBuilder` under the same tier. The context returned is the
    /// one that joins the prompt this time, if any.
    fn prepare_dialogue(
        &self,
        agent: &oxyn_ai::AgentSpec,
        session: SessionId,
        question: &str,
        sample: Option<oxyn_ai::context::RowSample>,
        tier: PrivacyTier,
    ) -> Result<(AgentSession, Option<oxyn_ai::AgentContext>), Failure> {
        let sampled = sample.is_some();
        if sampled {
            self.thread.withhold_memory(self.node);
        }
        let connection = self.connection;
        let dialect = oxyn_query::dialect_for(&connection.driver);
        // A sampled question starts from a fresh context: the sample can only
        // enter through the `ContextBuilder`, and a remembered session skips it.
        let remembered = if sampled {
            None
        } else {
            self.thread.memory_from(self.parent)
        };
        let mentions = &self.mentions.mentions;
        Ok(match remembered {
            Some(memory) if memory.tier == tier => {
                let mut dialogue = memory.session;
                if mentions.is_empty() {
                    dialogue.ask(question);
                    (dialogue, None)
                } else {
                    let catalog = self.inner.executor.catalog(connection.id).ok_or_else(|| {
                        setup(
                            "This connection has no catalog to work from yet: open its explorer first.",
                        )
                    })?;
                    let context = {
                        let cache = catalog.read();
                        ContextBuilder::new(&cache, tier)
                            .with_language(QueryLanguage::Sql(dialect))
                            .with_mentions(mentions.clone())
                            .mentioned_only()
                            .build()
                    };
                    dialogue
                        .ask_about(&context, question)
                        .map_err(|error| setup(error.to_string()))?;
                    (dialogue, Some(context))
                }
            }
            other => {
                if other.is_some() {
                    self.emit(AiEvent::MemoryReset {
                        reason: MemoryReset::TierChanged,
                    });
                } else if self.parent.is_some()
                    && (sampled || self.thread.memory_withheld(self.parent))
                {
                    self.emit(AiEvent::MemoryReset {
                        reason: MemoryReset::SampleNotKept,
                    });
                } else if self.thread.memory_restored(self.parent) {
                    // Read back from the workspace: what the model was told is
                    // not written, so this question starts over.
                    self.emit(AiEvent::MemoryReset {
                        reason: MemoryReset::Restarted,
                    });
                } else if self.parent.is_some() {
                    // An ancestor answered without leaving a provider session:
                    // an external agent did.
                    self.emit(AiEvent::MemoryReset {
                        reason: MemoryReset::DestinationChanged,
                    });
                }
                // From the local catalog only: an agent that fetched what it
                // needs would bypass both the gate and the bus.
                let catalog = self.inner.executor.catalog(connection.id).ok_or_else(|| {
                    setup(
                        "This connection has no catalog to work from yet: open its explorer first.",
                    )
                })?;
                if let Some(sample) = &sample {
                    self.emit(AiEvent::SampleApproved {
                        rows: u32::try_from(sample.rows.len()).unwrap_or(u32::MAX),
                        columns: u32::try_from(sample.columns.len()).unwrap_or(u32::MAX),
                    });
                }
                let context = {
                    let cache = catalog.read();
                    ContextBuilder::new(&cache, tier)
                        .with_language(QueryLanguage::Sql(dialect))
                        .focused_on(question.to_owned())
                        .with_mentions(mentions.clone())
                        .with_samples(sample.into_iter().collect())
                        .build()
                };
                let scope = ToolScope::new(connection.id, session, QueryLanguage::Sql(dialect));
                let mut dialogue = AgentSession::new(agent, &context, scope);
                dialogue.ask(question);
                (dialogue, Some(context))
            }
        })
    }

    /// Assembles a provider conversation — or takes the remembered one — then
    /// runs it.
    ///
    /// `Err` only for what prevents starting or comes from the provider. A
    /// policy refusal, a failed query, a cancellation are steps of the
    /// conversation and go through the observer.
    async fn converse(
        &self,
        session: SessionId,
        provider: AiProviderConfig,
        model: String,
        effort: Option<oxyn_llm::ReasoningEffort>,
        sample: Option<ApprovedSample>,
        question: String,
    ) -> Result<(), Failure> {
        let sampled = sample.is_some();
        // A sampled question is governed by the tier read back after its
        // sample: the one that let the rows through.
        let tier = sample
            .as_ref()
            .map_or(self.connection.privacy_tier, |sample| sample.tier);
        let approved_reach = sample.as_ref().map(|sample| sample.reach);
        let egress = sample.as_ref().map(|sample| sample.egress.clone());
        let agent = sql_agent();
        // First, before anything can fail: a sampled exchange must be marked
        // as leaving no memory whatever happens next.
        let (mut dialogue, context) = self
            .prepare(
                &agent,
                session,
                &question,
                sample.map(|sample| sample.rows),
                tier,
            )
            .await?;

        // Classified first, and refused before the keyring is read or a
        // transport exists (ADR-0023). Neither the reach nor the key is cached:
        // a key revoked must stop working, a name that resolved locally
        // yesterday may not today.
        let reach = classify(&provider.base_url).await?;
        check_endpoint(tier, reach)
            .map_err(|message| Failure::new(message, FailureCategory::Refused))?;
        // Classified once more since the sample was approved: rows approved
        // for this machine do not leave it on a name that changed its mind.
        if approved_reach.is_some_and(|approved| samples::wider(reach, approved)) {
            return Err(Failure::new(
                SampleRefused::OtherRecipient.to_string(),
                FailureCategory::Refused,
            ));
        }

        let credentials = Arc::clone(&self.inner.credentials);
        let declaration = provider.clone();
        let transport = tokio::task::spawn_blocking(move || {
            let key = credentials.provider_key(&declaration)?;
            oxyn_llm::build_provider(declaration.kind, &declaration.base_url, key)
        })
        .await
        .map_err(|error| setup(format!("preparing the provider: {error}")))?
        .map_err(|error| setup(error.to_string()))?;

        // Read before the runtime takes the transport: the model's declared
        // efforts, asked only when an effort was chosen — a question without
        // one costs no extra round trip.
        let declared = match effort {
            Some(_) => Some(
                tokio::time::timeout(super::MODELS_TIMEOUT, transport.models())
                    .await
                    .map_err(|_| {
                        Failure::new(
                            "The endpoint did not say which reasoning efforts this model accepts \
                             in time; nothing was sent",
                            FailureCategory::Provider,
                        )
                    })?
                    .map_err(|error| {
                        Failure::new(
                            format!("reading the model's reasoning efforts: {error}"),
                            FailureCategory::Provider,
                        )
                    })?,
            ),
            None => None,
        };
        let runtime = AgentRuntime::new(
            agent.clone(),
            transport,
            reach,
            ToolRegistry::builtin(),
            model.clone(),
        )
        .map_err(|error| setup(error.to_string()))?;
        let runtime = with_effort(runtime, effort, &model, declared.as_deref())?;

        // Written last before the send, with the reach that governs it: a
        // refusal above leaves no entry, and no row leaves without one.
        if let Some(mut record) = egress {
            record.reach = egress_reach(reach);
            self.record_egress(record).await?;
        }

        // Named as the approval screen names them, and recorded as the grant
        // path records them: the same provider, model and reach.
        let sampling = Sampling::new(
            Arc::clone(&self.inner.ai.asks),
            Recipient::provider(provider.id.clone(), model.clone(), reach),
            provider.label.clone(),
            format!("{} · {model}", provider.label),
        );
        self.emit(AiEvent::Started {
            destination: Destination {
                kind: "provider",
                label: provider.label.clone(),
                model: Some(model.clone()),
                reach: reach.into(),
                agent_version: None,
            },
            tier,
            context: context
                .as_ref()
                .map(|context| ContextSummary::of(context, self.mentions.ignored)),
            provenance: Some(Provenance::new(
                agent.id,
                dialogue.id(),
                provider.kind,
                model,
            )),
        });

        let sink = AgentSink {
            sink: ExecutorSink::for_agent(
                Arc::clone(&self.inner.executor),
                agent.id,
                dialogue.id(),
            ),
            executor: Arc::clone(&self.inner.executor),
            thread: Arc::clone(self.thread),
            node: self.node,
            // The internal loop awaits every call before its question ends.
            question: QuestionOpen::new(),
            sampling: Some(sampling),
        };
        let outcome = runtime
            .run(&mut dialogue, &sink, &self.observer(), self.cancel)
            .await
            .map_err(|error| Failure {
                class: class_of(&error),
                ..Failure::new(error.to_string(), category_of(&error))
            })?;
        // Only an exchange that ends on the model's own message is a base to
        // follow: a refusal leaves the question unanswered, a cancel or a turn
        // limit leaves tool calls without their answer. An exchange where the
        // model received a sample it asked for leaves none either: its session
        // holds the rows (ADR-0034).
        if !sampled
            && !self.thread.is_withheld(self.node)
            && matches!(
                outcome,
                AgentOutcome::Answered { .. } | AgentOutcome::Paused { .. }
            )
        {
            self.thread.remember(
                self.node,
                Memory {
                    session: dialogue,
                    tier,
                },
            );
        }
        Ok(())
    }

    /// A question to an external agent: no key, no endpoint, the tier refused
    /// before launch ([ADR-0026](../../../../../docs/adr/0026-agents-externes-acp.md)).
    ///
    /// The agent's session is kept for the next question, as long as it follows
    /// this answer, under the same tier — **unless the exchange carried a
    /// sample**, pinned by the user or asked for by the agent: the process
    /// that saw the rows would still know them at the next question, so it is
    /// released, and the next question starts another and says so
    /// ([ADR-0034](../../../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
    async fn ask_agent(
        &self,
        session: SessionId,
        agent: &ExternalAgentConfig,
        question: &str,
        sample: Option<ApprovedSample>,
    ) -> Result<(), Failure> {
        let result = self.agent_exchange(session, agent, question, sample).await;
        // Whatever the way out — answered, failed, stopped —, and whichever
        // way the rows came in.
        if self.thread.is_withheld(self.node) {
            self.thread.link_agent(None);
        }
        result
    }

    async fn agent_exchange(
        &self,
        session: SessionId,
        agent: &ExternalAgentConfig,
        question: &str,
        sample: Option<ApprovedSample>,
    ) -> Result<(), Failure> {
        let sampled = sample.is_some();
        // A sampled question is governed by the tier read back after its
        // sample: the one that let the rows through.
        let tier = sample
            .as_ref()
            .map_or(self.connection.privacy_tier, |sample| sample.tier);
        // First, before anything can fail: a sampled exchange leaves no
        // memory whatever happens next.
        if sampled {
            self.thread.withhold_memory(self.node);
        }
        let egress = sample.as_ref().map(|sample| sample.egress.clone());
        let counts = sample.as_ref().map(|sample| {
            (
                u32::try_from(sample.rows.rows.len()).unwrap_or(u32::MAX),
                u32::try_from(sample.rows.columns.len()).unwrap_or(u32::MAX),
            )
        });
        // A sampled question never continues a session: the process would
        // carry the exchanges before it into the rows' prompt, and the rows
        // into the questions after.
        let linked = if sampled {
            None
        } else {
            self.thread.agent_for(agent, tier, self.parent)
        };
        // The prompt is born through the only gate that builds one, under the
        // tier (ADR-0027), before anything is launched. An agent session that
        // starts here is told the structure of the database — the objects the
        // user mentioned first, and the sample approved for this question —,
        // rendered by the `ContextBuilder` as the internal assistant's is. One
        // that follows an answer already has the structure, as a remembered
        // provider session does: only the mentioned objects are rendered, by the
        // same gate, and precede the question.
        let language = QueryLanguage::Sql(oxyn_query::dialect_for(&self.connection.driver));
        let mentions = self.mentions.mentions.clone();
        // Before the prompt is rendered, what it needs is read — by Oxyn,
        // through the bus: the whole selection for a session that starts, the
        // mentions alone for one that follows.
        self.complete_catalog(self.want(question, linked.is_some()))
            .await;
        // From the local catalog only: an agent that fetched what it needs
        // would bypass both the gate and the bus. No catalog yet is said to the
        // agent as « 0 of 0 known relations », and ignores every mention.
        let catalog = self.inner.executor.catalog(self.connection.id);
        let empty = oxyn_catalog::CatalogCache::new();
        // The read lock is released before anything awaits.
        let prompt = {
            let guard = catalog.as_ref().map(|catalog| catalog.read());
            let cache = guard.as_deref().unwrap_or(&empty);
            match &linked {
                Some(_) => AgentPrompt::following(tier, question, cache, language, mentions),
                None => {
                    let samples: Vec<_> = sample.map(|sample| sample.rows).into_iter().collect();
                    AgentPrompt::with_schema(tier, question, cache, language, samples, mentions)
                }
            }
        }
        .map_err(|error| Failure::new(error.to_string(), FailureCategory::Refused))?;

        let LinkedAgent {
            session,
            tools,
            actor: (identity, conversation),
        } = match linked {
            Some(linked) => linked,
            None => {
                if let Some(parent) = self.parent {
                    self.emit(AiEvent::MemoryReset {
                        reason: if sampled || self.thread.is_withheld(parent) {
                            MemoryReset::SampleNotKept
                        } else if self.thread.has_agent_link() {
                            MemoryReset::AgentRestarted
                        } else {
                            MemoryReset::DestinationChanged
                        },
                    });
                }
                self.thread.link_agent(None);
                // The agent the panel started for this connection, when it is
                // this one — else a launch of its own. Under the launch lock,
                // so a start still launching is waited for rather than doubled.
                let link = {
                    let _launching = self.inner.ai.launching.lock().await;
                    match self
                        .inner
                        .ai
                        .take_waiting(self.connection.id, agent, tier, session)
                    {
                        Some(link) => link,
                        None => {
                            launch_agent(self.inner, self.connection, session, agent, tier).await?
                        }
                    }
                };
                let linked = LinkedAgent {
                    session: Arc::clone(&link.session),
                    tools: link.tools.clone(),
                    actor: link.actor,
                };
                self.thread.follow_agent_settings(&linked.session);
                self.thread.link_agent(Some(link));
                linked
            }
        };

        let Some(ready) = self.start(agent, &session).await? else {
            return Ok(());
        };
        // Written last before the send: a refusal or a failed start above
        // leaves no entry, and no row leaves without one. A sample the gate
        // dropped — over budget — did not leave, and is not recorded.
        if let Some(record) = egress
            && prompt
                .context()
                .is_some_and(|context| context.dropped_samples() == 0)
        {
            self.record_egress(record).await?;
            if let Some((rows, columns)) = counts {
                self.emit(AiEvent::SampleApproved { rows, columns });
            }
        }
        self.emit(AiEvent::Started {
            destination: Destination {
                kind: "agent",
                label: agent.label.clone(),
                model: None,
                reach: Reach::Unresolved.into(),
                agent_version: version_of(ready),
            },
            tier,
            // What left with the question, shown as the provider path shows it.
            context: prompt
                .context()
                .map(|context| ContextSummary::of(context, self.mentions.ignored)),
            provenance: None,
        });

        let observer: Arc<dyn AgentObserver> = Arc::new(AgentObserverFilter(self.observer()));
        // This question — its node, its stop — is what the agent's tool calls
        // answer until it ends. Dropped with the question, taking the tools
        // away: between two questions nothing runs.
        let question = QuestionOpen::new();
        let closing = CloseQuestion(question.clone());
        let _question = tools.open(
            Arc::new(AgentSink {
                sink: ExecutorSink::for_agent(
                    Arc::clone(&self.inner.executor),
                    identity,
                    conversation,
                ),
                executor: Arc::clone(&self.inner.executor),
                thread: Arc::clone(self.thread),
                node: self.node,
                question,
                sampling: Some(Sampling::new(
                    Arc::clone(&self.inner.ai.asks),
                    Recipient::agent(agent),
                    agent.label.clone(),
                    agent.label.clone(),
                )),
            }),
            Arc::clone(&observer),
            self.cancel.clone(),
        );
        let end = match session.prompt(&prompt, observer, self.cancel).await {
            Ok(end) => end,
            Err(error) => {
                let failure = agent_failure_of(agent, error, &session).await;
                if failure.ends_agent {
                    self.thread.link_agent(None);
                }
                return Err(failure);
            }
        };
        self.thread.agent_answered(self.node);
        // Closed before « finished » is shown: no request may appear after it.
        drop(closing);
        self.emit(AiEvent::Finished {
            ending: match end {
                TurnEnd::Cancelled => Ending::Cancelled { turns: 1 },
                TurnEnd::Answered { truncated } => Ending::Answered {
                    turns: 1,
                    truncated,
                    // The agent's protocol folds several reasons into one; the
                    // panel says « cut short », never a limit it cannot name.
                    cut: truncated.then_some(Cut::Unknown),
                },
                TurnEnd::Refused => Ending::Refused { turns: 1 },
                TurnEnd::TurnLimit => Ending::AgentLimit,
                _ => Ending::Unknown,
            },
        });
        Ok(())
    }

    /// Starts the agent unless already started, while the user can still stop,
    /// and within [`AGENT_START_TIMEOUT`].
    ///
    /// `Ok(None)` when stopped first: the ending is already said. A start that
    /// times out or dies takes the agent out of the conversation: the next
    /// question launches a new one rather than waiting on this one.
    async fn start(
        &self,
        agent: &ExternalAgentConfig,
        session: &ExternalSession,
    ) -> Result<Option<AgentReady>, Failure> {
        tokio::select! {
            ready = start_bounded(agent, session) => ready.map(Some).inspect_err(|failure| {
                if failure.ends_agent {
                    self.thread.link_agent(None);
                }
            }),
            () = self.cancel.cancelled() => {
                self.emit(AiEvent::Finished { ending: Ending::Cancelled { turns: 0 } });
                Ok(None)
            }
        }
    }
}

/// Checks the agent's program exists before launching it, so a missing one
/// reads as such rather than as a protocol failure.
async fn locate(agent: &ExternalAgentConfig) -> Result<(), Failure> {
    let command = agent.command.clone();
    let path = agent
        .env
        .iter()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| std::ffi::OsString::from(value));
    // An agent Oxyn does not know asks for no particular Node: nvm's default
    // is the one its user would have in a shell.
    let node_major =
        oxyn_ai::external::presets::preset_of(agent).map_or(0, |preset| preset.node_major);
    let found = tokio::task::spawn_blocking(move || {
        // The child resolves its program with its own `PATH` when the
        // declaration sets one, the parent's otherwise.
        let path = path.or_else(|| std::env::var_os("PATH"));
        let search = SearchPath::of(
            path.map(|value| std::env::split_paths(&value).collect())
                .unwrap_or_default(),
        );
        if search.find(&command).is_some() {
            return Ok(());
        }
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from);
        let elsewhere = SearchPath::usual(None, home.as_deref(), node_major)
            .find(&command)
            .and_then(|path| path.into_os_string().into_string().ok());
        Err(elsewhere)
    })
    .await
    .map_err(|error| setup(format!("looking for the agent: {error}")))?;
    found.map_err(|elsewhere| Failure {
        found_elsewhere: elsewhere,
        ..agent_failure(
            agent,
            ExternalError::NotFound {
                command: agent.command.clone(),
            },
        )
    })
}

fn agent_failure(agent: &ExternalAgentConfig, error: ExternalError) -> Failure {
    let category = match &error {
        ExternalError::RefusedByTier => FailureCategory::Refused,
        ExternalError::Invalid(_) => FailureCategory::Setup,
        ExternalError::NotFound { .. } => FailureCategory::AgentNotFound,
        ExternalError::AuthRequired { .. } => FailureCategory::AgentSignIn,
        ExternalError::Incompatible { .. } => FailureCategory::AgentIncompatible,
        ExternalError::Exited => FailureCategory::AgentExited,
        _ => FailureCategory::Agent,
    };
    let sign_in = match &error {
        ExternalError::AuthRequired { methods } => {
            let launch: Vec<String> = std::iter::once(agent.command.clone())
                .chain(agent.args.iter().cloned())
                .collect();
            Some(Box::new(SignInHelp {
                agent: agent.label.clone(),
                methods: methods
                    .iter()
                    .map(|method| SignInMethod::of(method, &launch))
                    .collect(),
                // The adapter's own CLI when Oxyn runs it as proposed: it signs
                // in whether or not `claude` is installed, and looking for
                // `claude` here would touch the disk on an async worker.
                terminal_command: match oxyn_ai::external::presets::pinned_preset_of(agent) {
                    Some(preset) => {
                        Some(preset_sign_in(preset, &agent.command, &agent.args, false))
                    }
                    None => preset_of(agent).map(|preset| preset.sign_in.to_owned()),
                },
            }))
        }
        _ => None,
    };
    Failure {
        message: error.to_string(),
        category,
        // An agent's protocol carries no error class.
        class: None,
        sign_in,
        found_elsewhere: None,
        exit: None,
        ends_agent: matches!(error, ExternalError::Exited | ExternalError::Unconfined),
    }
}

/// [`agent_failure`], with what the process said when the failure is its
/// death. The report is written by the session's driver as the process goes,
/// which can be a moment after the protocol noticed: it is waited for, briefly.
async fn agent_failure_of(
    agent: &ExternalAgentConfig,
    error: ExternalError,
    session: &ExternalSession,
) -> Failure {
    let exited = error == ExternalError::Exited;
    let mut failure = agent_failure(agent, error);
    if exited {
        failure.exit = exit_of(session).await;
    }
    failure
}

/// How long a death's report is looked for, and how often.
const EXIT_REPORT_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
const EXIT_REPORT_POLL: std::time::Duration = std::time::Duration::from_millis(50);

async fn exit_of(session: &ExternalSession) -> Option<Box<AgentExit>> {
    let deadline = tokio::time::Instant::now() + EXIT_REPORT_WAIT;
    loop {
        if let Some(report) = session.exit_report() {
            return Some(Box::new(AgentExit::of(&report)));
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(EXIT_REPORT_POLL).await;
    }
}

/// How long an agent may take to answer `initialize` and open its session.
///
/// Product bound. The first launch of an agent distributed through `npx`
/// downloads its adapter before it answers, which takes tens of seconds on an
/// ordinary connection; past two minutes the user is better told than left
/// looking at « Starting ». Not retried: the next start is the user's click.
pub(super) const AGENT_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// How long finding an agent's program, reading its configuration and
/// spawning it may each take — before `initialize`, which
/// [`AGENT_START_TIMEOUT`] bounds.
///
/// Product bound. Local file system work takes milliseconds; one that takes
/// this long is a network share or a disk that stopped answering.
const AGENT_LAUNCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

fn timed_out(agent: &ExternalAgentConfig) -> Failure {
    Failure {
        ends_agent: true,
        ..Failure::new(
            format!(
                "{} did not finish starting within {} seconds, so Oxyn stopped it. Nothing was \
                 sent.",
                agent.label,
                AGENT_START_TIMEOUT.as_secs()
            ),
            FailureCategory::AgentTimedOut,
        )
    }
}

/// Starts `session` — once: a second call waits for the first one's answer —
/// within [`AGENT_START_TIMEOUT`].
async fn start_bounded(
    agent: &ExternalAgentConfig,
    session: &ExternalSession,
) -> Result<AgentReady, Failure> {
    match tokio::time::timeout(AGENT_START_TIMEOUT, session.start()).await {
        Ok(Ok(ready)) => Ok(ready),
        Ok(Err(error)) => Err(agent_failure_of(agent, error, session).await),
        Err(_elapsed) => Err(timed_out(agent)),
    }
}

/// « Claude Code 0.78.0 », or whichever half the agent said.
fn version_of(ready: AgentReady) -> Option<String> {
    match (ready.name, ready.version) {
        (Some(name), Some(version)) => Some(format!("{name} {version}")),
        (name, version) => name.or(version),
    }
}

/// Launches `agent` for questions asked on `connection` in `session`, with
/// Oxyn's tools on the loopback (ADR-0030). Nothing is asked of it yet.
///
/// The one launch of an external agent, whether a question needs it or the
/// panel starts it ahead of one: the tier is refused before anything exists
/// (`launch_with_tools`), and the tools, the actor and the confinement are
/// the same either way.
async fn launch_agent(
    inner: &Inner,
    connection: &ConnectionConfig,
    session: SessionId,
    agent: &ExternalAgentConfig,
    tier: PrivacyTier,
) -> Result<AgentLink, Failure> {
    tokio::time::timeout(AGENT_LAUNCH_TIMEOUT, locate(agent))
        .await
        .map_err(|_| {
            setup(format!(
                "Looking for {} took more than {} seconds. Nothing was started.",
                agent.label,
                AGENT_LAUNCH_TIMEOUT.as_secs()
            ))
        })??;
    // The actor is minted per external conversation and shared by every
    // question's sink and the service: the executor refuses a command whose
    // actor is not the one bound to the conversation.
    let (identity, conversation) = (AgentId::new(), AgentSessionId::new());
    let spec = sql_agent();
    // The ceiling the internal assistant has, per question.
    let executor = Arc::clone(&inner.executor);
    let actor = Actor::agent(identity, conversation);
    // Asked of the executor, which holds the requests: a request this agent
    // left undecided in an earlier question counts, and so does a « read »
    // the executor reclassified.
    let waiting = move || {
        executor
            .approvals()
            .pending()
            .iter()
            .any(|request| request.actor == actor && !request.is_expired())
    };
    let tools = ToolTurns::new(spec.max_turns, Arc::new(waiting));
    let bridge = ToolBridge {
        service: Arc::new(ToolService::new(
            ToolRegistry::builtin(),
            spec.allowed_tools,
            ToolScope::new(
                connection.id,
                session,
                QueryLanguage::Sql(oxyn_query::dialect_for(&connection.driver)),
            ),
            Arc::new(StoredTier {
                executor: Arc::clone(&inner.executor),
                connection: connection.id,
            }),
            Actor::agent(identity, conversation),
        )),
        turns: tools.clone(),
    };
    // Bounded: callers hold the lock every other agent launch waits on, and a
    // file system that stops answering must not hold it for good.
    let (session, driver) = tokio::time::timeout(
        AGENT_LAUNCH_TIMEOUT,
        ExternalSession::launch_with_tools(agent, tier, bridge),
    )
    .await
    .map_err(|_| {
        setup(format!(
            "{} could not be launched within {} seconds: its configuration or its program \
             did not answer. Nothing was sent.",
            agent.label,
            AGENT_LAUNCH_TIMEOUT.as_secs()
        ))
    })?
    .map_err(|error| agent_failure(agent, error))?;
    tauri::async_runtime::spawn(driver);
    Ok(AgentLink {
        agent: agent.clone(),
        tier,
        leaf: None,
        session: Arc::new(session),
        tools,
        actor: (identity, conversation),
        _requests: WithdrawOnRelease::new(
            Arc::clone(&inner.executor),
            Actor::agent(identity, conversation),
        ),
    })
}

fn setup(message: impl Into<String>) -> Failure {
    Failure::new(message, FailureCategory::Setup)
}

/// The class a runtime error carries — `Ambiguous` for a stream cut before
/// the turn ended — read from the error, never from its words.
const fn class_of(error: &AiError) -> Option<ErrorClass> {
    error.class()
}

/// The runtime with the chosen effort, checked against what the provider
/// declares for `model`. A model missing from the listing declares nothing.
fn with_effort(
    runtime: AgentRuntime,
    effort: Option<oxyn_llm::ReasoningEffort>,
    model: &str,
    declared: Option<&[oxyn_llm::ModelInfo]>,
) -> Result<AgentRuntime, Failure> {
    let Some(effort) = effort else {
        return Ok(runtime);
    };
    let unlisted = oxyn_llm::ModelInfo::new(model);
    let info = declared
        .unwrap_or_default()
        .iter()
        .find(|info| info.id == model)
        .unwrap_or(&unlisted);
    runtime
        .with_reasoning_effort(effort, info)
        .map_err(|error| Failure::new(error.to_string(), category_of(&error)))
}

fn category_of(error: &AiError) -> FailureCategory {
    match error {
        // Asking again with the same effort changes nothing.
        AiError::RemoteProviderRefused { .. } | AiError::ReasoningEffortNotOffered { .. } => {
            FailureCategory::Refused
        }
        AiError::InvalidSpec(_) => FailureCategory::Setup,
        _ => FailureCategory::Provider,
    }
}

mod sampling;
mod startup;

#[cfg(test)]
mod tests;
