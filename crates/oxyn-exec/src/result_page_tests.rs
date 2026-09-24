//! Local pages are scoped to the producing connection and never require a driver.

use super::*;
use arrow::array::{Int64Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};
use oxyn_core::{AgentId, AgentSessionId, DefaultPolicy, DriverId, ExportFormat};
use oxyn_data::BatchIndex;

#[tokio::test]
async fn page_read_is_local_audited_and_scoped_for_both_actors() {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store.workspaces().create("pages").expect("workspace").id;
    let owner =
        ConnectionConfig::new("owner", DriverId::sqlite()).with_environment(Environment::Local);
    let other =
        ConnectionConfig::new("other", DriverId::sqlite()).with_environment(Environment::Local);
    let policy = Arc::new(DefaultPolicy::new());
    for config in [&owner, &other] {
        store
            .connections()
            .save(workspace, config)
            .expect("save configuration");
        policy.register(config);
    }
    // The registry contains no driver; no server call can accidentally succeed.
    let executor = Executor::builder(store.clone(), policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(&owner);
    executor.register_connection(&other);
    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
    let buffer = Arc::new(ResultBuffer::new(schema.clone(), 64 * 1024));
    for _ in 0..64 {
        buffer
            .push(
                RecordBatch::try_new(
                    schema.clone(),
                    vec![Arc::new(Int64Array::from_iter_values(0..512))],
                )
                .expect("batch"),
            )
            .expect("push");
    }
    buffer.mark_complete(ExecStats::default());
    let result = ResultId::new();
    executor.results.write().insert(
        result,
        StoredResult {
            connection: owner.id,
            buffer: buffer.clone(),
        },
    );
    assert!(buffer.cached_batch(BatchIndex::new(63)).is_none());
    for actor in [
        Actor::Human,
        Actor::agent(AgentId::new(), AgentSessionId::new()),
    ] {
        let denied = executor
            .dispatch(
                actor,
                Command::ReadResultPage {
                    connection: other.id,
                    result,
                    batch: 63,
                },
                &CancelToken::new(),
            )
            .await;
        assert!(matches!(
            denied,
            Err(OxynError::PolicyDenied { .. }) | Ok(Outcome::Denied { .. })
        ));
        let read = executor
            .dispatch(
                actor,
                Command::ReadResultPage {
                    connection: owner.id,
                    result,
                    batch: 63,
                },
                &CancelToken::new(),
            )
            .await
            .expect("local page");
        assert!(
            matches!(read, Outcome::ResultPageRead { result: found, batch: 63 } if found == result)
        );
    }
    assert!(buffer.cached_batch(BatchIndex::new(63)).is_some());
    let destination =
        std::env::temp_dir().join(format!("oxyn-wrong-connection-{}.csv", ResultId::new()));
    let exported = executor
        .dispatch(
            Actor::Human,
            Command::Export {
                connection: other.id,
                result,
                format: ExportFormat::Csv,
                destination: destination.clone(),
            },
            &CancelToken::new(),
        )
        .await;
    assert!(matches!(
        exported,
        Err(OxynError::PolicyDenied { .. }) | Ok(Outcome::Denied { .. })
    ));
    assert!(
        !destination.exists(),
        "a mismatched connection must not create an export file"
    );
    let journal = store.journal().recent(10).expect("audit");
    assert_eq!(journal.len(), 10);
    assert_eq!(
        journal
            .iter()
            .filter(|entry| entry.record.command_kind == "ReadResultPage")
            .count(),
        8
    );
    assert!(journal.iter().all(|entry| entry.record.statement.is_none()));
    assert_eq!(
        store.history().count().expect("SQL history"),
        0,
        "page reads are not new SQL executions"
    );
    for actor in [
        Actor::Human,
        Actor::agent(AgentId::new(), AgentSessionId::new()),
    ] {
        let denied = executor
            .dispatch(
                actor,
                Command::InspectResultValue {
                    connection: other.id,
                    result,
                    row: 0,
                    column: 0,
                    offset: 0,
                },
                &CancelToken::new(),
            )
            .await;
        assert!(matches!(
            denied,
            Err(OxynError::PolicyDenied { .. }) | Ok(Outcome::Denied { .. })
        ));
        let inspected = executor
            .dispatch(
                actor,
                Command::InspectResultValue {
                    connection: owner.id,
                    result,
                    row: 0,
                    column: 0,
                    offset: 0,
                },
                &CancelToken::new(),
            )
            .await
            .expect("value inspection");
        let Outcome::ValueInspected { page } = inspected else {
            panic!("value page")
        };
        assert_eq!(page.text, "0");
    }
    let inspection_audit = store.journal().recent(8).expect("inspection audit");
    assert!(
        inspection_audit
            .iter()
            .all(|entry| entry.record.command_kind == "InspectResultValue"
                && entry.record.statement.is_none())
    );
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(
        executor
            .dispatch(
                Actor::Human,
                Command::ReadResultPage {
                    connection: owner.id,
                    result,
                    batch: 62
                },
                &cancel
            )
            .await,
        Err(OxynError::Cancelled)
    ));
}

fn export_fixture() -> (
    Arc<Store>,
    Arc<DefaultPolicy>,
    WorkspaceId,
    ConnectionConfig,
) {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store.workspaces().create("export").expect("workspace").id;
    let connection =
        ConnectionConfig::new("export", DriverId::sqlite()).with_environment(Environment::Local);
    store
        .connections()
        .save(workspace, &connection)
        .expect("save configuration");
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    (store, policy, workspace, connection)
}

#[tokio::test]
async fn export_csv_writes_the_file_and_reports_exact_rows() {
    let (store, policy, workspace, connection) = export_fixture();
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(&connection);

    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
    let buffer = Arc::new(ResultBuffer::new(schema.clone(), 64 * 1024));
    buffer
        .push(
            RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(Int64Array::from_iter_values(0..10))],
            )
            .expect("batch"),
        )
        .expect("push");
    buffer.mark_complete(ExecStats::default());
    let result = ResultId::new();
    executor.results.write().insert(
        result,
        StoredResult {
            connection: connection.id,
            buffer,
        },
    );

    let destination = std::env::temp_dir().join(format!("oxyn-export-{}.csv", ResultId::new()));
    let outcome = executor
        .dispatch(
            Actor::Human,
            Command::Export {
                connection: connection.id,
                result,
                format: ExportFormat::Csv,
                destination: destination.clone(),
            },
            &CancelToken::new(),
        )
        .await
        .expect("export");
    let Outcome::Exported { rows, .. } = outcome else {
        panic!("expected an exported outcome");
    };
    assert_eq!(rows, 10);
    let content = std::fs::read_to_string(&destination).expect("exported file");
    assert_eq!(content.lines().count(), 11, "header plus ten rows");
    std::fs::remove_file(&destination).expect("temporary file cleanup");
}

#[test]
fn export_without_a_runtime_fails_and_creates_no_file() {
    let (store, policy, workspace, connection) = export_fixture();
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(&connection);

    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
    let buffer = Arc::new(ResultBuffer::new(schema, 64 * 1024));
    buffer.mark_complete(ExecStats::default());
    let result = ResultId::new();
    executor.results.write().insert(
        result,
        StoredResult {
            connection: connection.id,
            buffer,
        },
    );

    let destination =
        std::env::temp_dir().join(format!("oxyn-export-no-runtime-{}.csv", ResultId::new()));
    let outcome = futures::executor::block_on(executor.dispatch(
        Actor::Human,
        Command::Export {
            connection: connection.id,
            result,
            format: ExportFormat::Csv,
            destination: destination.clone(),
        },
        &CancelToken::new(),
    ));
    assert!(matches!(outcome, Err(OxynError::Config(_))));
    assert!(
        !destination.exists(),
        "a missing runtime must not create a partial export file"
    );
}
