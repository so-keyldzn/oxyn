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

use super::persistence;
use super::samples::{self, Grant, Offer, Presented, Recipient, SampleRefused};
use super::threads::{AgentLink, LinkedAgent, Memory, Scope, Thread, WithdrawOnRelease};
use super::{Backend, check_endpoint};
use crate::backend::Inner;
use crate::ipc::ai::{
    AgentSettingAnswer, AgentSettingChange, AgentSettingsView, AiEvent, AiUpdate, AskRequest,
    AskStarted, ContextSummary, Cut, Destination, DestinationChoice, Ending, FailureCategory,
    MemoryReset, Money, PlanEntry, SignInHelp, SignInMethod, ThreadSummary, ThreadView, ToolStatus,
    error_class, preset_of,
};
use crate::ipc::ai::{SampleApproval, SampleRequest};
use crate::ipc::{CatalogAddress, IpcError, RelationField};

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
    let (status, detail, class) = match outcome {
        DispatchOutcome::Completed { summary } => (ToolStatus::Completed, summary.clone(), None),
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
        let report = self.sink.dispatch(actor, command, cancel).await;
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
            DispatchReport::Completed {
                stats: Some(stats), ..
            } => self.thread.record_rows(call, stats.rows),
            DispatchReport::Failed { .. } if cancel.is_cancelled() => {
                self.thread.mark_cancelled(call);
            }
            _ => {}
        }
        drop(open);
        translate(report)
    }
}

/// A report in the runtime's terms: a correspondence of variants, no judgment.
fn translate(report: DispatchReport) -> DispatchOutcome {
    match report {
        DispatchReport::Completed { summary, .. } => DispatchOutcome::Completed { summary },
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
    sign_in: Option<SignInHelp>,
    found_elsewhere: Option<String>,
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
            sign_in: self.sign_in,
            found_elsewhere: self.found_elsewhere,
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

        // Read now, not when the panel opened: a tier changed since then must
        // govern this question (I-04).
        let config = self.config(connection)?;
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
        thread.emit(
            node,
            AiEvent::Question {
                text: question.clone(),
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
            };
            let result = async {
                // Checked before anything else, whatever the destination: a
                // refusal reads nothing.
                let sample = match (&approval, grant) {
                    (Some(approval), Some(grant)) => {
                        let recipient = match &resolved {
                            Resolved::Provider { config, model, .. } => Some(Recipient {
                                provider: config.id.clone(),
                                model: model.clone(),
                                // Classified again: an address edited since the
                                // offer must not inherit its approval.
                                reach: classify(&config.base_url).await?,
                            }),
                            Resolved::Agent(_) => None,
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
                                recipient.as_ref(),
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
                        run.ask_agent(session, &agent, &question).await
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
        );
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
        // A conversation open in this window is closed first; one that is only
        // on disk is deleted all the same.
        let _live = self.inner.ai.delete(connection, thread);
        let executor = Arc::clone(&self.inner.executor);
        // `ai_egress` is untouched: what left this machine outlives the
        // conversation that sent it (SECURITY).
        let _written =
            tokio::task::spawn_blocking(move || executor.store().conversations().delete(id)).await;
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
        thread: &str,
        method: &str,
    ) -> Result<(), IpcError> {
        let session = self
            .inner
            .ai
            .find(connection, thread)?
            .live_agent()
            .ok_or_else(|| {
                IpcError::invalid("The agent is no longer running. Ask your question again.")
            })?;
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
        thread: &str,
        change: AgentSettingChange,
    ) -> Result<AgentSettingAnswer, IpcError> {
        let session = self
            .inner
            .ai
            .find(connection, thread)?
            .live_agent()
            .ok_or_else(|| {
                IpcError::invalid("The agent is no longer running. Ask your question again.")
            })?;
        match session.change_setting(change.into_change()).await {
            Ok(settings) => Ok(AgentSettingAnswer::Sent {
                settings: AgentSettingsView::of(&settings),
            }),
            Err(ExternalError::Setting(refusal)) => Ok(AgentSettingAnswer::refused(&refusal)),
            Err(error) => Err(IpcError::invalid(error.to_string())),
        }
    }

    /// Offers a row sample of `address` for the next question of a
    /// conversation — never a value, only what would be read and where it
    /// would go.
    ///
    /// Offered under `Sampled` only, and to a built-in provider only: under
    /// any other tier the offer does not exist, and an external agent receives
    /// the question alone. The grant it issues is checked again, tier first,
    /// when the question presents it.
    ///
    /// # Errors
    /// Another tier, an agent, a relation absent from the catalog.
    pub async fn ai_request_sample(
        &self,
        connection: ConnectionId,
        thread: Option<String>,
        parent: Option<u32>,
        address: CatalogAddress,
        destination: DestinationChoice,
    ) -> Result<SampleRequest, IpcError> {
        let config = self.config(connection)?;
        if config.privacy_tier != PrivacyTier::Sampled {
            return Err(IpcError::invalid(
                "Row samples are offered only on a connection whose privacy tier is Sampled. \
                 Nothing was read.",
            ));
        }
        let Resolved::Provider {
            config: provider,
            model,
            ..
        } = self
            .resolve_destination(&destination, config.privacy_tier)
            .await?
        else {
            return Err(IpcError::invalid(SampleRefused::NotAProvider.to_string()));
        };
        let reach = classify(&provider.base_url)
            .await
            .map_err(|failure| IpcError::invalid(failure.message))?;
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
            recipient: Recipient {
                provider: provider.id.clone(),
                model,
                reach,
            },
        });
        Ok(SampleRequest {
            id,
            source: path.to_string(),
            address: CatalogAddress::of(&path),
            rows: samples::MAX_SAMPLE_ROWS,
            fields,
            destination: provider.label,
            reach: reach.into(),
        })
    }

    /// Withdraws a sample offer the user declined: its grant can no longer be
    /// presented.
    pub fn ai_withdraw_sample(&self, connection: ConnectionId, request: &str) {
        self.inner.ai.samples.withdraw(connection, request);
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
}

impl Run<'_> {
    fn emit(&self, event: AiEvent) {
        self.thread.emit(self.node, event);
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
        recipient: Option<&Recipient>,
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
        let Some(recipient) = recipient else {
            return Err(refused(SampleRefused::NotAProvider));
        };
        let mut egress = EgressRecord::new(
            self.connection.id,
            path.to_string(),
            columns.clone(),
            u32::try_from(rows.len()).unwrap_or(u32::MAX),
            recipient.provider.clone(),
            egress_reach(recipient.reach),
        )
        .read_by(preview)
        .with_model(&recipient.model);
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

    /// Appends to `ai_egress`, off the async workers: the store is SQLite.
    async fn record_egress(&self, record: EgressRecord) -> Result<(), Failure> {
        let executor = Arc::clone(&self.inner.executor);
        let written =
            tokio::task::spawn_blocking(move || executor.store().egress().append(&record)).await;
        // Neither the store's words nor the record: a name can be the data.
        if !matches!(written, Ok(Ok(_))) {
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
                tokio::task::spawn_blocking(move || samples::copy_rows(&buffer, &columns, &cancel))
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
        Ok(match remembered {
            Some(memory) if memory.tier == tier => (memory.session, None),
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
                        .with_dialect(dialect)
                        .focused_on(question.to_owned())
                        .with_samples(sample.into_iter().collect())
                        .build()
                };
                let scope = ToolScope::new(connection.id, session, QueryLanguage::Sql(dialect));
                (AgentSession::new(agent, &context, scope), Some(context))
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
        let (mut dialogue, context) = self.prepare_dialogue(
            &agent,
            session,
            &question,
            sample.map(|sample| sample.rows),
            tier,
        )?;

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

        dialogue.ask(question);

        self.emit(AiEvent::Started {
            destination: Destination {
                kind: "provider",
                label: provider.label.clone(),
                model: Some(model.clone()),
                reach: reach.into(),
                agent_version: None,
            },
            tier,
            context: context.as_ref().map(ContextSummary::of),
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
        // limit leaves tool calls without their answer.
        if !sampled
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
    /// this answer, under the same tier.
    async fn ask_agent(
        &self,
        session: SessionId,
        agent: &ExternalAgentConfig,
        question: &str,
    ) -> Result<(), Failure> {
        let tier = self.connection.privacy_tier;
        // The prompt is born through the only gate that builds one, under the
        // tier (ADR-0027).
        let prompt = AgentPrompt::from_user(tier, question)
            .map_err(|error| Failure::new(error.to_string(), FailureCategory::Refused))?;

        let LinkedAgent {
            session,
            tools,
            actor: (identity, conversation),
        } = match self.thread.agent_for(&agent.id, tier, self.parent) {
            Some(linked) => linked,
            None => {
                if self.parent.is_some() {
                    self.emit(AiEvent::MemoryReset {
                        reason: if self.thread.has_agent_link() {
                            MemoryReset::AgentRestarted
                        } else {
                            MemoryReset::DestinationChanged
                        },
                    });
                }
                self.thread.link_agent(None);
                locate(agent).await?;
                // Oxyn's tools, served to the agent on the loopback. Without
                // them the agent cannot read the database the user has open,
                // which is the whole point of running it here (ADR-0030).
                //
                // The actor is minted per external conversation and shared by
                // every question's sink and the service: the executor refuses a
                // command whose actor is not the one bound to the conversation.
                let (identity, conversation) = (AgentId::new(), AgentSessionId::new());
                let spec = sql_agent();
                // The ceiling the internal assistant has, per question.
                let executor = Arc::clone(&self.inner.executor);
                let actor = Actor::agent(identity, conversation);
                // Asked of the executor, which holds the requests: a request
                // this agent left undecided in an earlier question counts, and
                // so does a « read » the executor reclassified.
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
                            self.connection.id,
                            session,
                            QueryLanguage::Sql(oxyn_query::dialect_for(&self.connection.driver)),
                        ),
                        Arc::new(StoredTier {
                            executor: Arc::clone(&self.inner.executor),
                            connection: self.connection.id,
                        }),
                        Actor::agent(identity, conversation),
                    )),
                    turns: tools.clone(),
                };
                let (session, driver) = ExternalSession::launch_with_tools(agent, tier, bridge)
                    .await
                    .map_err(|error| agent_failure(agent, error))?;
                tauri::async_runtime::spawn(driver);
                let session = Arc::new(session);
                self.thread.follow_agent_settings(&session);
                self.thread.link_agent(Some(AgentLink {
                    agent: agent.id.clone(),
                    tier,
                    leaf: None,
                    session: Arc::clone(&session),
                    tools: tools.clone(),
                    actor: (identity, conversation),
                    _requests: WithdrawOnRelease::new(
                        Arc::clone(&self.inner.executor),
                        Actor::agent(identity, conversation),
                    ),
                }));
                LinkedAgent {
                    session,
                    tools,
                    actor: (identity, conversation),
                }
            }
        };

        let Some(ready) = self.start(agent, &session).await? else {
            return Ok(());
        };
        self.emit(AiEvent::Started {
            destination: Destination {
                kind: "agent",
                label: agent.label.clone(),
                model: None,
                reach: Reach::Unresolved.into(),
                agent_version: match (ready.name, ready.version) {
                    (Some(name), Some(version)) => Some(format!("{name} {version}")),
                    (name, version) => name.or(version),
                },
            },
            tier,
            context: None,
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
            }),
            Arc::clone(&observer),
            self.cancel.clone(),
        );
        let end = session
            .prompt(&prompt, observer, self.cancel)
            .await
            .map_err(|error| {
                if error == ExternalError::Exited {
                    self.thread.link_agent(None);
                }
                agent_failure(agent, error)
            })?;
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

    /// Starts the agent unless already started, while the user can still stop.
    ///
    /// `Ok(None)` when stopped first: the ending is already said.
    async fn start(
        &self,
        agent: &ExternalAgentConfig,
        session: &ExternalSession,
    ) -> Result<Option<AgentReady>, Failure> {
        tokio::select! {
            ready = session.start() => ready.map(Some).map_err(|error| agent_failure(agent, error)),
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
        let elsewhere = SearchPath::usual(None, home.as_deref())
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
            Some(SignInHelp {
                agent: agent.label.clone(),
                methods: methods
                    .iter()
                    .map(|method| SignInMethod::of(method, &launch))
                    .collect(),
                terminal_command: preset_of(agent).map(|preset| preset.sign_in),
            })
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
    }
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

#[cfg(test)]
mod tests;
