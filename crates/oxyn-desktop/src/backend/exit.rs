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

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use oxyn_core::{Capabilities, ConnectionId, Environment, Event, SessionId, TransactionState};
use oxyn_exec::ExecEvent;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::sync::broadcast::{self, error::TryRecvError};

use super::Backend;
use super::windows::{Answer, WindowKey};
use crate::ipc::recovery::{ExitScope, ExitTransaction, ShutdownSignal};

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
    /// Console statements sent and not answered yet. Counted apart from
    /// `state`: a late event of the previous statement must not make a
    /// running one look settled.
    running: usize,
}

impl ConsoleEntry {
    /// `Unknown` while a statement runs: its end has not said yet.
    fn shown_state(&self) -> TransactionState {
        if self.running > 0 {
            TransactionState::Unknown
        } else {
            self.state
        }
    }
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

/// How often a reader asks the follower again for the event stream, within
/// its grace: a follower that let go and took it back before the reader
/// queued is asked once more.
const READER_RETRY: Duration = Duration::from_millis(50);

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
    /// most `grace` for the follower to let go. Past it, this reading shows
    /// every state `Unknown` — listed rather than assumed settled — without
    /// keeping it: the events are still there for the next one.
    async fn settled(&self, grace: Duration) -> Vec<ConsoleEntry> {
        let deadline = tokio::time::Instant::now() + grace;
        let events = loop {
            self.reader.notify_one();
            let wait =
                READER_RETRY.min(deadline.saturating_duration_since(tokio::time::Instant::now()));
            match tokio::time::timeout(wait, self.events.lock()).await {
                Ok(events) => break Some(events),
                Err(_) if tokio::time::Instant::now() >= deadline => break None,
                Err(_) => {}
            }
        };
        let Some(mut events) = events else {
            let mut entries = self.now();
            for entry in &mut entries {
                entry.state = TransactionState::Unknown;
            }
            return entries;
        };
        loop {
            match events.try_recv() {
                Ok(event) => self.apply(Ok(event)),
                Err(TryRecvError::Lagged(_)) => self.apply(Err(Lag)),
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        drop(events);
        self.now()
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
            running: 0,
        });
    }

    pub(crate) fn closed(&self, session: SessionId) {
        self.entries.lock().retain(|entry| entry.session != session);
    }

    /// A console statement is sent to `session`, by `run_console` or by an
    /// approval: until the returned guard drops, an exit lists the session as
    /// `Unknown`.
    pub(crate) fn run_on(self: &Arc<Self>, session: SessionId) -> RunningStatement {
        let counted = self
            .entries
            .lock()
            .iter_mut()
            .find(|entry| entry.session == session)
            .map(|entry| entry.running += 1)
            .is_some();
        RunningStatement {
            consoles: Arc::clone(self),
            session: counted.then_some(session),
        }
    }
}

/// A console statement sent and not answered yet; see
/// [`ConsoleTransactions::run_on`].
pub(crate) struct RunningStatement {
    consoles: Arc<ConsoleTransactions>,
    /// `None` when the session is not a console's: nothing was counted.
    session: Option<SessionId>,
}

impl Drop for RunningStatement {
    fn drop(&mut self) {
        let Some(session) = self.session else { return };
        if let Some(entry) = self
            .consoles
            .entries
            .lock()
            .iter_mut()
            .find(|entry| entry.session == session)
        {
            entry.running = entry.running.saturating_sub(1);
        }
    }
}

/// A missed event.
struct Lag;

/// The journal of transactions a window did not acknowledge: closing their
/// sessions rolls them back.
pub(crate) fn warn_unacknowledged(transactions: &[ExitTransaction]) {
    for transaction in transactions {
        tracing::warn!(
            connection = %transaction.connection_name,
            session = %transaction.session,
            state = ?transaction.state,
            "the webview did not acknowledge; closing the session rolls back its transaction"
        );
    }
}

/// Where the exit stands before the ordered shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Hold {
    /// No exit asked, or the last one cancelled.
    #[default]
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

/// The exit's state before the ordered shutdown, and the windows it asked.
#[derive(Default)]
pub(crate) struct ExitHold {
    hold: Mutex<Hold>,
    /// The windows sent `ResolveTransactions` by the exit under way: a
    /// `Cancel` in any of them closes the dialog in all of them.
    asked: Mutex<Vec<WindowKey>>,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExitStep {
    /// Begin the ordered shutdown.
    Proceed,
    /// These windows acknowledged the transactions to resolve: the user
    /// decides, and they come to the front — they alone.
    Asked(Vec<WindowKey>),
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

    pub(crate) async fn exit_step_within(&self, grace: Duration) -> ExitStep {
        let hold = &self.inner.workbench.exit;
        {
            let mut state = hold.hold.lock();
            if !matches!(*state, Hold::Idle | Hold::Deciding) {
                return ExitStep::Held;
            }
            *state = Hold::Listing;
        }
        let open = self.open_transactions(grace, None).await;
        if open.is_empty() {
            return self.proceed_from(Hold::Listing);
        }
        let windows = &self.inner.windows;
        let asked: Vec<WindowKey> = open.keys().copied().collect();
        windows.expect_answers(&asked);
        if !hold.advance(Hold::Listing, Hold::Asking) {
            return ExitStep::Held;
        }
        *hold.asked.lock() = asked;
        // Each window is sent its own consoles, on its own channel: none
        // learns what another holds.
        let mut sent = Vec::new();
        for (window, transactions) in &open {
            let signal = ShutdownSignal::ResolveTransactions {
                transactions: transactions.clone(),
                scope: ExitScope::Application,
            };
            if windows.shutdown_signal(*window, signal) {
                sent.push(*window);
            }
        }
        let acknowledged = windows.answers(&sent, Answer::acknowledged, grace).await;
        for (window, transactions) in &open {
            if !acknowledged.contains(window) {
                warn_unacknowledged(transactions);
            }
        }
        if acknowledged.is_empty() {
            return self.proceed_from(Hold::Asking);
        }
        if hold.advance(Hold::Asking, Hold::Deciding) {
            ExitStep::Asked(acknowledged)
        } else {
            ExitStep::Held
        }
    }

    fn proceed_from(&self, from: Hold) -> ExitStep {
        if self.inner.workbench.exit.advance(from, Hold::Proceeding) {
            ExitStep::Proceed
        } else {
            ExitStep::Held
        }
    }

    /// A window has shown what it was asked to resolve — the exit's
    /// transactions, or its own close. Outside a wait for it, does nothing.
    pub fn shutdown_acknowledged(&self, window: WindowKey) {
        self.inner.windows.acknowledged(window);
    }

    /// `Cancel` in a window's dialog. During that window's own close, the
    /// close is abandoned; during an exit held by a transaction, the exit is
    /// abandoned for every window, before anything is flushed or recorded.
    /// Outside both, or once the ordered shutdown has begun, does nothing.
    pub fn cancel_exit(&self, window: WindowKey) {
        if self.cancel_window_close(window) {
            return;
        }
        {
            let mut hold = self.inner.workbench.exit.hold.lock();
            if !matches!(*hold, Hold::Listing | Hold::Asking | Hold::Deciding) {
                return;
            }
            *hold = Hold::Idle;
        }
        tracing::info!("exit cancelled with a transaction open");
        let asked = std::mem::take(&mut *self.inner.workbench.exit.asked.lock());
        for window in asked {
            let _ = self
                .inner
                .windows
                .shutdown_signal(window, ShutdownSignal::ExitCancelled);
        }
    }

    /// The console sessions whose transaction is open or not known, as the
    /// executor last observed them, with their connection's name and marking,
    /// by the window that owns them — `only` that window's when given.
    ///
    /// Read by the backend, never taken from the webview: a list it sent
    /// could be stale, or emptied by a script. A session no window claims —
    /// opened before any window existed — is shown in the first window.
    pub(crate) async fn open_transactions(
        &self,
        grace: Duration,
        only: Option<WindowKey>,
    ) -> BTreeMap<WindowKey, Vec<ExitTransaction>> {
        let entries = self.inner.workbench.consoles.settled(grace).await;
        let windows = &self.inner.windows;
        let first = windows.keys().first().copied();
        let mut open: BTreeMap<WindowKey, Vec<ExitTransaction>> = BTreeMap::new();
        for entry in self.holding(entries) {
            let Some(window) = windows.owner_of_session(entry.session).or(first) else {
                continue;
            };
            if only.is_some_and(|only| only != window) {
                continue;
            }
            // Read again: the marking may have changed since the console
            // opened. A connection gone from the store is shown as
            // production, never the reverse (I-02).
            let (name, environment) = match self.read_config(entry.connection).await {
                Ok(config) => (config.name, config.environment),
                Err(_) => (entry.name.clone(), Environment::Production),
            };
            open.entry(window).or_default().push(ExitTransaction {
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
                        && entry.shown_state() != TransactionState::Idle
                    {
                        holding.push(ConsoleEntry {
                            state: entry.shown_state(),
                            ..entry
                        });
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
