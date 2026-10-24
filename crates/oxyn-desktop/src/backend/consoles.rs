//! Independent consoles: each owns a session, its runs and its context
//! ([ADR-0015](../../../../docs/adr/0015-consoles-independantes.md)).
//!
//! A console session is opened through the bus beside the one the catalog and
//! previews read through, which no console may move
//! ([ADR-0019](../../../../docs/adr/0019-contexte-de-session.md)). Closing a
//! console closes its session and nothing else: two consoles never share a
//! transaction.

mod capabilities;

use std::sync::Arc;

use oxyn_core::{
    Actor, CancelToken, Command, CommandId, ConnectionId, ExecRequest, OxynError, QueryLanguage,
    SessionId,
};
use oxyn_exec::Outcome;

use super::Backend;
use super::documents::Writers;
use super::exit::{ConsoleTransactions, ExitHold};
use super::recovery::LocalWork;
use crate::ipc::consoles::{
    ConsoleRun, ConsoleSession, ContextOutcome, RunTarget, SessionPlace, bind, refuses_writes,
};
use crate::ipc::{self, CommandOutcome, IpcError};

/// The consoles' share of the backend state: document writers, the local
/// work the shutdown waits for, and the transactions that hold the exit.
pub(crate) struct Workbench {
    pub(crate) writers: Writers,
    pub(crate) local: LocalWork,
    pub(crate) consoles: Arc<ConsoleTransactions>,
    pub(crate) exit: ExitHold,
}

impl Workbench {
    pub(crate) fn new(local: LocalWork, consoles: Arc<ConsoleTransactions>) -> Self {
        Self {
            writers: Writers::default(),
            local,
            consoles,
            exit: ExitHold::default(),
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

/// Why a console cannot run on a session: see
/// [`oxyn_query::console_language`].
pub(crate) const NO_CONSOLE_LANGUAGE: &str = "This session declares no single query language \
     a console can write: Oxyn cannot tell which one to send.";

/// The exact text a run submits, or the sentence saying why nothing is.
///
/// A selection or the whole text goes as one request in any language: the
/// driver reads it, and refuses what it cannot run as one.
fn targeted_text(sql: &str, target: RunTarget, language: QueryLanguage) -> Result<String, String> {
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
            match oxyn_query::current_request(sql, language, cursor) {
                Ok(Some(text)) => Ok(text.to_owned()),
                Ok(None) if language.sql_dialect().is_some() => {
                    Err("No SQL statement at the cursor. Place the cursor in a \
                     statement or select SQL explicitly."
                        .into())
                }
                Ok(None) => Err(format!(
                    "No {language} request at the cursor. Place the cursor in a \
                     request or select the text to run."
                )),
                Err(oxyn_query::QueryError::UnsupportedLanguage { language }) => Err(format!(
                    "Oxyn cannot tell where a {language} request ends. Select the \
                     request to run, or run the whole console."
                )),
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
        let cancel = self.track(id)?;
        self.open_console_under(id, connection, &cancel).await
    }

    /// [`Self::open_console`] under a token the caller tracks: the first
    /// console of a connection opens under its opening's, so that cancelling
    /// the opening — its window closing — reaches it too.
    pub(crate) async fn open_console_under(
        &self,
        id: CommandId,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<ConsoleSession, IpcError> {
        let inner = &self.inner;
        let config = self.read_config(connection).await?;
        inner.policy.register(&config);
        let outcome = inner
            .executor
            .dispatch_as(id, Actor::Human, Command::Connect { connection }, cancel)
            .await;
        let (session, transaction_state) = match outcome {
            Ok(Outcome::Connected {
                session,
                transaction_state,
                ..
            }) => (session, transaction_state),
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
        // Listed at the exit while its transaction is open (ADR-0043).
        inner.workbench.consoles.opened(
            session,
            connection,
            config.name.clone(),
            transaction_state,
        );
        self.console_session(session, config.read_only, transaction_state)
    }

    /// What a console may know about its session.
    ///
    /// The transaction state is the one the executor read while opening it,
    /// through the bus (ADR-0039 §4): the bridge calls no driver itself.
    pub(crate) fn console_session(
        &self,
        session: SessionId,
        read_only: bool,
        transaction_state: oxyn_core::TransactionState,
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
            read_only: refuses_writes(read_only, capabilities),
            transaction_state: transaction_state.into(),
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
            // Only once closed: a session still open keeps holding the exit.
            _ => {
                self.inner.workbench.consoles.closed(session);
                Ok(())
            }
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
        let cancel = self.track(id)?;
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
        // Read before the text is cut: what the cursor targets depends on the
        // language, and the language on what this session declares.
        let session_capabilities = self
            .inner
            .executor
            .sessions()
            .get(session)
            .ok_or_else(|| IpcError::invalid("This console's session is no longer open"))?
            .capabilities();
        let language = oxyn_query::console_language(&config.driver, session_capabilities)
            .ok_or_else(|| IpcError::invalid(NO_CONSOLE_LANGUAGE))?;
        let text = targeted_text(&run.sql, run.target, language).map_err(IpcError::invalid)?;
        let text = match (run.explain, language.sql_dialect()) {
            (true, Some(dialect)) => {
                capabilities::explain_sql(&text, dialect).map_err(IpcError::invalid)?
            }
            (true, None) => {
                return Err(IpcError::invalid(format!(
                    "Explain is only offered for SQL; this console speaks {language}."
                )));
            }
            (false, _) => text,
        };
        if text.trim().is_empty() {
            return Err(IpcError::invalid("Write a query before running it."));
        }
        // The SQL checks read SQL: another language is the driver's to refuse.
        if let Some(dialect) = language.sql_dialect()
            && let Some(message) = capabilities::missing_for(&text, dialect, session_capabilities)
        {
            return Err(IpcError::invalid(message));
        }
        let params = bind(&run.parameters).map_err(|error| IpcError::invalid(error.to_string()))?;
        let mut request = ExecRequest::new(language, text).with_params(params);
        request.limits.read_only = config.read_only;
        // Listed as `Unknown` at the exit until it answers (ADR-0043). Held
        // for approval, it is counted again by `decide`.
        let _running = self.inner.workbench.consoles.run_on(session)?;
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
