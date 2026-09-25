//! An open transaction holds the exit (ADR-0043), over a SQLite file: the
//! only engine that declares `TRANSACTIONS` today.
//!
//! The webview is a channel that records what it is sent and, when told to,
//! acknowledges. `COMMIT` and `ROLLBACK` go through `run_console`, the path
//! the dialog takes; nothing in the exit step runs a statement itself.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use oxyn_core::{CommandId, ConnectionId, Environment, SessionId};
use parking_lot::Mutex;
use serde_json::Value;
use tauri::ipc::{Channel, InvokeResponseBody};

use crate::backend::{Backend, ExitStep};
use crate::ipc::consoles::{ConsoleRun, RunTarget};
use crate::ipc::{CommandOutcome, ConnectResponse, ConnectionDraft};

/// Short enough for a test, long enough for a loaded machine to deliver an
/// acknowledgement sent from inside the channel.
const GRACE: Duration = Duration::from_millis(300);

struct Bench {
    runtime: tokio::runtime::Runtime,
    backend: Backend,
    connection: ConnectionId,
    console: SessionId,
    path: std::path::PathBuf,
    /// Keeps the database file for the test's length.
    _directory: tempfile::TempDir,
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// A backend with one console on a SQLite file holding an empty table.
fn bench() -> Bench {
    let runtime = runtime();
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("exit.sqlite");
    let (backend, connection, console) = {
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let (connection, console) = connect(&runtime, &backend, &path);
        (backend, connection, console)
    };
    let bench = Bench {
        runtime,
        backend,
        connection,
        console,
        path,
        _directory: directory,
    };
    bench.ok("CREATE TABLE parent (id INTEGER PRIMARY KEY)");
    bench.ok("CREATE TABLE child (parent INTEGER REFERENCES parent (id) \
         DEFERRABLE INITIALLY DEFERRED)");
    bench
}

fn connect(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    path: &std::path::Path,
) -> (ConnectionId, SessionId) {
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "orders".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), path.to_string_lossy().into_owned())]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    };
    let ConnectResponse::Open(open) = runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    else {
        panic!("a local connection needs no approval");
    };
    (
        open.connection.parse().expect("connection id"),
        open.console.session.parse().expect("console session"),
    )
}

impl Bench {
    fn run(&self, sql: &str) -> Result<CommandOutcome, crate::ipc::IpcError> {
        let _guard = self.runtime.enter();
        self.runtime.block_on(self.backend.run_console(
            CommandId::new(),
            self.connection,
            self.console,
            ConsoleRun {
                sql: sql.into(),
                target: RunTarget::All,
                parameters: Vec::new(),
                explain: false,
            },
        ))
    }

    fn ok(&self, sql: &str) {
        assert!(
            matches!(self.run(sql), Ok(CommandOutcome::Executed { .. })),
            "{sql} runs"
        );
    }

    fn exit_step(&self) -> ExitStep {
        let _guard = self.runtime.enter();
        self.runtime.block_on(self.backend.exit_step_within(GRACE))
    }

    /// Rows committed in `child`, as a fresh launch on the same file sees
    /// them.
    fn committed_children(&self) -> u64 {
        let runtime = runtime();
        let _guard = runtime.enter();
        let reader = Backend::open_temporary().expect("second launch");
        let (connection, console) = connect(&runtime, &reader, &self.path);
        match runtime.block_on(reader.run_console(
            CommandId::new(),
            connection,
            console,
            ConsoleRun {
                sql: "SELECT parent FROM child".into(),
                target: RunTarget::All,
                parameters: Vec::new(),
                explain: false,
            },
        )) {
            Ok(CommandOutcome::Executed { rows, .. }) => rows,
            other => panic!("the count runs: {other:?}"),
        }
    }
}

/// A webview subscribed to the shutdown channel. It confirms drafts flushed,
/// and acknowledges `resolveTransactions` when `acknowledges`. Returns every
/// signal it received.
fn webview(backend: &Backend, acknowledges: bool) -> Arc<Mutex<Vec<Value>>> {
    let received = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&received);
    let inner = Arc::downgrade(&backend.inner);
    let window = backend.test_window();
    backend.subscribe_shutdown(
        window,
        Channel::new(move |body: InvokeResponseBody| {
            let InvokeResponseBody::Json(json) = body else {
                panic!("the shutdown channel sends JSON");
            };
            let signal: Value = serde_json::from_str(&json).expect("a JSON signal");
            let kind = signal["type"].as_str().unwrap_or_default().to_owned();
            log.lock().push(signal);
            if let Some(inner) = inner.upgrade() {
                let backend = Backend { inner };
                match kind.as_str() {
                    "flushDrafts" => backend.shutdown_flushed(window),
                    "resolveTransactions" if acknowledges => backend.shutdown_acknowledged(window),
                    _ => {}
                }
            }
            Ok(())
        }),
    );
    received
}

fn kinds(received: &Mutex<Vec<Value>>) -> Vec<String> {
    received
        .lock()
        .iter()
        .map(|signal| signal["type"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn without_an_open_transaction_the_exit_proceeds_without_asking() {
    let bench = bench();
    let received = webview(&bench.backend, true);
    assert_eq!(bench.exit_step(), ExitStep::Proceed);
    assert!(
        received.lock().is_empty(),
        "the webview is not asked: the exit is the one it was"
    );
}

#[test]
fn an_open_transaction_holds_the_exit_and_is_named() {
    let bench = bench();
    let received = webview(&bench.backend, true);
    bench.ok("BEGIN");
    bench.ok("INSERT INTO parent VALUES (1)");

    assert!(matches!(bench.exit_step(), ExitStep::Asked(_)));
    assert!(
        !bench.backend.shutdown_finished(),
        "nothing flushed, nothing recorded"
    );
    let signal = received
        .lock()
        .first()
        .cloned()
        .expect("the webview is asked");
    assert_eq!(signal["type"], "resolveTransactions");
    let listed = &signal["transactions"][0];
    assert_eq!(listed["session"], bench.console.to_string());
    assert_eq!(listed["connection"], bench.connection.to_string());
    assert_eq!(listed["connectionName"], "orders");
    assert_eq!(listed["environment"], "local");
    assert_eq!(listed["state"], "open");
    assert_eq!(
        signal["transactions"].as_array().map(Vec::len),
        Some(1),
        "the catalog's session is not a console"
    );

    // Asked again while the dialog is open — ⌘Q again, or the dialog once
    // every session is idle — it lists again, from the backend's state.
    assert!(matches!(bench.exit_step(), ExitStep::Asked(_)));
    assert_eq!(
        kinds(&received),
        ["resolveTransactions", "resolveTransactions"]
    );
}

#[test]
fn commit_then_idle_lets_the_exit_proceed_and_the_rows_stay() {
    let bench = bench();
    webview(&bench.backend, true);
    bench.ok("BEGIN");
    bench.ok("INSERT INTO parent VALUES (1)");
    bench.ok("INSERT INTO child VALUES (1)");
    assert!(matches!(bench.exit_step(), ExitStep::Asked(_)));

    bench.ok("COMMIT");
    assert_eq!(bench.exit_step(), ExitStep::Proceed);
    assert_eq!(bench.committed_children(), 1);
}

#[test]
fn rollback_then_idle_lets_the_exit_proceed_and_the_rows_go() {
    let bench = bench();
    webview(&bench.backend, true);
    bench.ok("BEGIN");
    bench.ok("INSERT INTO parent VALUES (1)");
    bench.ok("INSERT INTO child VALUES (1)");
    assert!(matches!(bench.exit_step(), ExitStep::Asked(_)));

    bench.ok("ROLLBACK");
    assert_eq!(bench.exit_step(), ExitStep::Proceed);
    assert_eq!(bench.committed_children(), 0);
}

/// A deferred foreign key fails at `COMMIT`, and SQLite keeps the
/// transaction open: the server's refusal holds the exit.
#[test]
fn a_refused_commit_keeps_the_exit_held() {
    let bench = bench();
    let received = webview(&bench.backend, true);
    bench.ok("BEGIN");
    bench.ok("INSERT INTO child VALUES (42)");
    assert!(matches!(bench.exit_step(), ExitStep::Asked(_)));

    let refused = bench
        .run("COMMIT")
        .expect_err("the deferred key refuses the commit");
    assert!(
        refused.message.contains("FOREIGN KEY"),
        "the server's message reaches the dialog: {}",
        refused.message
    );
    assert!(matches!(bench.exit_step(), ExitStep::Asked(_)));
    let last = received.lock().last().cloned().expect("asked again");
    assert_eq!(last["transactions"][0]["state"], "open");
    assert_eq!(bench.committed_children(), 0);
}

/// A `COMMIT` whose outcome is unknown — here the session left `Unknown`,
/// as a lost connection or a missed event leaves it — is listed again for the
/// user to decide, and the exit never sends anything to the session itself.
#[test]
fn an_unknown_outcome_is_listed_again_and_never_replayed() {
    let bench = bench();
    let received = webview(&bench.backend, true);
    bench.ok("BEGIN");
    bench.ok("INSERT INTO parent VALUES (7)");
    let mut events = bench.backend.subscribe();
    // A `COMMIT` sent and not answered: its outcome is not known.
    let consoles = &bench.backend.inner.workbench.consoles;
    let _in_flight = consoles.run_on(bench.console);
    // A late event of the previous statement does not make it look settled.
    consoles.apply(Ok(oxyn_exec::ExecEvent::new(
        CommandId::new(),
        Some(bench.connection),
        oxyn_core::Event::TransactionState {
            session: bench.console,
            state: oxyn_core::TransactionState::Idle,
        },
    )));

    assert!(matches!(bench.exit_step(), ExitStep::Asked(_)));
    assert!(matches!(bench.exit_step(), ExitStep::Asked(_)));
    for signal in received.lock().iter() {
        assert_eq!(signal["transactions"][0]["state"], "unknown");
    }
    assert!(
        matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "the exit dispatched nothing: no statement was replayed"
    );
    assert_eq!(bench.committed_children(), 0);
}

/// Without an acknowledgement within the grace, the exit goes on: the close
/// of the sessions rolls the transaction back.
#[test]
fn a_silent_webview_does_not_hold_the_exit_and_the_transaction_is_rolled_back() {
    let bench = bench();
    let received = webview(&bench.backend, false);
    bench.ok("BEGIN");
    bench.ok("INSERT INTO parent VALUES (1)");
    bench.ok("INSERT INTO child VALUES (1)");

    assert_eq!(bench.exit_step(), ExitStep::Proceed);
    assert_eq!(kinds(&received), ["resolveTransactions"]);
    assert!(bench.backend.begin_shutdown());
    {
        let _guard = bench.runtime.enter();
        bench
            .runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(30), bench.backend.shutdown()).await
            })
            .expect("the shutdown is bounded");
    }
    assert!(bench.backend.shutdown_finished());
    assert!(bench.backend.inner.executor.sessions().is_empty());
    assert_eq!(bench.committed_children(), 0);
}

#[test]
fn without_any_webview_the_exit_proceeds() {
    let bench = bench();
    bench.ok("BEGIN");
    bench.ok("INSERT INTO parent VALUES (1)");
    assert_eq!(bench.exit_step(), ExitStep::Proceed);
}

#[test]
fn cancel_abandons_the_exit_and_leaves_the_transaction_open() {
    let bench = bench();
    let received = webview(&bench.backend, true);
    // Outside an exit, neither command does anything.
    bench.backend.cancel_exit(bench.backend.test_window());
    bench
        .backend
        .shutdown_acknowledged(bench.backend.test_window());
    assert!(received.lock().is_empty());

    bench.ok("BEGIN");
    bench.ok("INSERT INTO parent VALUES (1)");
    assert!(matches!(bench.exit_step(), ExitStep::Asked(_)));
    bench.backend.cancel_exit(bench.backend.test_window());
    assert_eq!(kinds(&received), ["resolveTransactions", "exitCancelled"]);
    assert!(!bench.backend.shutdown_finished());
    assert!(
        bench.backend.begin_shutdown(),
        "the ordered shutdown never began"
    );
    bench.ok("INSERT INTO child VALUES (1)");
    bench.ok("COMMIT");
    assert_eq!(bench.committed_children(), 1);
}

/// The Dock's Quit cannot wait (ADR-0040): one warning per open transaction,
/// read without waiting, and the close is recorded as before.
#[test]
fn a_forced_exit_warns_of_each_open_transaction_and_records_the_close() {
    let bench = bench();
    bench.ok("BEGIN");
    bench.ok("INSERT INTO parent VALUES (1)");
    assert_eq!(bench.backend.warn_forced_exit_transactions(), 1);
    let _guard = bench.runtime.enter();
    assert!(bench.backend.close_on_forced_exit(Duration::from_secs(10)));
}
