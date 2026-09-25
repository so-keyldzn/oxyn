//! The windows of the process, and what each one owns
//! ([ADR-0043](../../../../docs/adr/0043-multi-fenetre.md)).
//!
//! Every window lives in this process, on the same backend. Its identity is a
//! [`WindowKey`] chosen here; its Tauri label derives from it, and no IPC
//! command takes a label from the webview. [`WindowRegistry`] is the one
//! source of truth on what a window owns: the sessions it opened, the commands
//! it sent, the results it was given, the assistant of a connection. A command
//! that names something another window owns is refused, whatever it names —
//! a script in one webview reaches nothing of another.
//!
//! Nothing here reaches a driver, the store or a window: the registry is
//! state in memory, read and written under one lock that no `await` crosses.

use std::collections::{HashMap, HashSet};

use oxyn_core::{CommandId, ConnectionId, DocumentId, Event, ResultId, SessionId};
use oxyn_exec::ExecEvent;
use parking_lot::Mutex;
use tauri::ipc::Channel;
use tokio::sync::{Notify, watch};
use uuid::Uuid;

use crate::ipc::IpcError;
use crate::ipc::recovery::ShutdownSignal;
use crate::ipc::windows::WindowSignal;

pub(crate) use self::closing::CloseStep;

mod closing;

/// The most windows open at once.
///
/// A guard against a repeated gesture or a compromised webview, not a
/// measure: the memory of one more webview is not measured. The 17th is
/// refused with a message that says so.
pub(crate) const MAX_WINDOWS: usize = 16;

/// What every window label starts with. `capabilities/main.json` grants its
/// permissions to `workspace-*`, and to nothing else.
const LABEL_PREFIX: &str = "workspace-";

/// The persistent identity of a window. Its label is
/// `workspace-<uuid without dashes>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct WindowKey(Uuid);

impl WindowKey {
    fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// The Tauri label of this window.
    pub(crate) fn label(self) -> String {
        format!("{LABEL_PREFIX}{}", self.0.simple())
    }

    /// The key a label names, if it is one of ours.
    fn from_label(label: &str) -> Option<Self> {
        let hex = label.strip_prefix(LABEL_PREFIX)?;
        if hex.len() != 32 {
            return None;
        }
        Uuid::try_parse(hex).ok().map(Self)
    }
}

/// A stream a window subscribes to, ended by the window's next subscription
/// to it or by the window's close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Stream {
    Events,
    RefreshSignals,
}

/// Where a window's close stands. « The last window » is decided on it: a
/// window whose close is confirmed no longer counts, one whose dialog is
/// open still does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Closing {
    /// No close asked, or the last one cancelled.
    Open,
    /// The close was asked; the window's transactions are being listed.
    Listing,
    /// A signal was sent; waiting for the webview to acknowledge it.
    Asking,
    /// Acknowledged: the user decides, without a time limit.
    Deciding,
    /// Confirmed, or unanswered: the window goes. Final.
    Closed,
}

/// One window of the registry.
struct Window {
    key: WindowKey,
    /// Built at launch, rather than by `New window`: only it may offer the
    /// recovery screen, which speaks of how the previous launch ended.
    initial: bool,
    closing: Closing,
    signals: Option<Channel<WindowSignal>>,
    shutdown: Option<Channel<ShutdownSignal>>,
    /// The generation counter of each stream. Dropped with the window, which
    /// ends the tasks that feed it.
    streams: HashMap<Stream, watch::Sender<u64>>,
    /// An exit or a close is waiting for this window's answer.
    awaited: Answers,
}

/// The answers an exit or a close waits for from one window.
#[derive(Debug, Default, Clone, Copy)]
struct Answers {
    flushed: bool,
    acknowledged: bool,
}

/// What a window owned when it was forgotten: what the close must release.
#[derive(Debug, Default)]
pub(crate) struct Owned {
    /// Its sessions, with their connection.
    pub(crate) sessions: Vec<(ConnectionId, SessionId)>,
    /// Its commands, some of which may still run or wait for a decision.
    pub(crate) commands: Vec<CommandId>,
    /// Its results, with the number of views it holds on each: one
    /// `forget_result` per view.
    pub(crate) results: Vec<(ResultId, usize)>,
    /// The connections whose assistant it held.
    pub(crate) assistants: Vec<ConnectionId>,
    /// The connections no other window holds any more.
    pub(crate) released: Vec<ConnectionId>,
}

#[derive(Default)]
struct State {
    /// In opening order.
    windows: Vec<Window>,
    focused: Option<WindowKey>,
    sessions: HashMap<SessionId, (WindowKey, ConnectionId)>,
    commands: HashMap<CommandId, WindowKey>,
    /// The windows reading each result, with how many views each holds.
    results: HashMap<ResultId, HashMap<WindowKey, usize>>,
    /// The window whose assistant serves each connection. The assistant's
    /// state is kept by connection (`backend/ai`), so a connection's
    /// conversations and agent belong to one window at a time.
    assistants: HashMap<ConnectionId, WindowKey>,
    /// The connections each window holds a workspace on.
    connections: HashMap<WindowKey, HashSet<ConnectionId>>,
    /// The window whose console writes each open document: one console, one
    /// window.
    documents: HashMap<DocumentId, WindowKey>,
}

impl State {
    fn window(&self, key: WindowKey) -> Option<&Window> {
        self.windows.iter().find(|window| window.key == key)
    }

    fn window_mut(&mut self, key: WindowKey) -> Option<&mut Window> {
        self.windows.iter_mut().find(|window| window.key == key)
    }
}

/// The windows of the process and what each one owns.
#[derive(Default)]
pub(crate) struct WindowRegistry {
    state: Mutex<State>,
    /// A window flushed its drafts or acknowledged a signal.
    answered: Notify,
}

/// The refusal of a command naming what another window owns.
fn elsewhere(what: &str) -> IpcError {
    IpcError::invalid(format!("This {what} belongs to another window"))
}

impl WindowRegistry {
    /// Reserves a new window, before it is built: the bound is checked here,
    /// under the lock, so two `New window` at once cannot both pass it.
    ///
    /// # Errors
    /// Past [`MAX_WINDOWS`].
    pub(crate) fn reserve(&self, initial: bool) -> Result<WindowKey, IpcError> {
        let mut state = self.state.lock();
        if state.windows.len() >= MAX_WINDOWS {
            return Err(IpcError::invalid(format!(
                "Oxyn opens at most {MAX_WINDOWS} windows. Close one first."
            )));
        }
        let key = WindowKey::new();
        state.windows.push(Window {
            key,
            initial,
            closing: Closing::Open,
            signals: None,
            shutdown: None,
            streams: HashMap::new(),
            awaited: Answers::default(),
        });
        Ok(key)
    }

    /// The window a webview label names.
    ///
    /// # Errors
    /// A label that is not a registered window's: a webview Oxyn did not
    /// build, or one already closed. Its command is refused.
    pub(crate) fn key_of(&self, label: &str) -> Result<WindowKey, IpcError> {
        WindowKey::from_label(label)
            .filter(|key| self.state.lock().window(*key).is_some())
            .ok_or_else(|| IpcError::invalid("This window is not one of Oxyn's"))
    }

    /// Removes a window and everything it owned, returning what must be
    /// released: its sessions, commands, results, assistants, and the
    /// connections nobody else holds now. Its streams end.
    pub(crate) fn forget(&self, key: WindowKey) -> Owned {
        let mut state = self.state.lock();
        state.windows.retain(|window| window.key != key);
        if state.focused == Some(key) {
            state.focused = None;
        }
        let mut owned = Owned::default();
        state.sessions.retain(|session, (owner, connection)| {
            let mine = *owner == key;
            if mine {
                owned.sessions.push((*connection, *session));
            }
            !mine
        });
        state.commands.retain(|command, owner| {
            let mine = *owner == key;
            if mine {
                owned.commands.push(*command);
            }
            !mine
        });
        state.results.retain(|result, readers| {
            if let Some(views) = readers.remove(&key) {
                owned.results.push((*result, views));
            }
            !readers.is_empty()
        });
        state.assistants.retain(|connection, owner| {
            let mine = *owner == key;
            if mine {
                owned.assistants.push(*connection);
            }
            !mine
        });
        // A document its window did not close stays open in the store,
        // claimed by no window: the next launch treats it as a working copy
        // no window claims (ADR-0043).
        state.documents.retain(|_, owner| *owner != key);
        let held = state.connections.remove(&key).unwrap_or_default();
        for connection in held {
            if !state
                .connections
                .values()
                .any(|others| others.contains(&connection))
            {
                owned.released.push(connection);
            }
        }
        self.answered.notify_waiters();
        owned
    }

    /// How many windows are registered.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.state.lock().windows.len()
    }

    /// Every window, in opening order.
    pub(crate) fn keys(&self) -> Vec<WindowKey> {
        self.state.lock().windows.iter().map(|w| w.key).collect()
    }

    /// Whether this window was built at launch.
    pub(crate) fn is_initial(&self, key: WindowKey) -> bool {
        self.state.lock().window(key).is_some_and(|w| w.initial)
    }

    // ---- Focus ------------------------------------------------------------

    /// Follows `WindowEvent::Focused`: the focused window is the last one to
    /// have gained the focus, and stays so until another gains it or it
    /// closes — the menu bar of macOS stays for the application when it
    /// passes behind another. A label that is not ours is ignored.
    pub(crate) fn focus(&self, label: &str, focused: bool) {
        let Some(key) = WindowKey::from_label(label) else {
            return;
        };
        let mut state = self.state.lock();
        if focused && state.window(key).is_some() {
            state.focused = Some(key);
        }
    }

    /// The window with the focus, if any: the target of a window-scoped menu
    /// action.
    pub(crate) fn focused(&self) -> Option<WindowKey> {
        self.state.lock().focused
    }

    // ---- Ownership --------------------------------------------------------

    /// Records `window` as the owner of `command`, **before** it is
    /// dispatched: no event of it can precede the record.
    ///
    /// # Errors
    /// The id is already another window's.
    pub(crate) fn claim_command(
        &self,
        window: WindowKey,
        command: CommandId,
    ) -> Result<(), IpcError> {
        let mut state = self.state.lock();
        match state.commands.get(&command) {
            Some(owner) if *owner != window => Err(elsewhere("command")),
            _ => {
                state.commands.insert(command, window);
                Ok(())
            }
        }
    }

    /// Checks that `command` is not another window's. An id nobody claimed —
    /// an agent's approval, a cancel that arrives before its command — is
    /// claimed by the caller.
    ///
    /// # Errors
    /// The command is another window's.
    pub(crate) fn check_command(
        &self,
        window: WindowKey,
        command: CommandId,
    ) -> Result<(), IpcError> {
        self.claim_command(window, command)
    }

    /// Whether `command` is another window's. Records nothing: a cancel for
    /// an id nobody claimed — one that has not been dispatched yet — must
    /// not grow the registry.
    pub(crate) fn command_elsewhere(&self, window: WindowKey, command: CommandId) -> bool {
        self.state
            .lock()
            .commands
            .get(&command)
            .is_some_and(|owner| *owner != window)
    }

    /// Forgets the owner of a command that has answered and waits for
    /// nothing.
    pub(crate) fn release_command(&self, window: WindowKey, command: CommandId) {
        let mut state = self.state.lock();
        if state.commands.get(&command) == Some(&window) {
            state.commands.remove(&command);
        }
    }

    /// Records a session this window opened, and the connection it holds.
    pub(crate) fn claim_session(
        &self,
        window: WindowKey,
        connection: ConnectionId,
        session: SessionId,
    ) {
        let mut state = self.state.lock();
        state.sessions.insert(session, (window, connection));
        state
            .connections
            .entry(window)
            .or_default()
            .insert(connection);
    }

    /// Checks that `session` is this window's.
    ///
    /// # Errors
    /// Another window's, or a session no window opened.
    pub(crate) fn check_session(
        &self,
        window: WindowKey,
        session: SessionId,
    ) -> Result<(), IpcError> {
        match self.state.lock().sessions.get(&session) {
            Some((owner, _)) if *owner == window => Ok(()),
            Some(_) => Err(elsewhere("console")),
            None => Err(IpcError::invalid("This session is not open in this window")),
        }
    }

    /// Forgets a closed session.
    pub(crate) fn release_session(&self, session: SessionId) {
        self.state.lock().sessions.remove(&session);
    }

    /// This window's sessions on `connection`.
    pub(crate) fn sessions_on(
        &self,
        window: WindowKey,
        connection: ConnectionId,
    ) -> Vec<SessionId> {
        self.state
            .lock()
            .sessions
            .iter()
            .filter(|(_, (owner, on))| *owner == window && *on == connection)
            .map(|(session, _)| *session)
            .collect()
    }

    /// The window that opened `session`, if any.
    pub(crate) fn owner_of_session(&self, session: SessionId) -> Option<WindowKey> {
        self.state
            .lock()
            .sessions
            .get(&session)
            .map(|(owner, _)| *owner)
    }

    /// Records that this window holds a workspace on `connection`.
    pub(crate) fn hold_connection(&self, window: WindowKey, connection: ConnectionId) {
        self.state
            .lock()
            .connections
            .entry(window)
            .or_default()
            .insert(connection);
    }

    /// This window lets `connection` go. Returns whether no window holds it
    /// any more: the backend then disconnects it, since only it sees every
    /// window.
    pub(crate) fn release_connection(&self, window: WindowKey, connection: ConnectionId) -> bool {
        let mut state = self.state.lock();
        if let Some(held) = state.connections.get_mut(&window) {
            held.remove(&connection);
        }
        !state
            .connections
            .values()
            .any(|held| held.contains(&connection))
    }

    /// Whether this window holds a workspace on `connection`.
    pub(crate) fn holds(&self, window: WindowKey, connection: ConnectionId) -> bool {
        self.state
            .lock()
            .connections
            .get(&window)
            .is_some_and(|held| held.contains(&connection))
    }

    /// The windows other than `window` that hold `connection`.
    pub(crate) fn other_holders(
        &self,
        window: WindowKey,
        connection: ConnectionId,
    ) -> Vec<WindowKey> {
        self.state
            .lock()
            .connections
            .iter()
            .filter(|(key, held)| **key != window && held.contains(&connection))
            .map(|(key, _)| *key)
            .collect()
    }

    /// Every window holding `connection`.
    pub(crate) fn holders(&self, connection: ConnectionId) -> Vec<WindowKey> {
        self.state
            .lock()
            .connections
            .iter()
            .filter(|(_, held)| held.contains(&connection))
            .map(|(key, _)| *key)
            .collect()
    }

    /// Records that this window reads `result`; `view` counts one more view
    /// whose `forget_result` will come.
    pub(crate) fn claim_result(&self, window: WindowKey, result: ResultId, view: bool) {
        let mut state = self.state.lock();
        let views = state
            .results
            .entry(result)
            .or_default()
            .entry(window)
            .or_insert(0);
        if view {
            *views = views.saturating_add(1);
        }
    }

    /// Checks that this window reads `result`. A result nobody was given —
    /// an agent's, shown by the assistant — is taken by its first reader.
    ///
    /// # Errors
    /// Another window reads it, and this one was never given it.
    pub(crate) fn check_result(&self, window: WindowKey, result: ResultId) -> Result<(), IpcError> {
        let mut state = self.state.lock();
        let readers = state.results.entry(result).or_default();
        if readers.is_empty() {
            readers.insert(window, 0);
        }
        if readers.contains_key(&window) {
            Ok(())
        } else {
            Err(elsewhere("result"))
        }
    }

    /// One view of `result` in this window is gone.
    pub(crate) fn forget_result(&self, window: WindowKey, result: ResultId) {
        let mut state = self.state.lock();
        let Some(readers) = state.results.get_mut(&result) else {
            return;
        };
        if let Some(views) = readers.get_mut(&window) {
            *views = views.saturating_sub(1);
            if *views == 0 {
                readers.remove(&window);
            }
        }
        if readers.is_empty() {
            state.results.remove(&result);
        }
    }

    /// Takes `document` as written by this window's console, or checks that it
    /// already is.
    ///
    /// # Errors
    /// A console of another window writes it.
    pub(crate) fn claim_document(
        &self,
        window: WindowKey,
        document: DocumentId,
    ) -> Result<(), IpcError> {
        let mut state = self.state.lock();
        match state.documents.get(&document) {
            Some(owner) if *owner != window => {
                Err(IpcError::invalid("This query is open in another window"))
            }
            _ => {
                state.documents.insert(document, window);
                Ok(())
            }
        }
    }

    /// Checks that no other window's console writes `document`.
    ///
    /// # Errors
    /// Another window's console writes it.
    pub(crate) fn check_document(
        &self,
        window: WindowKey,
        document: DocumentId,
    ) -> Result<(), IpcError> {
        match self.state.lock().documents.get(&document) {
            Some(owner) if *owner != window => {
                Err(IpcError::invalid("This query is open in another window"))
            }
            _ => Ok(()),
        }
    }

    /// This window's console let `document` go.
    pub(crate) fn release_document(&self, window: WindowKey, document: DocumentId) {
        let mut state = self.state.lock();
        if state.documents.get(&document) == Some(&window) {
            state.documents.remove(&document);
        }
    }

    /// Takes the assistant of `connection` for this window, or checks that it
    /// already has it.
    ///
    /// # Errors
    /// Another window has it: the caller brings that window to the front.
    pub(crate) fn claim_assistant(
        &self,
        window: WindowKey,
        connection: ConnectionId,
    ) -> Result<(), WindowKey> {
        let mut state = self.state.lock();
        match state.assistants.get(&connection) {
            Some(owner) if *owner != window => Err(*owner),
            _ => {
                state.assistants.insert(connection, window);
                Ok(())
            }
        }
    }

    /// Whether this window has the assistant of `connection`.
    pub(crate) fn has_assistant(&self, window: WindowKey, connection: ConnectionId) -> bool {
        self.state.lock().assistants.get(&connection) == Some(&window)
    }

    /// This window lets the assistant of `connection` go.
    pub(crate) fn release_assistant(&self, window: WindowKey, connection: ConnectionId) {
        let mut state = self.state.lock();
        if state.assistants.get(&connection) == Some(&window) {
            state.assistants.remove(&connection);
        }
    }

    // ---- Routing ----------------------------------------------------------

    /// Whether `window` receives this execution event, recording the result
    /// it announces as this window's before it is sent.
    ///
    /// An event goes to the window that owns its command; a transaction state
    /// goes to the window that owns its session. Anything else goes nowhere:
    /// no window sorts another's stream itself.
    pub(crate) fn route(&self, window: WindowKey, event: &ExecEvent) -> bool {
        let mut state = self.state.lock();
        if let Event::TransactionState { session, .. } = &event.event {
            return state.sessions.get(session).map(|(owner, _)| *owner) == Some(window);
        }
        if state.commands.get(&event.command) != Some(&window) {
            return false;
        }
        let announced = match &event.event {
            Event::SchemaReady { result }
            | Event::BatchReady { result, .. }
            | Event::Completed { result, .. } => Some(*result),
            _ => None,
        };
        if let Some(result) = announced {
            state
                .results
                .entry(result)
                .or_default()
                .entry(window)
                .or_insert(0);
        }
        true
    }

    // ---- Channels and streams --------------------------------------------

    /// Starts a new generation of `stream` for this window, ending the
    /// previous subscriber. `None` for a window that is gone.
    pub(crate) fn supersede(
        &self,
        window: WindowKey,
        stream: Stream,
    ) -> Option<watch::Receiver<u64>> {
        let mut state = self.state.lock();
        let entry = state.window_mut(window)?;
        let sender = entry
            .streams
            .entry(stream)
            .or_insert_with(|| watch::channel(0).0);
        sender.send_modify(|generation| *generation = generation.wrapping_add(1));
        // Subscribed after the bump: this generation is already seen.
        Some(sender.subscribe())
    }

    /// The window's own signals: a close asked, a connection changed.
    pub(crate) fn subscribe_signals(&self, window: WindowKey, channel: Channel<WindowSignal>) {
        if let Some(entry) = self.state.lock().window_mut(window) {
            entry.signals = Some(channel);
        }
    }

    /// Sends `signal` to `window` on its own channel. `false` when it has
    /// none or the send failed.
    pub(crate) fn signal(&self, window: WindowKey, signal: WindowSignal) -> bool {
        let channel = self
            .state
            .lock()
            .window(window)
            .and_then(|entry| entry.signals.clone());
        channel.is_some_and(|channel| channel.send(signal).is_ok())
    }

    /// Where the backend asks this window to flush its drafts or resolve its
    /// transactions. A reload replaces the channel.
    pub(crate) fn subscribe_shutdown(&self, window: WindowKey, channel: Channel<ShutdownSignal>) {
        if let Some(entry) = self.state.lock().window_mut(window) {
            entry.shutdown = Some(channel);
        }
    }

    /// Sends a shutdown signal to one window. `false` when it has no channel
    /// or the send failed.
    pub(crate) fn shutdown_signal(&self, window: WindowKey, signal: ShutdownSignal) -> bool {
        let channel = self
            .state
            .lock()
            .window(window)
            .and_then(|entry| entry.shutdown.clone());
        channel.is_some_and(|channel| channel.send(signal).is_ok())
    }

    // ---- Answers awaited by an exit or a close ----------------------------

    /// Forgets the answers of `windows`, before a signal that expects new
    /// ones.
    pub(crate) fn expect_answers(&self, windows: &[WindowKey]) {
        let mut state = self.state.lock();
        for key in windows {
            if let Some(entry) = state.window_mut(*key) {
                entry.awaited = Answers::default();
            }
        }
    }

    /// This window flushed its drafts.
    pub(crate) fn flushed(&self, window: WindowKey) {
        if let Some(entry) = self.state.lock().window_mut(window) {
            entry.awaited.flushed = true;
        }
        self.answered.notify_waiters();
    }

    /// This window acknowledged the signal it was sent.
    pub(crate) fn acknowledged(&self, window: WindowKey) {
        if let Some(entry) = self.state.lock().window_mut(window) {
            entry.awaited.acknowledged = true;
        }
        self.answered.notify_waiters();
    }

    /// Waits until every window of `windows` still registered has answered
    /// — flushed or acknowledged, as `answer` reads it — or `grace` elapses.
    /// Returns the windows that answered.
    pub(crate) async fn answers(
        &self,
        windows: &[WindowKey],
        answer: fn(Answer) -> bool,
        grace: std::time::Duration,
    ) -> Vec<WindowKey> {
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            let notified = self.answered.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let (answered, waiting) = {
                let state = self.state.lock();
                let mut answered = Vec::new();
                let mut waiting = false;
                for key in windows {
                    match state.window(*key) {
                        Some(entry) if answer(Answer(entry.awaited)) => answered.push(*key),
                        Some(_) => waiting = true,
                        None => {}
                    }
                }
                (answered, waiting)
            };
            if !waiting {
                return answered;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return answered;
            }
        }
    }

    // ---- Closing ----------------------------------------------------------

    /// Whether `window` is the last one that is not closing: closing it quits.
    /// Decided under the lock, with every other window's state.
    pub(crate) fn is_last(&self, window: WindowKey) -> bool {
        !self
            .state
            .lock()
            .windows
            .iter()
            .any(|entry| entry.key != window && entry.closing != Closing::Closed)
    }

    /// Moves this window's close from one of `from` to `to`; `false`, and
    /// nothing changes, when it stands elsewhere.
    pub(crate) fn advance_close(&self, window: WindowKey, from: &[Closing], to: Closing) -> bool {
        let mut state = self.state.lock();
        let Some(entry) = state.window_mut(window) else {
            return false;
        };
        if !from.contains(&entry.closing) {
            return false;
        }
        entry.closing = to;
        true
    }

    /// Where this window's close stands; `Closed` for a window that is gone.
    #[cfg(test)]
    pub(crate) fn closing(&self, window: WindowKey) -> Closing {
        self.state
            .lock()
            .window(window)
            .map_or(Closing::Closed, |entry| entry.closing)
    }
}

/// What one window has answered, for [`WindowRegistry::answers`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Answer(Answers);

impl Answer {
    pub(crate) const fn flushed(self) -> bool {
        self.0.flushed
    }

    pub(crate) const fn acknowledged(self) -> bool {
        self.0.acknowledged
    }
}

#[cfg(test)]
mod tests;
