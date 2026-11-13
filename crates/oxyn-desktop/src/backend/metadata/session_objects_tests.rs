//! Temporary data and homonyms are owned by the console, including metadata.
use super::*;

fn temporary_address(driver: &str, relation: &str) -> CatalogAddress {
    CatalogAddress {
        catalog: (driver == "duckdb").then(|| "temp".into()),
        namespace: Some(if driver == "sqlite" { "temp" } else { "main" }.into()),
        relation: Some(relation.into()),
    }
}

async fn describe_on(fixture: &Fixture, session: SessionId, address: &CatalogAddress) {
    fixture
        .backend
        .refresh_relation_facet(
            CommandId::new(),
            fixture.connection,
            session,
            address.clone(),
            RelationFacet::Detail,
        )
        .await
        .expect("session-local metadata");
}

#[tokio::test]
async fn a_catalog_session_keeps_variables_when_previewing_a_persistent_view() {
    let fixture = Fixture::memory("duckdb").await;
    fixture.seed(601).await;
    fixture
        .backend
        .execute(
            CommandId::new(),
            fixture.connection,
            fixture.catalog,
            "SET VARIABLE threshold = 400".into(),
        )
        .await
        .expect("source variable");
    let address = fixture.address("records_view");
    describe_on(&fixture, fixture.catalog, &address).await;
    let (result, rows) = page_on(
        &fixture,
        fixture.catalog,
        &address,
        PreviewShapeDraft {
            predicate: Some("id > getvariable('threshold')".into()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(rows.len(), 200);
    assert_eq!(rows.first().map(String::as_str), Some("401"));
    fixture.backend.forget_result(result);
    fixture.close().await;
}

#[tokio::test]
async fn a_catalog_with_user_sql_cannot_review_a_separate_session_object_operation() {
    let fixture = Fixture::memory("sqlite").await;
    fixture.seed(2).await;
    let address = fixture.address("records");
    fixture
        .backend
        .execute(
            CommandId::new(),
            fixture.connection,
            fixture.catalog,
            "CREATE TEMP TABLE records (private_id INTEGER)".into(),
        )
        .await
        .expect("temporary homonym on catalog source");
    let count = fixture
        .backend
        .inner
        .executor
        .sessions()
        .for_connection(fixture.connection)
        .len();
    let error = fixture
        .backend
        .review_object_operation(
            fixture.connection,
            fixture.catalog,
            &address,
            &crate::ipc::object_operations::ObjectOperation::Drop { cascade: false },
        )
        .expect_err("fresh session must not target the persistent homonym");
    assert!(error.message.contains("owning console"));
    assert_eq!(
        fixture
            .backend
            .inner
            .executor
            .sessions()
            .for_connection(fixture.connection)
            .len(),
        count
    );
    fixture.close().await;
}

async fn page_on(
    fixture: &Fixture,
    session: SessionId,
    address: &CatalogAddress,
    shape: PreviewShapeDraft,
) -> (ResultId, Vec<String>) {
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        fixture.backend.preview_relation(
            CommandId::new(),
            fixture.connection,
            session,
            address.clone(),
            shape,
        ),
    )
    .await
    .expect("original preview never waits for its own transaction state")
    .expect("page");
    let CommandOutcome::Executed { result, .. } = outcome else {
        panic!("executed: {outcome:?}")
    };
    let result = result.parse().expect("result id");
    let ResultWindow::Page(page) = fixture
        .backend
        .read_result_page(fixture.connection, result, 0, 1000)
        .await
        .expect("rows")
    else {
        panic!("page")
    };
    let values = page
        .rows
        .iter()
        .map(|row| match row.first() {
            Some(Cell::Text(value)) => value.clone(),
            other => panic!("fixture integer: {other:?}"),
        })
        .collect();
    (result, values)
}

#[rstest]
#[case("sqlite")]
#[case("duckdb")]
#[tokio::test]
async fn temporary_views_page_on_their_original_session_and_release_without_losing_data(
    #[case] driver: &'static str,
) {
    let fixture = Fixture::memory(driver).await;
    fixture
        .sql("CREATE TEMP TABLE records (id INTEGER, label TEXT)")
        .await;
    fixture.sql("INSERT INTO records WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 601) SELECT i, 'local' FROM n").await;
    fixture
        .sql("CREATE TEMP VIEW records_view AS SELECT * FROM records")
        .await;
    let address = temporary_address(driver, "records_view");
    describe_on(&fixture, fixture.console, &address).await;
    let own = fixture
        .backend
        .relation_facets_for_session(fixture.connection, &address, Some(fixture.console))
        .expect("owned facets");
    assert!(own.detail.value.is_some());
    let unrelated = fixture
        .backend
        .relation_facets_for_session(fixture.connection, &address, Some(fixture.catalog))
        .expect("separate cache");
    assert!(unrelated.detail.value.is_none());
    let mut held = None;
    for offset in [0, 200, 400, 600, 0] {
        let shape = PreviewShapeDraft {
            offset,
            continuation: held,
            ..Default::default()
        };
        let (result, rows) = page_on(&fixture, fixture.console, &address, shape).await;
        assert_eq!(rows.len(), if offset == 600 { 1 } else { 200 });
        assert_eq!(rows.first(), Some(&(offset + 1).to_string()));
        if let Some(previous) = held {
            fixture.backend.forget_result(previous);
        }
        held = Some(result);
    }
    let result = held.expect("last result");
    fixture
        .backend
        .run(
            CommandId::new(),
            Command::ReleasePreview {
                connection: fixture.connection,
                session: fixture.console,
                result,
            },
        )
        .await
        .expect("release source");
    // Releasing a preview does not close its console or its temporary database.
    describe_on(&fixture, fixture.console, &address).await;
    let (fresh, rows) = page_on(
        &fixture,
        fixture.console,
        &address,
        PreviewShapeDraft::default(),
    )
    .await;
    assert_eq!(rows.first().map(String::as_str), Some("1"));
    fixture.backend.forget_result(fresh);
    fixture.backend.forget_result(result);
    fixture
        .sql("INSERT INTO records VALUES (602, 'still local')")
        .await;
    fixture.close().await;
}

#[tokio::test]
async fn two_sqlite_consoles_do_not_share_homonymous_temporary_metadata_or_rows() {
    let fixture = Fixture::memory("sqlite").await;
    fixture
        .sql("CREATE TEMP TABLE same_name (left_column INTEGER); INSERT INTO same_name VALUES (11)")
        .await;
    let second = fixture
        .backend
        .inner
        .executor
        .dispatch(
            oxyn_core::Actor::Human,
            Command::Connect {
                connection: fixture.connection,
            },
            &oxyn_core::CancelToken::new(),
        )
        .await
        .expect("another session");
    let oxyn_exec::Outcome::Connected {
        session: second, ..
    } = second
    else {
        panic!("connected")
    };
    fixture.backend.execute(CommandId::new(), fixture.connection, second, "CREATE TEMP TABLE same_name (right_column TEXT); INSERT INTO same_name VALUES ('other')".into()).await.expect("other fixture");
    let address = temporary_address("sqlite", "same_name");
    for session in [fixture.console, second] {
        describe_on(&fixture, session, &address).await;
    }
    let first_cache = fixture
        .backend
        .inner
        .executor
        .session_catalog(fixture.connection, fixture.console)
        .expect("first cache");
    let second_cache = fixture
        .backend
        .inner
        .executor
        .session_catalog(fixture.connection, second)
        .expect("second cache");
    let path = address.to_path().expect("path");
    assert!(
        first_cache
            .read()
            .relation(&path)
            .expect("first")
            .field("left_column")
            .is_some()
    );
    assert!(
        second_cache
            .read()
            .relation(&path)
            .expect("second")
            .field("right_column")
            .is_some()
    );
    let (_, first) = page_on(
        &fixture,
        fixture.console,
        &address,
        PreviewShapeDraft::default(),
    )
    .await;
    let (_, second_rows) = page_on(&fixture, second, &address, PreviewShapeDraft::default()).await;
    assert_eq!(first, ["11"]);
    assert_eq!(second_rows, ["other"]);
    fixture.close().await;
}

#[tokio::test]
async fn another_windows_catalog_is_not_mistaken_for_a_console() {
    let fixture = Fixture::memory("sqlite").await;
    fixture.seed(601).await;
    let oxyn_exec::Outcome::Connected {
        session: second, ..
    } = fixture
        .backend
        .inner
        .executor
        .dispatch(
            oxyn_core::Actor::Human,
            Command::Connect {
                connection: fixture.connection,
            },
            &oxyn_core::CancelToken::new(),
        )
        .await
        .expect("second window catalog")
    else {
        panic!("connected")
    };
    assert!(
        fixture
            .backend
            .inner
            .executor
            .is_catalog_session(fixture.connection, second)
    );
    assert!(
        !fixture
            .backend
            .inner
            .executor
            .is_catalog_session(fixture.connection, fixture.console)
    );
    let address = fixture.address("records_view");
    describe_on(&fixture, second, &address).await;
    let (_, values) = page_on(&fixture, second, &address, PreviewShapeDraft::default()).await;
    assert_eq!(values.len(), 200);
    tokio::time::timeout(
        Duration::from_secs(10),
        fixture
            .backend
            .refresh_catalog(CommandId::new(), fixture.connection, second, None),
    )
    .await
    .expect("second catalog stays responsive")
    .expect("metadata");
    let old = fixture
        .backend
        .inner
        .executor
        .catalog(fixture.connection)
        .expect("preferred catalog");
    fixture
        .backend
        .run(
            CommandId::new(),
            Command::CloseSession {
                connection: fixture.connection,
                session: fixture.catalog,
            },
        )
        .await
        .expect("close preferred owner");
    let oxyn_exec::Outcome::Connected {
        session: console, ..
    } = fixture
        .backend
        .inner
        .executor
        .dispatch(
            oxyn_core::Actor::Human,
            Command::OpenConsoleSession {
                connection: fixture.connection,
            },
            &oxyn_core::CancelToken::new(),
        )
        .await
        .expect("new console")
    else {
        panic!("connected")
    };
    assert!(
        !fixture
            .backend
            .inner
            .executor
            .is_catalog_session(fixture.connection, console)
    );
    fixture
        .backend
        .inner
        .executor
        .dispatch(
            oxyn_core::Actor::Human,
            Command::Connect {
                connection: fixture.connection,
            },
            &oxyn_core::CancelToken::new(),
        )
        .await
        .expect("replacement catalog");
    let current = fixture
        .backend
        .inner
        .executor
        .catalog(fixture.connection)
        .expect("new generation");
    old.write().set_server_info(oxyn_catalog::ServerInfo::new(
        "late old owner",
        "1",
        Capabilities::empty(),
    ));
    assert!(
        current.read().server_info().is_none(),
        "a late publication cannot enter the replacement generation"
    );
    fixture.close().await;
}

#[tokio::test]
async fn temporary_continuation_survives_catalog_metadata_eviction() {
    let fixture = Fixture::memory("sqlite").await;
    fixture.sql("CREATE TEMP TABLE records (id INTEGER); INSERT INTO records WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 601) SELECT i FROM n").await;
    let address = temporary_address("sqlite", "records");
    describe_on(&fixture, fixture.console, &address).await;
    let (first, _) = page_on(
        &fixture,
        fixture.console,
        &address,
        PreviewShapeDraft::default(),
    )
    .await;
    fixture
        .backend
        .inner
        .executor
        .session_catalog(fixture.connection, fixture.console)
        .expect("cache")
        .write()
        .evict(&oxyn_catalog::CatalogScope::Server);
    assert!(matches!(
        fixture
            .backend
            .preview_pagination_for_session(
                fixture.connection,
                &address,
                &PreviewShapeDraft::default(),
                200,
                Some(fixture.console),
                Some(first)
            )
            .expect("result proves continuation"),
        Pagination::Retained { next: true, .. }
    ));
    let (_, rows) = page_on(
        &fixture,
        fixture.console,
        &address,
        PreviewShapeDraft {
            offset: 200,
            continuation: Some(first),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(rows.first().map(String::as_str), Some("201"));
    fixture.close().await;
}

#[tokio::test]
async fn a_related_row_template_cannot_read_another_connections_result() {
    let fixture = Fixture::memory("sqlite").await;
    let CommandOutcome::Executed { result, .. } = fixture
        .backend
        .execute(
            CommandId::new(),
            fixture.connection,
            fixture.console,
            "SELECT 42 AS id".into(),
        )
        .await
        .expect("owned result")
    else {
        panic!("executed")
    };
    let ConnectResponse::Open(other) = fixture
        .backend
        .connect(
            CommandId::new(),
            ConnectionDraft {
                driver: "sqlite".into(),
                name: "other connection".into(),
                environment: Environment::Local,
                privacy_tier: PrivacyTier::Local,
                read_only: false,
                values: [("path".into(), ":memory:".into())].into(),
                secrets: Default::default(),
            },
        )
        .await
        .expect("other connection")
    else {
        panic!("connected")
    };
    let other_connection = other.connection.parse().expect("connection id");
    let error = fixture
        .backend
        .related_rows_template_for_session(
            other_connection,
            &fixture.address("records"),
            false,
            0,
            Some(RelatedRowsSource {
                result: result.parse().expect("result"),
                row: 0,
            }),
            Some(other.session.parse().expect("session")),
        )
        .expect_err("foreign row values must not enter a template");
    assert!(
        error
            .message
            .contains("result does not belong to this connection")
    );
    fixture
        .backend
        .run(
            CommandId::new(),
            Command::Disconnect {
                connection: other_connection,
            },
        )
        .await
        .expect("close second connection");
    fixture.close().await;
}
