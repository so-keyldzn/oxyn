//! The executor: the **mandatory** passage point of every command.
//!
//! If a path reaches a driver without going through
//! [`Executor::dispatch`], the product's safety architecture is broken: the
//! second path will not be audited like the first, and it is that one the AI
//! will take ([I-01](../../../CLAUDE.md#i-01), ADR-0004).
//!
//! # The sequence, in this order and without shortcut
//!
//! 1. **Reclassify.** The intent carried by the command comes from the caller,
//!    and an agent is a caller. `oxyn-query` reads the text again; it is the
//!    result of that reading that is submitted, logged and executed — never
//!    what the caller declared (ARCHITECTURE §8, I-07).
//! 2. **Submit to the `PolicyGate`**, with the environment of the target
//!    connection and not the one the caller announces.
//! 3. **On `RequireApproval`, execute nothing.** The command is set aside
//!    ([`crate::approval`]) and the interface receives
//!    [`Event::ApprovalRequested`]. Execution only resumes through
//!    [`Executor::approve`], on the command's exact identifier.
//! 4. **Log before and after.** The policy decision is written *before* any
//!    execution, the result *after*. A denied command appears in the log like
//!    the others: a log that records only what worked says nothing about what
//!    an agent attempted.
//! 5. **Execute as a stream**, feeding an
//!    [`oxyn_data::ResultBuffer`] through an
//!    [`oxyn_data::BatchSink`] — hence with back-pressure and spill to disk
//!    (I-06).
//! 6. **Emit events** to the interface through [`EventBus`].
//!
//! # What blocks an execution, and what does not
//!
//! A log write failure **before** execution prevents execution: the audit
//! trail is the promise, not a side effect. A log write failure **after**
//! execution does not undo it — the command took place, and returning an error
//! would suggest otherwise; it is shouted at `error` level.
//!
//! # What this version does not do
//!
//! Page and value reads, preferences, the query library, connections (save,
//! delete, read), credential resolution, exports, and every write to the
//! audit journal and the query history all go through the application's
//! Tokio blocking pool as owned operations
//! ([ADR-0035](../../../docs/adr/0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md)).
//! The executor does not create a runtime of its own: without one, the
//! handlers that need it return a configuration error, while audit writes
//! run inline instead of failing outright — the audit trail must not go
//! silent at shutdown. Evicting retained results after an execution
//! ([`prune_results`](Executor::prune_results), called from
//! `execute_statement`), which can delete spill files, stays on the
//! dispatching worker on purpose.
//! [`load_connections`](Executor::load_connections) is synchronous too, but
//! never runs there: it is reserved for assembling state before the window
//! opens. Because journal rows are
//! now inserted from the blocking pool, their insertion order can differ from
//! the order of their `ts` field. An abandonment while a `RequireApproval`
//! decision is being written leaves no pending approval request behind: the
//! outcome is the same as one later rejected or expired.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxyn_catalog::SharedCatalog;
use oxyn_core::{
    Actor, CancelToken, Capabilities, CatalogRefreshScope, Command, CommandId, ConnectionConfig,
    ConnectionId, Decision, DocumentId, Environment, Event, ExecRequest, ExecStats, OxynError,
    PolicyGate, Preview, Result, ResultId, SessionId, StatementHandle, WorkspaceId,
};
use oxyn_data::{
    BatchProgress, BatchSink, BatchSource, BufferLimits, DEFAULT_MEMORY_BUDGET, ExportOptions,
    Pressure, ResultBuffer, SinkOutcome, export_to_path,
};
use oxyn_driver::{Cursor, DriverRegistry};
use oxyn_store::history::Reconciliation;
use oxyn_store::{Document, HistoryRecord, HistoryStatus, JournalRecord, Store};
use parking_lot::{Mutex, RwLock};

use crate::abandon::{AbandonGuard, AbandonedOutcomes, OutcomeGuard};
use crate::approval::{ApprovalRegistry, PendingCommand};
use crate::cancel::{CancelRegistry, CancelReport, RunningStatement};
use crate::events::EventBus;
use crate::sessions::{CredentialResolver, NoCredentials, SessionRegistry, SessionSlot};

/// What a command produced.
///
/// `#[non_exhaustive]`: new outcomes will appear with new commands, and a
/// caller must not break for that. It is the opposite of the choice made on
/// [`Command`], whose exhaustiveness **must** break the dispatch.
#[derive(Debug)]
#[non_exhaustive]
pub enum Outcome {
    /// A session was opened.
    Connected {
        /// The connection.
        connection: ConnectionId,
        /// The open session.
        session: SessionId,
        /// What the new session reports about its transaction, read here so
        /// that nothing above the bus calls the driver for it (ADR-0039 §4).
        transaction_state: oxyn_core::TransactionState,
    },

    /// The sessions of a connection were closed.
    /// One session was closed; sibling sessions and the catalog remain available.
    SessionClosed { session: SessionId },

    /// The server accepted a new resolution context for one session.
    ///
    /// Carries what the session reports afterwards, not what was requested: a
    /// server may normalise or reject part of it, and the interface must show
    /// the former.
    SessionContextSet {
        /// The session that moved. No sibling session is affected.
        session: SessionId,
        /// What the session reports now, `None` when it reports nothing.
        context: Option<oxyn_driver::SessionContext>,
    },
    Disconnected {
        /// The connection.
        connection: ConnectionId,
        /// How many sessions were closed.
        closed: usize,
    },

    /// Metadata was refreshed without including its contents in the outcome.
    CatalogRefreshed {
        /// Connection owning the updated cache.
        connection: ConnectionId,
        /// Requested scope, including Root for the legacy command.
        scope: CatalogRefreshScope,
    },

    /// The local catalog cache, handed to whoever was allowed to read it.
    ///
    /// The cache itself, not a rendering: rendering is `oxyn-ai`'s, under the
    /// connection's privacy tier.
    CatalogDescribed {
        /// Connection owning the cache.
        connection: ConnectionId,
        /// The cache, shared rather than copied.
        catalog: oxyn_catalog::CatalogHandle,
    },

    /// A statement was executed and its result is available.
    Executed {
        /// The result, as the interface will designate it afterwards.
        result: ResultId,
        /// The execution's handle, target of a cancellation.
        statement: StatementHandle,
        /// The buffer, shared with the interface **without copy**: it is what
        /// the grid reads while batches keep arriving.
        buffer: Arc<ResultBuffer>,
        /// What the execution cost.
        stats: ExecStats,
        /// Why the stream stopped. Only
        /// [`Exhausted`](SinkOutcome::Exhausted) describes a whole result.
        sink: SinkOutcome,
    },

    /// A bounded text page of a single value. Its Debug implementation hides text.
    ValueInspected {
        /// The page, without changing the result buffer.
        page: oxyn_data::value_page::ValuePage,
    },

    /// A locally stored page is now available in the result's bounded cache.
    ResultPageRead {
        /// Existing result; no new execution was created.
        result: ResultId,
        /// Zero-based batch index.
        batch: usize,
    },

    /// Preferences read or saved in the local workspace.
    WorkspacePreferences {
        snapshot: oxyn_core::PreferencesSnapshot,
    },

    /// A window's layout saved or removed in the local workspace.
    WindowLayoutWritten,

    /// A bounded library page, with no complete SQL bodies.
    QueryDocumentsListed {
        page: oxyn_store::documents::DocumentPage,
    },
    /// A local history page; retained identities may expire before opening.
    HistoryListed {
        page: oxyn_store::history::HistoryPage,
    },
    /// One selected execution record, never automatically replayed.
    HistoryEntryRead {
        entry: Box<oxyn_store::HistoryEntry>,
    },
    /// The user declared an unresolved write inspected; it no longer warns.
    HistoryEntryReconciled { entry: i64 },
    /// A bounded page of the connections history recorded, removed ones included.
    HistoryConnectionsListed {
        page: oxyn_store::history::HistoryConnectionPage,
    },
    /// A document was closed or deleted locally.
    DocumentClosed { document: DocumentId },
    /// The same retained buffer, without an execution event.
    RetainedResultOpened {
        result: ResultId,
        buffer: Arc<ResultBuffer>,
    },

    /// A cancellation was requested.
    Cancelled {
        /// What was actually done, client and server.
        report: CancelReport,
    },

    /// A result was written to a file.
    Exported {
        /// The exported result.
        result: ResultId,
        /// Rows written.
        rows: usize,
        /// Bytes written.
        bytes: u64,
    },

    /// A workspace document was read back.
    DocumentOpened {
        /// The document. Boxed: it is by far the largest variant.
        document: Box<Document>,
    },

    /// A workspace document was written.
    DocumentWritten {
        /// The document.
        document: DocumentId,
    },

    /// A configuration opened a session, which was closed at once.
    ///
    /// Nothing was saved and no session remains: the outcome says the server
    /// accepted these parameters, at this moment, and nothing more.
    ConnectionTested {
        /// The configuration tried, never registered with the executor.
        connection: ConnectionId,
    },

    /// A connection was saved or modified.
    ConnectionSaved {
        /// The connection.
        connection: ConnectionId,
    },

    /// The AI providers declared on this machine, in label order.
    ///
    /// An empty list is the default installation, not a failure: it is what
    /// decides whether the AI workspace exists at all. No reach is reported —
    /// classification is recomputed elsewhere, never stored
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    AiProvidersListed {
        /// Declarations, without any key. Boxed nowhere: the list is short and
        /// bounded by what the user typed.
        providers: Vec<oxyn_core::AiProviderConfig>,
    },

    /// An AI provider declaration was written locally.
    AiProviderSaved {
        /// The declaration that now exists.
        provider: oxyn_core::ProviderId,
    },

    /// An AI provider declaration was removed locally.
    ///
    /// Documents keep the provenance they were written with: it says where a
    /// text came from, not which provider is still declared.
    AiProviderRemoved {
        /// The declaration asked for.
        provider: oxyn_core::ProviderId,
        /// Did it exist?
        existed: bool,
    },

    /// The external agents declared on this machine, in label order.
    ///
    /// No reach is reported, and unlike a provider it is not because the value
    /// would go stale: an external agent's reach is **unknowable**
    /// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
    ExternalAgentsListed {
        /// Declarations. There is no key to withhold: this mode holds none.
        agents: Vec<oxyn_core::ExternalAgentConfig>,
    },

    /// An external agent declaration was written locally.
    ExternalAgentSaved {
        /// The declaration that now exists.
        agent: oxyn_core::ProviderId,
    },

    /// An external agent declaration was removed locally.
    ExternalAgentRemoved {
        /// The declaration asked for.
        agent: oxyn_core::ProviderId,
        /// Did it exist?
        existed: bool,
    },

    /// A connection was deleted from the workspace.
    ConnectionDeleted {
        /// The connection.
        connection: ConnectionId,
        /// Did it exist?
        existed: bool,
    },

    /// **Nothing was executed.** The command awaits an explicit approval.
    NeedsApproval {
        /// The identifier under which the approval is given
        /// ([`Executor::approve`]).
        command: CommandId,
        /// What the user must decide on.
        reason: String,
        /// What is needed to judge without reading elsewhere.
        preview: Option<Preview>,
    },

    /// **Nothing was executed**, and no approval will unblock the command.
    Denied {
        /// The denied command, as it appears in the log.
        command: CommandId,
        /// The reason, showable as is.
        reason: String,
    },
}

impl Outcome {
    /// Was the command denied?
    #[must_use]
    pub const fn is_denied(&self) -> bool {
        matches!(self, Self::Denied { .. })
    }

    /// Is the command awaiting an approval?
    #[must_use]
    pub const fn needs_approval(&self) -> bool {
        matches!(self, Self::NeedsApproval { .. })
    }

    /// Did the command produce an effect?
    ///
    /// `false` for [`NeedsApproval`](Self::NeedsApproval) and
    /// [`Denied`](Self::Denied). The trap it closes: a caller — an agent in
    /// particular — that assumes an `INSERT` took place and builds on that
    /// assumption.
    #[must_use]
    pub const fn took_effect(&self) -> bool {
        !matches!(self, Self::NeedsApproval { .. } | Self::Denied { .. })
    }

    /// The rows produced or affected, when the notion makes sense.
    #[must_use]
    pub fn rows(&self) -> Option<u64> {
        match self {
            Self::Executed { stats, .. } => Some(stats.rows),
            Self::Exported { rows, .. } => u64::try_from(*rows).ok(),
            _ => None,
        }
    }
}

pub(crate) struct StoredResult {
    pub(crate) connection: ConnectionId,
    pub(crate) buffer: Arc<ResultBuffer>,
}

/// The executor.
///
/// Shared through `Arc` between the interface, the agent runtime
/// ([`crate::ExecutorSink`]) and background tasks. All its methods take
/// `&self`.
pub struct Executor {
    drivers: Arc<DriverRegistry>,
    store: Arc<Store>,
    policy: Arc<dyn PolicyGate>,
    credentials: Arc<dyn CredentialResolver>,
    sessions: SessionRegistry,
    running: CancelRegistry,
    approvals: ApprovalRegistry,
    events: EventBus,
    results: RwLock<crate::retained::RetainedResults>,
    catalogs: RwLock<HashMap<ConnectionId, Arc<crate::catalog::ConnectionCatalog>>>,
    connections: Arc<RwLock<HashMap<ConnectionId, ConnectionConfig>>>,
    /// Orders every write of a connection — to the store, then to
    /// `connections` — inside one blocking task. The cache feeds the
    /// environment given to the `PolicyGate` (I-02): two saves whose disk and
    /// cache writes interleave, or a save abandoned between the two, would
    /// leave `production` on disk and `development` in the cache. Readers of
    /// `connections` never take this lock.
    connection_writes: Arc<Mutex<()>>,
    workspace: WorkspaceId,
    /// The launch that writes window layouts: the next launch adopts a
    /// window's line only once this one has ended (ADR-0043).
    app_session: oxyn_core::AppSessionId,
    memory_budget: usize,
    abandoned: AbandonedOutcomes,
    /// History rows still `running` since then belong to this launch: their
    /// outcome is not known yet, so they cannot be declared reconciled.
    started_at: chrono::DateTime<chrono::Utc>,
}

impl Executor {
    /// Starts wiring an executor.
    ///
    /// The `Store` and the `PolicyGate` are required from the call: an executor
    /// without a log or without a policy has no acceptable degraded form.
    #[must_use]
    pub fn builder(store: Arc<Store>, policy: Arc<dyn PolicyGate>) -> ExecutorBuilder {
        ExecutorBuilder::new(store, policy)
    }

    // ── The passage point ───────────────────────────────────────────────────

    /// Submits a command.
    ///
    /// It is **the** method: the interface, agents and plugins all go through
    /// it, with the same code behind.
    ///
    /// A denial and an approval request are **not** errors: they are
    /// [`Outcome`]s. An `Err` describes a failure — an unreachable server, a
    /// timeout, an unreadable log.
    ///
    /// # Errors
    /// Any error of the driver, the buffer or the local state; and
    /// [`OxynError::Internal`] if the policy decision could not be logged
    /// before execution — in which case **nothing is executed**.
    pub async fn dispatch(
        &self,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        self.dispatch_as(CommandId::new(), actor, command, cancel)
            .await
    }

    /// [`dispatch`](Self::dispatch), under an identifier supplied by the caller.
    ///
    /// Serves to correlate **before** the answer arrives: an agent sink returns
    /// the identifier in its report, an interface displays it in its status
    /// bar. `id` must be fresh — reusing it would mix two commands in the audit
    /// log, where it is the correlation key.
    ///
    /// # Errors
    /// Those of [`dispatch`](Self::dispatch).
    pub async fn dispatch_as(
        &self,
        id: CommandId,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        // 1. What the text does, not what the caller says about it.
        let command = reclassified(command);
        let connection = command.target_connection();

        // 2. The environment of the target connection, not the announced one.
        let env = self.environment_of(&command);
        let retained = self.policy.retained_environment(&command, env);
        let command = bounded_to(command, retained);

        // 3. The single passage point.
        let decision = self.policy.authorize(&actor, &command, env);

        // 4. Log BEFORE, in every branch: see ADR-0035. A decision that cannot
        // be written is not executed.
        match &decision {
            Decision::Deny { reason } => {
                let reason = reason.clone();
                let record = decision_record(id, &actor, &command, &decision, None);
                let denied = self
                    .history_record(&actor, &command)
                    .map(|entry| entry.denied(reason.clone()));
                // Both writes are attempted independently: a failure on the
                // decision must never swallow the denial's history entry, and
                // vice versa — unchanged by this move to the blocking pool.
                if self
                    .write_audit(move |store| {
                        if let Err(err) = store.journal().append(&record) {
                            tracing::error!(error = %err, command = %id, "policy decision could not be journaled");
                        }
                        if let Some(entry) = denied
                            && let Err(err) = store.history().record(&entry)
                        {
                            tracing::error!(error = %err, "a denied statement could not be recorded in the query history");
                        }
                    })
                    .await
                    .is_err()
                {
                    tracing::error!(command = %id, "policy decision could not be journaled");
                }
                self.events.publish(
                    id,
                    connection,
                    Event::failed(&OxynError::PolicyDenied {
                        reason: reason.clone(),
                    }),
                );
                Ok(Outcome::Denied {
                    command: id,
                    reason,
                })
            }

            Decision::RequireApproval { reason, preview } => {
                let reason = reason.clone();
                let preview = preview.clone();
                let record = decision_record(id, &actor, &command, &decision, None);
                let write = self
                    .write_audit(move |store| -> Result<()> {
                        store.journal().append(&record)?;
                        Ok(())
                    })
                    .await
                    .and_then(|inner| inner);
                if let Err(err) = write {
                    tracing::error!(error = %err, command = %id, "policy decision could not be journaled");
                    return Err(err);
                }
                // If the caller drops this future right here, the decision
                // stays in the journal with no approval request pending:
                // nothing is queued and nothing runs — the same outcome as a
                // request later rejected or expired (see ADR-0035).
                //
                // Nothing is executed. The command set aside is the one the
                // gate saw — reclassified — not the original text.
                let pending = self.approvals.submit(id, actor, command, reason, preview)?;
                self.events.publish(
                    id,
                    connection,
                    Event::ApprovalRequested {
                        command: id,
                        reason: pending.reason.clone(),
                        preview: pending.preview.clone(),
                    },
                );
                Ok(Outcome::NeedsApproval {
                    command: id,
                    reason: pending.reason,
                    preview: pending.preview,
                })
            }

            Decision::Allow => {
                self.run(id, &actor, &command, &decision, cancel, None)
                    .await
            }
        }
    }

    /// Gives the approval a command is waiting for, and executes it.
    ///
    /// `approved_by` is **who** approved: the log records it. There is no
    /// `Actor` parameter on purpose — this method is called from the
    /// interface, by a human. An agent has no way to reach it: the tools exposed
    /// to agents are exactly the [`Command`]s, and there is no approval command.
    ///
    /// The command **goes through the `PolicyGate` again** before leaving: the
    /// connection may have been marked production or read-only during the
    /// wait, and a denial then wins over the approval given.
    ///
    /// # Errors
    /// [`OxynError::PolicyDenied`] if no command is waiting under this
    /// identifier or if the request expired — in both cases **nothing is
    /// executed**; otherwise the errors of [`dispatch`](Self::dispatch).
    pub async fn approve(
        &self,
        approved_by: &str,
        command: CommandId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let mut pending = self.approvals.take(command)?;
        let connection = pending.command.target_connection();

        // The connection may have been marked production while waiting.
        let env = self.environment_of(&pending.command);
        let retained = self.policy.retained_environment(&pending.command, env);
        pending.command = bounded_to(pending.command, retained);
        let decision = self.policy.authorize(&pending.actor, &pending.command, env);

        if let Decision::Deny { reason } = &decision {
            let reason = reason.clone();
            let record =
                decision_record(command, &pending.actor, &pending.command, &decision, None);
            let denied = self
                .history_record(&pending.actor, &pending.command)
                .map(|entry| entry.denied(reason.clone()));
            // The late denial is logged too: it is even the most interesting
            // trace of all, since an approval had been given.
            // Both writes are attempted independently, same as an ordinary
            // denial in `dispatch_as`.
            if self
                .write_audit(move |store| {
                    if let Err(err) = store.journal().append(&record) {
                        tracing::error!(error = %err, command = %command, "late denial could not be journaled");
                    }
                    if let Some(entry) = denied
                        && let Err(err) = store.history().record(&entry)
                    {
                        tracing::error!(error = %err, "a denied statement could not be recorded in the query history");
                    }
                })
                .await
                .is_err()
            {
                tracing::error!(command = %command, "late denial could not be journaled");
            }
            self.events.publish(
                command,
                connection,
                Event::failed(&OxynError::PolicyDenied {
                    reason: reason.clone(),
                }),
            );
            return Ok(Outcome::Denied { command, reason });
        }

        // No more destructuring of `pending` before execution: `run` writes
        // the decision itself, as the first operation of its window.
        self.run(
            command,
            &pending.actor,
            &pending.command,
            &decision,
            cancel,
            Some(approved_by),
        )
        .await
    }

    /// Removes a request the user answered "no" to.
    ///
    /// Nothing is executed, and the command can no longer be: it will have to
    /// be re-emitted, hence go through the gate again.
    pub fn reject(&self, command: CommandId) -> Option<PendingCommand> {
        self.approvals.reject(command)
    }

    // ── Execution ───────────────────────────────────────────────────────────

    /// Executes an already authorized command, and logs its decision and its
    /// outcome around the execution.
    ///
    /// `decision` is what `PolicyGate::authorize` returned to the caller —
    /// never `Deny`, which both callers handle before reaching this method.
    ///
    /// # Errors
    /// Those of [`execute_command`](Self::execute_command), and
    /// [`OxynError::Internal`] if the decision could not be logged — in which
    /// case **nothing is executed**.
    async fn run(
        &self,
        id: CommandId,
        actor: &Actor,
        command: &Command,
        decision: &Decision,
        cancel: &CancelToken,
        approved_by: Option<&str>,
    ) -> Result<Outcome> {
        // Armed as soon as the policy allows this command, before its
        // decision is even written (ADR-0035): no `.await` runs between the
        // `Allow` a caller matched and this line, so no abandonment can slip
        // in ahead of the guard.
        let guard = OutcomeGuard::new(&self.abandoned, id, actor, command, approved_by);

        // One operation: the decision, then — only if it was written — the
        // "in progress" entry in the query history. The history is written
        // here and not at dispatch: a command put on hold for approval then
        // rejected would otherwise leave an eternal "in progress" row, although
        // it was never submitted to the server.
        let record = decision_record(id, actor, command, decision, approved_by);
        let starting = self.history_record(actor, command);
        let write = self
            .write_audit(move |store| -> Result<Option<(i64, HistoryRecord)>> {
                store.journal().append(&record)?;
                Ok(
                    starting.and_then(|record| match store.history().record(&record) {
                        Ok(history_id) => Some((history_id, record)),
                        Err(err) => {
                            tracing::error!(
                                error = %err,
                                "a submitted statement could not be recorded in the query history"
                            );
                            None
                        }
                    }),
                )
            })
            .await
            .and_then(|inner| inner);

        let in_progress = match write {
            Ok(in_progress) => in_progress,
            Err(err) => {
                tracing::error!(error = %err, command = %id, "policy decision could not be journaled");
                // Nothing ran: there is no outcome to write, and no hole in
                // the audit trail to leave behind.
                guard.settle();
                return Err(err);
            }
        };

        let start = Instant::now();
        let result_outcome = self.execute_command(id, command, cancel).await;
        guard.settle();
        let duration = start.elapsed();

        let mut outcome = outcome_record(
            id,
            actor,
            command,
            duration,
            result_outcome.as_ref().ok().and_then(Outcome::rows),
            approved_by,
        );
        if let Err(err) = &result_outcome {
            outcome = outcome.failed(err);
        }
        let finishing = finished_history_record(in_progress, &result_outcome, duration);

        // Submitted right away, with no `.await` between `guard.settle()`
        // above and this call: the window `OutcomeGuard` covers stays
        // exactly the one it covers today, between the start of execution
        // and the moment this operation is queued.
        let written = self
            .write_audit(move |store| {
                if let Err(err) = store.journal().append(&outcome) {
                    tracing::error!(
                        error = %err,
                        command = %id,
                        "failed to journal the outcome of a command that already ran"
                    );
                }
                let (history_id, record) = finishing?;
                match store.history().finish(history_id, &record) {
                    Ok(true) => Some(record.status),
                    Ok(false) => None,
                    Err(err) => {
                        tracing::error!(error = %err, "the outcome of a statement could not be written to the query history");
                        None
                    }
                }
            })
            .await;
        match written {
            // Announced once the row is written, so that a view reading the
            // history again sees the outcome and not « running ».
            Ok(Some(HistoryStatus::Succeeded)) => {
                self.events
                    .publish(id, command.target_connection(), Event::HistoryRecorded);
            }
            Ok(_) => {}
            Err(_) => {
                tracing::error!(command = %id, "the audit writer stopped after a command already ran");
            }
        }

        result_outcome
    }

    /// The dispatch proper.
    ///
    /// The `match` is **exhaustive** and without `_ =>`: [`Command`] is a
    /// closed enum precisely so that adding a command makes compilation fail
    /// here. A `_ =>` would silently swallow a command the `PolicyGate` has
    /// just authorized.
    async fn execute_command(
        &self,
        id: CommandId,
        command: &Command,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        match command {
            Command::Connect { connection } => self.connect(*connection, cancel).await,
            Command::TestConnection { config } => self.test_connection(config, cancel).await,

            Command::Disconnect { connection } => self.disconnect(*connection).await,
            Command::CloseSession {
                connection,
                session,
            } => {
                if cancel.is_cancelled() {
                    return Err(OxynError::Cancelled);
                }
                self.close_session(*connection, *session).await
            }

            Command::SetSessionContext {
                connection,
                session,
                catalog,
                namespace,
            } => {
                if cancel.is_cancelled() {
                    return Err(OxynError::Cancelled);
                }
                self.set_session_context(
                    *connection,
                    *session,
                    catalog.as_deref(),
                    namespace.as_deref(),
                    cancel,
                )
                .await
            }

            Command::Execute {
                connection,
                session,
                request,
            } => {
                self.execute_statement(id, *connection, *session, (**request).clone(), cancel, None)
                    .await
            }

            Command::PreviewRelation {
                connection,
                session,
                catalog,
                namespace,
                relation,
                limit,
                shape,
            } => {
                if !(1..=1000).contains(limit) {
                    return Err(OxynError::Config(
                        "preview limit must be in 1..=1000".into(),
                    ));
                }
                let path = oxyn_catalog::CatalogPath::for_relation(
                    catalog.as_deref(),
                    namespace.as_deref(),
                    relation,
                )?;
                let slot = self
                    .sessions
                    .get(*session)
                    .ok_or_else(|| OxynError::Connection("preview session is not open".into()))?;
                if slot.connection() != *connection {
                    return Err(OxynError::Config(
                        "preview session belongs to another connection".into(),
                    ));
                }
                // Refused here rather than left to the driver: the capability is
                // what **this session** declares, and a request it cannot honor
                // must not reach the SQL composition, where the temptation
                // would be to ignore it (ADR-0003, ADR-0020).
                let capabilities = slot.capabilities();
                if !shape.sort.is_empty()
                    && !capabilities.contains(oxyn_core::Capabilities::PREVIEW_SORT)
                {
                    return Err(OxynError::NotSupported {
                        capability: "preview sort".to_owned(),
                    });
                }
                if shape.predicate().is_some()
                    && !capabilities.contains(oxyn_core::Capabilities::PREVIEW_FILTER)
                {
                    return Err(OxynError::NotSupported {
                        capability: "preview filter".to_owned(),
                    });
                }
                let request = slot.preview_request(&path, *limit, shape, cancel).await?;
                let mut request = oxyn_query::reclassify(&request).qualify(request);
                if request.is_mutating() {
                    // Two distinct refusals, because they call for two different
                    // actions. `Unknown` means "this text could not be
                    // classified", and on a preview the only part written by
                    // hand is the predicate: calling it "not read-only" would
                    // send the user looking for a missing right when they have
                    // a typo. The refusal stays in both cases — a text the
                    // classifier does not understand counts as mutating, and it
                    // is that caution that protects.
                    let reason = if request.intent == oxyn_core::StatementIntent::Unknown {
                        "this preview filter could not be read as a condition; \
                         check its syntax"
                    } else {
                        "driver preview request is not read-only"
                    };
                    return Err(OxynError::PolicyDenied {
                        reason: reason.to_owned(),
                    });
                }
                // Enforce the command contract even if a driver omitted its limits.
                request.limits.read_only = true;
                let row_limit = usize::try_from(*limit).map_err(|_| {
                    OxynError::Config("preview limit exceeds platform capacity".into())
                })?;
                // The SQL is already bounded. One extra receive slot lets the
                // cursor report its real end instead of a client-side truncation.
                // The buffer still rejects every row beyond the requested limit.
                request.limits.max_rows = Some(row_limit.saturating_add(1));
                self.execute_statement(id, *connection, *session, request, cancel, Some(row_limit))
                    .await
            }

            Command::Cancel { statement, .. } => {
                let report = self.running.cancel(&self.sessions, *statement).await;
                Ok(Outcome::Cancelled { report })
            }

            Command::RefreshCatalog { connection } => {
                self.refresh_catalog(id, *connection, &CatalogRefreshScope::Root, cancel)
                    .await
            }
            Command::RefreshCatalogScope { connection, scope } => {
                self.refresh_catalog(id, *connection, scope, cancel).await
            }
            Command::DescribeCatalog { connection, focus } => {
                if focus
                    .as_deref()
                    .is_some_and(|focus| focus.len() > oxyn_core::MAX_CATALOG_FOCUS_BYTES)
                {
                    return Err(OxynError::Config(format!(
                        "a catalog focus is limited to {} bytes",
                        oxyn_core::MAX_CATALOG_FOCUS_BYTES
                    )));
                }
                // The cache as it stands: no server is contacted, and no
                // rendering is done here — what of it reaches a prompt is
                // decided under the connection's tier, in `oxyn-ai`.
                let catalog = self.catalog(*connection).ok_or_else(|| {
                    OxynError::Config(
                        "this connection is not open, so it has no catalog: the user must \
                         connect to it first"
                            .into(),
                    )
                })?;
                Ok(Outcome::CatalogDescribed {
                    connection: *connection,
                    catalog: oxyn_catalog::CatalogHandle::new(catalog),
                })
            }

            Command::InspectResultValue {
                connection,
                result,
                row,
                column,
                offset,
            } => {
                let buffer = self.result_on_connection(*connection, *result)?;
                let (row, column, offset) = (*row, *column, *offset);
                let cancel = cancel.clone();
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("value inspection requires the application runtime".into())
                })?;
                let page = runtime
                    .spawn_blocking(move || -> Result<_> {
                        let (batch, row) = buffer.read_row(row, &cancel)?.ok_or_else(|| {
                            OxynError::Config("selected row is no longer available".into())
                        })?;
                        oxyn_data::value_page::inspect_value(&batch, row, column, offset, &cancel)?
                            .ok_or_else(|| {
                                OxynError::Config("selected column is no longer available".into())
                            })
                    })
                    .await
                    .map_err(|_| OxynError::Internal("value inspection worker stopped".into()))??;
                Ok(Outcome::ValueInspected { page })
            }

            Command::ReadResultPage {
                connection,
                result,
                batch,
            } => {
                let buffer = self.result_on_connection(*connection, *result)?;
                let cancel = cancel.clone();
                let position = *batch;
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("result page loading requires the application runtime".into())
                })?;
                let loaded = runtime
                    .spawn_blocking(move || {
                        buffer.load_page(oxyn_data::BatchIndex::new(position), &cancel)
                    })
                    .await
                    .map_err(|_| OxynError::Internal("result page worker stopped".into()))??;
                if !loaded {
                    return Err(OxynError::Config(
                        "result page is no longer available".into(),
                    ));
                }
                Ok(Outcome::ResultPageRead {
                    result: *result,
                    batch: *batch,
                })
            }

            Command::Export {
                connection,
                result,
                format,
                destination,
                ..
            } => {
                let buffer = self.result_on_connection(*connection, *result)?;
                // Before anything is written: otherwise a format this version cannot
                // write fails after the user has named the destination. The check
                // lives here rather than in the view because a plugin and an
                // `Actor::Agent` reach this command too — a check that exists only
                // in the interface is not a check of the bus (I-01).
                if !oxyn_data::is_supported(*format) {
                    return Err(OxynError::NotSupported {
                        capability: format!("export:{}", format.extension()),
                    });
                }
                // A truncated buffer, or duplicate names a JSON object would
                // collapse, are refused here for the same reason: before the
                // destination is touched, and for every actor.
                oxyn_data::ensure_exportable(&buffer, *format, &ExportOptions::default())?;
                let format = *format;
                let destination = destination.clone();
                let cancel_owned = cancel.clone();
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("export requires the application runtime".into())
                })?;
                let summary = runtime
                    .spawn_blocking(move || -> Result<_> {
                        // Never `File::create`: it would truncate the file already
                        // there before knowing whether the export succeeds.
                        Ok(export_to_path(
                            &buffer,
                            format,
                            &destination,
                            &ExportOptions::default(),
                            &cancel_owned,
                        )?)
                    })
                    .await
                    .map_err(|_| OxynError::Internal("export worker stopped".into()))??;
                Ok(Outcome::Exported {
                    result: *result,
                    rows: summary.rows,
                    bytes: summary.bytes,
                })
            }

            Command::ReadWorkspacePreferences { workspace } => {
                if *workspace != self.workspace {
                    return Err(OxynError::Config(
                        "workspace does not match this executor".into(),
                    ));
                }
                let store = self.store.clone();
                let workspace = *workspace;
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("preference loading requires the application runtime".into())
                })?;
                let snapshot = runtime
                    .spawn_blocking(move || store.preferences().load(workspace))
                    .await
                    .map_err(|_| OxynError::Internal("preference worker stopped".into()))??;
                Ok(Outcome::WorkspacePreferences { snapshot })
            }
            Command::WriteWorkspacePreferences {
                workspace,
                snapshot,
            } => {
                if *workspace != self.workspace {
                    return Err(OxynError::Config(
                        "workspace does not match this executor".into(),
                    ));
                }
                snapshot.validate()?;
                let store = self.store.clone();
                let workspace = *workspace;
                let snapshot = (**snapshot).clone();
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("preference saving requires the application runtime".into())
                })?;
                let snapshot = runtime
                    .spawn_blocking(move || store.preferences().save(workspace, &snapshot))
                    .await
                    .map_err(|_| OxynError::Internal("preference worker stopped".into()))??;
                Ok(Outcome::WorkspacePreferences { snapshot })
            }

            Command::WriteWindowLayout { workspace, change } => {
                self.check_workspace(*workspace)?;
                if let oxyn_core::WindowLayoutChange::Save(layout) = &**change {
                    layout.validate()?;
                }
                let store = self.store.clone();
                let workspace = *workspace;
                let change = (**change).clone();
                let session = self.app_session;
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config(
                        "window layout saving requires the application runtime".into(),
                    )
                })?;
                runtime
                    .spawn_blocking(move || {
                        let windows = store.windows();
                        match change {
                            oxyn_core::WindowLayoutChange::Save(layout) => {
                                windows.save(workspace, session, &layout)
                            }
                            oxyn_core::WindowLayoutChange::Remove(window) => {
                                windows.remove(workspace, window)
                            }
                        }
                    })
                    .await
                    .map_err(|_| OxynError::Internal("window layout worker stopped".into()))??;
                Ok(Outcome::WindowLayoutWritten)
            }

            Command::ListQueryDocuments { workspace, filter } => {
                self.check_workspace(*workspace)?;
                filter.validate()?;
                let store = self.store.clone();
                let workspace = *workspace;
                let filter = (**filter).clone();
                let page = self
                    .local_worker(cancel, move |cancel| {
                        store.documents().page(workspace, &filter, &cancel)
                    })
                    .await?;
                Ok(Outcome::QueryDocumentsListed { page })
            }
            Command::SaveQueryDocument { workspace, update } => {
                self.check_workspace(*workspace)?;
                update.validate()?;
                let store = self.store.clone();
                let workspace = *workspace;
                let update = (**update).clone();
                let document = self
                    .local_worker(cancel, move |cancel| {
                        store.documents().update_query(workspace, &update, &cancel)
                    })
                    .await?;
                Ok(Outcome::DocumentOpened {
                    document: Box::new(document),
                })
            }
            Command::CloseQueryDocument {
                expected_revision,
                workspace,
                document,
                revision,
                discard,
            } => {
                self.check_workspace(*workspace)?;
                let store = self.store.clone();
                let workspace = *workspace;
                let target = *document;
                let revision = *revision;
                let discard = *discard;
                let expected_revision = *expected_revision;
                self.local_worker(cancel, move |cancel| {
                    store.documents().close_query_checked(
                        workspace,
                        target,
                        oxyn_store::documents::DocumentRevision {
                            next: revision,
                            expected: expected_revision,
                        },
                        discard,
                        false,
                        &cancel,
                    )
                })
                .await?;
                Ok(Outcome::DocumentClosed { document: target })
            }
            Command::DeleteQueryDocument {
                workspace,
                document,
                revision,
            } => {
                self.check_workspace(*workspace)?;
                let store = self.store.clone();
                let workspace = *workspace;
                let target = *document;
                let revision = *revision;
                self.local_worker(cancel, move |cancel| {
                    store
                        .documents()
                        .close_query(workspace, target, revision, true, true, &cancel)
                })
                .await?;
                Ok(Outcome::DocumentClosed { document: target })
            }
            Command::ReadHistory { filter } => {
                filter.validate()?;
                let store = self.store.clone();
                let filter = (**filter).clone();
                let page = self
                    .local_worker(cancel, move |cancel| store.history().page(&filter, &cancel))
                    .await?;
                Ok(Outcome::HistoryListed { page })
            }
            Command::ReadHistoryEntry { entry } => {
                let store = self.store.clone();
                let target = *entry;
                let entry = self
                    .local_worker(cancel, move |cancel| store.history().get(target, &cancel))
                    .await?
                    .ok_or_else(|| {
                        OxynError::Config("history entry is no longer available".into())
                    })?;
                Ok(Outcome::HistoryEntryRead {
                    entry: Box::new(entry),
                })
            }
            Command::ReconcileHistoryEntry { entry } => {
                let store = self.store.clone();
                let target = *entry;
                let live_since = self.started_at;
                let reconciliation = self
                    .local_worker(cancel, move |cancel| {
                        store.history().reconcile(target, live_since, &cancel)
                    })
                    .await?;
                match reconciliation {
                    Reconciliation::Recorded => {
                        Ok(Outcome::HistoryEntryReconciled { entry: target })
                    }
                    Reconciliation::StillRunning => Err(OxynError::Config(
                        "this write is still running; wait for its outcome before reconciling it"
                            .into(),
                    )),
                    _ => Err(OxynError::Config(
                        "history entry is gone or has no unresolved outcome".into(),
                    )),
                }
            }
            Command::ListHistoryConnections { workspace, filter } => {
                self.check_workspace(*workspace)?;
                filter.validate()?;
                let store = self.store.clone();
                let workspace = *workspace;
                let filter = *filter;
                let page = self
                    .local_worker(cancel, move |cancel| {
                        store.history().connections(workspace, &filter, &cancel)
                    })
                    .await?;
                Ok(Outcome::HistoryConnectionsListed { page })
            }
            Command::OpenRetainedResult { connection, result } => {
                Ok(Outcome::RetainedResultOpened {
                    result: *result,
                    buffer: self.result_on_connection(*connection, *result)?,
                })
            }
            Command::OpenDocument {
                workspace,
                document,
            } => {
                self.check_workspace(*workspace)?;
                self.open_document(*workspace, *document, cancel).await
            }
            Command::WriteDocument {
                workspace,
                document,
                text,
            } => {
                self.check_workspace(*workspace)?;
                self.write_document(*workspace, *document, text, cancel)
                    .await
            }

            Command::CreateConnection { config } | Command::UpdateConnection { config } => {
                self.save_connection(config, cancel).await
            }

            Command::DeleteConnection { connection } => {
                self.delete_connection(*connection, cancel).await
            }

            // The three provider commands stay local: nothing here resolves a
            // name or opens a connection to a model. The local/remote
            // classification is recomputed when a runtime opens (ADR-0023), and
            // a DNS resolution done here would make it age in the database
            // under another name.
            Command::ListAiProviders => {
                let store = self.store.clone();
                let providers = self
                    .local_worker(cancel, move |_cancel| store.providers().list())
                    .await?;
                Ok(Outcome::AiProvidersListed { providers })
            }
            Command::SaveAiProvider { config } => {
                // Validated before reaching the pool: a URL carrying credentials
                // must not travel further than necessary.
                config.validate()?;
                let store = self.store.clone();
                let config = (**config).clone();
                let provider = config.id.clone();
                self.local_worker(cancel, move |_cancel| store.providers().save(&config))
                    .await?;
                Ok(Outcome::AiProviderSaved { provider })
            }
            Command::RemoveAiProvider { id } => {
                let store = self.store.clone();
                let provider = id.clone();
                let dest = id.clone();
                let existed = self
                    .local_worker(cancel, move |_cancel| store.providers().remove(&dest))
                    .await?;
                Ok(Outcome::AiProviderRemoved { provider, existed })
            }

            // The same three actions for an external agent. No URL validation
            // here: there is none. The declaration is validated before the
            // pool, for the reason that also holds for providers — a command
            // carrying a control character must not travel further than
            // necessary.
            Command::ListExternalAgents => {
                let store = self.store.clone();
                let agents = self
                    .local_worker(cancel, move |_cancel| store.external_agents().list())
                    .await?;
                Ok(Outcome::ExternalAgentsListed { agents })
            }
            Command::SaveExternalAgent { agent } => {
                agent.validate()?;
                let store = self.store.clone();
                let declaration = (**agent).clone();
                let identity = declaration.id.clone();
                self.local_worker(cancel, move |_cancel| {
                    store.external_agents().save(&declaration)
                })
                .await?;
                Ok(Outcome::ExternalAgentSaved { agent: identity })
            }
            Command::RemoveExternalAgent { id } => {
                let store = self.store.clone();
                let agent = id.clone();
                let dest = id.clone();
                let existed = self
                    .local_worker(cancel, move |_cancel| store.external_agents().remove(&dest))
                    .await?;
                Ok(Outcome::ExternalAgentRemoved { agent, existed })
            }
        }
    }

    /// Opens a session.
    async fn connect(&self, connection: ConnectionId, cancel: &CancelToken) -> Result<Outcome> {
        let config = self.connection_config(connection, cancel).await?;
        let session = self.open_driver_session(&config, cancel).await?;
        let slot = self.sessions.insert(SessionSlot::new(connection, session));
        // Read, not assumed: a fresh SQLite connection is in autocommit, and
        // the console starts `Idle` because the engine said so. Before any
        // lock below: none may be held across this await.
        let transaction_state = slot.transaction_state().await;
        let catalog = self
            .catalogs
            .write()
            .entry(connection)
            .or_insert_with(|| Arc::new(crate::catalog::ConnectionCatalog::new()))
            .clone();
        let mut preferred = catalog.session.write();
        if preferred
            .and_then(|id| self.sessions.get(id))
            .is_none_or(|session| !session.is_open())
        {
            *preferred = Some(slot.id());
        }
        drop(preferred);
        Ok(Outcome::Connected {
            connection,
            session: slot.id(),
            transaction_state,
        })
    }

    /// Opens a session on a configuration the workspace does not hold, then
    /// closes it.
    ///
    /// The session never enters the registry: nothing can run on it, and no
    /// catalog is attached to it. The error, if any, is the driver's own,
    /// class included — the one a real opening would give.
    async fn test_connection(
        &self,
        config: &ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let session = self.open_driver_session(config, cancel).await?;
        if let Err(err) = session.close().await {
            // The opening succeeded, which is what was asked; a refused close
            // frees the local resources all the same.
            tracing::warn!(error = %err, "the server refused a clean close of a test session");
        }
        Ok(Outcome::ConnectionTested {
            connection: config.id,
        })
    }

    /// Resolves the credentials and asks the driver for a session.
    async fn open_driver_session(
        &self,
        config: &ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<Box<dyn oxyn_driver::Session>> {
        let driver = self.drivers.require(&config.driver)?;
        // The system keyring lookup is blocking; it never runs on the shared
        // runtime that also carries drivers, network I/O and LLM calls (I-05).
        let resolver = Arc::clone(&self.credentials);
        let lookup = config.clone();
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| OxynError::Config("connecting requires the application runtime".into()))?;
        let credentials = runtime
            .spawn_blocking(move || resolver.resolve(&lookup))
            .await
            .map_err(|_| OxynError::Internal("credential worker stopped".into()))??;
        driver.connect(config, &credentials, cancel).await
    }

    /// Closes the sessions of a connection, after cancelling what runs on them.
    async fn disconnect(&self, connection: ConnectionId) -> Result<Outcome> {
        // Cancellation first: closing without cancelling leaves the queries
        // running server-side, connection taken and lock held.
        self.running
            .cancel_connection(&self.sessions, connection)
            .await;

        if let Some(catalog) = self.catalogs.write().remove(&connection) {
            catalog.closed.cancel();
        }
        let mut closed = 0;
        for slot in self.sessions.drain_connection(connection) {
            if let Err(err) = slot.close().await {
                // Local resources are released in every case; a closing refused
                // by the server cannot be made up for.
                tracing::warn!(error = %err, "the server refused a clean session close");
            }
            closed += 1;
        }
        Ok(Outcome::Disconnected { connection, closed })
    }

    async fn close_session(&self, connection: ConnectionId, session: SessionId) -> Result<Outcome> {
        let Some(slot) = self.sessions.get(session) else {
            return Ok(Outcome::SessionClosed { session });
        };
        if slot.connection() != connection {
            return Err(OxynError::PolicyDenied {
                reason: "session does not belong to this connection".into(),
            });
        }
        slot.begin_close();
        for running in self.running.for_session(session) {
            self.running.cancel(&self.sessions, running.statement).await;
        }
        self.sessions.remove(session);
        slot.close().await?;
        Ok(Outcome::SessionClosed { session })
    }

    /// Declares where one session resolves unqualified names.
    ///
    /// Refuses the session the catalog and previews read through: the explorer
    /// shows a qualified tree, and moving it under the user because a console
    /// changed schema would make the same click mean two things on two days
    /// ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)).
    async fn set_session_context(
        &self,
        connection: ConnectionId,
        session: SessionId,
        catalog: Option<&str>,
        namespace: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let Some(slot) = self.sessions.get(session) else {
            return Err(OxynError::PolicyDenied {
                reason: "unknown session".into(),
            });
        };
        if slot.connection() != connection {
            return Err(OxynError::PolicyDenied {
                reason: "session does not belong to this connection".into(),
            });
        }
        if !slot
            .capabilities()
            .contains(oxyn_core::Capabilities::SESSION_CONTEXT)
        {
            return Err(OxynError::NotSupported {
                capability: "session context".to_owned(),
            });
        }
        let reserved = self
            .catalogs
            .read()
            .get(&connection)
            .and_then(|state| *state.session.read());
        if reserved == Some(session) {
            return Err(OxynError::PolicyDenied {
                reason: "this session serves the catalog and previews; \
                         open a console to change context"
                    .into(),
            });
        }
        // Validation by the catalog path rather than here: a control character
        // in a name must be refused before it reaches a driver about to quote it.
        let path = oxyn_catalog::CatalogPath::from_levels(
            catalog.map(ToOwned::to_owned),
            namespace.map(ToOwned::to_owned),
            None,
        )
        .map_err(|error| OxynError::Config(error.to_string()))?;
        let context = oxyn_driver::SessionContext::from_path(&path);
        slot.set_context(&context, cancel).await?;
        Ok(Outcome::SessionContextSet {
            session,
            context: slot.context(cancel).await?,
        })
    }

    /// Returns the connected source's in-memory cache without any I/O.
    ///
    /// None before Connect or after Disconnect. Hold read guards briefly and
    /// never across await; refreshes are issued through dispatch only.
    #[must_use]
    pub fn catalog(&self, connection: ConnectionId) -> Option<SharedCatalog> {
        self.catalogs
            .read()
            .get(&connection)
            .map(|state| Arc::clone(&state.cache))
    }

    /// Marks a connection's whole catalog stale after a DDL succeeds.
    ///
    /// The scope is the whole connection, never the touched object: naming
    /// that object would mean reconstructing an identifier from the executed
    /// SQL text, which I-10 forbids. Data is kept, only marked
    /// [`Invalidated`](oxyn_catalog::Freshness::Invalidated) — the next read
    /// re-fetches it.
    fn invalidate_catalog(&self, connection: ConnectionId) {
        if let Some(state) = self.catalogs.read().get(&connection).cloned() {
            state.cache.write().invalidate_all();
        }
    }

    async fn refresh_catalog(
        &self,
        id: CommandId,
        connection: ConnectionId,
        scope: &CatalogRefreshScope,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let state = self
            .catalogs
            .read()
            .get(&connection)
            .cloned()
            .ok_or_else(|| OxynError::Connection("no open catalog session".to_owned()))?;
        let mut budget = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(OxynError::Cancelled),
            _ = state.closed.cancelled() => return Err(OxynError::Cancelled),
            guard = state.refresh.lock() => guard,
        };
        let preferred = *state.session.read();
        let slot = preferred
            .and_then(|session| self.sessions.get(session))
            .or_else(|| {
                if preferred.is_none() {
                    self.sessions
                        .for_connection(connection)
                        .into_iter()
                        .find(|slot| slot.is_open())
                } else {
                    None
                }
            })
            .filter(|slot| slot.is_open())
            .ok_or_else(|| OxynError::Connection("no open catalog session".to_owned()))?;
        let operation = cancel.child();
        let read = slot.read_catalog(scope, &operation);
        tokio::pin!(read);
        let patch = tokio::select! {
            biased;
            _ = state.closed.cancelled() => {
                operation.cancel();
                // Let the provider finish cancellation and protocol cleanup.
                let _ = read.await;
                return Err(OxynError::Cancelled);
            }
            result = &mut read => result?,
        };
        // The registry read guard orders publication against Disconnect.
        let catalogs = self.catalogs.read();
        if cancel.is_cancelled()
            || state.closed.is_cancelled()
            || !catalogs
                .get(&connection)
                .is_some_and(|current| Arc::ptr_eq(current, &state))
        {
            return Err(OxynError::Cancelled);
        }
        let evicted = budget.reserve(scope, &patch)?;
        {
            let mut cache = state.cache.write();
            for scope in &evicted {
                cache.evict(scope);
            }
            patch.apply(&mut cache)?;
        }
        self.events
            .publish(id, Some(connection), Event::CatalogUpdated);
        Ok(Outcome::CatalogRefreshed {
            connection,
            scope: scope.clone(),
        })
    }

    /// Executes a statement and drains its cursor into a buffer.
    async fn execute_statement(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        request: ExecRequest,
        cancel: &CancelToken,
        preview_limit: Option<usize>,
    ) -> Result<Outcome> {
        // Default `ExecLimits` forbids writing: a mutating request that did not
        // explicitly lift that bound is inconsistent, and inconsistency is
        // settled on the cautious side. It is the last barrier before the
        // driver.
        if request.is_mutating() && request.limits.read_only {
            return Err(OxynError::PolicyDenied {
                reason: "this execution is bounded to read-only: \
                         a write must lift that bound explicitly"
                    .to_owned(),
            });
        }

        let Some(slot) = self.sessions.get(session) else {
            return Err(OxynError::Connection(
                "no session is open under this identifier".to_owned(),
            ));
        };
        if slot.connection() != connection {
            return Err(OxynError::Internal(
                "the target session does not belong to the command's connection".to_owned(),
            ));
        }

        // Captured before `request` is moved into `slot.execute`: it is the
        // only signal kept from the executed text (I-10).
        let intent = request.intent;
        let mutating = request.is_mutating();
        // A **child** token: cancelling this execution does not cancel the tab
        // that started it, whereas cancelling the tab does cancel it.
        let ct = cancel.child();

        let operation = async {
            // Armed before the driver is called: a caller that drops this future
            // at any `.await` below never reaches the cleanup written after it.
            let mut guard = AbandonGuard::new(
                &self.running,
                &self.events,
                id,
                connection,
                (ct.clone(), mutating),
            );
            let run = self
                .run_statement(&slot, id, request, &ct, preview_limit, &mut guard)
                .await;
            // The single exit of every path that reached the driver — success,
            // drain failure, early failure of `slot.execute`, cancellation: a
            // failure or a Stop is precisely what closes a SQLite transaction
            // in silence (ADR-0039 §3). Read with the guard still armed, so a
            // future dropped during this await still announces `Cancelled`;
            // and no `.await` between `settle` and the terminal event.
            self.publish_transaction_state(id, &slot).await;
            guard.settle();
            self.conclude(id, connection, intent, run)
        };
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => result,
            _ = slot.closing_token().cancelled() => { ct.cancel(); operation.await }
        }
    }

    /// Runs a statement up to the release of its cursor.
    ///
    /// `Err` is an early failure of `slot.execute`: nothing was drained, and
    /// nothing is announced — the caller gets the error. Every other end,
    /// failed drain and cancellation included, comes back as [`Drained`] for
    /// [`conclude`](Self::conclude) to announce. The cursor is dropped before
    /// this returns: SQLite keeps its connection busy while one lives, and the
    /// transaction state read next would wait behind it.
    async fn run_statement(
        &self,
        slot: &SessionSlot,
        id: CommandId,
        request: ExecRequest,
        ct: &CancelToken,
        preview_limit: Option<usize>,
        guard: &mut AbandonGuard<'_>,
    ) -> Result<Drained> {
        let connection = slot.connection();
        let limits = request.limits.clone();
        // Probing for the end at the row limit makes the server produce one
        // more batch: harmless for a read, an extra side effect otherwise.
        let confirm_end = preview_limit.is_some() || !request.is_mutating();
        // Armed before the driver is called: `execute` waits for the session,
        // prepares, and — SQLite — computes the whole first batch, an entire
        // aggregate. A clock started at the drain would let that run unbounded.
        let deadline = limits.timeout.and_then(Deadline::from_now);
        let cursor = Self::execute_within(slot, request, ct, deadline).await?;
        let statement = cursor.handle();
        self.running.register(RunningStatement::new(
            statement,
            id,
            connection,
            slot.id(),
            slot.capabilities(),
            ct.clone(),
        ));

        let result = ResultId::new();
        let buffer = Arc::new(ResultBuffer::with_limits(
            BatchSource::schema(&cursor),
            BufferLimits::default()
                .with_memory_budget(self.memory_budget)
                .with_max_rows(preview_limit.or(limits.max_rows)),
        ));
        self.results.write().insert(
            result,
            StoredResult {
                connection,
                buffer: Arc::clone(&buffer),
            },
        );
        guard.track(statement, Arc::clone(&buffer));

        // The schema is known before the first row: the grid draws its columns
        // while the data arrives.
        self.events
            .publish(id, Some(connection), Event::SchemaReady { result });

        let outcome = self
            .drain(
                Coordinates {
                    command: id,
                    connection,
                    result,
                },
                &buffer,
                cursor,
                ct,
                deadline,
                confirm_end,
            )
            .await;

        // An abandonment — timeout or cancellation — must reach the server.
        // A confirming drain only ends unverified when its probe timed out. A
        // write whose outcome is unknown may still be running.
        let interrupted = matches!(outcome, Ok(SinkOutcome::Cancelled))
            || matches!(
                outcome,
                Err(OxynError::Timeout { .. } | OxynError::OutcomeUnknown(_))
            )
            || (confirm_end && matches!(outcome, Ok(SinkOutcome::RowLimitUnverified)));
        if interrupted {
            self.running.cancel(&self.sessions, statement).await;
        }
        Ok(Drained {
            result,
            statement,
            buffer,
            outcome,
        })
    }

    /// Obtains the cursor, within `deadline` if there is one.
    ///
    /// On expiry the driver is interrupted through the execution's token —
    /// `sqlite3_interrupt`, a cancel request — and awaited until it lets go:
    /// dropping its future instead would leave the statement running and the
    /// session busy (DRIVER-CONTRACT §2). A cursor that won the race is
    /// dropped. The error is [`OxynError::Timeout`], ambiguous: a write may
    /// have applied, and nothing here replays it (I-13).
    async fn execute_within(
        slot: &SessionSlot,
        request: ExecRequest,
        ct: &CancelToken,
        deadline: Option<Deadline>,
    ) -> Result<Box<dyn Cursor>> {
        let Some(deadline) = deadline else {
            return slot.execute(request, ct).await;
        };
        let execute = slot.execute(request, ct);
        tokio::pin!(execute);
        tokio::select! {
            biased;
            started = &mut execute => started,
            () = tokio::time::sleep_until(deadline.at) => {
                ct.cancel();
                if let Ok(cursor) = execute.await {
                    drop(cursor);
                }
                Err(OxynError::Timeout { after: deadline.after })
            }
        }
    }

    /// Publishes the session's transaction state, for a session that has one.
    ///
    /// Before the terminal event, which promises nothing follows it for this
    /// execution. The read takes no token from the execution: see
    /// [`SessionSlot::transaction_state`].
    async fn publish_transaction_state(&self, id: CommandId, slot: &SessionSlot) {
        if !slot.capabilities().contains(Capabilities::TRANSACTIONS) {
            return;
        }
        let state = slot.transaction_state().await;
        self.events.publish(
            id,
            Some(slot.connection()),
            Event::TransactionState {
                session: slot.id(),
                state,
            },
        );
    }

    /// Announces the end of an execution, with no `.await`: it runs after
    /// `AbandonGuard::settle`, and an abandonment in between would announce
    /// nothing at all.
    fn conclude(
        &self,
        id: CommandId,
        connection: ConnectionId,
        intent: oxyn_core::StatementIntent,
        run: Result<Drained>,
    ) -> Result<Outcome> {
        let Drained {
            result,
            statement,
            buffer,
            outcome,
        } = run?;
        self.prune_results();

        match outcome {
            Ok(sink) => {
                let stats = buffer.stats();
                if sink != SinkOutcome::Cancelled && intent == oxyn_core::StatementIntent::Ddl {
                    // After a DDL confirmed by the server, not before: a
                    // cancelled sink may have changed nothing.
                    self.invalidate_catalog(connection);
                    self.events
                        .publish(id, Some(connection), Event::CatalogUpdated);
                }
                let event = if sink == SinkOutcome::Cancelled {
                    Event::Cancelled
                } else {
                    Event::Completed {
                        result,
                        stats,
                        intent,
                    }
                };
                self.events.publish(id, Some(connection), event);
                Ok(Outcome::Executed {
                    result,
                    statement,
                    buffer,
                    stats,
                    sink,
                })
            }
            Err(error) => {
                self.events
                    .publish(id, Some(connection), Event::failed(&error));
                Err(error)
            }
        }
    }

    /// Drains a cursor, under back-pressure and until the deadline.
    ///
    /// The deadline is applied **here** and not in `oxyn-data`: it is the
    /// executor that owns the log and the right to issue the server-side
    /// cancellation. A timeout set lower would only abandon the future. It is
    /// the deadline of the whole execution, armed before `execute`: draining
    /// only has what is left of it.
    ///
    /// A read hands back control as soon as it is stopped. A write is read to
    /// its driver's verdict, the only one that knows whether the server
    /// applied it, within what is left of the deadline and at most
    /// [`WRITE_SETTLE_GRACE`] after the Stop; past that, its outcome is
    /// [`OxynError::OutcomeUnknown`], never a cancellation.
    async fn drain(
        &self,
        coords: Coordinates,
        buffer: &Arc<ResultBuffer>,
        cursor: Box<dyn Cursor>,
        ct: &CancelToken,
        deadline: Option<Deadline>,
        confirm_end: bool,
    ) -> Result<SinkOutcome> {
        let sink = BatchSink::new(Arc::clone(buffer));
        // What may not be read past its limit is a write, and a write's stop
        // must not be announced before its driver says how it ended: a Stop
        // shown as harmless invites running it again (I-13).
        let sink = if confirm_end {
            sink.with_end_confirmation()
        } else {
            sink.awaiting_terminal_outcome()
        };
        let mut source = cursor;
        let events = &self.events;

        // Emitting on a channel, yes; drawing, no: this hook runs *inside* the
        // draining loop and delays the next batch.
        let on_batch = |progress: BatchProgress| {
            events.publish(
                coords.command,
                Some(coords.connection),
                Event::BatchReady {
                    result: coords.result,
                    rows: progress.rows,
                },
            );
        };

        // The block ends the borrow the draining future holds on `source`.
        let ending = {
            let drained = sink.drain_with(&mut source, ct, on_batch);
            tokio::select! {
                biased;
                outcome = drained => return outcome,
                () = expiry(deadline) => Ending::Expired,
                // Only a write waits for its driver past a Stop; a read hands
                // back control at once.
                () = unsettled_stop(ct, !confirm_end) => Ending::Unsettled,
            }
        };

        // Read before closing: a buffer already at its row limit was only
        // waiting for the end-of-stream probe.
        let at_limit = buffer.pressure() == Pressure::RowLimit;
        // The draining future was just abandoned, perhaps after consuming
        // bytes from the stream: the cursor is burnt. The buffer is closed so
        // that the interface stops waiting — what was already received stays
        // readable, and marked truncated.
        buffer.mark_truncated();
        buffer.mark_complete(BatchSource::stats(&source));
        match ending {
            // Every row the limit allows arrived in time: the read succeeded,
            // only its completeness is unknown. Not a write's: past its rows,
            // it was still being drained to its commit.
            Ending::Expired if at_limit && confirm_end => Ok(SinkOutcome::RowLimitUnverified),
            Ending::Expired => Err(OxynError::Timeout {
                after: deadline.map(|deadline| deadline.after).unwrap_or_default(),
            }),
            Ending::Unsettled => Err(OxynError::OutcomeUnknown(format!(
                "the write was stopped, and its driver did not say within {} s whether the \
                 server applied it; check the data before running it again",
                WRITE_SETTLE_GRACE.as_secs()
            ))),
        }
    }

    fn check_workspace(&self, workspace: WorkspaceId) -> Result<()> {
        if workspace != self.workspace {
            return Err(OxynError::Config(
                "workspace does not match this executor".into(),
            ));
        }
        Ok(())
    }

    async fn local_worker<T: Send + 'static>(
        &self,
        cancel: &CancelToken,
        task: impl FnOnce(CancelToken) -> oxyn_store::Result<T> + Send + 'static,
    ) -> Result<T> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            OxynError::Config("local library operations require the application runtime".into())
        })?;
        let cancel = cancel.clone();
        runtime
            .spawn_blocking(move || task(cancel))
            .await
            .map_err(|_| OxynError::Internal("local library worker stopped".into()))?
            .map_err(Into::into)
    }

    /// Runs a write meant for the audit trail — the policy journal or the
    /// query history — on the blocking pool, and awaits it before returning.
    ///
    /// Unlike [`local_worker`](Self::local_worker), this never fails for want
    /// of a runtime: the audit trail must not go silent at shutdown, when no
    /// Tokio runtime may be current any more ([`journal_abandoned_off_runtime`](Self::journal_abandoned_off_runtime)
    /// already relied on this fallback before it moved here). With a runtime,
    /// `spawn_blocking` is the first thing this call does — before any other
    /// `.await` in its body — so a caller that submits an operation and
    /// awaits it right away leaves no window where the operation is queued
    /// but not yet running. Without one, `write` runs inline.
    ///
    /// # Errors
    /// [`OxynError::Internal`] if the blocking task was cancelled or
    /// panicked. Once submitted, though, the write belongs to that task:
    /// dropping the future this call returns does not cancel it.
    async fn write_audit<T: Send + 'static>(
        &self,
        write: impl FnOnce(&Store) -> T + Send + 'static,
    ) -> Result<T> {
        let store = Arc::clone(&self.store);
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => runtime
                .spawn_blocking(move || write(&store))
                .await
                .map_err(|_| OxynError::Internal("audit writer stopped".into())),
            Err(_) => Ok(write(&store)),
        }
    }

    async fn open_document(
        &self,
        workspace: WorkspaceId,
        document: DocumentId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let store = self.store.clone();
        let doc = self
            .local_worker(cancel, move |cancel| {
                store.documents().get_cancellable(document, &cancel)
            })
            .await?
            .filter(|doc| doc.workspace == workspace)
            .ok_or_else(|| OxynError::Config("document does not exist in this workspace".into()))?;
        Ok(Outcome::DocumentOpened {
            document: Box::new(doc),
        })
    }

    async fn write_document(
        &self,
        workspace: WorkspaceId,
        document: DocumentId,
        text: &str,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let store = self.store.clone();
        let text = text.to_owned();
        let doc = self
            .local_worker(cancel, move |cancel| {
                let doc = store
                    .documents()
                    .get_cancellable(document, &cancel)?
                    .filter(|doc| doc.workspace == workspace)
                    .ok_or_else(|| oxyn_store::StoreError::Corrupted {
                        field: "documents",
                        detail: "document is not in this workspace".into(),
                    })?;
                let revision = doc
                    .revision
                    .max(doc.saved_revision)
                    .checked_add(1)
                    .ok_or_else(|| oxyn_store::StoreError::Corrupted {
                        field: "documents.revision",
                        detail: "revision exhausted".into(),
                    })?;
                let update = oxyn_core::QueryDocumentUpdate {
                    expected_revision: None,
                    document,
                    revision,
                    title: doc.title,
                    language: doc.language,
                    text,
                    connection: doc.connection,
                    save_named: doc.is_saved,
                    is_open: doc.is_open,
                    // This path rewrites the text of an existing document
                    // without knowing anything of its origin. `None` does not
                    // erase it: the provenance already set stays (ADR-0023).
                    provenance: None,
                };
                let saved = store
                    .documents()
                    .update_query(workspace, &update, &cancel)?;
                if saved.content != update.text || saved.revision != update.revision {
                    return Err(oxyn_store::StoreError::Corrupted {
                        field: "documents.revision",
                        detail: "document changed while writing".into(),
                    });
                }
                Ok(saved)
            })
            .await?;
        Ok(Outcome::DocumentWritten { document: doc.id })
    }

    /// Saves or updates a connection.
    async fn save_connection(
        &self,
        config: &ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        // `Connections::save` **refuses** a parameter carrying a secret name: it
        // is the last point where a password can be stopped before the disk
        // (I-03). Nothing is duplicated here.
        let store = Arc::clone(&self.store);
        let connections = Arc::clone(&self.connections);
        let writes = Arc::clone(&self.connection_writes);
        let workspace = self.workspace;
        let config = config.clone();
        let connection = config.id;
        // The cache is updated by the blocking task itself, under
        // `connection_writes`: it runs to completion even if this future is
        // dropped, and in the same order as the disk writes.
        self.local_worker(cancel, move |_cancel| {
            let _ordered = writes.lock();
            store.connections().save(workspace, &config)?;
            connections.write().insert(config.id, config);
            Ok(())
        })
        .await?;
        Ok(Outcome::ConnectionSaved { connection })
    }

    /// Deletes a connection, after closing what was using it.
    async fn delete_connection(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        self.running
            .cancel_connection(&self.sessions, connection)
            .await;
        for slot in self.sessions.drain_connection(connection) {
            if let Err(err) = slot.close().await {
                tracing::warn!(error = %err, "the server refused a clean session close");
            }
        }
        let store = Arc::clone(&self.store);
        let connections = Arc::clone(&self.connections);
        let writes = Arc::clone(&self.connection_writes);
        let existed = self
            .local_worker(cancel, move |_cancel| {
                let _ordered = writes.lock();
                let existed = store.connections().delete(connection)?;
                connections.write().remove(&connection);
                Ok(existed)
            })
            .await?;
        Ok(Outcome::ConnectionDeleted {
            connection,
            existed,
        })
    }

    // ── Log and history ──────────────────────────────────────────────────────
    //
    // Each write is built in memory here — [`decision_record`],
    // [`Self::history_record`], [`finished_history_record`] — then submitted to
    // the blocking pool by [`Self::write_audit`], called from [`Self::run`] and
    // [`Self::dispatch_as`] (ADR-0035). None of these methods touches the
    // `Store`: they only build the value an owned operation will write later.
    //
    // The history answers "what did I run yesterday?", where the audit log
    // answers "what was authorized, and to whom?". Only executions appear in
    // it: `HistoryRecord::from_command` returns `None` for everything else, and
    // a `Connect` among the queries would make the list unreadable. The log
    // records them all. A history that cannot be written is a nuisance, not a
    // broken promise: the failure is shouted, never propagated.

    /// The history entry of an execution command, connection named.
    ///
    /// The name is copied so that the row stays readable after the connection
    /// is deleted. What does **not** go in: the bound values, which
    /// `ExecRequest` keeps apart from the text, and which contain precisely what
    /// a history read again six months later must not expose (I-03).
    fn history_record(&self, actor: &Actor, command: &Command) -> Option<HistoryRecord> {
        let mut record = HistoryRecord::from_command(actor, command)?;
        record.connection_name = record
            .connection
            .and_then(|id| self.connections.read().get(&id).map(|c| c.name.clone()));
        Some(record)
    }

    // ── What the executor knows about connections ────────────────────────────

    /// Makes a connection known to the executor.
    ///
    /// Serves two purposes: finding the environment to submit to the
    /// `PolicyGate`, and opening the session without reading the local state
    /// again.
    ///
    /// **Does not replace registration with the policy.**
    /// [`PolicyGate`] exposes no registration method — it is a boundary, not a
    /// registry —, so `oxyn-desktop` also calls
    /// [`DefaultPolicy::register`](oxyn_core::DefaultPolicy::register).
    pub fn register_connection(&self, config: &ConnectionConfig) {
        let _ordered = self.connection_writes.lock();
        self.connections.write().insert(config.id, config.clone());
    }

    /// Forgets a connection.
    pub fn forget_connection(&self, connection: ConnectionId) {
        let _ordered = self.connection_writes.lock();
        self.connections.write().remove(&connection);
    }

    /// Loads a workspace's connections into the executor, and returns their
    /// number.
    ///
    /// # Errors
    /// Those of the local state.
    pub fn load_connections(&self) -> Result<usize> {
        let _ordered = self.connection_writes.lock();
        let configs = self.store.connections().list(self.workspace)?;
        let mut guard = self.connections.write();
        for config in &configs {
            guard.insert(config.id, config.clone());
        }
        Ok(configs.len())
    }

    /// A connection's configuration, from the cache or the local state.
    async fn connection_config(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<ConnectionConfig> {
        // Read in its own statement: the guard is released before the `.await`
        // below, never held across it.
        let cached = self.connections.read().get(&connection).cloned();
        if let Some(config) = cached {
            return Ok(config);
        }
        let store = Arc::clone(&self.store);
        let connections = Arc::clone(&self.connections);
        let writes = Arc::clone(&self.connection_writes);
        // Read and cached under `connection_writes`: a delete that lands
        // between the two would otherwise be undone by re-caching its config.
        let found = self
            .local_worker(cancel, move |_cancel| {
                let _ordered = writes.lock();
                let found = store.connections().get(connection)?;
                if let Some(config) = &found {
                    connections.write().insert(config.id, config.clone());
                }
                Ok(found)
            })
            .await?;
        match found {
            Some(config) => Ok(config),
            None => Err(OxynError::Config(
                "this connection does not exist in the workspace".to_owned(),
            )),
        }
    }

    /// The environment to submit to the `PolicyGate`.
    ///
    /// A connection the executor does not know counts as
    /// [`Production`](Environment::Production): it is the same "closed by
    /// default" as the gate itself, and ignoring an unknown marking would amount
    /// to treating production as local.
    ///
    /// Public for the interface, which must know whether the approval it is
    /// asked for concerns production (ADR-0037) by reading the very computation
    /// submitted to the gate.
    #[must_use]
    pub fn environment_of(&self, command: &Command) -> Environment {
        match command {
            // The connection is not registered yet: its own declaration is
            // authoritative, and the gate will cross-check it.
            Command::CreateConnection { config }
            | Command::UpdateConnection { config }
            | Command::TestConnection { config } => config.environment,
            other => other
                .target_connection()
                .and_then(|id| self.connections.read().get(&id).map(|c| c.environment))
                .unwrap_or_default(),
        }
    }

    // ── Access ──────────────────────────────────────────────────────────────

    /// The event channel.
    #[must_use]
    pub const fn events(&self) -> &EventBus {
        &self.events
    }

    /// Opens a subscription to execution events.
    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<crate::ExecEvent> {
        self.events.subscribe()
    }

    /// The commands awaiting approval.
    #[must_use]
    pub const fn approvals(&self) -> &ApprovalRegistry {
        &self.approvals
    }

    /// The executions in progress.
    #[must_use]
    pub const fn running(&self) -> &CancelRegistry {
        &self.running
    }

    /// The open sessions.
    #[must_use]
    pub const fn sessions(&self) -> &SessionRegistry {
        &self.sessions
    }

    /// The local state.
    #[must_use]
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// The current workspace.
    #[must_use]
    pub const fn workspace(&self) -> WorkspaceId {
        self.workspace
    }

    fn result_on_connection(
        &self,
        connection: ConnectionId,
        result: ResultId,
    ) -> Result<Arc<ResultBuffer>> {
        self.result_on(connection, result)?
            .ok_or_else(|| OxynError::Config("result is no longer available".into()))
    }

    /// The buffer of a retained result, for a reader that names its connection.
    ///
    /// The ownership check shared by the commands and the page reads, wherever
    /// the rows are — in memory, in the page cache or on disk. `Ok(None)` when
    /// the result is no longer retained.
    ///
    /// # Errors
    ///
    /// [`OxynError::PolicyDenied`] when the result belongs to another
    /// connection.
    pub fn result_on(
        &self,
        connection: ConnectionId,
        result: ResultId,
    ) -> Result<Option<Arc<ResultBuffer>>> {
        let results = self.results.read();
        let Some(entry) = results.get(&result) else {
            return Ok(None);
        };
        if entry.connection != connection {
            return Err(OxynError::PolicyDenied {
                reason: "result does not belong to this connection".into(),
            });
        }
        Ok(Some(Arc::clone(&entry.buffer)))
    }

    /// Enforces idle retention limits. Call off the UI thread: evictions can remove spill files.
    pub fn prune_results(&self) {
        let evicted = self.results.write().prune();
        drop(evicted);
    }

    /// A result's buffer, as long as it is retained.
    #[must_use]
    pub fn result(&self, result: ResultId) -> Option<Arc<ResultBuffer>> {
        self.results
            .read()
            .get(&result)
            .map(|entry| Arc::clone(&entry.buffer))
    }

    /// Forgets a result: the tab was closed.
    ///
    /// The buffer is only freed when nobody holds it any more — the grid may be
    /// reading it.
    pub fn forget_result(&self, result: ResultId) -> Option<Arc<ResultBuffer>> {
        self.results
            .write()
            .remove(&result)
            .map(|entry| entry.buffer)
    }

    /// Cancels everything and closes every session.
    ///
    /// To be called when the application closes: without it, queries in
    /// progress continue server-side.
    ///
    /// Returns no error — there is nothing more to do about a closing failure
    /// at that point: it is logged at `warn` level. Returns the number of
    /// closed sessions.
    pub async fn shutdown(&self) -> usize {
        // Before closing sessions: a caller usually bounds this call, and a
        // server that never answers the close would cut it before the end —
        // taking with it every outcome already queued.
        self.journal_abandoned_off_runtime().await;
        for (_, catalog) in self.catalogs.write().drain() {
            catalog.closed.cancel();
        }
        let sessions = self.sessions.drain_all();
        for slot in &sessions {
            for entry in self.running.for_session(slot.id()) {
                entry.token().cancel();
            }
            if let Err(err) = slot.close().await {
                tracing::warn!(error = %err, "the server refused a clean session close");
            }
        }
        // Again, last: closing may have abandoned more, and nothing after this
        // point runs the periodic writer.
        self.journal_abandoned_off_runtime().await;
        sessions.len()
    }

    /// [`journal_abandoned`](Self::journal_abandoned) for an async caller.
    ///
    /// The queue is taken here, in memory; the write goes through
    /// [`write_audit`](Self::write_audit). Once taken, the records belong to
    /// the blocking task: dropping this future does not cancel it, so they
    /// are written even if the caller gives up.
    async fn journal_abandoned_off_runtime(&self) {
        let records = self.abandoned.take();
        if records.is_empty() {
            return;
        }
        if self
            .write_audit(move |store| append_outcomes(store, records))
            .await
            .is_err()
        {
            tracing::error!("the audit writer stopped during shutdown");
        }
    }

    /// Writes to the audit journal the outcomes of commands whose caller
    /// dropped them before they ended, and returns how many were written.
    ///
    /// Such a command has its policy decision journaled but never reaches the
    /// line that journals its outcome. Its record is built when its future is
    /// dropped — same constructor as an ordinary outcome, marked
    /// `abandoned by its caller; outcome unknown` and classed
    /// [`Ambiguous`](oxyn_core::ErrorClass::Ambiguous) — and waits in memory
    /// for this call.
    ///
    /// **Blocks on disk I/O**: call it from the blocking pool, next to
    /// [`prune_results`](Self::prune_results), never from the UI thread.
    /// [`shutdown`](Self::shutdown) calls it last.
    ///
    /// # Limits
    /// A queued outcome lives in memory only until this runs. **It is lost if
    /// the process dies before** — within the second that separates two calls
    /// of the periodic writer, or when an application exits without
    /// [`shutdown`](Self::shutdown). At most 256 outcomes wait at once; beyond,
    /// or if the queue is being emptied at the instant of the drop, the outcome
    /// is lost and `tracing::error!` reports the count, never the command.
    pub fn journal_abandoned(&self) -> usize {
        append_outcomes(&self.store, self.abandoned.take())
    }
}

/// Appends queued abandoned outcomes; blocks on disk. Returns how many were written.
fn append_outcomes(store: &Store, records: std::collections::VecDeque<JournalRecord>) -> usize {
    let mut written = 0;
    for record in records {
        match store.journal().append(&record) {
            Ok(_) => written += 1,
            Err(err) => tracing::error!(
                error = %err,
                command = ?record.command_id,
                "failed to journal the outcome of an abandoned command"
            ),
        }
    }
    written
}

/// Builds the audit record for a policy decision, without writing it.
///
/// Pure: no `Store` access, so the record it returns is safe to move into a
/// `spawn_blocking` closure ([`Executor::write_audit`]).
fn decision_record(
    id: CommandId,
    actor: &Actor,
    command: &Command,
    decision: &Decision,
    approved_by: Option<&str>,
) -> JournalRecord {
    let mut record = JournalRecord::new(actor, command, decision).with_command_id(id);
    if let Some(who) = approved_by {
        record = record.approved_by(who);
    }
    record
}

/// Builds the finished entry for [`Executor::history_record`]'s "in
/// progress" row, without writing it. `None` when nothing was started —
/// either the command carries no history entry, or starting it already
/// failed.
///
/// Pure, for the same reason as [`decision_record`].
fn finished_history_record(
    in_progress: Option<(i64, HistoryRecord)>,
    result_outcome: &Result<Outcome>,
    duration: Duration,
) -> Option<(i64, HistoryRecord)> {
    let (id, record) = in_progress?;
    let mut record = match result_outcome {
        // An interrupted draining returns `Ok`: the command did not fail, but
        // it did not return its whole result. Classifying it "succeeded" would
        // make a truncated result read as a complete one.
        Ok(Outcome::Executed {
            stats,
            sink: SinkOutcome::Cancelled,
            ..
        }) => record
            .succeeded(duration, Some(stats.rows))
            .failed(&OxynError::Cancelled),
        Ok(outcome) => record.succeeded(duration, outcome.rows()),
        Err(error) => {
            // A failure has a duration too: "timed out after 30 s" and
            // "rejected in 2 ms" do not describe the same incident.
            let mut failure = record.failed(error);
            failure.duration = Some(duration);
            failure
        }
    };
    if let Ok(Outcome::Executed { result, .. }) = result_outcome {
        record.result = Some(*result);
    }
    Some((id, record))
}

/// The audit record of a command that ran, before its failure is known.
///
/// The one constructor for outcomes — ordinary and abandoned — so that both
/// carry exactly the same fields, and no more.
pub(crate) fn outcome_record(
    id: CommandId,
    actor: &Actor,
    command: &Command,
    duration: Duration,
    rows: Option<u64>,
    approved_by: Option<&str>,
) -> JournalRecord {
    let decision = match approved_by {
        Some(_) => Decision::approval("command executed after explicit approval", None),
        None => Decision::Allow,
    };
    let record = JournalRecord::new(actor, command, &decision)
        .with_command_id(id)
        .completed(duration, rows);
    match approved_by {
        Some(who) => record.approved_by(who),
        None => record,
    }
}

impl fmt::Debug for Executor {
    /// Returns counters, not contents: neither a connection configuration,
    /// nor a query text, nor credentials may land in a trace
    /// (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Executor")
            .field("policy", &self.policy.name())
            .field("sessions", &self.sessions.len())
            .field("running", &self.running.len())
            .field("pending_approvals", &self.approvals.len())
            .field("results", &self.results.read().len())
            .finish_non_exhaustive()
    }
}

/// The one deadline of an execution: [`ExecLimits::timeout`] counted from the
/// moment the driver is first called, not from the first batch.
#[derive(Debug, Clone, Copy)]
struct Deadline {
    /// The limit, as the error reports it.
    after: Duration,
    at: tokio::time::Instant,
}

impl Deadline {
    /// `None` for a limit beyond what the clock can count: unbounded in
    /// practice, and an `Instant` addition that overflows panics.
    fn from_now(after: Duration) -> Option<Self> {
        let at = tokio::time::Instant::now().checked_add(after)?;
        Some(Self { after, at })
    }
}

/// How long a stopped write may take to say how it ended.
///
/// Past a Stop, a write's driver stops the server and waits for its answer:
/// the PostgreSQL cancellation request, the MySQL kill drained for
/// `ER_QUERY_INTERRUPTED` within its own five seconds. The bound sits above
/// those, so that it only fires for a driver that no longer answers — and then
/// the outcome is reported as unknown, never as cancelled (I-13).
const WRITE_SETTLE_GRACE: Duration = Duration::from_secs(10);

/// Why a drain was abandoned before its source ended.
enum Ending {
    /// The execution's deadline elapsed.
    Expired,
    /// A write was stopped and its driver did not conclude within
    /// [`WRITE_SETTLE_GRACE`].
    Unsettled,
}

/// Resolves at `deadline`, never without one.
async fn expiry(deadline: Option<Deadline>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.at).await,
        None => std::future::pending().await,
    }
}

/// Resolves [`WRITE_SETTLE_GRACE`] after `ct` fires, never when `bounded` is
/// false.
async fn unsettled_stop(ct: &CancelToken, bounded: bool) {
    if !bounded {
        return std::future::pending().await;
    }
    ct.cancelled().await;
    tokio::time::sleep(WRITE_SETTLE_GRACE).await;
}

/// What a draining event must name for the interface to know which tab to
/// update.
///
/// Carried together because they are only used together — and because
/// passing them one by one would make the draining loop an eight-parameter
/// function, which is the sign that a type was missed.
#[derive(Debug, Clone, Copy)]
struct Coordinates {
    /// The command at the origin of the execution.
    command: CommandId,
    /// The target connection.
    connection: ConnectionId,
    /// The result being fed.
    result: ResultId,
}

/// An execution that reached its cursor, cursor released, not yet announced.
struct Drained {
    result: ResultId,
    statement: StatementHandle,
    buffer: Arc<ResultBuffer>,
    /// How the drain ended: `Err` for a failure, `Ok(Cancelled)` for a Stop.
    outcome: Result<SinkOutcome>,
}

/// Reclassifies an execution command from its text alone.
///
/// The intent and risk carried by the command come from the caller, and an
/// agent is a caller: they are **replaced**, never cross-checked. The other
/// commands go through unchanged — their intent is a property of their
/// variant, not a declaration.
fn reclassified(command: Command) -> Command {
    match command {
        Command::Execute {
            connection,
            session,
            request,
        } => {
            let classification = oxyn_query::reclassify(&request);
            Command::Execute {
                connection,
                session,
                request: Box::new(classification.qualify(*request)),
            }
        }
        other => other,
    }
}

/// Bounds a read aimed at production to a read-only execution.
///
/// A read passes the gate without confirmation, yet a `SELECT` that calls a
/// `VOLATILE` function — directly or through a view — writes, and no reading
/// of the text can tell. Only the server can refuse it: on production the read
/// leaves bounded to read-only, and the driver opens a read-only transaction
/// for it (I-02). A write keeps the bounds its caller lifted: it was confirmed
/// under the connection's name before reaching here.
///
/// A request made only of transaction verbs is not a read and stays unbounded:
/// it calls no function, and a bound the driver cannot honor inside an open
/// transaction would refuse the `COMMIT` or `ROLLBACK` that ends it (issue
/// #182). Who may send one is the gate's question, and an agent may not.
fn bounded_to(command: Command, env: Environment) -> Command {
    match command {
        Command::Execute {
            connection,
            session,
            mut request,
        } => {
            if env.is_production()
                && !request.is_mutating()
                && !(request.transaction_control
                    && oxyn_query::only_controls_transactions(&request))
            {
                request.limits.read_only = true;
            }
            Command::Execute {
                connection,
                session,
                request,
            }
        }
        other => other,
    }
}

/// The wiring of an [`Executor`].
pub struct ExecutorBuilder {
    drivers: Arc<DriverRegistry>,
    store: Arc<Store>,
    policy: Arc<dyn PolicyGate>,
    credentials: Arc<dyn CredentialResolver>,
    approvals: ApprovalRegistry,
    events: EventBus,
    workspace: WorkspaceId,
    app_session: oxyn_core::AppSessionId,
    memory_budget: usize,
}

impl ExecutorBuilder {
    /// Minimal wiring: a local state and a policy.
    #[must_use]
    pub fn new(store: Arc<Store>, policy: Arc<dyn PolicyGate>) -> Self {
        Self {
            drivers: Arc::new(DriverRegistry::new()),
            store,
            policy,
            credentials: Arc::new(NoCredentials),
            approvals: ApprovalRegistry::new(),
            events: EventBus::new(),
            workspace: WorkspaceId::new(),
            // A launch no row names: its lines read as a finished launch's.
            app_session: oxyn_core::AppSessionId::new(),
            memory_budget: DEFAULT_MEMORY_BUDGET,
        }
    }

    /// The drivers compiled into this binary.
    #[must_use]
    pub fn with_drivers(mut self, drivers: Arc<DriverRegistry>) -> Self {
        self.drivers = drivers;
        self
    }

    /// What resolves credentials — `oxyn-secrets`, in production.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Arc<dyn CredentialResolver>) -> Self {
        self.credentials = credentials;
        self
    }

    /// The workspace in which connections and documents are written.
    #[must_use]
    pub fn with_workspace(mut self, workspace: WorkspaceId) -> Self {
        self.workspace = workspace;
        self
    }

    /// The launch that writes the window layout (ADR-0043).
    #[must_use]
    pub fn with_app_session(mut self, session: oxyn_core::AppSessionId) -> Self {
        self.app_session = session;
        self
    }

    /// The validity duration of an approval request.
    #[must_use]
    pub fn with_approval_ttl(mut self, ttl: Duration) -> Self {
        self.approvals = ApprovalRegistry::with_ttl(ttl);
        self
    }

    /// The memory budget of a result buffer, in bytes.
    #[must_use]
    pub fn with_memory_budget(mut self, bytes: usize) -> Self {
        self.memory_budget = bytes;
        self
    }

    /// The depth of the event channel.
    #[must_use]
    pub fn with_event_capacity(mut self, capacity: usize) -> Self {
        self.events = EventBus::with_capacity(capacity);
        self
    }

    /// Builds the executor.
    #[must_use]
    pub fn build(self) -> Executor {
        Executor {
            drivers: self.drivers,
            store: self.store,
            policy: self.policy,
            credentials: self.credentials,
            sessions: SessionRegistry::new(),
            running: CancelRegistry::new(),
            approvals: self.approvals,
            events: self.events,
            results: RwLock::new(crate::retained::RetainedResults::default()),
            catalogs: RwLock::new(HashMap::new()),
            connections: Arc::new(RwLock::new(HashMap::new())),
            connection_writes: Arc::new(Mutex::new(())),
            workspace: self.workspace,
            app_session: self.app_session,
            memory_budget: self.memory_budget,
            abandoned: AbandonedOutcomes::default(),
            started_at: chrono::Utc::now(),
        }
    }
}

impl fmt::Debug for ExecutorBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutorBuilder")
            .field("policy", &self.policy.name())
            .field("drivers", &self.drivers.len())
            .field("memory_budget", &self.memory_budget)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures::executor::block_on;
    use oxyn_core::{
        AgentId, AgentSessionId, Capabilities, DefaultPolicy, DriverId, ErrorClass, ExecLimits,
        MutationRisk, QueryLanguage, SqlDialect, StatementIntent,
    };
    use oxyn_store::{ActorKind, HistoryStatus, PolicyOutcome};

    /// A complete bench: in-memory local state, default policy, no open
    /// session. No driver is registered — the tests that follow are about what
    /// happens **before** a driver is reached, and that is precisely what
    /// matters.
    struct Harness {
        executor: Executor,
        policy: Arc<DefaultPolicy>,
        store: Arc<Store>,
    }

    impl Harness {
        fn new(connection: &ConnectionConfig) -> Self {
            let store = Arc::new(Store::open_in_memory().expect("in-memory local state"));
            let setup = store
                .workspaces()
                .create("tests")
                .expect("workspace creation");
            store
                .connections()
                .save(setup.id, connection)
                .expect("saving the connection");

            let policy = Arc::new(DefaultPolicy::new());
            policy.register(connection);

            // `policy.clone()` and not `Arc::clone(&policy)`: the function
            // form resolves `T` from the expected type — hence `dyn PolicyGate` —
            // and demands a `&Arc<dyn PolicyGate>` before any coercion. In method
            // syntax, `T` comes from the receiver, and the obtained
            // `Arc<DefaultPolicy>` coerces at assignment.
            let gate: Arc<dyn PolicyGate> = policy.clone();
            let executor = Executor::builder(Arc::clone(&store), gate)
                .with_workspace(setup.id)
                .build();
            executor.register_connection(connection);

            Self {
                executor,
                policy,
                store,
            }
        }
    }

    fn agent() -> Actor {
        Actor::agent(AgentId::new(), AgentSessionId::new())
    }

    fn execution(conn: ConnectionId, text: &str, intent: StatementIntent) -> Command {
        Command::Execute {
            connection: conn,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), text)
                    .with_intent(intent)
                    .with_limits(
                        ExecLimits::default()
                            .writable()
                            .with_timeout(None::<Duration>),
                    ),
            ),
        }
    }

    // ── The two tests that protect the product's promise ────────────────────

    /// **An agent deletes nothing in production.**
    ///
    /// The agent declares a read; the text says otherwise. Reclassification
    /// takes place *before* the gate, so the gate sees an unbounded `DELETE` on
    /// a production connection, and refuses — a refusal, not a stronger
    /// confirmation (I-02, ADR-0004).
    ///
    /// The proof that reclassification did take place is not in the refusal:
    /// it is in the **log**, which recorded the `write` intent, not the one the
    /// agent had declared.
    #[test]
    fn an_agent_cannot_delete_on_a_production_connection() {
        let connection = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let bench = Harness::new(&connection);
        let mut events = bench.executor.subscribe();

        // The agent declares itself read-only.
        let cmd = execution(
            connection.id,
            "DELETE FROM clients WHERE 1=1",
            StatementIntent::Read,
        );

        let outcome = block_on(bench.executor.dispatch(agent(), cmd, &CancelToken::new()))
            .expect("a denial is not a failure");

        let Outcome::Denied { command, reason } = outcome else {
            panic!("an agent must be refused on a production connection: {outcome:?}");
        };
        assert!(
            reason.contains("production"),
            "the reason must name production: {reason}"
        );

        // Nothing awaits approval: it is a refusal, not a confirmation.
        assert!(
            bench.executor.approvals().is_empty(),
            "a denial puts nothing on hold for approval"
        );

        // The log saw the **real** intent, not the declared one.
        let trace = bench
            .store
            .journal()
            .recent(1)
            .expect("reading the log back")
            .pop()
            .expect("one entry");
        assert_eq!(trace.record.command_id, Some(command));
        assert_eq!(trace.record.actor_kind, ActorKind::Agent);
        assert_eq!(trace.record.decision, PolicyOutcome::Denied);
        assert_eq!(
            trace.record.intent,
            StatementIntent::Write,
            "the intent declared by the agent must have been overwritten"
        );
        assert_eq!(trace.record.risk, MutationRisk::UnboundedDelete);
        assert_eq!(
            trace.record.statement.as_deref(),
            Some("DELETE FROM clients WHERE 1=1")
        );

        // And the interface learns it through the channel, not by polling a state.
        let received = events.try_recv().expect("an event was emitted");
        assert_eq!(received.command, command);
        assert!(
            matches!(received.event, Event::Failed { .. }),
            "{received:?}"
        );
    }

    /// **A denied command leaves a trace.**
    ///
    /// A log that records only what worked says nothing about what was
    /// attempted — and that is exactly what an audit looks for.
    #[test]
    fn the_log_holds_an_entry_even_when_the_command_is_denied() {
        // Read-only connection: the refusal applies to a human too.
        let connection = ConnectionConfig::new("replica", DriverId::postgres())
            .with_environment(Environment::Local)
            .read_only();
        let bench = Harness::new(&connection);

        assert_eq!(
            bench.store.journal().count().expect("count"),
            0,
            "the log starts empty"
        );

        let command = execution(
            connection.id,
            "DROP TABLE clients",
            StatementIntent::Unknown,
        );
        let outcome = block_on(
            bench
                .executor
                .dispatch(Actor::Human, command, &CancelToken::new()),
        )
        .expect("a denial is not a failure");
        assert!(outcome.is_denied(), "{outcome:?}");
        assert!(!outcome.took_effect());

        assert_eq!(
            bench.store.journal().count().expect("count"),
            1,
            "a denied command is logged like the others"
        );
        let trace = bench
            .store
            .journal()
            .recent(1)
            .expect("read back")
            .pop()
            .expect("one entry");
        assert_eq!(trace.record.decision, PolicyOutcome::Denied);
        assert_eq!(trace.record.actor_kind, ActorKind::Human);
        assert_eq!(trace.record.command_kind, "Execute");
        assert_eq!(
            trace.record.statement.as_deref(),
            Some("DROP TABLE clients")
        );
        assert!(
            trace
                .record
                .decision_reason
                .as_deref()
                .is_some_and(|pattern| pattern.contains("read-only")),
            "{:?}",
            trace.record.decision_reason
        );

        // And the trace survives what the user can erase.
        bench.store.history().clear().expect("purging the history");
        assert_eq!(bench.store.journal().count().expect("count"), 1);
    }

    // ── The rest of the sequence ────────────────────────────────────────────

    #[test]
    fn an_agent_write_outside_production_waits_for_approval_and_executes_nothing() {
        let conn = ConnectionConfig::new("atelier", DriverId::postgres())
            .with_environment(Environment::Development);
        let bench = Harness::new(&conn);

        let cmd = execution(
            conn.id,
            "UPDATE clients SET active = true WHERE id = 1",
            StatementIntent::Read,
        );
        let outcome = block_on(bench.executor.dispatch(agent(), cmd, &CancelToken::new()))
            .expect("an approval request is not a failure");

        let Outcome::NeedsApproval {
            command, preview, ..
        } = outcome
        else {
            panic!("an agent write must request an approval: {outcome:?}");
        };

        // The preview names the connection, never its identifier.
        let preview = preview.expect("a preview");
        assert_eq!(preview.connection, "atelier");
        assert!(!preview.connection.contains(&conn.id.to_string()));

        // Nothing was executed: no session was even looked for.
        assert_eq!(bench.executor.approvals().len(), 1);
        assert!(bench.executor.running().is_empty());

        // And the trace of the request is already in the log.
        let trace = bench
            .store
            .journal()
            .recent(1)
            .expect("read back")
            .pop()
            .expect("one entry");
        assert_eq!(trace.record.decision, PolicyOutcome::ApprovalRequired);
        assert_eq!(trace.record.command_id, Some(command));
    }

    #[rstest::rstest]
    #[case::agent_production(true, Environment::Production)]
    #[case::agent_development(true, Environment::Development)]
    #[case::human_production(false, Environment::Production)]
    fn copy_server_side_effects_reach_the_policy_as_writes(
        #[case] is_agent: bool,
        #[case] environment: Environment,
    ) {
        for sql in ["COPY t TO PROGRAM 'x'", "COPY t TO '/tmp/x.csv'"] {
            let connection = ConnectionConfig::new("customer database", DriverId::postgres())
                .with_environment(environment);
            let bench = Harness::new(&connection);
            let actor = if is_agent { agent() } else { Actor::Human };
            let command = execution(connection.id, sql, StatementIntent::Read);
            let outcome = block_on(bench.executor.dispatch(actor, command, &CancelToken::new()))
                .expect("the gate decides before looking for a session");
            if is_agent && environment.is_production() {
                assert!(matches!(outcome, Outcome::Denied { .. }), "{outcome:?}");
                assert!(bench.executor.approvals().is_empty());
            } else {
                let Outcome::NeedsApproval {
                    preview, reason, ..
                } = outcome
                else {
                    panic!("server-side COPY requires approval: {outcome:?}");
                };
                assert_eq!(
                    preview.expect("a named connection preview").connection,
                    connection.name
                );
                assert!(reason.contains("database server"), "{reason}");
                assert_eq!(bench.executor.approvals().len(), 1);
            }
            assert!(bench.executor.running().is_empty());
        }
    }

    #[test]
    fn an_unknown_or_stale_approval_executes_nothing() {
        let connection = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let bench = Harness::new(&connection);

        let outcome = block_on(bench.executor.approve(
            "nicolas",
            CommandId::new(),
            &CancelToken::new(),
        ));
        let error = outcome.expect_err("a moot approval is refused");
        assert!(matches!(error, OxynError::PolicyDenied { .. }), "{error:?}");
        assert_eq!(
            bench.store.journal().count().expect("count"),
            0,
            "nothing went through the gate: there is nothing to log"
        );
    }

    #[test]
    fn an_approval_does_not_survive_a_stricter_marking() {
        // The connection is open when the approval is requested, marked
        // read-only when it is given. The late refusal wins over the approval.
        let connection = ConnectionConfig::new("atelier", DriverId::postgres())
            .with_environment(Environment::Development);
        let bench = Harness::new(&connection);

        let cmd = execution(
            connection.id,
            "UPDATE clients SET active = true WHERE id = 1",
            StatementIntent::Write,
        );
        let outcome = block_on(bench.executor.dispatch(agent(), cmd, &CancelToken::new()))
            .expect("approval request");
        let Outcome::NeedsApproval { command, .. } = outcome else {
            panic!("{outcome:?}");
        };

        // Meanwhile, the user marks the connection read-only.
        let strict = connection.clone().read_only();
        bench.policy.register(&strict);
        bench.executor.register_connection(&strict);

        let outcome = block_on(
            bench
                .executor
                .approve("nicolas", command, &CancelToken::new()),
        )
        .expect("a late denial is not a failure");
        assert!(outcome.is_denied(), "{outcome:?}");
    }

    #[test]
    fn a_write_under_read_only_limits_is_refused() {
        // `ExecLimits::default()` is read-only: writing is always an explicit
        // request. It is the last barrier before the driver.
        let conn = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let bench = Harness::new(&conn);

        let command = Command::Execute {
            connection: conn.id,
            session: SessionId::new(),
            request: Box::new(ExecRequest::new(
                QueryLanguage::Sql(SqlDialect::Sqlite),
                "INSERT INTO clients (name) VALUES ('x')",
            )),
        };

        let error = block_on(
            bench
                .executor
                .dispatch(Actor::Human, command, &CancelToken::new()),
        )
        .expect_err("the inconsistency is settled on the cautious side");
        assert!(matches!(error, OxynError::PolicyDenied { .. }), "{error:?}");

        // Two entries: the policy decision, then the outcome.
        assert_eq!(bench.store.journal().count().expect("count"), 2);
    }

    #[test]
    fn a_local_read_asks_nothing_and_fails_for_lack_of_a_session() {
        // The gate allows; execution fails because no session is open. What
        // this test checks is that the failure comes **after** the gate, and
        // that both log entries are written.
        let connection = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let bench = Harness::new(&connection);

        let command = execution(connection.id, "SELECT 1", StatementIntent::Unknown);
        let err = block_on(
            bench
                .executor
                .dispatch(Actor::Human, command, &CancelToken::new()),
        )
        .expect_err("no session is open");
        assert!(matches!(err, OxynError::Connection(_)), "{err:?}");

        let entries = bench.store.journal().recent(2).expect("read back");
        assert_eq!(entries.len(), 2, "decision before, outcome after");
        assert!(
            entries
                .iter()
                .all(|e| e.record.decision == PolicyOutcome::Allowed),
            "the read was indeed authorized"
        );
        assert!(
            entries.iter().any(|e| e.record.error.is_some()),
            "the execution failure is in the log"
        );
    }

    #[test]
    fn a_connection_unknown_to_the_executor_counts_as_production() {
        let connection = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let bench = Harness::new(&connection);
        bench.executor.forget_connection(connection.id);

        let command = execution(connection.id, "SELECT 1", StatementIntent::Read);
        assert_eq!(
            bench.executor.environment_of(&command),
            Environment::Production
        );
    }

    #[test]
    fn the_executor_debug_shows_no_content() {
        let connection = ConnectionConfig::new("base client", DriverId::postgres())
            .with_param("host", "internal.example");
        let bench = Harness::new(&connection);
        let rendered = format!("{:?}", bench.executor);
        assert!(!rendered.contains("internal.example"), "{rendered}");
        assert!(rendered.contains("DefaultPolicy"), "{rendered}");
    }

    #[test]
    fn a_cancellation_is_always_allowed() {
        // Refusing a cancellation protects nothing and leaves a query running.
        let conn = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let bench = Harness::new(&conn);

        let outcome = block_on(bench.executor.dispatch(
            agent(),
            Command::Cancel {
                connection: conn.id,
                statement: StatementHandle::new(),
            },
            &CancelToken::new(),
        ))
        .expect("cancelling is not refused");

        let Outcome::Cancelled { report } = outcome else {
            panic!("{outcome:?}");
        };
        assert!(!report.was_running, "the target execution does not exist");
    }

    // ── The query history ───────────────────────────────────────────────────

    /// A session that returns a single two-row batch, without a server.
    ///
    /// It only serves to prove the history's nominal path: without an
    /// execution that succeeds, the `succeeded` row exists in no test, and it
    /// is exactly the one that was missing until now.
    struct StubSession;

    #[async_trait::async_trait]
    impl oxyn_driver::Session for StubSession {
        fn capabilities(&self) -> Capabilities {
            Capabilities::SQL | Capabilities::TABLES
        }
        async fn execute(&self, _: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
            Ok(Box::new(FakeCursor {
                handle: StatementHandle::new(),
                rendered: false,
                stats: ExecStats::default(),
            }))
        }
        async fn cancel(&self, _: StatementHandle) -> Result<()> {
            Ok(())
        }
        fn catalog(&self) -> &dyn oxyn_catalog::CatalogProvider {
            unreachable!("history tests introspect nothing")
        }
        async fn ping(&self) -> Result<Duration> {
            Ok(Duration::ZERO)
        }
        async fn close(self: Box<Self>) -> Result<()> {
            Ok(())
        }
    }

    struct RecordingSession {
        executed: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl oxyn_driver::Session for RecordingSession {
        fn capabilities(&self) -> Capabilities {
            Capabilities::SQL | Capabilities::TABLES
        }
        async fn execute(&self, request: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
            self.executed.lock().push(request.text);
            Ok(Box::new(FakeCursor {
                handle: StatementHandle::new(),
                rendered: false,
                stats: ExecStats::default(),
            }))
        }
        async fn cancel(&self, _: StatementHandle) -> Result<()> {
            Ok(())
        }
        fn catalog(&self) -> &dyn oxyn_catalog::CatalogProvider {
            unreachable!("password-redaction tests introspect nothing")
        }
        async fn ping(&self) -> Result<Duration> {
            Ok(Duration::ZERO)
        }
        async fn close(self: Box<Self>) -> Result<()> {
            Ok(())
        }
    }

    struct FakeCursor {
        handle: StatementHandle,
        rendered: bool,
        stats: ExecStats,
    }

    fn fake_schema() -> arrow::datatypes::SchemaRef {
        Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int32, false),
        ]))
    }

    #[async_trait::async_trait]
    impl Cursor for FakeCursor {
        fn handle(&self) -> StatementHandle {
            self.handle
        }
        fn schema(&self) -> arrow::datatypes::SchemaRef {
            fake_schema()
        }
        async fn next_batch(&mut self) -> Result<Option<arrow::record_batch::RecordBatch>> {
            if self.rendered {
                return Ok(None);
            }
            self.rendered = true;
            let batch = arrow::record_batch::RecordBatch::try_new(
                fake_schema(),
                vec![Arc::new(arrow::array::Int32Array::from(vec![1, 2]))],
            )
            .expect("the column matches the schema built just above");
            self.stats.record_batch(2, 0);
            Ok(Some(batch))
        }
        fn stats(&self) -> ExecStats {
            self.stats
        }
    }

    /// **An execution that succeeds leaves a complete trace.**
    ///
    /// Duration and rows included: a history that does not say how many rows a
    /// query returned does not answer the question it is asked.
    #[test]
    fn a_successful_execution_is_recorded_with_its_duration_and_rows() {
        let conn = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let bench = Harness::new(&conn);
        let session = bench
            .executor
            .sessions
            .insert(SessionSlot::new(conn.id, Box::new(StubSession)));

        let command = Command::Execute {
            connection: conn.id,
            session: session.id(),
            request: Box::new(
                ExecRequest::new(
                    QueryLanguage::Sql(SqlDialect::Sqlite),
                    "SELECT id FROM clients",
                )
                .with_intent(StatementIntent::Read)
                // No timeout: `block_on` has no tokio clock, and the default
                // timeout would require one.
                .with_limits(ExecLimits::default().with_timeout(None::<Duration>)),
            ),
        };
        block_on(
            bench
                .executor
                .dispatch(Actor::Human, command, &CancelToken::new()),
        )
        .expect("the execution succeeds");

        let entry = bench
            .store
            .history()
            .recent(10)
            .expect("reading the history back")
            .pop()
            .expect("an execution leaves an entry");
        assert_eq!(entry.record.status, HistoryStatus::Succeeded);
        assert_eq!(entry.record.rows, Some(2));
        assert!(entry.record.duration.is_some(), "a duration is measured");
        assert_eq!(entry.record.statement, "SELECT id FROM clients");
        // The name is copied to survive the connection's deletion.
        assert_eq!(entry.record.connection_name.as_deref(), Some("atelier"));
        assert!(entry.record.error.is_none());
    }

    #[test]
    fn password_literals_execute_verbatim_but_are_redacted_from_disk() {
        let cases = [
            (
                DriverId::postgres(),
                SqlDialect::Postgres,
                "ALTER ROLE app PASSWORD 'witness-secret'",
            ),
            (
                DriverId::mysql(),
                SqlDialect::MySql,
                "CREATE USER u IDENTIFIED BY 'witness-secret'",
            ),
        ];

        for (driver, dialect, sql) in cases {
            let connection = ConnectionConfig::new("credential test", driver)
                .with_environment(Environment::Local);
            let bench = Harness::new(&connection);
            let executed = Arc::new(Mutex::new(Vec::new()));
            let session = bench.executor.sessions.insert(SessionSlot::new(
                connection.id,
                Box::new(RecordingSession {
                    executed: Arc::clone(&executed),
                }),
            ));
            let command = Command::Execute {
                connection: connection.id,
                session: session.id(),
                request: Box::new(
                    ExecRequest::new(QueryLanguage::Sql(dialect), sql).with_limits(
                        ExecLimits::default()
                            .writable()
                            .with_timeout(None::<Duration>),
                    ),
                ),
            };

            block_on(
                bench
                    .executor
                    .dispatch(Actor::Human, command, &CancelToken::new()),
            )
            .expect("the server accepts the statement");

            assert_eq!(executed.lock().as_slice(), [sql]);
            let history = bench.store.history().recent(1).expect("history");
            let journal = bench.store.journal().recent(2).expect("journal");
            assert_eq!(history.len(), 1);
            assert_eq!(
                journal.len(),
                2,
                "the decision and outcome are both retained"
            );
            assert_eq!(history[0].record.status, HistoryStatus::Succeeded);
            assert!(!history[0].record.statement.contains("witness-secret"));
            assert!(
                journal.iter().all(|entry| {
                    entry
                        .record
                        .statement
                        .as_deref()
                        .is_none_or(|statement| !statement.contains("witness-secret"))
                }),
                "the audit journal must not retain the password literal"
            );
        }
    }

    /// The `HistoryRecorded` received so far, without waiting.
    fn announced_registrations(
        received_events: &mut tokio::sync::broadcast::Receiver<crate::events::ExecEvent>,
    ) -> Vec<Option<ConnectionId>> {
        std::iter::from_fn(|| received_events.try_recv().ok())
            .filter(|received| received.event == Event::HistoryRecorded)
            .map(|received| received.connection)
            .collect()
    }

    /// **A successful read announces its record, whether it comes from the
    /// human or the agent** (ADR-0022): an open library has no other way to
    /// learn that a row was added. The announcement follows the writing of the
    /// outcome: read again at that moment, the row is no longer "in
    /// progress".
    #[test]
    fn a_successful_read_announces_its_history_record() {
        let conn = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let bench = Harness::new(&conn);
        let session = bench
            .executor
            .sessions
            .insert(SessionSlot::new(conn.id, Box::new(StubSession)));
        let mut events = bench.executor.subscribe();

        for actor in [Actor::Human, agent()] {
            let command = Command::Execute {
                connection: conn.id,
                session: session.id(),
                request: Box::new(
                    ExecRequest::new(
                        QueryLanguage::Sql(SqlDialect::Sqlite),
                        "SELECT id FROM clients",
                    )
                    .with_intent(StatementIntent::Read)
                    .with_limits(ExecLimits::default().with_timeout(None::<Duration>)),
                ),
            };
            block_on(bench.executor.dispatch(actor, command, &CancelToken::new()))
                .expect("the execution succeeds");

            assert_eq!(
                announced_registrations(&mut events),
                vec![Some(conn.id)],
                "one announcement per successful execution, attached to its connection"
            );
            let entry = bench
                .store
                .history()
                .recent(1)
                .expect("read back")
                .pop()
                .expect("an execution leaves an entry");
            assert_eq!(entry.record.status, HistoryStatus::Succeeded);
        }
    }

    /// **A failure announces nothing**: reading the library again on an error
    /// teaches nothing the displayed error does not already say.
    #[test]
    fn a_failure_announces_no_record() {
        let connection = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let bench = Harness::new(&connection);
        let mut events = bench.executor.subscribe();

        let command = execution(connection.id, "SELECT 1", StatementIntent::Read);
        block_on(
            bench
                .executor
                .dispatch(Actor::Human, command, &CancelToken::new()),
        )
        .expect_err("no session under this identifier");

        assert!(announced_registrations(&mut events).is_empty());
    }

    /// **An execution that fails leaves the error, not a silence.**
    ///
    /// With its **family**, which is the data the right to replay depends on:
    /// it is what will forbid offering "rerun" on an `INSERT` whose server-side
    /// effect is unknown (I-13).
    #[test]
    fn a_failure_is_recorded_with_its_error_and_family() {
        let connection = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let bench = Harness::new(&connection);

        // No session is open: execution fails before the driver.
        let command = execution(connection.id, "SELECT 1", StatementIntent::Read);
        block_on(
            bench
                .executor
                .dispatch(Actor::Human, command, &CancelToken::new()),
        )
        .expect_err("no session under this identifier");

        let entry = bench
            .store
            .history()
            .recent(10)
            .expect("read back")
            .pop()
            .expect("a failure leaves an entry");
        assert_eq!(entry.record.status, HistoryStatus::Failed);
        assert!(entry.record.error.is_some(), "the error is kept");
        assert!(entry.record.duration.is_some(), "a failure has a duration");
        // The family is kept as data, never inferred from the message.
        assert_eq!(
            entry.record.error_class,
            Some(ErrorClass::Transient),
            "a missing session is reopened: it is transient"
        );
    }

    /// **A denial appears in the history, with its reason.**
    ///
    /// A history that shows only what worked leaves the user looking for a
    /// query they did run.
    #[test]
    fn a_denial_is_recorded_with_its_reason_and_no_bound_value() {
        let conn = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let bench = Harness::new(&conn);

        // The secret travels as a bound value, never in the text (I-03).
        let command = Command::Execute {
            connection: conn.id,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(
                    QueryLanguage::Sql(SqlDialect::Postgres),
                    "DELETE FROM clients WHERE token = $1",
                )
                .with_intent(StatementIntent::Read)
                .with_params(vec![oxyn_core::ScalarValue::Text(
                    "hunter2-the-secret".to_owned(),
                )])
                .with_limits(ExecLimits::default().writable()),
            ),
        };
        let outcome = block_on(
            bench
                .executor
                .dispatch(agent(), command, &CancelToken::new()),
        )
        .expect("a denial is not a failure");
        assert!(outcome.is_denied(), "{outcome:?}");

        let entries = bench.store.history().recent(10).expect("read back");
        assert_eq!(entries.len(), 1, "one denial, one row — not two");
        let record = &entries[0].record;
        assert_eq!(record.status, HistoryStatus::Denied);
        assert!(
            record
                .error
                .as_deref()
                .is_some_and(|pattern| pattern.contains("production")),
            "{:?}",
            record.error
        );
        // The recorded intent is the one the text carries, not the one the
        // agent declared.
        assert_eq!(record.intent, StatementIntent::Write);
        assert!(
            !format!("{record:?}").contains("hunter2"),
            "no bound value reaches the history (I-03)"
        );
    }

    /// **The history records only executions.**
    ///
    /// A `Connect` or a `Cancel` among the queries would make the list
    /// unreadable; they stay in the log, which records them all.
    #[test]
    fn a_command_that_is_not_an_execution_does_not_touch_history() {
        let conn = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let bench = Harness::new(&conn);

        block_on(bench.executor.dispatch(
            Actor::Human,
            Command::Cancel {
                connection: conn.id,
                statement: StatementHandle::new(),
            },
            &CancelToken::new(),
        ))
        .expect("cancelling is not refused");

        assert_eq!(bench.store.history().count().expect("count"), 0);
        assert!(
            bench.store.journal().count().expect("count") > 0,
            "the log, for its part, records them all"
        );
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
pub(crate) mod catalog_tests;

#[cfg(test)]
#[path = "preview_tests.rs"]
mod preview_tests;

#[cfg(test)]
#[path = "result_page_tests.rs"]
mod result_page_tests;

#[cfg(test)]
#[path = "preference_tests.rs"]
mod preference_tests;

#[cfg(test)]
#[path = "library_tests.rs"]
mod library_tests;

#[cfg(test)]
#[path = "session_close_tests.rs"]
mod session_close_tests;

#[cfg(test)]
#[path = "provider_tests.rs"]
mod provider_tests;

#[cfg(test)]
#[path = "abandon_tests.rs"]
mod abandon_tests;

#[cfg(test)]
#[path = "timeout_tests.rs"]
mod timeout_tests;

#[cfg(test)]
#[path = "transaction_state_tests.rs"]
mod transaction_state_tests;

#[cfg(test)]
#[path = "connection_tests.rs"]
mod connection_tests;

#[cfg(test)]
#[path = "export_tests.rs"]
mod export_tests;

#[cfg(test)]
#[path = "row_limit_tests.rs"]
mod row_limit_tests;

#[cfg(test)]
#[path = "production_read_tests.rs"]
mod production_read_tests;

#[cfg(test)]
#[path = "agent_transaction_tests.rs"]
mod agent_transaction_tests;

#[cfg(test)]
#[path = "stopped_write_tests.rs"]
mod stopped_write_tests;
