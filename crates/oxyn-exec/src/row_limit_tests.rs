//! The receive limit of an ordinary statement (issue #122): a stop at the
//! limit is only called complete once the end of the stream was observed.

use super::*;
use oxyn_core::{DefaultPolicy, DriverId, ExecLimits, ExportFormat, QueryLanguage, SqlDialect};
use oxyn_driver_sqlite::{BatchLimits, SqliteDriver};

const LIMIT: usize = 3;

async fn connected() -> (Executor, ConnectionId, SessionId) {
    let connection = ConnectionConfig::new("memory", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store.workspaces().create("limit").expect("workspace").id;
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(
            // One row per batch: the buffer lands exactly on its limit, and
            // only a further read can tell whether more rows follow.
            SqliteDriver::new().with_batch_limits(BatchLimits::new().with_max_rows(1)),
        ))
        .expect("SQLite registration");
    let executor = Executor::builder(store, policy)
        .with_drivers(Arc::new(drivers))
        .with_workspace(workspace)
        .build();
    executor.register_connection(&connection);
    let Outcome::Connected { session, .. } = executor
        .dispatch(
            Actor::Human,
            Command::Connect {
                connection: connection.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect("memory connection")
    else {
        panic!("expected a connected session");
    };
    (executor, connection.id, session)
}

async fn execute(
    executor: &Executor,
    connection: ConnectionId,
    session: SessionId,
    text: &str,
    limits: ExecLimits,
) -> (ResultId, ExecStats, SinkOutcome) {
    let outcome = executor
        .dispatch(
            Actor::Human,
            Command::Execute {
                connection,
                session,
                request: Box::new(
                    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), text)
                        .with_limits(limits),
                ),
            },
            &CancelToken::new(),
        )
        .await
        .expect("fixture SQL");
    let Outcome::Executed {
        result,
        stats,
        sink,
        ..
    } = outcome
    else {
        panic!("expected a result, got {outcome:?}");
    };
    (result, stats, sink)
}

fn rows(count: usize) -> String {
    format!(
        "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < {count}) \
         SELECT n FROM s"
    )
}

async fn exports(executor: &Executor, connection: ConnectionId, result: ResultId) -> bool {
    let folder = tempfile::tempdir().expect("temporary folder");
    executor
        .dispatch(
            Actor::Human,
            Command::Export {
                connection,
                result,
                format: ExportFormat::Csv,
                destination: folder.path().join("export.csv"),
            },
            &CancelToken::new(),
        )
        .await
        .is_ok()
}

#[tokio::test]
async fn a_read_of_exactly_the_limit_is_whole_and_one_more_row_is_not() {
    let (executor, connection, session) = connected().await;
    for (count, whole) in [
        (LIMIT - 1, true),
        (LIMIT, true),
        (LIMIT + 1, false),
        (LIMIT * 1_000, false),
    ] {
        let (result, stats, sink) = execute(
            &executor,
            connection,
            session,
            &rows(count),
            ExecLimits::default().with_max_rows(LIMIT),
        )
        .await;
        let expected = if whole {
            SinkOutcome::Exhausted
        } else {
            SinkOutcome::RowLimit
        };
        assert_eq!(sink, expected, "{count} rows");
        assert_eq!(
            usize::try_from(stats.rows).ok(),
            Some(count.min(LIMIT)),
            "{count} rows"
        );
        assert_eq!(stats.truncated, !whole, "{count} rows");
        assert_eq!(
            exports(&executor, connection, result).await,
            whole,
            "{count} rows: only a whole result exports"
        );
    }
}

/// A write is not read further than its limit: the probe would make the
/// server do more work with side effects. The stop is said unverified, and
/// stays unexportable.
#[tokio::test]
async fn a_write_stopped_at_the_limit_is_unverified_and_not_exported() {
    let (executor, connection, session) = connected().await;
    execute(
        &executor,
        connection,
        session,
        "CREATE TABLE t (n INTEGER)",
        ExecLimits::default().writable(),
    )
    .await;
    let (result, stats, sink) = execute(
        &executor,
        connection,
        session,
        "INSERT INTO t VALUES (1), (2), (3) RETURNING n",
        ExecLimits::default().writable().with_max_rows(LIMIT),
    )
    .await;

    assert_eq!(sink, SinkOutcome::RowLimitUnverified);
    assert!(stats.truncated, "never presented as whole");
    assert!(!exports(&executor, connection, result).await);
}
