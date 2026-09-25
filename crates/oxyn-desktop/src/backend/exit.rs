//! The step before the ordered shutdown: a transaction still open on a console
//! holds the exit until the user commits, rolls back or cancels
//! ([ADR-0043](../../../../docs/adr/0043-multi-fenetre.md), « Transaction
//! ouverte à la sortie »).
//!
//! Nothing here reads a session. The state listed is the last one the executor
//! observed and published — at the console's opening, then at the end of each
//! execution ([ADR-0039](../../../../docs/adr/0039-etat-de-transaction-d-une-session.md)) —
//! so the bridge calls no driver outside the bus ([I-01](../../../../CLAUDE.md#i-01)).
//! Nothing here runs a statement either: `COMMIT` and `ROLLBACK` come from the
//! webview through `run_console`, as if typed.

use std::sync::Arc;
use std::time::Duration;

use oxyn_core::{Capabilities, ConnectionId, Environment, Event, SessionId, TransactionState};
use oxyn_exec::ExecEvent;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::sync::broadcast::{self, error::TryRecvError};

use super::Backend;
use crate::ipc::recovery::{ExitTransaction, ShutdownSignal};

/// How long the webview may take to acknowledge `ResolveTransactions`, and
/// the listing to catch up with the executor's events.
///
/// The drafts' grace ([`super::recovery`]): a webview frozen or reloading
/// must not keep the application open.
const ACKNOWLEDGE_GRACE: Duration = Duration::from_secs(2);

/// One console session, and the last transaction state the executor
/// published for it.
#[derive(Clone)]
struct ConsoleEntry {
    session: SessionId,
    connection: ConnectionId,
    /// The connection's name when the console opened: what the journal of a
    /// forced exit names, which cannot wait for the store.
    name: String,
    state: TransactionState,
}

/// The console sessions, in opening order, with their observed state.
///
/// The executor's events are applied by a task of their own; a reader first
/// drains what that task has not applied yet, so that a `COMMIT` answered a
/// moment ago is not listed as still open.
pub(crate) struct ConsoleTransactions {
    entries: Mutex<Vec<ConsoleEntry>>,
    events: tokio::sync::Mutex<broadcast::Receiver<ExecEvent>>,
    /// A reader is waiting for `events`: the task lets it go.
    reader: Notify,
}

impl ConsoleTransactions {
    pub(crate) fn new(events: broadcast::Receiver<ExecEvent>) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            events: tokio::sync::Mutex::new(events),
            reader: Notify::new(),
        }
    }

    fn apply(&self, received: Result<ExecEvent, Lag>) {
        let mut entries = self.entries.lock();
        match received {
            Ok(ExecEvent {
                event: Event::TransactionState { session, state },
                ..
            }) => {
                if let Some(entry) = entries.iter_mut().find(|entry| entry.session == session) {
                    entry.state = state;
                }
            }
            Ok(_) => {}
            // Some states were missed: none of them is known any more.
            Err(Lag) => {
                for entry in entries.iter_mut() {
                    entry.state = TransactionState::Unknown;
                }
            }
        }
    }

    /// Applies the executor's events until the executor is gone.
    ///
    /// Holds only this registry, never the backend: it ends with the
    /// executor's event bus.
    pub(crate) async fn follow(self: Arc<Self>) {
        loop {
            let mut events = self.events.lock().await;
            let received = tokio::select! {
                biased;
                () = self.reader.notified() => continue,
                received = events.recv() => received,
            };
            drop(events);
            match received {
                Ok(event) => self.apply(Ok(event)),
                Err(broadcast::error::RecvError::Lagged(_)) => self.apply(Err(Lag)),
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    /// The entries once every event already published is applied, waiting at
    /// most `grace` for the follower to let go. Past it, every state is
    /// `Unknown`: listed rather than assumed settled.
    async fn settled(&self, grace: Duration) -> Vec<ConsoleEntry> {
        self.reader.notify_one();
        match tokio::time::timeout(grace, self.events.lock()).await {
            Ok(mut events) => loop {
                match events.try_recv() {
                    Ok(event) => self.apply(Ok(event)),
                    Err(TryRecvError::Lagged(_)) => self.apply(Err(Lag)),
                    Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                }
            },
            Err(_) => self.apply(Err(Lag)),
        }
        self.entries.lock().clone()
    }

    /// The entries as they stand, without waiting: the forced exit's reading.
    fn now(&self) -> Vec<ConsoleEntry> {
        self.entries.lock().clone()
    }

    pub(crate) fn opened(
        &self,
        session: SessionId,
        connection: ConnectionId,
        name: String,
        state: TransactionState,
    ) {
        self.entries.lock().push(ConsoleEntry {
            session,
            connection,
            name,
            state,
        });
    }

    pub(crate) fn closed(&self, session: SessionId) {
        self.entries.lock().retain(|entry| entry.session != session);
    }

    /// A statement is about to run: until its end publishes a state, the
    /// session's is not known, and an exit asked meanwhile lists it. Returns
    /// the state before, for [`Self::not_run`].
    pub(crate) fn running(&self, session: SessionId) -> Option<TransactionState> {
        let mut entries = self.entries.lock();
        let entry = entries.iter_mut().find(|entry| entry.session == session)?;
        Some(std::mem::replace(
            &mut entry.state,
            TransactionState::Unknown,
        ))
    }

    /// The statement announced by [`Self::running`] never reached the
    /// session — refused, or held for approval. Puts `before` back unless a
    /// state was published since.
    pub(crate) fn not_run(&self, session: SessionId, before: Option<TransactionState>) {
        let Some(before) = before else { return };
        if let Some(entry) = self
            .entries
            .lock()
            .iter_mut()
            .find(|entry| entry.session == session && entry.state == TransactionState::Unknown)
        {
            entry.state = before;
        }
    }
}

/// A missed event.
struct Lag;

/// Where the exit stands before the ordered shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// No exit asked, or the last one cancelled.
    Idle,
    /// Listing the open transactions.
    Listing,
    /// `ResolveTransactions` sent; waiting for the webview to acknowledge it.
    Asking,
    /// Acknowledged: the user decides, without a time limit.
    Deciding,
    /// The ordered shutdown may begin. Final.
    Proceeding,
}

/// The exit's state before the ordered shutdown, and the acknowledgement it
/// waits for.
pub(crate) struct ExitHold {
    hold: Mutex<Hold>,
    acknowledged: Notify,
}

impl Default for ExitHold {
    fn default() -> Self {
        Self {
            hold: Mutex::new(Hold::Idle),
            acknowledged: Notify::new(),
        }
    }
}

impl ExitHold {
    /// Moves from `from` to `to`; `false`, and nothing changes, when the exit
    /// is elsewhere — cancelled meanwhile, most often.
    fn advance(&self, from: Hold, to: Hold) -> bool {
        let mut hold = self.hold.lock();
        if *hold != from {
            return false;
        }
        *hold = to;
        true
    }
}

/// What the exit does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitStep {
    /// Begin the ordered shutdown.
    Proceed,
    /// The webview acknowledged the transactions to resolve: the user
    /// decides, and the window comes to the front.
    Asked,
    /// Nothing to do: an exit is already being listed or asked, the
    /// shutdown has begun, or the exit was cancelled.
    Held,
}

impl Backend {
    /// The step before the ordered shutdown, run at every exit asked.
    ///
    /// No console holds a transaction: [`ExitStep::Proceed`] at once, as
    /// before this step existed. Otherwise the webview is asked to resolve
    /// them; once it acknowledges, the exit waits for it without a time
    /// limit, and a later exit asked — the dialog's own, once every session is
    /// `Idle` — lists again. A webview that does not acknowledge does not
    /// hold the exit: its transactions are rolled back by the close of the
    /// sessions, and the journal names them.
    pub(crate) async fn exit_step(&self) -> ExitStep {
        self.exit_step_within(ACKNOWLEDGE_GRACE).await
    }

    async fn exit_step_within(&self, grace: Duration) -> ExitStep {
        let hold = &self.inner.workbench.exit;
        {
            let mut state = hold.hold.lock();
            if !matches!(*state, Hold::Idle | Hold::Deciding) {
                return ExitStep::Held;
            }
            *state = Hold::Listing;
        }
        let open = self.open_transactions(grace).await;
        if open.is_empty() {
            return self.proceed_from(Hold::Listing);
        }
        let channel = self.shutdown_channel();
        let acknowledged = hold.acknowledged.notified();
        tokio::pin!(acknowledged);
        acknowledged.as_mut().enable();
        if !hold.advance(Hold::Listing, Hold::Asking) {
            return ExitStep::Held;
        }
        let signal = ShutdownSignal::ResolveTransactions {
            transactions: open.clone(),
        };
        let sent = channel.is_some_and(|channel| channel.send(signal).is_ok());
        if sent && tokio::time::timeout(grace, acknowledged).await.is_ok() {
            return if hold.advance(Hold::Asking, Hold::Deciding) {
                ExitStep::Asked
            } else {
                ExitStep::Held
            };
        }
        for transaction in &open {
            tracing::warn!(
                connection = %transaction.connection_name,
                session = %transaction.session,
                state = ?transaction.state,
                "the webview did not acknowledge the exit; closing the session rolls back its transaction"
            );
        }
        self.proceed_from(Hold::Asking)
    }

    fn proceed_from(&self, from: Hold) -> ExitStep {
        if self.inner.workbench.exit.advance(from, Hold::Proceeding) {
            ExitStep::Proceed
        } else {
            ExitStep::Held
        }
    }

    /// The webview has shown the transactions to resolve. Outside an exit
    /// waiting for it, does nothing.
    pub fn shutdown_acknowledged(&self) {
        let exit = &self.inner.workbench.exit;
        if *exit.hold.lock() == Hold::Asking {
            exit.acknowledged.notify_waiters();
        }
    }

    /// Abandons an exit held by an open transaction: nothing was flushed or
    /// recorded, and the application stays as it was. Outside such an exit,
    /// or once the ordered shutdown has begun, does nothing.
    pub fn cancel_exit(&self) {
        {
            let mut hold = self.inner.workbench.exit.hold.lock();
            if !matches!(*hold, Hold::Listing | Hold::Asking | Hold::Deciding) {
                return;
            }
            *hold = Hold::Idle;
        }
        tracing::info!("exit cancelled with a transaction open");
        if let Some(channel) = self.shutdown_channel() {
            let _ = channel.send(ShutdownSignal::ExitCancelled);
        }
    }

    /// The console sessions whose transaction is open or not known, as the
    /// executor last observed them, with their connection's name and marking.
    ///
    /// Read by the backend, never taken from the webview: a list it sent
    /// could be stale, or emptied by a script.
    async fn open_transactions(&self, grace: Duration) -> Vec<ExitTransaction> {
        let entries = self.inner.workbench.consoles.settled(grace).await;
        let mut open = Vec::new();
        for entry in self.holding(entries) {
            // Read again: the marking may have changed since the console
            // opened. A connection gone from the store is shown as
            // production, never the reverse (I-02).
            let (name, environment) = match self.read_config(entry.connection).await {
                Ok(config) => (config.name, config.environment),
                Err(_) => (entry.name.clone(), Environment::Production),
            };
            open.push(ExitTransaction {
                session: entry.session.to_string(),
                connection: entry.connection.to_string(),
                connection_name: name,
                environment,
                state: entry.state.into(),
            });
        }
        open
    }

    /// The entries of sessions still open that declare `TRANSACTIONS` and
    /// report a transaction open or unknown. Forgets the others' sessions
    /// once closed — by a disconnection, for instance.
    fn holding(&self, entries: Vec<ConsoleEntry>) -> Vec<ConsoleEntry> {
        let sessions = self.inner.executor.sessions();
        let mut holding = Vec::new();
        for entry in entries {
            match sessions.get(entry.session) {
                Some(slot) if slot.is_open() => {
                    if slot.capabilities().contains(Capabilities::TRANSACTIONS)
                        && entry.state != TransactionState::Idle
                    {
                        holding.push(entry);
                    }
                }
                _ => self.inner.workbench.consoles.closed(entry.session),
            }
        }
        holding
    }

    /// An exit nothing can hold — the Dock's Quit, a logout (ADR-0040): one
    /// `warn` per console session whose transaction is open or unknown, read
    /// without waiting for what is running. The end of the process rolls
    /// them back; nothing else changes on this path. Returns how many.
    pub(crate) fn warn_forced_exit_transactions(&self) -> usize {
        let holding = self.holding(self.inner.workbench.consoles.now());
        for entry in &holding {
            tracing::warn!(
                connection = %entry.name,
                session = %entry.session,
                state = ?entry.state,
                "forced exit with a transaction open or unknown; the end of the process rolls it back"
            );
        }
        holding.len()
    }
}

#[cfg(test)]
mod tests;
