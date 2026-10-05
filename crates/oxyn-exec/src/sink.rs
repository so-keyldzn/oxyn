//! The door through which agents reach execution — **the same as the
//! interface's**.
//!
//! The agent runtime (`oxyn-ai`) hands its commands to a `CommandSink`.
//! [`ExecutorSink`] is what this sink plugs into: it calls nothing but
//! [`Executor::dispatch`], the method the interface calls itself. There is
//! therefore no second API "for the AI" to audit separately — that is the
//! whole point of ADR-0004, and it is what makes the promise tenable: an agent
//! can do nothing the user cannot reach, and everything it does appears in the
//! same log.
//!
//! # Why this module does not literally implement `CommandSink`
//!
//! `oxyn-ai` is **not** in `oxyn-exec`'s dependency contract (see its
//! `Cargo.toml`), and the dependency contract is a settled architecture
//! choice. The trait therefore cannot be implemented here. This module provides
//! the complete execution and a [`DispatchReport`] shaped like
//! `DispatchOutcome`, so that the wiring in `oxyn-desktop` — which depends on
//! both — is a translation without logic:
//!
//! ```ignore
//! #[async_trait]
//! impl CommandSink for MySink {
//!     async fn dispatch(
//!         &self,
//!         _call: &CallHandle,
//!         actor: Actor,
//!         command: Command,
//!         cancel: &CancelToken,
//!     ) -> DispatchOutcome
//!     {
//!         match self.0.dispatch(actor, command, cancel).await {
//!             DispatchReport::Completed { stats: Some(stats), .. } => {
//!                 DispatchOutcome::completed(&stats)
//!             }
//!             DispatchReport::Completed { summary, .. } => {
//!                 DispatchOutcome::Completed { summary }
//!             }
//!             DispatchReport::AwaitingApproval { reason, .. } => {
//!                 DispatchOutcome::AwaitingApproval { reason }
//!             }
//!             DispatchReport::Denied { reason, .. } => DispatchOutcome::Denied { reason },
//!             DispatchReport::Failed { class, error, .. } => {
//!                 DispatchOutcome::Failed { class, message: error }
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! The sink returns **facts**, never an already filtered text: it mentions
//! neither `ToolOutcome` nor the privacy tier. It is the AI runtime that
//! applies the latter, because it belongs to the connection and only the
//! runtime knows it ([I-04](../../../CLAUDE.md#i-04)). A sink that filtered on
//! its side would be a second place to forget it.
//!
//! The four rules of the `CommandSink` contract are held by
//! [`Executor::dispatch`] itself: reclassification before decision, `Actor`
//! passed without modification, logging including denials, cancellation
//! propagated down to the server.
//!
//! # What this sink adds
//!
//! One thing only, and it is not cosmetic:
//! [`ExecutorSink::for_agent`] **refuses** any command whose actor is not the
//! agent the sink is attached to. An agent sink that let `Actor::Human`
//! through would offer an agent the means to impersonate the user, and
//! therefore to escape the whole "agent" half of the policy matrix.

use std::fmt;
use std::sync::Arc;

use oxyn_core::{
    Actor, AgentId, AgentSessionId, CancelToken, Command, CommandId, ErrorClass, ExecStats,
    OxynError, ResultId,
};
use oxyn_data::SinkOutcome;

use crate::executor::{Executor, Outcome};

/// What to tell the model about a command it requested.
///
/// Deliberately poor: the model learns what happened, never the rows. Results
/// live as `RecordBatch`es in the buffer and are shown to the **user**;
/// passing them through the conversation would send them to the provider,
/// which the connection's privacy tier does not necessarily allow (I-04).
///
/// There is **no** error variant that interrupts the conversation: an
/// execution failure is an answer to give the model, not an incident.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DispatchReport {
    /// The command produced an effect.
    Completed {
        /// The command, to correlate with the log.
        command: CommandId,
        /// What to say about it, **in English**: it is a prompt, not an
        /// interface message.
        summary: String,
        /// The measurements, when the command produces some — an execution
        /// produces some, opening a document does not.
        stats: Option<ExecStats>,
        /// The result retained by the executor, when the command produces one:
        /// it is what lets the rows be shown **to the user**, through the same
        /// path as a console's grid.
        ///
        /// It tells the model nothing and must never reach it: the wiring to
        /// `DispatchOutcome` ignores it, on purpose
        /// ([I-04](../../../CLAUDE.md#i-04)).
        result: Option<ResultId>,
    },

    /// The local catalog was read: the cache handle, not a rendering.
    ///
    /// What the model learns from it is decided by `oxyn-ai`, under the
    /// connection's tier ([I-04](../../../CLAUDE.md#i-04)): this report carries
    /// the facts, it renders nothing.
    CatalogRead {
        /// The command.
        command: CommandId,
        /// The connection's cache.
        catalog: oxyn_catalog::CatalogHandle,
    },

    /// **Nothing was executed.** The user was asked and has not answered
    /// yet.
    AwaitingApproval {
        /// The command set aside; it is under this identifier that the approval
        /// will be given.
        command: CommandId,
        /// The reason, as the `PolicyGate` wrote it.
        reason: String,
    },

    /// **Nothing was executed**, and no confirmation will unblock the
    /// command.
    Denied {
        /// The denied command.
        command: CommandId,
        /// The reason, as the `PolicyGate` wrote it.
        reason: String,
    },

    /// The execution failed.
    Failed {
        /// The command.
        command: CommandId,
        /// The error's family, carried as **data**.
        ///
        /// Without it, a caller would have to guess it again from the message —
        /// which the driver contract explicitly forbids, because a message
        /// changes and a caller that parsed it breaks silently
        /// ([DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md)).
        class: ErrorClass,
        /// The message, as it will be shown **to the user**: the server's,
        /// code included.
        ///
        /// What an agent sees of it is another question, and it is not decided
        /// here: the privacy tier belongs to the connection, and it is the AI
        /// runtime that applies it ([I-04](../../../CLAUDE.md#i-04)). This
        /// report carries the facts; it does not filter.
        error: String,
    },
}

impl DispatchReport {
    /// Did the command really produce an effect?
    ///
    /// `false` for a pending approval. The trap it closes: a model that assumes
    /// its `INSERT` took place and builds on that assumption.
    #[must_use]
    pub const fn took_effect(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::CatalogRead { .. })
    }

    /// The command that produced this report.
    #[must_use]
    pub const fn command(&self) -> CommandId {
        match self {
            Self::Completed { command, .. }
            | Self::CatalogRead { command, .. }
            | Self::AwaitingApproval { command, .. }
            | Self::Denied { command, .. }
            | Self::Failed { command, .. } => *command,
        }
    }

    /// The report of a command the user approved: what
    /// [`Executor::approve`] returned, said as [`ExecutorSink::dispatch`]
    /// would have said it had it gone without waiting.
    ///
    /// For the agent call that was waiting for this decision: it returns to the
    /// model what the command really did, in the same form as any other. A
    /// second translation, written for this case alone, would sooner or later
    /// tell the model something else.
    #[must_use]
    pub fn decided(command: CommandId, decided: &oxyn_core::Result<Outcome>) -> Self {
        match decided {
            Ok(outcome) => Self::from_outcome(command, outcome),
            Err(error) => Self::from_error(command, error),
        }
    }

    /// Translates an execution outcome.
    #[must_use]
    fn from_outcome(command: CommandId, outcome: &Outcome) -> Self {
        match outcome {
            Outcome::Executed {
                result,
                stats,
                sink,
                ..
            } => {
                let (result, stats) = (*result, *stats);
                let mut summary = format!("{} rows, {} batches", stats.rows, stats.batches);
                if *sink == SinkOutcome::RowLimitUnverified {
                    // Not proven whole, not proven cut either: claiming
                    // missing rows would be as wrong as claiming none.
                    summary.push_str(" (row limit reached; completeness not verified)");
                } else if stats.truncated || sink.is_truncated() {
                    // Tells the model explicitly that this is not everything: a
                    // truncated result that looks complete leads to wrong
                    // conclusions about real data.
                    summary.push_str(" (truncated; this is not the whole result)");
                }
                Self::Completed {
                    command,
                    summary,
                    stats: Some(stats),
                    result: Some(result),
                }
            }

            Outcome::CatalogDescribed { catalog, .. } => Self::CatalogRead {
                command,
                catalog: catalog.clone(),
            },

            Outcome::NeedsApproval {
                command, reason, ..
            } => Self::AwaitingApproval {
                command: *command,
                reason: reason.clone(),
            },

            Outcome::Denied { command, reason } => Self::Denied {
                command: *command,
                reason: reason.clone(),
            },

            other => Self::Completed {
                command,
                summary: summarize(other),
                stats: None,
                result: None,
            },
        }
    }

    /// Translates a failure.
    #[must_use]
    fn from_error(command: CommandId, error: &OxynError) -> Self {
        Self::Failed {
            command,
            class: error.class(),
            error: error.to_string(),
        }
    }
}

/// Summarizes an outcome without measurements, **in English**: it is a prompt.
fn summarize(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Connected { .. } => "session opened".to_owned(),
        Outcome::CatalogRefreshed { .. } => "catalog refreshed".to_owned(),
        Outcome::Disconnected { closed, .. } => format!("{closed} session(s) closed"),
        Outcome::Cancelled { report } => {
            if report.was_running {
                "cancellation requested".to_owned()
            } else {
                "nothing was running under that handle".to_owned()
            }
        }
        Outcome::Exported { rows, .. } => format!("{rows} rows written"),
        Outcome::DocumentOpened { .. } => "document read".to_owned(),
        Outcome::DocumentWritten { .. } => "document written".to_owned(),
        Outcome::ConnectionSaved { .. } => "connection saved".to_owned(),
        Outcome::ConnectionDeleted { existed, .. } => {
            if *existed {
                "connection deleted".to_owned()
            } else {
                "no such connection".to_owned()
            }
        }
        // The three other variants are handled before getting here; the
        // generic rendering avoids an `unreachable!` on a path that adding a
        // variant could make reachable.
        _ => "done".to_owned(),
    }
}

/// An agent's command sink.
///
/// Does **nothing** but call [`Executor::dispatch`] and translate the outcome.
/// It is deliberately an object without logic: any rule living here would be
/// a rule the interface does not apply.
pub struct ExecutorSink {
    executor: Arc<Executor>,
    /// The agent this sink is attached to, if it is. A command coming from
    /// another actor is then refused.
    bound: Option<(AgentId, AgentSessionId)>,
}

impl ExecutorSink {
    /// Open sink: the received actor is passed as is.
    ///
    /// To be kept for general wiring — a sink attached to a specific agent
    /// ([`for_agent`](Self::for_agent)) refuses impersonation, this one does not.
    #[must_use]
    pub fn new(executor: Arc<Executor>) -> Self {
        Self {
            executor,
            bound: None,
        }
    }

    /// Sink attached to an agent and its conversation.
    ///
    /// Any command carrying another actor — `Actor::Human` included — is
    /// **refused without reaching the executor**. It is the barrier that keeps
    /// an agent from presenting itself as the user, and therefore from escaping
    /// the "agent" half of the policy matrix.
    #[must_use]
    pub fn for_agent(executor: Arc<Executor>, agent: AgentId, session: AgentSessionId) -> Self {
        Self {
            executor,
            bound: Some((agent, session)),
        }
    }

    /// The executor behind this sink.
    #[must_use]
    pub fn executor(&self) -> &Arc<Executor> {
        &self.executor
    }

    /// Submits a command and returns what to tell the model about it.
    ///
    /// Returns no `Result`: an execution failure **is** an answer
    /// ([`DispatchReport::Failed`]), not an incident that interrupts the
    /// conversation. What interrupts it comes from the provider, not the database.
    pub async fn dispatch(
        &self,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> DispatchReport {
        // The identifier is minted here so that the report carries it, including
        // when nothing reaches the executor: it is the correlation key with the
        // log.
        let id = CommandId::new();
        if let Some(refusal) = self.reject_impersonation(id, &actor) {
            return refusal;
        }

        match self.executor.dispatch_as(id, actor, command, cancel).await {
            Ok(outcome) => DispatchReport::from_outcome(id, &outcome),
            Err(error) => DispatchReport::from_error(id, &error),
        }
    }

    /// Refuses a command whose actor is not the sink's.
    fn reject_impersonation(&self, id: CommandId, actor: &Actor) -> Option<DispatchReport> {
        let (agent, session) = self.bound?;
        let expected = Actor::agent(agent, session);
        if *actor == expected {
            return None;
        }
        Some(DispatchReport::Denied {
            command: id,
            reason: "this actor is not the one bound to the conversation: \
                     an agent command cannot be presented as coming from the user"
                .to_owned(),
        })
    }
}

impl fmt::Debug for ExecutorSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutorSink")
            .field("bound_to_agent", &self.bound.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::executor::block_on;
    use oxyn_core::{
        ConnectionConfig, ConnectionId, DefaultPolicy, DriverId, Environment, ExecLimits,
        ExecRequest, PolicyGate, QueryLanguage, SessionId, SqlDialect, StatementIntent,
    };
    use oxyn_store::Store;

    use super::*;

    fn harness(connection: &ConnectionConfig) -> Arc<Executor> {
        let store = Arc::new(Store::open_in_memory().expect("in-memory local state"));
        let setup = store.workspaces().create("tests").expect("workspace");
        store
            .connections()
            .save(setup.id, connection)
            .expect("connection");

        let policy = Arc::new(DefaultPolicy::new());
        policy.register(connection);
        let policy: Arc<dyn PolicyGate> = policy;

        let exec = Executor::builder(store, policy)
            .with_workspace(setup.id)
            .build();
        exec.register_connection(connection);
        Arc::new(exec)
    }

    fn execution(conn: ConnectionId, text: &str) -> Command {
        Command::Execute {
            connection: conn,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), text).with_limits(
                    ExecLimits::default()
                        .writable()
                        .with_timeout(None::<Duration>),
                ),
            ),
        }
    }

    #[test]
    fn an_agent_cannot_impersonate_the_user() {
        let connection = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let sink =
            ExecutorSink::for_agent(harness(&connection), AgentId::new(), AgentSessionId::new());

        let report = block_on(sink.dispatch(
            Actor::Human,
            execution(connection.id, "DELETE FROM clients"),
            &CancelToken::new(),
        ));

        assert!(
            matches!(report, DispatchReport::Denied { .. }),
            "{report:?}"
        );
        assert!(!report.took_effect());
        // And nothing reached the executor: the log is empty.
        assert_eq!(sink.executor().store().journal().count().expect("count"), 0);
    }

    #[test]
    fn the_sink_goes_through_the_same_dispatch_as_the_interface() {
        // The proof: an agent asking for a DELETE in production receives a
        // denial written by the `PolicyGate`, and the log keeps its trace —
        // exactly as if the interface had submitted the command.
        let connection = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let exec = harness(&connection);
        let agent = AgentId::new();
        let session = AgentSessionId::new();
        let sink = ExecutorSink::for_agent(Arc::clone(&exec), agent, session);

        let report = block_on(sink.dispatch(
            Actor::agent(agent, session),
            execution(connection.id, "DELETE FROM clients"),
            &CancelToken::new(),
        ));

        let DispatchReport::Denied { reason, .. } = &report else {
            panic!("{report:?}");
        };
        assert!(reason.contains("production"), "{reason}");
        assert_eq!(exec.store().journal().count().expect("count"), 1);

        let trace = exec
            .store()
            .journal()
            .for_agent(agent, 1)
            .expect("read back")
            .pop()
            .expect("an entry attributed to the agent");
        assert_eq!(trace.record.intent, StatementIntent::Write);
    }

    #[test]
    fn an_agent_write_outside_production_is_returned_as_pending() {
        let connection = ConnectionConfig::new("atelier", DriverId::postgres())
            .with_environment(Environment::Development);
        let agent = AgentId::new();
        let session = AgentSessionId::new();
        let sink = ExecutorSink::for_agent(harness(&connection), agent, session);

        let report = block_on(sink.dispatch(
            Actor::agent(agent, session),
            execution(
                connection.id,
                "UPDATE clients SET active = true WHERE id = 1",
            ),
            &CancelToken::new(),
        ));

        let DispatchReport::AwaitingApproval { command, .. } = report else {
            panic!("{report:?}");
        };
        // The returned identifier is the one under which the approval will be given.
        assert!(
            sink.executor()
                .approvals()
                .pending()
                .iter()
                .any(|p| p.id == command)
        );
    }

    #[test]
    fn a_failure_is_an_answer_to_the_model_not_an_incident() {
        // No session is open: execution fails after the gate.
        let connection = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let agent = AgentId::new();
        let session = AgentSessionId::new();
        let sink = ExecutorSink::for_agent(harness(&connection), agent, session);

        let report = block_on(sink.dispatch(
            Actor::agent(agent, session),
            execution(connection.id, "SELECT 1"),
            &CancelToken::new(),
        ));

        let DispatchReport::Failed { class, .. } = report else {
            panic!("{report:?}");
        };
        assert!(
            class.is_retryable(),
            "a closed session is reopened: {class:?}"
        );
    }

    #[test]
    fn an_open_sink_passes_the_actor_as_is() {
        let connection = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let sink = ExecutorSink::new(harness(&connection));

        let report = block_on(sink.dispatch(
            Actor::Human,
            execution(connection.id, "SELECT 1"),
            &CancelToken::new(),
        ));
        // The gate allowed it (read, local connection): the failure comes from
        // the absence of a session, not from a denial.
        assert!(
            matches!(report, DispatchReport::Failed { .. }),
            "{report:?}"
        );
    }
}
