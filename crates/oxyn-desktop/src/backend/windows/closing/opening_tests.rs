//! A connection opening that ends after its window closed (ADR-0043): what it
//! opened is closed, and nothing is recorded under the closed window's key —
//! its deletion is not refused, and the last window that lets the connection
//! go disconnects it.
//!
//! An opening resolves its credentials once per session, on the blocking
//! pool: a keyring whose read is held stops the opening there, between its
//! catalog's session and its console's, for as long as the test needs.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use oxyn_core::{CancelToken, CommandId, ConnectionId, DocumentId, Environment, SessionId};
use oxyn_secrets::{MemorySecretStore, SecretRef, SecretStore, SecretString};
use oxyn_store::Store;
use parking_lot::Mutex;

use crate::backend::Backend;
use crate::backend::windows::WindowKey;
use crate::commands::windows::run_owned;
use crate::ipc::settings::ConnectionChange;
use crate::ipc::windows::HandoffRequest;
use crate::ipc::{ConnectResponse, ConnectionDraft, IpcError, OpenConnection};

/// Long enough for a loaded machine to reach the held read.
const PATIENCE: Duration = Duration::from_secs(20);

/// A held read: told when it is reached, let through when released.
struct Gate {
    /// Reads let through before the held one.
    pass: usize,
    reached: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

/// A keyring in memory whose next read can be held.
#[derive(Default)]
struct HeldKeyring {
    entries: MemorySecretStore,
    gate: Mutex<Option<Gate>>,
}

// Not derived: the gate's channels are not worth printing, and the entries
// are secrets.
impl std::fmt::Debug for HeldKeyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldKeyring").finish_non_exhaustive()
    }
}

impl HeldKeyring {
    /// Holds the read that comes after `pass` others. Returns where it is
    /// told the read was reached, and what releases it.
    fn hold_after(&self, pass: usize) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (reached, reached_rx) = mpsc::channel();
        let (release_tx, release) = mpsc::channel();
        *self.gate.lock() = Some(Gate {
            pass,
            reached,
            release,
        });
        (reached_rx, release_tx)
    }
}

impl SecretStore for HeldKeyring {
    fn put(&self, reference: &SecretRef, secret: SecretString) -> oxyn_secrets::Result<()> {
        self.entries.put(reference, secret)
    }

    fn get(&self, reference: &SecretRef) -> oxyn_secrets::Result<Option<SecretString>> {
        let held = {
            let mut gate = self.gate.lock();
            match gate.as_mut() {
                Some(open) if open.pass > 0 => {
                    open.pass -= 1;
                    None
                }
                _ => gate.take(),
            }
        };
        // On the blocking pool, where the resolver reads: waiting here holds
        // one session's opening and nothing else.
        if let Some(gate) = held {
            let _ = gate.reached.send(());
            let _ = gate.release.recv_timeout(PATIENCE);
        }
        self.entries.get(reference)
    }

    fn delete(&self, reference: &SecretRef) -> oxyn_secrets::Result<()> {
        self.entries.delete(reference)
    }
}

struct Bench {
    runtime: tokio::runtime::Runtime,
    backend: Backend,
    keyring: Arc<HeldKeyring>,
    connection: ConnectionId,
    _directory: tempfile::TempDir,
}

/// A local SQLite connection with a password, so that each of its sessions
/// reads the keyring when it opens. Created by the backend, whose sessions of
/// that first opening no window claims.
fn bench() -> Bench {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("opening.sqlite");
    let keyring = Arc::new(HeldKeyring::default());
    let backend = {
        let _guard = runtime.enter();
        Backend::assemble(
            Arc::new(Store::open_in_memory().expect("temporary store")),
            Arc::clone(&keyring) as Arc<dyn SecretStore>,
        )
        .expect("temporary backend")
    };
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "orders".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), path.to_string_lossy().into_owned())].into(),
        secrets: BTreeMap::from([("password".to_owned(), "not-a-real-password".to_owned())]),
    };
    let open = match runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    {
        ConnectResponse::Open(open) => open,
        _ => panic!("a local connection opens at once"),
    };
    let connection = open.connection.parse().expect("connection id");
    Bench {
        runtime,
        backend,
        keyring,
        connection,
        _directory: directory,
    }
}

/// `reconnect` as the Tauri command runs it: recorded as `window`'s opening,
/// owned by it, then adopted.
async fn reconnect_in(
    backend: Backend,
    window: WindowKey,
    connection: ConnectionId,
) -> Result<OpenConnection, IpcError> {
    let id = CommandId::new();
    let opening = backend.inner.windows.begin_opening(window, id)?;
    let open = run_owned(
        &backend,
        window,
        id,
        backend.reconnect(id, connection),
        |_| false,
    )
    .await?;
    backend.adopt_open(opening, &open).await?;
    Ok(open)
}

/// `reconnect` as the Tauri command runs it, up to the adoption: the
/// sessions are open, and the window has not adopted them yet.
fn opened_unadopted(bench: &Bench, window: WindowKey) -> (super::Opening<'_>, OpenConnection) {
    let backend = &bench.backend;
    let id = CommandId::new();
    let opening = backend
        .inner
        .windows
        .begin_opening(window, id)
        .expect("an open window");
    let open = bench
        .runtime
        .block_on(backend.reconnect(id, bench.connection))
        .expect("reopens");
    (opening, open)
}

fn sessions_of(open: &OpenConnection) -> [SessionId; 2] {
    [
        open.session.parse().expect("catalog session"),
        open.console.session.parse().expect("console session"),
    ]
}

impl Bench {
    /// The sessions the executor holds open on the connection.
    fn open_sessions(&self) -> HashSet<SessionId> {
        self.backend
            .inner
            .executor
            .sessions()
            .for_connection(self.connection)
            .iter()
            .filter(|slot| slot.is_open())
            .map(|slot| slot.id())
            .collect()
    }

    /// The window's own reopening of the connection, adopted.
    fn reopen_in(&self, window: WindowKey) -> [SessionId; 2] {
        let open = self
            .runtime
            .block_on(reconnect_in(self.backend.clone(), window, self.connection))
            .expect("reopens");
        sessions_of(&open)
    }

    /// A console no window claims: only `Command::Disconnect` closes it.
    fn stray_console(&self) -> SessionId {
        self.runtime
            .block_on(self.backend.open_console(CommandId::new(), self.connection))
            .expect("a console opens")
            .session
            .parse()
            .expect("session id")
    }
}

#[test]
fn an_opening_whose_window_closed_while_its_console_opened_leaves_nothing() {
    let bench = bench();
    let _guard = bench.runtime.enter();
    let backend = &bench.backend;
    let live = backend.reserve_window(true).expect("a window");
    let closing = backend.reserve_window(false).expect("a window");
    let other = backend.reserve_window(false).expect("a window");
    let kept = bench.reopen_in(live);
    let stray = bench.stray_console();
    let before = bench.open_sessions();

    // The catalog's read passes; the console's is held.
    let (reached, release) = bench.keyring.hold_after(1);
    let opening = bench
        .runtime
        .spawn(reconnect_in(backend.clone(), closing, bench.connection));
    reached
        .recv_timeout(PATIENCE)
        .expect("the console's opening reads the keyring");
    bench.runtime.block_on(backend.release_window(closing));
    release.send(()).expect("the read is still held");
    let answer = bench.runtime.block_on(opening).expect("joins");

    assert!(
        answer.is_err(),
        "the closed window gets nothing: {answer:?}"
    );
    assert!(
        !backend.inner.windows.names(closing),
        "no record names the closed window"
    );
    assert_eq!(
        bench.open_sessions(),
        before,
        "its catalog's and console's sessions are both closed"
    );

    // The last live window lets the connection go: it is disconnected,
    // the stray console with it.
    bench
        .runtime
        .block_on(backend.release_connection(live, bench.connection))
        .expect("released");
    for session in kept.into_iter().chain([stray]) {
        assert!(!bench.open_sessions().contains(&session));
    }
    assert!(bench.open_sessions().is_empty(), "Command::Disconnect ran");

    // Nothing holds it for the closed window: another deletes it.
    let id = CommandId::new();
    let deleted = bench.runtime.block_on(run_owned(
        backend,
        other,
        id,
        backend.delete_connection(id, bench.connection),
        |_| false,
    ));
    assert!(
        matches!(deleted, Ok(ConnectionChange::Deleted)),
        "{:?}",
        deleted.err()
    );
}

#[test]
fn an_opening_adopted_after_its_window_closed_is_closed_and_disconnected() {
    let bench = bench();
    let _guard = bench.runtime.enter();
    let backend = &bench.backend;
    let closing = backend.reserve_window(true).expect("a window");
    let stray = bench.stray_console();
    // Opened to the end before the window closed: too late for the
    // cancellation, only the adoption sees it.
    let (opening, open) = opened_unadopted(&bench, closing);
    bench.runtime.block_on(backend.release_window(closing));

    let refused = bench
        .runtime
        .block_on(backend.adopt_open(opening, &open))
        .expect_err("the window has closed");
    assert_eq!(refused.message, "This window has closed");
    assert!(!backend.inner.windows.names(closing));
    for session in sessions_of(&open).into_iter().chain([stray]) {
        assert!(!bench.open_sessions().contains(&session));
    }
    assert!(
        bench.open_sessions().is_empty(),
        "no window holds it: disconnected"
    );
    assert!(backend.inner.windows.holders(bench.connection).is_empty());
}

#[test]
fn a_late_adoption_leaves_another_window_connection_open() {
    let bench = bench();
    let _guard = bench.runtime.enter();
    let backend = &bench.backend;
    let live = backend.reserve_window(true).expect("a window");
    let closing = backend.reserve_window(false).expect("a window");
    let kept = bench.reopen_in(live);
    let (opening, open) = opened_unadopted(&bench, closing);
    bench.runtime.block_on(backend.release_window(closing));

    assert!(
        bench
            .runtime
            .block_on(backend.adopt_open(opening, &open))
            .is_err()
    );
    let open_now = bench.open_sessions();
    for session in sessions_of(&open) {
        assert!(!open_now.contains(&session), "the late sessions are closed");
    }
    for session in kept {
        assert!(open_now.contains(&session), "the live window's stay open");
    }
}

#[test]
fn a_closing_window_leaves_another_window_opening_in_flight() {
    let bench = bench();
    let _guard = bench.runtime.enter();
    let backend = &bench.backend;
    let closing = backend.reserve_window(true).expect("a window");
    let other = backend.reserve_window(false).expect("a window");
    // The closing window holds the connection, and opens it once more; the
    // other window has opened its sessions and not adopted them yet: no
    // window but the closing one holds the connection.
    let held = bench.reopen_in(closing);
    let (late, late_open) = opened_unadopted(&bench, closing);
    let (in_flight, flying) = opened_unadopted(&bench, other);
    assert_eq!(
        backend.inner.windows.holders(bench.connection),
        vec![closing]
    );

    // Its close lets go of what it held, then its late opening is refused:
    // two decisions to disconnect, and neither may.
    bench.runtime.block_on(backend.release_window(closing));
    assert!(
        bench
            .runtime
            .block_on(backend.adopt_open(late, &late_open))
            .is_err()
    );

    let open_now = bench.open_sessions();
    for session in held.into_iter().chain(sessions_of(&late_open)) {
        assert!(
            !open_now.contains(&session),
            "the closed window's are closed"
        );
    }
    for session in sessions_of(&flying) {
        assert!(
            open_now.contains(&session),
            "the opening in flight keeps its sessions: no `Disconnect` ran"
        );
    }
    bench
        .runtime
        .block_on(backend.adopt_open(in_flight, &flying))
        .expect("an open window");
    assert_eq!(backend.inner.windows.holders(bench.connection), vec![other]);
    assert!(!backend.inner.windows.names(closing));
}

#[test]
fn a_move_whose_target_closed_while_its_catalog_opened_leaves_the_console_home() {
    let bench = bench();
    let _guard = bench.runtime.enter();
    let backend = &bench.backend;
    let source = backend.reserve_window(true).expect("a window");
    let [_, console] = bench.reopen_in(source);
    let target = backend.reserve_window(false).expect("a target");
    let before = bench.open_sessions();
    let request = HandoffRequest::Console {
        connection: bench.connection.to_string(),
        session: console.to_string(),
        document: DocumentId::new().to_string(),
        result: None,
        parameters: Vec::new(),
    };

    // The target's catalog session is held at its credential read.
    let (reached, release) = bench.keyring.hold_after(0);
    let moving = bench.runtime.spawn({
        let backend = backend.clone();
        async move { backend.hand_off(source, target, request).await }
    });
    reached
        .recv_timeout(PATIENCE)
        .expect("the target's catalog reads the keyring");
    bench.runtime.block_on(backend.release_window(target));
    release.send(()).expect("the read is still held");
    let answer = bench.runtime.block_on(moving).expect("joins");

    assert_eq!(
        answer.expect_err("the target has closed").message,
        "This window has closed"
    );
    let windows = &backend.inner.windows;
    assert!(!windows.names(target), "no record names the closed target");
    assert!(
        windows.check_session(source, console).is_ok(),
        "the console stays the source's"
    );
    assert_eq!(
        bench.open_sessions(),
        before,
        "only the target's fresh catalog is closed"
    );
    assert!(backend.inner.handoffs.take(target).is_none());
}

#[test]
fn a_catalog_session_opened_after_its_cancellation_is_closed() {
    let bench = bench();
    let _guard = bench.runtime.enter();
    let backend = &bench.backend;
    let config = bench
        .runtime
        .block_on(backend.read_config(bench.connection))
        .expect("saved");
    let before = bench.open_sessions();

    let (reached, release) = bench.keyring.hold_after(0);
    let cancel = CancelToken::new();
    let opening = bench.runtime.spawn({
        let backend = backend.clone();
        let cancel = cancel.clone();
        async move { backend.catalog_session(&config, &cancel).await }
    });
    reached
        .recv_timeout(PATIENCE)
        .expect("the opening reads the keyring");
    cancel.cancel();
    release.send(()).expect("the read is still held");
    let answer = bench.runtime.block_on(opening).expect("joins");

    assert!(answer.is_err(), "cancelled: {answer:?}");
    assert_eq!(bench.open_sessions(), before, "no session outlives it");
}
