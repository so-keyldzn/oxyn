//! Independent console transactions over the same configured SQLite memory database.

use super::*;

fn opened(response: oneshot::Receiver<Result<ConnectionResponse>>) -> OpenConnection {
    match response.blocking_recv().expect("response").expect("open") {
        ConnectionResponse::Open(open) => open,
        _ => panic!("local fixture does not require approval"),
    }
}
fn sql(
    backend: &Backend,
    connection: ConnectionId,
    session: SessionId,
    text: &str,
) -> Result<Outcome, OxynError> {
    backend
        .dispatch(
            CommandId::new(),
            crate::workspace::execution_command(
                connection,
                session,
                false,
                SqlDialect::Sqlite,
                text.into(),
                Vec::new(),
            ),
            CancelToken::new(),
        )
        .blocking_recv()
        .expect("response")
}
fn count(outcome: Outcome) -> i64 {
    let Outcome::Executed { buffer, .. } = outcome else {
        panic!("query result");
    };
    let batch = buffer
        .cached_batch(oxyn_data::BatchIndex::new(0))
        .expect("small result");
    batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("SQLite count")
        .value(0)
}

#[test]
fn closing_one_console_rolls_back_its_transaction_without_closing_siblings() {
    let backend = Backend::open_temporary().expect("backend");
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "shared memory".into(),
        environment: Environment::Local,
        values: [("path".into(), ":memory:".into())].into_iter().collect(),
        secrets: Default::default(),
    };
    let workspace = opened(backend.connect(draft.clone(), CancelToken::new()));
    let first = workspace
        .initial_console
        .as_ref()
        .expect("dedicated first console");
    let second = opened(backend.reconnect_console(workspace.connection, CancelToken::new()));
    assert_ne!(workspace.session, first.session);
    assert_ne!(first.session, second.session);
    sql(
        &backend,
        workspace.connection,
        first.session,
        "CREATE TABLE isolation_probe(n INTEGER)",
    )
    .expect("create");
    assert_eq!(
        count(
            sql(
                &backend,
                workspace.connection,
                second.session,
                "SELECT COUNT(*) FROM isolation_probe"
            )
            .expect("same database")
        ),
        0
    );
    sql(&backend, workspace.connection, first.session, "BEGIN").expect("begin");
    sql(
        &backend,
        workspace.connection,
        first.session,
        "INSERT INTO isolation_probe VALUES (7)",
    )
    .expect("uncommitted write");
    if let Ok(result) = sql(
        &backend,
        workspace.connection,
        second.session,
        "SELECT COUNT(*) FROM isolation_probe",
    ) {
        assert_eq!(count(result), 0, "a sibling cannot see uncommitted data");
    }
    backend
        .dispatch(
            CommandId::new(),
            Command::CloseSession {
                connection: workspace.connection,
                session: first.session,
            },
            CancelToken::new(),
        )
        .blocking_recv()
        .expect("response")
        .expect("close first console");
    assert_eq!(
        count(
            sql(
                &backend,
                workspace.connection,
                second.session,
                "SELECT COUNT(*) FROM isolation_probe"
            )
            .expect("sibling remains usable")
        ),
        0
    );
    assert_eq!(
        count(
            sql(
                &backend,
                workspace.connection,
                workspace.session,
                "SELECT COUNT(*) FROM isolation_probe"
            )
            .expect("catalog session remains usable")
        ),
        0
    );
    let other = opened(backend.connect(
        ConnectionDraft {
            name: "different profile".into(),
            ..draft
        },
        CancelToken::new(),
    ));
    assert!(
        sql(
            &backend,
            other.connection,
            other.session,
            "SELECT * FROM isolation_probe"
        )
        .is_err()
    );
}

#[test]
fn submitted_document_writes_finish_even_when_their_view_receiver_is_dropped() {
    let backend = Backend::open_temporary().expect("backend");
    let document = oxyn_core::DocumentId::new();
    for revision in 1..=12 {
        drop(backend.dispatch(
            CommandId::new(),
            Command::SaveQueryDocument {
                workspace: backend.workspace_id(),
                update: Box::new(oxyn_core::QueryDocumentUpdate {
                    expected_revision: None,
                    document,
                    revision,
                    title: "Saved.sql".into(),
                    language: oxyn_core::QueryLanguage::Sql(SqlDialect::Sqlite),
                    text: format!("SELECT {revision}"),
                    connection: None,
                    save_named: true,
                    is_open: true,
                    provenance: None,
                }),
            },
            CancelToken::new(),
        ));
    }
    backend.runtime.block_on(backend.wait_for_local_writes());
    let doc = backend
        .inner
        .executor
        .store()
        .documents()
        .get(document)
        .expect("read")
        .expect("saved");
    assert_eq!(doc.saved_revision, 12);
    assert_eq!(doc.saved_content.as_deref(), Some("SELECT 12"));
}
