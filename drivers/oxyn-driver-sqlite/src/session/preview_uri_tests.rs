use super::*;

async fn assert_private_rows(source: &dyn Session) {
    let path = CatalogPath::for_relation(None, Some("main"), "records").expect("path");
    let shape = PreviewShape {
        continuable: true,
        ..Default::default()
    };
    let mut prepared = source
        .prepare_preview(&path, 1, &shape, &CancelToken::new())
        .await
        .expect("prepared");
    assert_eq!(prepared.mode, PreviewMode::Dedicated);
    prepared.request.limits.max_rows = None;
    let reader = source
        .open_preview_session(&path, &CancelToken::new())
        .await
        .expect("same URI target with read-only access");
    let mut cursor = reader
        .execute_preview(prepared.request, &CancelToken::new())
        .await
        .expect("read");
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch().await.expect("batch") {
        rows += batch.num_rows();
    }
    assert_eq!(rows, 3);
    drop(cursor);
    let write = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), "DELETE FROM items")
        .with_limits(ExecLimits::unbounded());
    assert!(reader.execute(write, &CancelToken::new()).await.is_err());
    reader.close().await.expect("close reader");
}

const SEED: &str = "CREATE TABLE items (id INTEGER); INSERT INTO items VALUES (1),(2),(3); CREATE VIEW records AS SELECT * FROM items";

#[tokio::test]
async fn writable_uri_modes_keep_read_only_preview_on_original_database() {
    let directory = tempfile::tempdir().expect("own database");
    let file = directory.path().join("uri.sqlite");
    let source = open_at(file.to_str().expect("UTF-8 fixture path")).await;
    sql(&*source, SEED).await;
    source.close().await.expect("seed closed");
    for query in [
        "mode=rw",
        "mode=rwc",
        "%6dode=%72%77%63",
        "mode=ro",
        "cache=shared&mode=rw&immutable=1",
    ] {
        let uri = format!("file:{}?{query}#mode=rwc", file.display());
        let source = open_at(&uri).await;
        assert_private_rows(&*source).await;
        source.close().await.expect("source closed");
    }
}

#[tokio::test]
async fn a_named_shared_memory_uri_keeps_its_identity_and_memory_mode() {
    let uri = format!(
        "file:preview-{}?mode=memory&cache=shared",
        oxyn_core::CommandId::new()
    );
    let source = open_at(&uri).await;
    sql(&*source, SEED).await;
    assert_private_rows(&*source).await;
    source.close().await.expect("closed");
    let source = open_at("file::memory:?cache=shared").await;
    sql(&*source, SEED).await;
    assert_private_rows(&*source).await;
    source.close().await.expect("closed shared :memory: URI");
}

#[tokio::test]
async fn private_memory_and_temporary_uris_never_read_a_fresh_empty_database() {
    for uri in [
        "file:private-preview?mode=memory",
        "file::memory:",
        "file:",
        "file:?mode=memory&cache=shared",
    ] {
        let source = open_at(uri).await;
        sql(&*source, SEED).await;
        let path = CatalogPath::for_relation(None, Some("main"), "records").expect("path");
        let relation = source
            .catalog()
            .describe_relation(&path, &CancelToken::new())
            .await
            .expect("source metadata");
        assert!(relation.retained_preview, "{uri}");
        let prepared = source
            .prepare_preview(&path, 50, &PreviewShape::default(), &CancelToken::new())
            .await
            .expect("original source preview");
        assert_eq!(prepared.mode, PreviewMode::Original);
        let mut cursor = source
            .execute_preview(prepared.request, &CancelToken::new())
            .await
            .expect("original source");
        assert_eq!(
            cursor
                .next_batch()
                .await
                .expect("batch")
                .expect("rows")
                .num_rows(),
            3
        );
        drop(cursor);
        assert!(
            source
                .open_preview_session(&path, &CancelToken::new())
                .await
                .is_err()
        );
        source.close().await.expect("closed");
    }
}

#[tokio::test]
async fn a_removed_uri_file_is_not_recreated_by_a_private_reader() {
    let directory = tempfile::tempdir().expect("own database");
    let file = directory.path().join("removed.sqlite");
    let uri = format!("file:{}?mode=rwc", file.display());
    let source = open_at(&uri).await;
    sql(&*source, SEED).await;
    std::fs::remove_file(&file).expect("remove only our fixture");
    let path = CatalogPath::for_relation(None, Some("main"), "records").expect("path");
    assert!(
        source
            .open_preview_session(&path, &CancelToken::new())
            .await
            .is_err()
    );
    assert!(!file.exists());
    source.close().await.expect("closed");
}
