//! `Drop…`, `Truncate…` and `Rename…` from the catalog: composed here, shown
//! whole in a review, and submitted like any SQL the user runs
//! ([ADR-0042](../../../../docs/adr/0042-revue-sur-place-des-operations-destructrices.md)).
//!
//! No command variant and no execution path of its own: the text becomes a
//! `Command::Execute` under `Actor::Human`, which `oxyn-query` reclassifies
//! and the gate decides ([I-01](../../../../CLAUDE.md#i-01)). What this module
//! adds is where it runs — a session opened for the review, so a lock waited
//! on freezes neither the catalog nor a console, and no console's transaction
//! can undo it — and who approves it on production: `confirm_held`, the host's
//! dialog, never the webview ([ADR-0037](../../../../docs/adr/0037-dialogue-natif-pour-les-confirmations-critiques.md)).
//!
//! No agent reaches any of it: `oxyn-ai` has no tool for it, and the test
//! `object_operations_are_not_tools` fails the day one appears (I-07).

use std::collections::HashMap;

use oxyn_catalog::model::RelationKind;
use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope, QuoteStyle, quote_identifier};
use oxyn_core::{
    Actor, CancelToken, Capabilities, Command, CommandId, ConnectionId, ErrorClass, ExecRequest,
    MutationRisk, OxynError, QueryLanguage, SessionId, SqlDialect, StatementIntent,
};
use oxyn_exec::Outcome;
use parking_lot::Mutex;

use super::Backend;
use super::confirm::HostAnswer;
use super::metadata::qualified_name;
use crate::ipc::metadata::{Facet, IncomingKeyRow};
use crate::ipc::object_operations::{
    Dependents, ObjectOperation, ObjectOperationOutcome, ObjectOperationReview, OperationKind,
};
use crate::ipc::{CatalogAddress, IpcError};

/// Why an entry is greyed, in the words of ADR-0042.
pub(crate) const HOSTILE_NAME: &str =
    "This name holds control characters: write the statement in a console.";
pub(crate) const NO_DDL: &str = "This connection does not accept schema changes.";
pub(crate) const NO_TRUNCATE: &str = "This database has no TRUNCATE statement.";

/// The review sessions kept open while their command waits for a decision,
/// by the id of that command.
#[derive(Default)]
pub(crate) struct ReviewSessions {
    held: Mutex<HashMap<CommandId, (ConnectionId, SessionId)>>,
}

/// A character that would make the shown name differ from the one executed.
///
/// Escaping it, as `proposal::visible` does for a template nobody runs, would
/// show another object than the one the statement names.
fn hides_text(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
}

fn hostile(name: &str) -> bool {
    name.chars().any(hides_text)
}

/// Why `operation` is not offered on this relation, or `None` when it is.
///
/// By declared capability only, never by product name
/// ([ADR-0003](../../../../docs/adr/0003-driver-capabilities.md)).
pub(crate) fn unavailable(
    operation: OperationKind,
    kind: RelationKind,
    capabilities: Capabilities,
) -> Option<&'static str> {
    let applies = match operation {
        OperationKind::Drop => matches!(
            kind,
            RelationKind::Table | RelationKind::View | RelationKind::MaterializedView
        ),
        OperationKind::Truncate | OperationKind::Rename => kind == RelationKind::Table,
    };
    if !applies {
        return Some("This operation does not apply to this kind of object.");
    }
    if !capabilities.contains(Capabilities::DDL) {
        return Some(NO_DDL);
    }
    if operation == OperationKind::Truncate && !capabilities.contains(Capabilities::TRUNCATE) {
        return Some(NO_TRUNCATE);
    }
    None
}

/// The one statement an operation submits, or why none is composed.
///
/// Every identifier is quoted for the dialect — never [`QuoteStyle::Bare`]
/// ([I-10](../../../../CLAUDE.md#i-10)). No `IF EXISTS`: an object already
/// gone must fail, or a stale catalog would pass for a success. No comment:
/// what is shown is what is sent, and what `query_history` records.
pub(crate) fn compose(
    path: &CatalogPath,
    kind: RelationKind,
    operation: &ObjectOperation,
    dialect: SqlDialect,
    restrict_dependents: bool,
) -> Result<String, String> {
    if path.segments().any(hostile) {
        return Err(HOSTILE_NAME.to_owned());
    }
    let style = QuoteStyle::for_dialect(dialect);
    let relation = qualified_name(path, dialect);
    let cascade = |cascade: bool| -> Result<&'static str, String> {
        match (cascade, restrict_dependents) {
            (false, _) => Ok(""),
            (true, true) => Ok(" CASCADE"),
            (true, false) => Err("CASCADE is not offered on this connection.".to_owned()),
        }
    };
    match operation {
        ObjectOperation::Drop { cascade: with } => {
            let object = match kind {
                RelationKind::Table => "TABLE",
                RelationKind::View => "VIEW",
                RelationKind::MaterializedView => "MATERIALIZED VIEW",
                _ => return Err("Only tables and views are dropped from here.".to_owned()),
            };
            Ok(format!("DROP {object} {relation}{}", cascade(*with)?))
        }
        ObjectOperation::Truncate { cascade: with } => {
            if kind != RelationKind::Table {
                return Err("Only a table is truncated.".to_owned());
            }
            Ok(format!("TRUNCATE TABLE {relation}{}", cascade(*with)?))
        }
        ObjectOperation::Rename { column, new_name } => {
            if kind != RelationKind::Table {
                return Err("Only a table or its columns are renamed from here.".to_owned());
            }
            if hostile(new_name) || column.as_deref().is_some_and(hostile) {
                return Err(HOSTILE_NAME.to_owned());
            }
            if new_name.is_empty() {
                return Err("Type the new name.".to_owned());
            }
            let current = column.as_deref().or(path.relation()).unwrap_or_default();
            if new_name == current {
                return Err("The new name is the current one.".to_owned());
            }
            let new_name = quote_identifier(new_name, style);
            Ok(match column {
                Some(column) => format!(
                    "ALTER TABLE {relation} RENAME COLUMN {} TO {new_name}",
                    quote_identifier(column, style)
                ),
                None => format!("ALTER TABLE {relation} RENAME TO {new_name}"),
            })
        }
    }
}

/// Whether `sql` is one statement of the announced kind.
///
/// Not an authorization — the gate decides — but what keeps the button's label
/// true: a `Rename` button cannot send a `DROP`.
pub(crate) fn check_statement(
    sql: &str,
    dialect: SqlDialect,
    operation: OperationKind,
) -> Result<(), &'static str> {
    let fragments = oxyn_query::split(sql, dialect);
    let [_] = fragments.as_slice() else {
        return Err("A review submits exactly one statement.");
    };
    let classification = oxyn_query::classify(sql, dialect);
    let [statement] = classification.statements.as_slice() else {
        return Err("A review submits exactly one statement.");
    };
    let matches = match operation {
        OperationKind::Drop => statement.risk == MutationRisk::DropObject,
        OperationKind::Truncate => statement.risk == MutationRisk::Truncate,
        OperationKind::Rename => {
            statement.intent == StatementIntent::Ddl && statement.risk == MutationRisk::None
        }
    };
    if matches {
        Ok(())
    } else {
        Err("This statement is not the operation the review announced.")
    }
}

/// What the box may say about the objects that depend on `path`.
fn dependents(
    cache: &CatalogCache,
    path: &CatalogPath,
    kind: RelationKind,
    operation: &ObjectOperation,
    capabilities: Capabilities,
) -> Dependents {
    let destroys = matches!(
        operation,
        ObjectOperation::Drop { .. } | ObjectOperation::Truncate { .. }
    );
    if !destroys || kind != RelationKind::Table {
        return Dependents::NotApplicable;
    }
    if !capabilities.contains(Capabilities::INCOMING_FOREIGN_KEYS) {
        return Dependents::NotReported;
    }
    Dependents::IncomingKeys(Facet {
        freshness: cache
            .freshness(&CatalogScope::IncomingForeignKeys(path.clone()))
            .into(),
        value: cache
            .incoming_foreign_keys(path)
            .map(|keys| keys.iter().map(IncomingKeyRow::from).collect()),
    })
}

/// A submission's end, from what the executor answered.
fn settled(outcome: Result<Outcome, OxynError>) -> ObjectOperationOutcome {
    match outcome {
        Ok(Outcome::Executed { .. }) => ObjectOperationOutcome::Applied,
        Ok(Outcome::Denied { reason, .. }) => ObjectOperationOutcome::Denied { reason },
        // A DDL cancelled once sent may have been committed before the
        // cancellation arrived: the same words as a timeout (I-13).
        Ok(Outcome::Cancelled { .. }) | Err(OxynError::Cancelled) => {
            ObjectOperationOutcome::Ambiguous {
                message: "Stopped after the statement was sent.".to_owned(),
            }
        }
        Ok(_) => ObjectOperationOutcome::Failed {
            message: "The executor answered something other than an execution.".to_owned(),
        },
        Err(error) if error.class() == ErrorClass::Ambiguous => ObjectOperationOutcome::Ambiguous {
            message: error.to_string(),
        },
        Err(error) => ObjectOperationOutcome::Failed {
            message: error.to_string(),
        },
    }
}

fn not_sent(message: impl Into<String>) -> ObjectOperationOutcome {
    ObjectOperationOutcome::NotSent {
        message: message.into(),
    }
}

impl Backend {
    /// The statement an operation would submit, and what the box shows around
    /// it. A read of the configuration and of the catalog cache: no server,
    /// no `Command`.
    ///
    /// `session` is the catalog's session, whose capabilities decide what is
    /// offered; the run checks them again on its own session.
    ///
    /// Blocking — the store and the cache lock: call it from the blocking
    /// pool, never from the main thread (I-05).
    ///
    /// # Errors
    /// The relation is not in the cache, the operation is not offered on it,
    /// or no statement can be composed — the message says why.
    pub fn review_object_operation(
        &self,
        connection: ConnectionId,
        session: SessionId,
        address: &CatalogAddress,
        operation: &ObjectOperation,
    ) -> Result<ObjectOperationReview, IpcError> {
        let config = self.config(connection)?;
        let dialect = oxyn_query::dialect_for(&config.driver);
        let capabilities = self
            .inner
            .executor
            .sessions()
            .get(session)
            // Another connection's session would lend the box capabilities
            // that are not this connection's.
            .filter(|slot| slot.connection() == connection)
            .ok_or_else(|| IpcError::invalid("This session is no longer open"))?
            .capabilities();
        let path = address.to_path()?;
        let cache = self
            .inner
            .executor
            .catalog(connection)
            .ok_or_else(|| IpcError::invalid("This connection has no catalog cache"))?;
        let cache = cache.read();
        let kind = cache
            .relation_summary(&path)
            .map(|summary| summary.kind)
            .ok_or_else(|| {
                IpcError::invalid("This object is no longer in the catalog: refresh it.")
            })?;
        if let Some(reason) = unavailable(operation.kind(), kind, capabilities) {
            return Err(IpcError::invalid(reason));
        }
        let object_name = match operation {
            ObjectOperation::Rename {
                column: Some(column),
                ..
            } => {
                let known = cache
                    .relation(&path)
                    .is_some_and(|relation| relation.fields.iter().any(|f| f.name == *column));
                if !known {
                    return Err(IpcError::invalid(
                        "This column is no longer in the catalog: refresh it.",
                    ));
                }
                column.clone()
            }
            _ => path.relation().unwrap_or_default().to_owned(),
        };
        let restrict_dependents = capabilities.contains(Capabilities::RESTRICT_DEPENDENTS);
        let sql = compose(&path, kind, operation, dialect, restrict_dependents)
            .map_err(IpcError::invalid)?;
        Ok(ObjectOperationReview {
            sql,
            connection_name: config.name.clone(),
            environment: config.environment,
            object_name,
            relation_kind: kind.as_str().to_owned(),
            transactional_ddl: capabilities.contains(Capabilities::TRANSACTIONAL_DDL),
            restrict_dependents,
            dependents: dependents(&cache, &path, kind, operation, capabilities),
        })
    }

    /// Submits a reviewed statement on a session of its own.
    ///
    /// Cancellable under `id` from the session opening to the statement. On a
    /// held command, the host's dialog decides when the approval is critical
    /// (`production`); otherwise the command waits, its session kept, and the
    /// box hands over to `ApprovalDialog`, whose decision goes through
    /// `decide`. The box itself approves nothing.
    ///
    /// # Errors
    /// None of its own: a refusal before sending — the statement is not one
    /// of the announced kind, `id` already runs — is
    /// [`ObjectOperationOutcome::NotSent`], so that an error of the call can
    /// only mean the answer was lost after sending.
    pub async fn run_object_operation(
        &self,
        id: CommandId,
        connection: ConnectionId,
        operation: OperationKind,
        sql: String,
    ) -> Result<ObjectOperationOutcome, IpcError> {
        let inner = &self.inner;
        // Every refusal before sending is an answer, not an error: the front
        // then reads any error of this call as « may have been sent » (I-13).
        let config = match self.read_config(connection).await {
            Ok(config) => config,
            Err(error) => return Ok(not_sent(error.message)),
        };
        let dialect = oxyn_query::dialect_for(&config.driver);
        if let Err(refusal) = check_statement(&sql, dialect, operation) {
            return Ok(not_sent(refusal));
        }
        let cancel = match self.track(id) {
            Ok(cancel) => cancel,
            Err(error) => return Ok(not_sent(error.message)),
        };
        inner.policy.register(&config);

        let session = match self.open_review_session(connection, &cancel).await {
            Ok(session) => session,
            Err(outcome) => return Ok(outcome),
        };
        let capabilities = inner
            .executor
            .sessions()
            .get(session)
            .map(|slot| slot.capabilities())
            .unwrap_or_else(Capabilities::empty);
        let required = match operation {
            OperationKind::Truncate => Capabilities::DDL | Capabilities::TRUNCATE,
            OperationKind::Drop | OperationKind::Rename => Capabilities::DDL,
        };
        if !capabilities.contains(required) {
            self.close_review_session(connection, session).await;
            let reason = if capabilities.contains(Capabilities::DDL) {
                NO_TRUNCATE
            } else {
                NO_DDL
            };
            return Ok(ObjectOperationOutcome::Denied {
                reason: reason.to_owned(),
            });
        }

        let mut request = ExecRequest::new(QueryLanguage::Sql(dialect), sql);
        request.limits.read_only = config.read_only;
        let command = Command::Execute {
            connection,
            session,
            request: Box::new(request),
        };
        // Held until the write ends: the shutdown waits for it (ADR-0021).
        let _local = self.local_write(&command);
        let dispatched = inner
            .executor
            .dispatch_as(id, Actor::Human, command, &cancel)
            .await;
        let (held, reason, preview) = match dispatched {
            Ok(Outcome::NeedsApproval {
                command,
                reason,
                preview,
            }) => (command, reason, preview),
            other => {
                self.close_review_session(connection, session).await;
                return Ok(settled(other));
            }
        };

        // `id` is still tracked: nothing else may take it while the host's
        // dialog is open (`confirm_held`).
        let answer = match self.confirm_held(held).await {
            Ok(answer) => answer,
            // Another critical dialog is open: nothing is consumed, and the
            // approval dialog's `decide` will ask the host again.
            Err(busy) => {
                self.hold_review_session(held, connection, session);
                return Ok(ObjectOperationOutcome::NeedsApproval {
                    command: held.to_string(),
                    reason: busy.message,
                    preview: preview.map(Into::into),
                });
            }
        };
        match answer {
            HostAnswer::NotCritical => {
                self.hold_review_session(held, connection, session);
                Ok(ObjectOperationOutcome::NeedsApproval {
                    command: held.to_string(),
                    reason,
                    preview: preview.map(Into::into),
                })
            }
            HostAnswer::Refused => {
                inner.executor.reject(held);
                self.close_review_session(connection, session).await;
                Ok(ObjectOperationOutcome::Denied {
                    reason: "Not confirmed in the Oxyn dialog: nothing was run".to_owned(),
                })
            }
            HostAnswer::Confirmed => {
                let approved = inner.executor.approve("human", held, &cancel).await;
                self.close_review_session(connection, session).await;
                Ok(settled(approved))
            }
        }
    }

    /// `Command::Connect`, the pair `open_console` uses: autocommit, no
    /// session context, no transaction inherited.
    async fn open_review_session(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<SessionId, ObjectOperationOutcome> {
        let executor = &self.inner.executor;
        let session = match executor
            .dispatch(Actor::Human, Command::Connect { connection }, cancel)
            .await
        {
            Ok(Outcome::Connected { session, .. }) => session,
            Ok(Outcome::NeedsApproval { command, .. }) => {
                executor.reject(command);
                return Err(not_sent(
                    "Open this connection from the connection form to review its policy.",
                ));
            }
            Ok(Outcome::Denied { reason, .. }) => {
                return Err(ObjectOperationOutcome::Denied { reason });
            }
            Ok(_) => return Err(not_sent("The executor did not open a session.")),
            Err(OxynError::Cancelled) => return Err(not_sent("Stopped before anything was sent.")),
            Err(error) => return Err(not_sent(error.to_string())),
        };
        // Stopped while the server answered: the session exists and nobody
        // will use it.
        if cancel.is_cancelled() {
            self.close_review_session(connection, session).await;
            return Err(not_sent("Stopped before anything was sent."));
        }
        Ok(session)
    }

    fn hold_review_session(
        &self,
        command: CommandId,
        connection: ConnectionId,
        session: SessionId,
    ) {
        self.inner
            .object_operations
            .held
            .lock()
            .insert(command, (connection, session));
    }

    /// Closes a review session; a failure leaves it to the connection's close.
    async fn close_review_session(&self, connection: ConnectionId, session: SessionId) {
        if self.close_console(connection, session).await.is_err() {
            tracing::warn!("a review session could not be closed");
        }
    }

    /// Closes the review session a decided command was held on, if any.
    ///
    /// Called once `decide` has approved, rejected or found it expired.
    pub(crate) async fn release_review(&self, command: CommandId) {
        let held = self.inner.object_operations.held.lock().remove(&command);
        if let Some((connection, session)) = held {
            self.close_review_session(connection, session).await;
        }
    }

    /// Closes the review sessions whose command no longer waits: expired, or
    /// resolved by a path other than `decide`. A command being decided is
    /// tracked, and its session is left to `decide`.
    pub(crate) async fn release_stale_reviews(&self) {
        let approvals = self.inner.executor.approvals();
        let stale: Vec<CommandId> = self
            .inner
            .object_operations
            .held
            .lock()
            .keys()
            .copied()
            // In this order: `decide` tracks the id before `approve` takes the
            // pending entry. Read the other way round, a decision starting
            // between the two reads would look finished, and its session
            // would close under the statement it runs.
            .filter(|command| {
                approvals
                    .peek(*command)
                    .is_none_or(|pending| pending.is_expired())
                    && !self.is_tracked(*command)
            })
            .collect();
        for command in stale {
            self.release_review(command).await;
        }
    }
}

#[cfg(test)]
mod tests;
