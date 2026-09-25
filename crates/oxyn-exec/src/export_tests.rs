//! Export contracts at the bus: what is refused, and what the destination keeps.

use super::*;
use arrow::array::{Int64Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use futures::FutureExt;
use futures::future::BoxFuture;
use oxyn_core::{DefaultPolicy, DriverId, ExecLimits, ExportFormat, QueryLanguage, SqlDialect};
use oxyn_data::BatchSource;
use oxyn_driver_sqlite::SqliteDriver;

const SENTINEL: &str = "EXISTING USER DOCUMENT\n";

fn sqlite_executor() -> (Executor, ConnectionConfig) {
    let connection = ConnectionConfig::new("memory", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store.workspaces().create("export").expect("workspace").id;
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(SqliteDriver::new()))
        .expect("SQLite registration");
    let executor = Executor::builder(store, policy)
        .with_drivers(Arc::new(drivers))
        .with_workspace(workspace)
        .build();
    executor.register_connection(&connection);
    (executor, connection)
}

async fn connect(executor: &Executor, connection: &ConnectionConfig) -> SessionId {
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
    session
}

async fn execute(
    executor: &Executor,
    connection: ConnectionId,
    session: SessionId,
    text: &str,
    limits: ExecLimits,
) -> (ResultId, ExecStats) {
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
    let Outcome::Executed { result, stats, .. } = outcome else {
        panic!("expected a result, got {outcome:?}");
    };
    (result, stats)
}

/// A destination that already holds the user's document.
fn existing_document() -> (tempfile::TempDir, std::path::PathBuf) {
    let folder = tempfile::tempdir().expect("temporary folder");
    let destination = folder.path().join("export.csv");
    std::fs::write(&destination, SENTINEL).expect("existing document");
    (folder, destination)
}

fn export(connection: ConnectionId, result: ResultId, destination: &std::path::Path) -> Command {
    Command::Export {
        connection,
        result,
        format: ExportFormat::Csv,
        destination: destination.to_path_buf(),
    }
}

/// Asserts the refusal left the folder exactly as it was.
fn assert_untouched(folder: &tempfile::TempDir, destination: &std::path::Path) {
    assert_eq!(
        std::fs::read_to_string(destination).expect("document reread"),
        SENTINEL
    );
    assert_eq!(
        std::fs::read_dir(folder.path()).expect("folder").count(),
        1,
        "nothing but the existing document"
    );
}

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]))
}

fn batch() -> RecordBatch {
    RecordBatch::try_new(schema(), vec![Arc::new(Int64Array::from(vec![1, 2, 3]))])
        .expect("the column matches the schema")
}

/// Hands out batches forever; cancels `cancel_after_first` once one is out.
struct Endless {
    served: usize,
    cancel_after_first: Option<CancelToken>,
}

impl BatchSource for Endless {
    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn next_batch(&mut self) -> BoxFuture<'_, Result<Option<RecordBatch>>> {
        self.served += 1;
        if let Some(ct) = &self.cancel_after_first {
            ct.cancel();
        }
        futures::future::ready(Ok(Some(batch()))).boxed()
    }
}

/// Drains `source` through the real sink and stores the buffer as a result.
async fn stored_through_sink(
    executor: &Executor,
    connection: ConnectionId,
    limits: BufferLimits,
    mut source: Endless,
    ct: &CancelToken,
) -> (ResultId, SinkOutcome) {
    let buffer = Arc::new(ResultBuffer::with_limits(schema(), limits));
    let outcome = BatchSink::new(Arc::clone(&buffer))
        .drain(&mut source, ct)
        .await
        .expect("drained");
    assert!(source.served > 0, "the buffer holds received rows");
    assert!(buffer.is_complete() && buffer.row_count() > 0);
    let result = ResultId::new();
    executor
        .results
        .write()
        .insert(result, StoredResult { connection, buffer });
    (result, outcome)
}

#[tokio::test]
async fn a_result_stopped_by_its_row_limit_is_refused_and_the_file_kept() {
    let (executor, connection) = sqlite_executor();
    let session = connect(&executor, &connection).await;
    let (result, stats) = execute(
        &executor,
        connection.id,
        session,
        "SELECT 1 AS value UNION ALL SELECT 2",
        ExecLimits::default().with_max_rows(1),
    )
    .await;
    assert!(stats.truncated, "the row limit cut the result");

    let (folder, destination) = existing_document();
    let refused = executor
        .dispatch(
            Actor::Human,
            export(connection.id, result, &destination),
            &CancelToken::new(),
        )
        .await;

    assert!(matches!(refused, Err(OxynError::Config(_))), "{refused:?}");
    assert_untouched(&folder, &destination);
}

#[tokio::test]
async fn a_saturated_result_is_refused_and_the_file_kept() {
    let (executor, connection) = sqlite_executor();
    let (result, outcome) = stored_through_sink(
        &executor,
        connection.id,
        // Room for a few batches in memory, none on disk: the endless source
        // fills it, and the sink stops on saturation with rows received.
        BufferLimits::default()
            .with_memory_budget(batch().get_array_memory_size() * 4)
            .without_spill(),
        Endless {
            served: 0,
            cancel_after_first: None,
        },
        &CancelToken::new(),
    )
    .await;
    assert_eq!(outcome, SinkOutcome::Saturated);

    let (folder, destination) = existing_document();
    let refused = executor
        .dispatch(
            Actor::Human,
            export(connection.id, result, &destination),
            &CancelToken::new(),
        )
        .await;

    assert!(matches!(refused, Err(OxynError::Config(_))), "{refused:?}");
    assert_untouched(&folder, &destination);
}

#[tokio::test]
async fn a_cancelled_result_is_refused_and_the_file_kept() {
    let (executor, connection) = sqlite_executor();
    let reception = CancelToken::new();
    let (result, outcome) = stored_through_sink(
        &executor,
        connection.id,
        BufferLimits::default(),
        Endless {
            served: 0,
            cancel_after_first: Some(reception.clone()),
        },
        &reception,
    )
    .await;
    assert_eq!(outcome, SinkOutcome::Cancelled);

    let (folder, destination) = existing_document();
    let refused = executor
        .dispatch(
            Actor::Human,
            export(connection.id, result, &destination),
            &CancelToken::new(),
        )
        .await;

    assert!(matches!(refused, Err(OxynError::Config(_))), "{refused:?}");
    assert_untouched(&folder, &destination);
}

/// The preview SQL carries its own `LIMIT` and ends normally: the limited
/// result is complete and stays exportable (UX-SPEC, `Export preview…`).
#[tokio::test]
async fn a_preview_whose_limited_sql_ended_normally_still_exports() {
    let (executor, connection) = sqlite_executor();
    let session = connect(&executor, &connection).await;
    execute(
        &executor,
        connection.id,
        session,
        "CREATE TABLE t (n INTEGER)",
        ExecLimits::default().writable(),
    )
    .await;
    execute(
        &executor,
        connection.id,
        session,
        "INSERT INTO t VALUES (1), (2), (3), (4)",
        ExecLimits::default().writable(),
    )
    .await;
    let outcome = executor
        .dispatch(
            Actor::Human,
            Command::PreviewRelation {
                connection: connection.id,
                session,
                catalog: None,
                namespace: Some("main".into()),
                relation: "t".into(),
                limit: 2,
                shape: oxyn_core::PreviewShape::unordered(),
            },
            &CancelToken::new(),
        )
        .await
        .expect("preview");
    let Outcome::Executed { result, stats, .. } = outcome else {
        panic!("expected a preview result, got {outcome:?}");
    };
    assert!(!stats.truncated);

    let (_folder, destination) = existing_document();
    let exported = executor
        .dispatch(
            Actor::Human,
            export(connection.id, result, &destination),
            &CancelToken::new(),
        )
        .await
        .expect("export of a complete preview");

    assert!(matches!(exported, Outcome::Exported { rows: 2, .. }));
    assert_eq!(
        std::fs::read_to_string(&destination).expect("export reread"),
        "n\n1\n2\n"
    );
}
