//! Cancellation is observed at the disposable server while a dump waits on a table lock.
use super::*;
use crate::ipc::consoles::{ConsoleRun, RunTarget};
use std::time::Duration;

async fn scalar(
    backend: &Backend,
    connection: ConnectionId,
    session: SessionId,
    sql: String,
) -> String {
    let crate::ipc::CommandOutcome::Executed { result, .. } = backend
        .execute(CommandId::new(), connection, session, sql)
        .await
        .expect("server observation")
    else {
        panic!("SELECT result")
    };
    let crate::ipc::results::ResultWindow::Page(page) = backend
        .read_result_page(connection, result.parse().expect("result"), 0, 1)
        .await
        .expect("bounded observation")
    else {
        panic!("retained page")
    };
    page.rows
        .first()
        .and_then(|row| row.first())
        .and_then(|cell| match cell {
            crate::ipc::Cell::Text(value) => Some(value.clone()),
            _ => None,
        })
        .expect("count value")
}
pub(super) async fn check(
    backend: &Backend,
    open: &crate::ipc::OpenConnection,
    owner: WindowId,
    tool: NativeGrantId,
    directory: &Path,
) {
    let connection = open.connection.parse().expect("connection");
    let session = open.session.parse().expect("catalog session");
    let console = open.console.session.parse().expect("fixture console");
    // PostgreSQL console transactions are deliberately not offered by this
    // driver. One fixture DO statement owns an implicit transaction and lock.
    let lock_command = CommandId::new();
    let locker = backend.clone();
    let locking = tokio::spawn(async move {
        locker.run_console(lock_command, connection, console, ConsoleRun {
            sql: "DO $fixture$ BEGIN LOCK TABLE native_fixture IN ACCESS EXCLUSIVE MODE; PERFORM pg_sleep(30); END $fixture$".into(),
            target: RunTarget::All, parameters: vec![], explain: false,
        }).await
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if scalar(backend, connection, session, "SELECT count(*) FROM pg_locks WHERE relation='native_fixture'::regclass AND mode='AccessExclusiveLock' AND granted".into()).await == "1" { break; }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.expect("fixture lock observed on server");
    let path = directory.join("cancelled.dump");
    let file = backend
        .inner
        .native_runner
        .select_output(
            owner,
            oxyn_transfer::SourceBinding {
                connection,
                session,
            },
            NativeTransferFormat::CustomArchive,
            &path,
        )
        .expect("new output");
    let review = backend
        .prepare_native_transfer(
            CommandId::new(),
            NativeTransferRequest {
                connection,
                session,
                owner,
                operation: NativeTransferOperation::Export,
                format: NativeTransferFormat::CustomArchive,
                tool,
                file,
                no_owner: false,
                no_privileges: false,
            },
        )
        .await
        .expect("prepared dump");
    let command = CommandId::new();
    let child = backend.clone();
    let running = tokio::spawn(async move {
        child
            .start_native_transfer(command, connection, owner, review.job.id)
            .await
    });
    let predicate = format!("application_name='oxyn-native-{command}'");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if scalar(backend, connection, session, format!("SELECT count(*) FROM pg_stat_activity WHERE {predicate} AND wait_event_type='Lock'")).await == "1" { break; }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.expect("dump is observed waiting on server lock");
    assert!(backend.cancel(command));
    let stopped = tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("bounded cancellation")
        .expect("joined native caller")
        .expect("terminal observation");
    assert_eq!(stopped.outcome, Some(NativeTransferOutcome::Cancelled));
    assert!(!path.exists(), "cancelled export is not published");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if scalar(
                backend,
                connection,
                session,
                format!("SELECT count(*) FROM pg_stat_activity WHERE {predicate}"),
            )
            .await
                == "0"
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("native backend disappeared before lock release");
    assert!(backend.cancel(lock_command), "fixture lock still active");
    let fixture_outcome = tokio::time::timeout(Duration::from_secs(10), locking)
        .await
        .expect("bounded fixture lock cleanup")
        .expect("joined fixture lock")
        .expect_err("DO cancellation is conservatively ambiguous, never retried");
    assert!(!fixture_outcome.retryable);
}
