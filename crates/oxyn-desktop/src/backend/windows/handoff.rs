//! `Open in new window`: a tab moves to a window built for it
//! ([ADR-0043](../../../../../docs/adr/0043-multi-fenetre.md), « Déplacer une
//! console »).
//!
//! A console moves with its session, neither closed nor reopened: its open
//! transaction, its context and the result it shows follow it, and nothing
//! runs again. The new window opens its own catalog session. An object tab
//! moves as a place: the new window opens the connection and reads it like a
//! selection.
//!
//! The move is prepared before the window exists, and the window adopts it
//! once, when its webview asks: a webview that loads before the move is ready
//! must not find nothing.

use std::collections::HashMap;

use oxyn_core::{CancelToken, ConnectionId, DocumentId, ResultId, SessionId};
use parking_lot::Mutex;

use super::WindowKey;
use crate::backend::Backend;
use crate::ipc::IpcError;
use crate::ipc::windows::{ConsoleHandoff, HandedResult, HandoffRequest};

/// The moves prepared for windows whose webview has not adopted them yet.
#[derive(Default)]
pub(crate) struct Handoffs(Mutex<HashMap<WindowKey, ConsoleHandoff>>);

impl Handoffs {
    fn put(&self, window: WindowKey, handoff: ConsoleHandoff) {
        self.0.lock().insert(window, handoff);
    }

    /// The move prepared for `window`, once. Dropped with its bound values
    /// when the window closes first (I-03).
    pub(crate) fn take(&self, window: WindowKey) -> Option<ConsoleHandoff> {
        self.0.lock().remove(&window)
    }
}

/// A console move, its identifiers read.
struct ConsoleMove {
    connection: ConnectionId,
    session: SessionId,
    document: DocumentId,
    result: Option<(ResultId, HandedResult)>,
}

fn parse<T: std::str::FromStr>(what: &str, text: &str) -> Result<T, IpcError> {
    text.parse()
        .map_err(|_| IpcError::invalid(format!("invalid {what} identifier")))
}

impl Backend {
    /// Checks that `source` may move what `request` names, before any window
    /// is built: its own console, idle, or a connection it holds.
    ///
    /// # Errors
    /// What another window owns, a console whose statement still runs, or a
    /// connection this window does not hold.
    pub(crate) async fn check_handoff(
        &self,
        source: WindowKey,
        request: &HandoffRequest,
    ) -> Result<(), IpcError> {
        match request {
            HandoffRequest::Console {
                connection,
                session,
                document,
                result,
                ..
            } => {
                let moved = console_move(connection, session, document, result.as_ref())?;
                self.check_console_move(source, &moved).await
            }
            HandoffRequest::Object { connection, .. } => {
                let connection: ConnectionId = parse("connection", connection)?;
                if self.inner.windows.holds(source, connection) {
                    Ok(())
                } else {
                    Err(IpcError::invalid(
                        "This connection is not open in this window",
                    ))
                }
            }
        }
    }

    async fn check_console_move(
        &self,
        source: WindowKey,
        moved: &ConsoleMove,
    ) -> Result<(), IpcError> {
        let windows = &self.inner.windows;
        windows.check_session(source, moved.session)?;
        windows.check_document(source, moved.document)?;
        if let Some((result, _)) = &moved.result {
            windows.check_result(source, *result)?;
        }
        // The outcome of a statement comes back to the webview that sent it:
        // moved in flight, it would reach a window that no longer holds the
        // console (ADR-0043).
        match self.inner.workbench.consoles.observed(moved.session).await {
            Some((_, false)) => Ok(()),
            Some((_, true)) => Err(IpcError::invalid(
                "Wait for the statement to finish before moving the console.",
            )),
            None => Err(IpcError::invalid(
                "This console has no session to move. Attach it to a connection first.",
            )),
        }
    }

    /// Moves what `request` names from `source` to `target`, a window reserved
    /// and not built yet, and prepares what `target` adopts.
    ///
    /// # Errors
    /// The checks of [`Self::check_handoff`], run again; the connection's
    /// configuration unreadable; the new catalog session refused.
    pub(crate) async fn hand_off(
        &self,
        source: WindowKey,
        target: WindowKey,
        request: HandoffRequest,
    ) -> Result<(), IpcError> {
        match request {
            HandoffRequest::Console {
                connection,
                session,
                document,
                result,
                parameters,
            } => {
                let moved = console_move(&connection, &session, &document, result.as_ref())?;
                self.check_console_move(source, &moved).await?;
                let config = self.read_config(moved.connection).await?;
                let catalog = self.catalog_session(&config, &CancelToken::new()).await?;
                let windows = &self.inner.windows;
                windows.claim_session(target, moved.connection, catalog);
                windows.hand_over(
                    source,
                    target,
                    moved.connection,
                    moved.session,
                    moved.document,
                )?;
                // The target reads the result before the source lets its view
                // go: in between, retention never finds it without a reader
                // (ADR-0017).
                if let Some((id, _)) = &moved.result
                    && let Some(buffer) = self.inner.executor.result(*id)
                {
                    self.add_reader(*id, &buffer, true);
                    windows.claim_result(target, *id, true);
                }
                let state = self
                    .inner
                    .workbench
                    .consoles
                    .observed(moved.session)
                    .await
                    .map_or(oxyn_core::TransactionState::Unknown, |(state, _)| state);
                let console = self.console_session(moved.session, config.read_only, state)?;
                let open = self.workspace_of(&config, catalog, console)?;
                self.report_window_consoles(target, vec![moved.document], Some(moved.document))
                    .await?;
                self.inner.handoffs.put(
                    target,
                    ConsoleHandoff::Console {
                        open,
                        document: moved.document.to_string(),
                        result: moved.result.map(|(_, handed)| handed),
                        parameters,
                    },
                );
                Ok(())
            }
            HandoffRequest::Object { connection, place } => {
                let connection: ConnectionId = parse("connection", &connection)?;
                if !self.inner.windows.holds(source, connection) {
                    return Err(IpcError::invalid(
                        "This connection is not open in this window",
                    ));
                }
                let open = self
                    .reconnect(oxyn_core::CommandId::new(), connection)
                    .await?;
                let windows = &self.inner.windows;
                windows.hold_connection(target, connection);
                for session in [&open.session, &open.console.session] {
                    if let Ok(session) = session.parse::<SessionId>() {
                        windows.claim_session(target, connection, session);
                    }
                }
                // The workspace reopens its window's object tab as a place,
                // as it does at launch: nothing is read until it is shown.
                self.write_object_location(target, connection, Some(place))
                    .await?;
                self.inner
                    .handoffs
                    .put(target, ConsoleHandoff::Object { open });
                Ok(())
            }
        }
    }

    /// Gives a console back to `source` when its new window could not be
    /// built: the user's session, and any transaction open on it, stay.
    pub(crate) fn hand_back(&self, target: WindowKey, source: WindowKey) {
        let Some(ConsoleHandoff::Console { open, document, .. }) = self.inner.handoffs.take(target)
        else {
            return;
        };
        let (Ok(connection), Ok(session), Ok(document)) = (
            open.connection.parse::<ConnectionId>(),
            open.console.session.parse::<SessionId>(),
            document.parse::<DocumentId>(),
        ) else {
            return;
        };
        if let Err(error) = self
            .inner
            .windows
            .hand_over(target, source, connection, session, document)
        {
            tracing::warn!(error = %error.message, "a console could not return to its window");
        }
    }
}

fn console_move(
    connection: &str,
    session: &str,
    document: &str,
    result: Option<&HandedResult>,
) -> Result<ConsoleMove, IpcError> {
    Ok(ConsoleMove {
        connection: parse("connection", connection)?,
        session: parse("session", session)?,
        document: parse("document", document)?,
        result: result
            .map(|handed| Ok::<_, IpcError>((parse("result", &handed.result)?, handed.clone())))
            .transpose()?,
    })
}

#[cfg(test)]
mod tests;
