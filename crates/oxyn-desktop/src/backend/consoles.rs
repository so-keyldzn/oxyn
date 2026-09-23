//! Independent consoles: each owns a session, its runs and its context
//! ([ADR-0015](../../../../docs/adr/0015-consoles-independantes.md)).
//!
//! A console session is opened through the bus beside the one the catalog and
//! previews read through, which no console may move
//! ([ADR-0019](../../../../docs/adr/0019-contexte-de-session.md)). Closing a
//! console closes its session and nothing else: two consoles never share a
//! transaction.

mod capabilities;

use oxyn_core::{
    Actor, CancelToken, Capabilities, Command, CommandId, ConnectionId, ExecRequest, OxynError,
    QueryLanguage, SessionId,
};
use oxyn_exec::Outcome;

use super::Backend;
use super::documents::Writers;
use super::recovery::LocalWork;
use crate::ipc::consoles::{
    ConsoleRun, ConsoleSession, ContextOutcome, RunTarget, SessionPlace, bind,
};
use crate::ipc::{self, CommandOutcome, IpcError};

/// The consoles' share of the backend state: document writers and the local
/// work the shutdown waits for.
pub(crate) struct Workbench {
    pub(crate) writers: Writers,
    pub(crate) local: LocalWork,
}

impl Workbench {
    pub(crate) fn new(local: LocalWork) -> Self {
        Self {
            writers: Writers::default(),
            local,
        }
    }
}

/// A UTF-16 offset from the editor, as a byte offset into `text`.
///
/// `None` when the offset is beyond the text or inside a surrogate pair: a
/// position that falls on no character selects nothing.
fn byte_offset(text: &str, utf16: usize) -> Option<usize> {
    let mut units = 0_usize;
    for (byte, character) in text.char_indices() {
        if units == utf16 {
            return Some(byte);
        }
        units = units.saturating_add(character.len_utf16());
        if units > utf16 {
            return None;
        }
    }
    (units == utf16).then_some(text.len())
}

/// The exact text a run submits, or the sentence saying why nothing is.
fn targeted_text(
    sql: &str,
    target: RunTarget,
    dialect: oxyn_core::SqlDialect,
) -> Result<String, String> {
    match target {
        RunTarget::All => Ok(sql.to_owned()),
        RunTarget::Selection { start, end } => {
            let (Some(from), Some(to)) = (byte_offset(sql, start), byte_offset(sql, end)) else {
                return Err("The selection no longer matches the text.".into());
            };
            sql.get(from.min(to)..from.max(to))
                .map(str::to_owned)
                .ok_or_else(|| "The selection no longer matches the text.".into())
        }
        RunTarget::Statement { cursor } => {
            let Some(cursor) = byte_offset(sql, cursor) else {
                return Err("The cursor no longer matches the text.".into());
            };
            match oxyn_query::current_statement(sql, dialect, cursor) {
                Ok(Some(fragment)) => Ok(fragment.text.to_owned()),
                Ok(None) => Err("No SQL statement at the cursor. Place the cursor in a \
                     statement or select SQL explicitly."
                    .into()),
                Err(error) => Err(format!(
                    "{error}. Select the exact SQL to run when statement boundaries \
                     cannot be determined."
                )),
            }
        }
    }
}

impl Backend {
    /// Opens an independent session for a new console. Runs no user SQL.
    pub async fn open_console(
        &self,
        id: CommandId,
        connection: ConnectionId,
    ) -> Result<ConsoleSession, IpcError> {
        let inner = &self.inner;
        let cancel = self.track(id);
        let _running = super::Running { inner, id };
        let config = self.read_config(connection).await?;
        inner.policy.register(&config);
        let outcome = inner
            .executor
            .dispatch_as(id, Actor::Human, Command::Connect { connection }, &cancel)
            .await;
        let session = match outcome {
            Ok(Outcome::Connected { session, .. }) => session,
            Ok(Outcome::NeedsApproval { command, .. }) => {
                inner.executor.reject(command);
                return Err(IpcError::invalid(
                    "Open this connection from the connection form to review its policy.",
                ));
            }
            Ok(Outcome::Denied { reason, .. }) => return Err(IpcError::invalid(reason)),
            Ok(_) => return Err(IpcError::invalid("The executor did not open a session")),
            Err(OxynError::Cancelled) => {
                return Err(IpcError::invalid("Opening the console was cancelled."));
            }
            Err(error) => return Err(error.into()),
        };
        // Cancelled while the server answered: the session exists, nobody
        // will use it, and it must not outlive the gesture.
        if cancel.is_cancelled() {
            self.close_console(connection, session).await?;
            return Err(IpcError::invalid("Opening the console was cancelled."));
        }
        self.console_session(session, config.read_only)
    }

    /// What a console may know about its session.
    pub(crate) fn console_session(
        &self,
        session: SessionId,
        read_only: bool,
    ) -> Result<ConsoleSession, IpcError> {
        let capabilities = self
            .inner
            .executor
            .sessions()
            .get(session)
            .ok_or_else(|| IpcError::invalid("The opened session is no longer registered"))?
            .capabilities();
        Ok(ConsoleSession {
            session: session.to_string(),
            capabilities: ipc::capability_names(capabilities),
            read_only: read_only || capabilities.contains(Capabilities::READ_ONLY_SESSION),
        })
    }

    /// Closes one console's session. Siblings and the catalog keep theirs; an
    /// uncommitted transaction on this one is rolled back by the server.
    pub async fn close_console(
        &self,
        connection: ConnectionId,
        session: SessionId,
    ) -> Result<(), IpcError> {
        match self
            .inner
            .executor
            .dispatch(
                Actor::Human,
                Command::CloseSession {
                    connection,
                    session,
                },
                &CancelToken::new(),
            )
            .await?
        {
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            _ => Ok(()),
        }
    }

    /// Declares where one console session resolves unqualified names.
    ///
    /// Answers with what the session reports afterwards; a cancellation reaches
    /// the server and leaves the session where it was.
    pub async fn set_session_context(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        place: SessionPlace,
    ) -> Result<ContextOutcome, IpcError> {
        let inner = &self.inner;
        let cancel = self.track(id);
        let _running = super::Running { inner, id };
        match inner
            .executor
            .dispatch_as(
                id,
                Actor::Human,
                Command::SetSessionContext {
                    connection,
                    session,
                    catalog: place.catalog,
                    namespace: place.namespace,
                },
                &cancel,
            )
            .await
        {
            Ok(Outcome::SessionContextSet {
                session: answered,
                context,
            }) if answered == session => Ok(ContextOutcome::Set {
                place: context.map(|context| SessionPlace {
                    catalog: context.catalog().map(str::to_owned),
                    namespace: context.namespace().map(str::to_owned),
                }),
            }),
            Ok(Outcome::SessionContextSet { .. }) => Err(IpcError::invalid(
                "The executor answered for another session; nothing was applied.",
            )),
            Ok(Outcome::Denied { reason, .. }) => Err(IpcError::invalid(reason)),
            Ok(_) => Err(IpcError::invalid("Unexpected response to a context change")),
            Err(OxynError::Cancelled) => Ok(ContextOutcome::Cancelled),
            Err(error) => Err(error.into()),
        }
    }

    /// The places a console may move to, from the catalog cache only.
    ///
    /// The first is always the server default. Schemas come from the default
    /// catalog: offering another database's would offer a move PostgreSQL
    /// refuses. No query is sent; an unloaded catalog yields the default alone.
    pub fn session_context_choices(
        &self,
        connection: ConnectionId,
    ) -> Result<Vec<SessionPlace>, IpcError> {
        let mut choices = vec![SessionPlace::default()];
        let Some(cache) = self.inner.executor.catalog(connection) else {
            return Ok(choices);
        };
        let cache = cache.read();
        let catalog = cache
            .catalogs()
            .find(|catalog| catalog.is_default)
            .map(|catalog| catalog.name().to_owned());
        choices.extend(
            cache
                .namespaces(catalog.as_deref())
                .map(|namespace| SessionPlace {
                    catalog: catalog.clone(),
                    namespace: Some(namespace.name().to_owned()),
                }),
        );
        Ok(choices)
    }

    /// Runs what the console targets, with its bound values.
    ///
    /// Every refusal before dispatch — no statement at the cursor, a missing
    /// capability, a value that does not convert — is an error naming what to
    /// do. None of them quotes a bound value ([I-03](../../../../CLAUDE.md#i-03)).
    pub async fn run_console(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        run: ConsoleRun,
    ) -> Result<CommandOutcome, IpcError> {
        let config = self.read_config(connection).await?;
        let dialect = oxyn_query::dialect_for(&config.driver);
        let text = targeted_text(&run.sql, run.target, dialect).map_err(IpcError::invalid)?;
        let text = if run.explain {
            capabilities::explain_sql(&text, dialect).map_err(IpcError::invalid)?
        } else {
            text
        };
        if text.trim().is_empty() {
            return Err(IpcError::invalid("Write a query before running it."));
        }
        let session_capabilities = self
            .inner
            .executor
            .sessions()
            .get(session)
            .ok_or_else(|| IpcError::invalid("This console's session is no longer open"))?
            .capabilities();
        if let Some(message) = capabilities::missing_for(&text, dialect, session_capabilities) {
            return Err(IpcError::invalid(message));
        }
        let params = bind(&run.parameters).map_err(|error| IpcError::invalid(error.to_string()))?;
        let mut request = ExecRequest::new(QueryLanguage::Sql(dialect), text).with_params(params);
        request.limits.read_only = config.read_only;
        self.run(
            id,
            Command::Execute {
                connection,
                session,
                request: Box::new(request),
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests;
