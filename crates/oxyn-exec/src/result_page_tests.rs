//! Local pages are scoped to the producing connection and never require a driver.

use super::*;
use arrow::array::{Int64Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};
use oxyn_core::{AgentId, AgentSessionId, DefaultPolicy, DriverId};
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
