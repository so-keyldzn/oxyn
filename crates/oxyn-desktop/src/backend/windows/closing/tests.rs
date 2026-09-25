//! Two windows on one SQLite file (ADR-0043): the exit asks each window about
//! its own consoles only, the close of a window that is not the last closes
//! its sessions and nothing of the other's, and the last one's close quits.
//!
//! Each webview is a channel that records what it is sent and, when told to,
//! answers. Sessions are claimed as `commands/` claims them: by the window
//! that opened them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use oxyn_core::{CommandId, ConnectionId, Environment, SessionId};
use parking_lot::Mutex;
use serde_json::Value;
use tauri::ipc::{Channel, InvokeResponseBody};

use crate::backend::Backend;
use crate::backend::exit::ExitStep;
use crate::backend::windows::{CloseStep, Closing, WindowKey};
use crate::ipc::consoles::{ConsoleRun, RunTarget};
use crate::ipc::{CommandOutcome, ConnectResponse, ConnectionDraft};

/// Short for a test, long enough for a loaded machine to deliver an answer
/// sent from inside the channel.
const GRACE: Duration = Duration::from_millis(300);

/// One window: its key, its catalog and console sessions, and what its
/// webview received.
struct Pane {
    key: WindowKey,
    catalog: SessionId,
    console: SessionId,
    received: Arc<Mutex<Vec<Value>>>,
}

struct Bench {
    runtime: tokio::runtime::Runtime,
    backend: Backend,
    connection: ConnectionId,
    left: Pane,
    right: Pane,
    _directory: tempfile::TempDir,
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// A webview: records every signal of both its channels and answers — flushed
/// drafts always, acknowledgements when `answers`.
fn webview(backend: &Backend, key: WindowKey, answers: bool) -> Arc<Mutex<Vec<Value>>> {
    let received = Arc::new(Mutex::new(Vec::new()));
    let respond = move |log: Arc<Mutex<Vec<Value>>>,
                        inner: std::sync::Weak<crate::backend::Inner>| {
        move |body: InvokeResponseBody| {
            let InvokeResponseBody::Json(json) = body else {
                panic!("a channel sends JSON");
            };
            let signal: Value = serde_json::from_str(&json).expect("a JSON signal");
            let kind = signal["type"].as_str().unwrap_or_default().to_owned();
            log.lock().push(signal);
            if let Some(inner) = inner.upgrade() {
                let backend = Backend { inner };
                match kind.as_str() {
                    "flushDrafts" => backend.shutdown_flushed(key),
                    "resolveTransactions" | "closeRequested" if answers => {
                        backend.shutdown_acknowledged(key);
                    }
                    _ => {}
                }
            }
            Ok(())
        }
    };
    let inner = Arc::downgrade(&backend.inner);
    backend.subscribe_shutdown(
        key,
        Channel::new(respond(Arc::clone(&received), inner.clone())),
    );
    backend
        .inner
        .windows
        .subscribe_signals(key, Channel::new(respond(Arc::clone(&received), inner)));
    received
}

fn kinds(received: &Mutex<Vec<Value>>) -> Vec<String> {
    received
        .lock()
        .iter()
        .map(|signal| signal["type"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// Two windows on the same SQLite file: the left one connected, the right
/// one reopened the saved connection on sessions of its own. On `Local`, the
/// file holds an empty table.
fn bench(left_answers: bool, right_answers: bool) -> Bench {
    bench_on(Environment::Local, left_answers, right_answers)
}

fn bench_on(environment: Environment, left_answers: bool, right_answers: bool) -> Bench {
    let runtime = runtime();
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("windows.sqlite");
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let left_key = backend.reserve_window(true).expect("a first window");
    let right_key = backend.reserve_window(false).expect("a second window");
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "orders".into(),
        environment,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), path.to_string_lossy().into_owned())]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    };
    let left_open = match runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    {
        ConnectResponse::Open(open) => open,
        // Production: the host of these tests confirms at once.
        ConnectResponse::Approval { command, .. } => {
            match runtime
                .block_on(backend.decide_connection(command.parse().expect("an id"), true))
                .expect("approved")
            {
                Some(ConnectResponse::Open(open)) => open,
                _ => panic!("an approved connection opens"),
            }
        }
    };
    let connection: ConnectionId = left_open.connection.parse().expect("connection id");
    let right_open = runtime
        .block_on(backend.reconnect(CommandId::new(), connection))
        .expect("reopens");
    let pane = |key: WindowKey, open: &crate::ipc::OpenConnection, answers: bool| {
        let catalog: SessionId = open.session.parse().expect("catalog session");
        let console: SessionId = open.console.session.parse().expect("console session");
        backend
            .inner
            .windows
            .claim_session(key, connection, catalog);
        backend
            .inner
            .windows
            .claim_session(key, connection, console);
        Pane {
            key,
            catalog,
            console,
            received: webview(&backend, key, answers),
        }
    };
    let left = pane(left_key, &left_open, left_answers);
    let right = pane(right_key, &right_open, right_answers);
    drop(_guard);
    let bench = Bench {
        runtime,
        backend,
        connection,
        left,
        right,
        _directory: directory,
    };
    if environment == Environment::Local {
        bench.ok(&bench.left, "CREATE TABLE parent (id INTEGER PRIMARY KEY)");
    }
    bench
}

impl Bench {
    fn ok(&self, pane: &Pane, sql: &str) {
        let _guard = self.runtime.enter();
        let outcome = self.runtime.block_on(self.backend.run_console(
            CommandId::new(),
            self.connection,
            pane.console,
            ConsoleRun {
                sql: sql.into(),
                target: RunTarget::All,
                parameters: Vec::new(),
                explain: false,
            },
        ));
        assert!(
            matches!(outcome, Ok(CommandOutcome::Executed { .. })),
            "{sql} runs: {outcome:?}"
        );
    }

    fn exit_step(&self) -> ExitStep {
        let _guard = self.runtime.enter();
        self.runtime.block_on(self.backend.exit_step_within(GRACE))
    }

    fn close_step(&self, pane: &Pane) -> CloseStep {
        let _guard = self.runtime.enter();
        self.runtime
            .block_on(self.backend.window_close_step_within(pane.key, GRACE))
    }

    fn release(&self, pane: &Pane) {
        let _guard = self.runtime.enter();
        self.runtime.block_on(self.backend.release_window(pane.key));
    }

    fn is_open(&self, session: SessionId) -> bool {
        self.backend
            .inner
            .executor
            .sessions()
            .get(session)
            .is_some_and(|slot| slot.is_open())
    }
}

/// The sessions a `resolveTransactions` signal lists.
fn listed(signal: &Value) -> Vec<String> {
    signal["transactions"]
        .as_array()
        .map(|all| {
            all.iter()
                .map(|transaction| {
                    transaction["session"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_exit_asks_each_window_about_its_own_transactions_only() {
    let bench = bench(true, true);
    bench.ok(&bench.left, "BEGIN");
    bench.ok(&bench.left, "INSERT INTO parent VALUES (1)");
    // SQLite takes one writer at a time: the right window's transaction is
    // open, and reads.
    bench.ok(&bench.right, "BEGIN");
    bench.ok(&bench.right, "SELECT * FROM parent");

    let ExitStep::Asked(mut asked) = bench.exit_step() else {
        panic!("both windows hold a transaction");
    };
    asked.sort();
    let mut both = vec![bench.left.key, bench.right.key];
    both.sort();
    assert_eq!(asked, both, "the windows that come to the front");

    for (pane, other) in [(&bench.left, &bench.right), (&bench.right, &bench.left)] {
        let signal = pane.received.lock().first().cloned().expect("asked");
        assert_eq!(signal["type"], "resolveTransactions");
        assert_eq!(signal["scope"], "application");
        assert_eq!(listed(&signal), [pane.console.to_string()]);
        assert!(!listed(&signal).contains(&other.console.to_string()));
    }

    // Cancel in one window closes the dialog in both.
    bench.backend.cancel_exit(bench.right.key);
    assert_eq!(
        kinds(&bench.left.received),
        ["resolveTransactions", "exitCancelled"]
    );
    assert_eq!(
        kinds(&bench.right.received),
        ["resolveTransactions", "exitCancelled"]
    );
    assert!(!bench.backend.shutdown_finished());
}

#[test]
fn a_session_no_window_claims_is_given_to_the_window_asked_about_it() {
    let bench = bench(true, true);
    bench.ok(&bench.right, "BEGIN");
    bench
        .backend
        .inner
        .windows
        .release_session(bench.right.console);
    let ExitStep::Asked(asked) = bench.exit_step() else {
        panic!("the transaction holds the exit");
    };
    assert_eq!(asked, vec![bench.left.key], "the first window is asked");
    // Its dialog's `Rollback` goes through `run_console`, which now accepts it.
    assert!(
        bench
            .backend
            .inner
            .windows
            .check_session(bench.left.key, bench.right.console)
            .is_ok()
    );
}

#[test]
fn a_silent_window_does_not_hold_the_exit_of_the_others() {
    let bench = bench(true, false);
    bench.ok(&bench.left, "BEGIN");
    bench.ok(&bench.left, "INSERT INTO parent VALUES (1)");
    // SQLite takes one writer at a time: the right window's transaction is
    // open, and reads.
    bench.ok(&bench.right, "BEGIN");
    bench.ok(&bench.right, "SELECT * FROM parent");
    assert_eq!(bench.exit_step(), ExitStep::Asked(vec![bench.left.key]));

    // The left one resolves: the silent right one no longer holds anything.
    bench.ok(&bench.left, "ROLLBACK");
    assert_eq!(bench.exit_step(), ExitStep::Proceed);
}

#[test]
fn the_last_window_close_is_the_exit() {
    let bench = bench(true, true);
    assert_eq!(bench.close_step(&bench.left), CloseStep::Asked);
    // The left one is still deciding: it counts, the right one is not last.
    assert_ne!(bench.close_step(&bench.right), CloseStep::Quit);
    assert!(bench.backend.confirm_window_close(bench.left.key));
    // Confirmed, the left one no longer counts.
    assert_eq!(bench.close_step(&bench.right), CloseStep::Quit);
}

#[test]
fn closing_a_window_asks_it_and_closes_its_sessions_only() {
    let bench = bench(true, true);
    assert_eq!(bench.close_step(&bench.right), CloseStep::Asked);
    assert_eq!(kinds(&bench.right.received), ["closeRequested"]);
    assert!(
        bench.left.received.lock().is_empty(),
        "the other window is not asked"
    );
    // Asked twice while the dialog is open: nothing new is sent.
    assert_eq!(bench.close_step(&bench.right), CloseStep::Asked);

    assert!(bench.backend.confirm_window_close(bench.right.key));
    assert!(!bench.backend.confirm_window_close(bench.right.key), "once");
    bench.release(&bench.right);
    assert!(!bench.is_open(bench.right.console));
    assert!(!bench.is_open(bench.right.catalog));
    assert!(
        bench.is_open(bench.left.console),
        "the other window's console stays"
    );
    assert!(bench.is_open(bench.left.catalog));
    // The connection is still the left window's.
    assert!(
        bench
            .backend
            .inner
            .windows
            .holds(bench.left.key, bench.connection)
    );
    bench.ok(&bench.left, "SELECT 1");
}

#[test]
fn a_window_transaction_holds_its_close_and_only_its_own_is_listed() {
    let bench = bench(true, true);
    bench.ok(&bench.left, "BEGIN");
    bench.ok(&bench.left, "INSERT INTO parent VALUES (1)");
    // SQLite takes one writer at a time: the right window's transaction is
    // open, and reads.
    bench.ok(&bench.right, "BEGIN");
    bench.ok(&bench.right, "SELECT * FROM parent");

    assert_eq!(bench.close_step(&bench.right), CloseStep::Asked);
    let signal = bench.right.received.lock().first().cloned().expect("asked");
    assert_eq!(signal["type"], "resolveTransactions");
    assert_eq!(signal["scope"], "window");
    assert_eq!(listed(&signal), [bench.right.console.to_string()]);
    assert!(bench.left.received.lock().is_empty());

    // Rolled back from the dialog, the close is asked again: no transaction
    // left, the window is asked about its consoles.
    bench.ok(&bench.right, "ROLLBACK");
    assert_eq!(bench.close_step(&bench.right), CloseStep::Asked);
    assert_eq!(
        kinds(&bench.right.received),
        ["resolveTransactions", "closeRequested"]
    );
    // Cancel keeps the window, and the left window's transaction was never
    // touched.
    bench.backend.cancel_exit(bench.right.key);
    assert_eq!(
        bench.backend.inner.windows.closing(bench.right.key),
        Closing::Open
    );
    bench.ok(&bench.left, "COMMIT");
}

#[test]
fn a_silent_window_closes_without_its_documents_and_rolls_back_its_sessions() {
    let bench = bench(true, false);
    // SQLite takes one writer at a time: the right window's transaction is
    // open, and reads.
    bench.ok(&bench.right, "BEGIN");
    bench.ok(&bench.right, "SELECT * FROM parent");
    assert_eq!(bench.close_step(&bench.right), CloseStep::Close);
    bench.release(&bench.right);
    assert!(!bench.is_open(bench.right.console));
    assert!(bench.is_open(bench.left.console));
}

#[test]
fn an_approval_pending_in_a_closed_window_is_rejected() {
    let bench = bench_on(Environment::Production, true, true);
    let _guard = bench.runtime.enter();
    let outcome = bench
        .runtime
        .block_on(bench.backend.execute(
            CommandId::new(),
            bench.connection,
            bench.right.console,
            "CREATE TABLE held (id INTEGER)".into(),
        ))
        .expect("the policy answers");
    let CommandOutcome::NeedsApproval { command, .. } = outcome else {
        panic!("a production write waits for a decision, got {outcome:?}");
    };
    let command: CommandId = command.parse().expect("a command id");
    // Kept by its window while it waits, as `commands/` keeps it.
    bench
        .backend
        .inner
        .windows
        .claim_command(bench.right.key, command)
        .expect("free");
    assert!(
        bench
            .backend
            .inner
            .executor
            .approvals()
            .peek(command)
            .is_some()
    );

    bench.release(&bench.right);
    assert!(
        bench
            .backend
            .inner
            .executor
            .approvals()
            .peek(command)
            .is_none(),
        "rejected with its window"
    );
    // Nothing ran: the table does not exist.
    let read = bench.runtime.block_on(bench.backend.run_console(
        CommandId::new(),
        bench.connection,
        bench.left.console,
        ConsoleRun {
            sql: "SELECT * FROM held".into(),
            target: RunTarget::All,
            parameters: Vec::new(),
            explain: false,
        },
    ));
    assert!(read.is_err(), "the rejected write did not run: {read:?}");
}

#[test]
fn the_ordered_shutdown_asks_every_window_to_flush() {
    let bench = bench(true, true);
    let _guard = bench.runtime.enter();
    assert!(bench.backend.begin_shutdown());
    bench
        .runtime
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(30), bench.backend.shutdown()).await
        })
        .expect("the shutdown is bounded");
    assert_eq!(kinds(&bench.left.received), ["flushDrafts"]);
    assert_eq!(kinds(&bench.right.received), ["flushDrafts"]);
    assert!(bench.backend.shutdown_finished());
}
