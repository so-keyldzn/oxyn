//! A stopped idle portal releases its read transaction without dropping temp data.
use super::*;

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn original_preview_keeps_temporary_state_and_the_same_backend_after_release() {
    let Some(session) = session().await else {
        return;
    };
    apply(
        &*session,
        "CREATE TEMP TABLE oxyn_temp_rows AS SELECT generate_series(1, 601) AS id, '{[1,4)}'::int4multirange AS bands",
    )
    .await;
    apply(
        &*session,
        "CREATE TEMP VIEW oxyn_temp_view AS SELECT * FROM oxyn_temp_rows",
    )
    .await;
    let owner = first_text(&*session, "SELECT pg_backend_pid()::text").await;
    let namespace = first_text(
        &*session,
        "SELECT nspname::text FROM pg_namespace WHERE oid=pg_my_temp_schema()",
    )
    .await;
    let path =
        oxyn_catalog::CatalogPath::for_relation(None, Some(namespace.as_str()), "oxyn_temp_view")
            .expect("temporary view");
    for _ in 0..3 {
        let token = CancelToken::new();
        let prepared = session
            .prepare_preview(&path, 50, &PreviewShape::default(), &token)
            .await
            .expect("temporary preparation");
        assert_eq!(prepared.mode, oxyn_driver::PreviewMode::Original);
        let mut cursor = session
            .execute_original_preview(prepared.request, &token)
            .await
            .expect("owning portal");
        let first = cursor
            .next_batch()
            .await
            .expect("first batch")
            .expect("rows");
        assert_eq!(first.num_rows(), 200);
        assert_eq!(
            first.schema().field(1).data_type(),
            &arrow::datatypes::DataType::Utf8
        );
        drop(cursor);
        let remaining = tokio::time::timeout(
            Duration::from_secs(10),
            first_text(&*session, "SELECT count(*)::text FROM oxyn_temp_rows"),
        )
        .await
        .expect("idle portal cleanup finishes");
        assert_eq!(remaining, "601");
        assert_eq!(
            first_text(&*session, "SELECT pg_backend_pid()::text").await,
            owner
        );
    }
    let prepared = session
        .prepare_preview(&path, 50, &PreviewShape::default(), &CancelToken::new())
        .await
        .expect("prepared again");
    let mut cursor = session
        .execute_original_preview(prepared.request, &CancelToken::new())
        .await
        .expect("whole bounded stream");
    let mut count = 0;
    while let Some(batch) = cursor.next_batch().await.expect("remaining batch") {
        assert!(batch.num_rows() <= 200);
        count += batch.num_rows();
    }
    assert_eq!(count, 601);
    drop(cursor);
    assert_eq!(
        first_text(&*session, "SELECT count(*)::text FROM oxyn_temp_rows").await,
        "601"
    );
    session.close().await.expect("close session");
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn original_preview_catalog_does_not_publish_another_sessions_temp_schema() {
    let Some(first) = session().await else {
        return;
    };
    let Some(second) = session().await else {
        return;
    };
    apply(&*first, "CREATE TEMP TABLE private_rows (id integer)").await;
    let owned = first_text(
        &*first,
        "SELECT nspname::text FROM pg_namespace WHERE oid=pg_my_temp_schema()",
    )
    .await;
    let names = second
        .catalog()
        .list_namespaces(None, &CancelToken::new())
        .await
        .expect("second catalog");
    assert!(!names.iter().any(|namespace| namespace.name() == owned));
    first.close().await.expect("first close");
    second.close().await.expect("second close");
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn original_preview_streams_a_large_logical_result_in_byte_bounded_batches() {
    let Some(source) = session().await else {
        return;
    };
    apply(&*source, "CREATE TEMP VIEW oxyn_original_large AS SELECT i, repeat('x', 32768) AS payload FROM generate_series(1, 1000000) AS n(i)").await;
    let namespace = first_text(
        &*source,
        "SELECT nspname::text FROM pg_namespace WHERE oid=pg_my_temp_schema()",
    )
    .await;
    let path = oxyn_catalog::CatalogPath::for_relation(
        None,
        Some(namespace.as_str()),
        "oxyn_original_large",
    )
    .expect("path");
    let prepared = source
        .prepare_preview(&path, 50, &PreviewShape::default(), &CancelToken::new())
        .await
        .expect("prepared");
    let mut cursor = source
        .execute_original_preview(prepared.request, &CancelToken::new())
        .await
        .expect("stream");
    // The logical payload exceeds 30 GiB; only requested batches reach Oxyn.
    for _ in 0..12 {
        let batch = cursor
            .next_batch()
            .await
            .expect("batch read")
            .expect("more data");
        assert!(batch.get_array_memory_size() < 2 * 1024 * 1024);
        assert!(batch.num_rows() > 0);
    }
    drop(cursor);
    source
        .finish_original_preview(&CancelToken::new())
        .await
        .expect("cleanup acknowledged");
    assert_eq!(
        first_text(
            &*source,
            "SELECT (to_regclass('pg_temp.oxyn_original_large') IS NOT NULL)::text"
        )
        .await,
        "true"
    );
    source.close().await.expect("source close");
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn original_preview_cancellation_stops_an_active_fetch_on_the_server() {
    let Some(source) = session().await else {
        return;
    };
    let Some(observer) = session().await else {
        return;
    };
    apply(
        &*source,
        "CREATE TEMP VIEW oxyn_original_wait AS SELECT 1 AS id, pg_sleep(30)::text AS paused",
    )
    .await;
    let pid: i32 = first_text(&*source, "SELECT pg_backend_pid()::text")
        .await
        .parse()
        .expect("backend pid");
    let namespace = first_text(
        &*source,
        "SELECT nspname::text FROM pg_namespace WHERE oid=pg_my_temp_schema()",
    )
    .await;
    let path = oxyn_catalog::CatalogPath::for_relation(
        None,
        Some(namespace.as_str()),
        "oxyn_original_wait",
    )
    .expect("path");
    let prepared = source
        .prepare_preview(&path, 50, &PreviewShape::default(), &CancelToken::new())
        .await
        .expect("prepared");
    let mut cursor = source
        .execute_original_preview(prepared.request, &CancelToken::new())
        .await
        .expect("portal");
    let handle = cursor.handle();
    let active = format!(
        "SELECT count(*)::text FROM pg_stat_activity WHERE pid = {pid} AND state = 'active'"
    );
    let mut next = Box::pin(cursor.next_batch());
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                result = &mut next => panic!("FETCH should still be active: {result:?}"),
                () = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            if first_text(&*observer, &active).await == "1" {
                break;
            }
        }
    })
    .await
    .expect("server observed the active fetch");
    let (read, cancelled) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(next, source.cancel(handle))
    })
    .await
    .expect("server cancellation is bounded");
    assert!(matches!(read, Err(OxynError::Cancelled)));
    cancelled.expect("server cancellation request");
    drop(cursor);
    source
        .finish_original_preview(&CancelToken::new())
        .await
        .expect("cleanup task joined");
    tokio::time::timeout(Duration::from_secs(2), async {
        while first_text(&*observer, &active).await != "0" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the server query actually stopped");
    // An interrupted fetch whose restoration was unproven cannot become a
    // fresh backend under the same logical owner and lose its state silently.
    assert!(
        source
            .execute(read_request("SELECT 1"), &CancelToken::new())
            .await
            .is_err()
    );
    source.close().await.expect("source close");
    observer.close().await.expect("observer close");
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn original_preview_sql_error_preserves_temporary_rows_and_backend_identity() {
    let Some(source) = session().await else {
        return;
    };
    apply(
        &*source,
        "CREATE TEMP TABLE oxyn_original_invalid_filter (id integer)",
    )
    .await;
    apply(
        &*source,
        "INSERT INTO oxyn_original_invalid_filter VALUES (42)",
    )
    .await;
    let pid = first_text(&*source, "SELECT pg_backend_pid()::text").await;
    assert!(
        source
            .execute(
                read_request("SELECT missing_column FROM oxyn_original_invalid_filter"),
                &CancelToken::new(),
            )
            .await
            .is_err()
    );
    assert_eq!(
        first_text(&*source, "SELECT pg_backend_pid()::text").await,
        pid
    );
    let namespace = first_text(
        &*source,
        "SELECT nspname::text FROM pg_namespace WHERE oid=pg_my_temp_schema()",
    )
    .await;
    let path = oxyn_catalog::CatalogPath::for_relation(
        None,
        Some(namespace.as_str()),
        "oxyn_original_invalid_filter",
    )
    .expect("path");
    let shape = PreviewShape {
        predicate: Some("missing_column = 1".into()),
        ..Default::default()
    };
    let prepared = source
        .prepare_preview(&path, 50, &shape, &CancelToken::new())
        .await
        .expect("predicate prepared as supplied");
    assert!(
        source
            .execute_original_preview(prepared.request, &CancelToken::new())
            .await
            .is_err()
    );
    assert_eq!(
        first_text(
            &*source,
            "SELECT count(*)::text FROM oxyn_original_invalid_filter"
        )
        .await,
        "1"
    );
    assert_eq!(
        first_text(&*source, "SELECT pg_backend_pid()::text").await,
        pid
    );
    source.close().await.expect("close source");
}
