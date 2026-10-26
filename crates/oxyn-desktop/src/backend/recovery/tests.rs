use super::*;

#[test]
fn a_clean_close_waits_for_local_writes_and_is_not_offered_recovery() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let backend = Backend::assemble(
        Arc::clone(&store),
        Arc::new(oxyn_secrets::MemorySecretStore::new()),
    )
    .expect("backend");
    assert!(
        !backend.recovery_status(backend.test_window()).abnormal,
        "a first launch speaks of no crash"
    );
    assert!(
        !backend
            .recovery_status(backend.test_window())
            .unresolved_write,
        "nor of a write it never saw"
    );

    let write = backend
        .local_write(&Command::WriteDocument {
            workspace: backend.inner.executor.workspace(),
            document: oxyn_core::DocumentId::new(),
            text: String::new(),
        })
        .expect("a document write is a local write");
    let closing = runtime.spawn({
        let backend = backend.clone();
        async move { backend.close_session_after_local_writes().await }
    });
    runtime.block_on(tokio::time::sleep(Duration::from_millis(50)));
    assert!(
        !closing.is_finished(),
        "the close waits for the pending write"
    );
    drop(write);
    runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(5), closing).await })
        .expect("closes once the write ends")
        .expect("task joins");

    let workspace = backend.inner.executor.workspace();
    let (_, previous) = store.sessions().begin(workspace).expect("next launch");
    assert_eq!(previous, PreviousShutdown::Clean);
}

/// The reported defect: an ordinary close, then recovery at every launch.
#[test]
fn an_ordinary_shutdown_is_not_offered_recovery_at_the_next_launch() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

    let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
    assert!(first.begin_shutdown());
    runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(30), first.shutdown()).await })
        .expect("the shutdown is bounded");
    assert!(first.shutdown_finished());
    // Long enough for an unrecorded close to read as a crash.
    exit_and_age(&store, first);

    let next = Backend::assemble(store, secrets).expect("relaunch");
    assert!(!next.recovery_status(next.test_window()).abnormal);
}

/// A webview subscribed to the shutdown. `confirms` says whether it answers
/// `FlushDrafts`; `reachable` whether the signal reaches it at all. Holds
/// the backend weakly: the backend owns the channel.
fn webview(backend: &Backend, confirms: bool, reachable: bool) -> Arc<AtomicUsize> {
    let asked = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&asked);
    let inner = Arc::downgrade(&backend.inner);
    let window = backend.test_window();
    backend.subscribe_shutdown(
        window,
        Channel::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            if !reachable {
                return Err(tauri::Error::FailedToReceiveMessage);
            }
            if let (true, Some(inner)) = (confirms, inner.upgrade()) {
                Backend { inner }.shutdown_flushed(window);
            }
            Ok(())
        }),
    );
    asked
}

/// Runs the shutdown of `launch`, bounded well under `flush_grace` when the
/// webview confirms, then ends the launch.
fn shut_down(
    runtime: &tokio::runtime::Runtime,
    store: &Store,
    launch: Backend,
    flush_grace: Duration,
) {
    runtime
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(30), launch.shutdown_within(flush_grace)).await
        })
        .expect("the shutdown is bounded");
    assert!(launch.shutdown_finished());
    exit_and_age(store, launch);
}

/// The webview confirms its flush: the shutdown goes on at once, not at
/// the end of the grace, and the next launch speaks of no crash.
#[test]
fn a_confirmed_flush_records_the_close_without_waiting_for_the_grace() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

    let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
    let asked = webview(&first, true, true);
    // Longer than the outer bound: waiting for it would fail the test.
    shut_down(&runtime, &store, first, Duration::from_secs(3600));
    assert_eq!(
        asked.load(Ordering::SeqCst),
        1,
        "the webview was asked once"
    );

    let next = Backend::assemble(store, secrets).expect("relaunch");
    assert!(!next.recovery_status(next.test_window()).abnormal);
}

/// A webview that never confirms, or cannot be reached: the close is
/// recorded all the same (ADR-0040), after the grace, and the running
/// sessions are still closed. Recovery is not offered for drafts at most
/// 250 ms old.
#[test]
fn an_unconfirmed_flush_still_records_the_close_after_the_grace() {
    for reachable in [true, false] {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

        let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
        let asked = webview(&first, false, reachable);
        let sessions = Arc::clone(&first.inner);
        shut_down(&runtime, &store, first, Duration::from_millis(100));
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert!(sessions.executor.sessions().is_empty());
        assert!(sessions.executor.running().is_empty());
        drop(sessions);

        let next = Backend::assemble(store, secrets).expect("relaunch");
        assert!(
            !next.recovery_status(next.test_window()).abnormal,
            "reachable: {reachable}; an unconfirmed flush is not a crash"
        );
    }
}

/// The confirmation arrives once the close is recorded: it changes
/// nothing, and the next launch speaks of no crash.
#[test]
fn a_late_flush_confirmation_changes_nothing() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

    let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
    webview(&first, false, true);
    runtime
        .block_on(async {
            tokio::time::timeout(
                Duration::from_secs(30),
                first.shutdown_within(Duration::from_millis(100)),
            )
            .await
        })
        .expect("the shutdown is bounded");
    first.shutdown_flushed(first.test_window());
    assert!(first.shutdown_finished());
    exit_and_age(&store, first);

    let next = Backend::assemble(store, secrets).expect("relaunch");
    assert!(!next.recovery_status(next.test_window()).abnormal);
}

/// The Dock's Quit: macOS ends the loop without `ExitRequested`, and the
/// close is recorded from `RunEvent::Exit` (ADR-0040).
#[test]
fn a_forced_exit_records_the_close_and_is_not_offered_recovery() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

    let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
    assert!(first.close_on_forced_exit(Duration::from_secs(10)));
    exit_and_age(&store, first);

    let next = Backend::assemble(store, secrets).expect("relaunch");
    assert!(!next.recovery_status(next.test_window()).abnormal);
}

/// A write still running when macOS ends the process: the close is never
/// written over it, and the wait does not outlast its grace.
#[test]
fn a_forced_exit_over_a_pending_write_is_bounded_and_leaves_the_close_unwritten() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

    let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
    let write = first
        .local_write(&Command::WriteDocument {
            workspace: first.inner.executor.workspace(),
            document: oxyn_core::DocumentId::new(),
            text: String::new(),
        })
        .expect("a document write is a local write");
    let grace = Duration::from_millis(100);
    let started = std::time::Instant::now();
    assert!(!first.close_on_forced_exit(grace));
    assert!(
        started.elapsed() < grace + Duration::from_secs(5),
        "the main thread waits for the grace, not for the write"
    );
    // The process ends with the write still running.
    exit_and_age(&store, first);

    let next = Backend::assemble(store, secrets).expect("relaunch");
    assert!(
        next.recovery_status(next.test_window()).abnormal,
        "a close nobody could confirm offers recovery"
    );
    drop(write);
}

/// The case recovery exists for: a process killed without its close.
#[test]
fn a_killed_launch_is_offered_recovery_once() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

    // `kill -9`: no shutdown, and the heartbeat stops with the process.
    let killed = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
    exit_and_age(&store, killed);

    let relaunch = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("relaunch");
    assert!(relaunch.recovery_status(relaunch.test_window()).abnormal);
    runtime.block_on(relaunch.shutdown());
    exit_and_age(&store, relaunch);

    let next = Backend::assemble(store, secrets).expect("the launch after");
    assert!(
        !next.recovery_status(next.test_window()).abnormal,
        "a crash already announced does not come back after an ordinary close"
    );
}

/// Ends a launch, then ages its heartbeat past the abandonment threshold.
fn exit_and_age(store: &Store, launch: Backend) {
    let session = launch.inner.workbench.local.session;
    drop(launch);
    store
        .mark_session_stale_for_tests(session)
        .expect("aged heartbeat");
}

#[test]
fn an_abandoned_session_is_reported_as_abnormal() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = super::super::current_workspace(&store).expect("workspace");
    let (crashed, _) = store.sessions().begin(workspace).expect("first launch");
    store
        .mark_session_stale_for_tests(crashed)
        .expect("aged heartbeat");
    let backend = Backend::assemble(store, Arc::new(oxyn_secrets::MemorySecretStore::new()))
        .expect("backend");
    assert!(backend.recovery_status(backend.test_window()).abnormal);
    assert!(
        !backend
            .recovery_status(backend.test_window())
            .unresolved_write,
        "a crash alone is not a write with an unknown outcome"
    );
}

#[test]
fn an_expired_write_in_history_is_reported_as_unresolved() {
    use oxyn_core::{Actor, OxynError, QueryLanguage, StatementIntent};
    use oxyn_store::history::HistoryRecord;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let expired = HistoryRecord::new(
        &Actor::Human,
        QueryLanguage::SQL,
        "INSERT INTO t VALUES (1)",
    )
    .with_intent(StatementIntent::Write)
    .failed(&OxynError::Timeout {
        after: Duration::from_secs(30),
    });
    store.history().record(&expired).expect("recorded");
    let backend = Backend::assemble(store, Arc::new(oxyn_secrets::MemorySecretStore::new()))
        .expect("backend");
    let status = backend.recovery_status(backend.test_window());
    assert!(status.unresolved_write);
    assert!(!status.abnormal, "and says nothing of a crash");
}

#[test]
fn a_reconciled_write_no_longer_warns_at_the_next_launch() {
    use oxyn_core::{Actor, OxynError, QueryLanguage, StatementIntent};
    use oxyn_store::history::HistoryRecord;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let entry = store
        .history()
        .record(
            &HistoryRecord::new(
                &Actor::Human,
                QueryLanguage::SQL,
                "INSERT INTO t VALUES (1)",
            )
            .with_intent(StatementIntent::Write)
            .failed(&OxynError::Timeout {
                after: Duration::from_secs(30),
            }),
        )
        .expect("recorded");
    let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());
    let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("backend");
    assert!(first.recovery_status(first.test_window()).unresolved_write);

    runtime
        .block_on(first.reconcile_history_entry(entry))
        .expect("the user inspected the server");
    assert!(
        first.recovery_status(first.test_window()).unresolved_write,
        "this launch keeps what it observed at startup"
    );

    let next = Backend::assemble(store, secrets).expect("next launch");
    assert!(!next.recovery_status(next.test_window()).unresolved_write);
}

#[test]
fn the_shutdown_stops_running_statements_and_closes_their_sessions() {
    use crate::ipc::{CommandOutcome, ConnectResponse, ConnectionDraft};
    use oxyn_core::{CommandId, ConnectionId, Environment, SessionId};

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "shutdown".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: std::collections::BTreeMap::new(),
    };
    let Ok(ConnectResponse::Open(open)) =
        runtime.block_on(backend.connect(CommandId::new(), draft))
    else {
        panic!("a local in-memory connection opens without approval");
    };
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let session: SessionId = open.session.parse().expect("session id");
    let id = CommandId::new();

    // Its only row arrives after a billion steps: without a cancel reaching
    // the engine, it runs for minutes.
    let running = runtime.spawn({
        let backend = backend.clone();
        async move {
            backend
                .execute(
                    id,
                    connection,
                    session,
                    "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n \
                         WHERE x < 1000000000) SELECT count(*) FROM n"
                        .into(),
                )
                .await
        }
    });
    runtime.block_on(tokio::time::sleep(Duration::from_millis(200)));
    assert!(!running.is_finished(), "the statement is still running");

    runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(30), backend.shutdown()).await })
        .expect("the shutdown is bounded");

    let outcome = runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(10), running).await })
        .expect("the statement stops with the application")
        .expect("task joins");
    assert!(
        !matches!(outcome, Ok(CommandOutcome::Executed { complete: true, .. })),
        "the statement was stopped, not run to its end"
    );
    assert!(backend.inner.executor.sessions().is_empty());
    assert!(backend.inner.executor.running().is_empty());
}

#[test]
fn reads_are_not_counted_as_local_writes() {
    assert!(!is_local_write(&Command::ReadHistoryEntry { entry: 1 }));
}
