use oxyn_catalog::CatalogPath;
use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, Environment, ExecLimits, ExecRequest, PreviewShape,
    QueryLanguage, SqlDialect,
};
use oxyn_driver::{Credentials, Driver, PreviewMode, Session};

async fn open() -> Box<dyn Session> {
    open_at(":memory:").await
}

async fn open_at(path: &str) -> Box<dyn Session> {
    crate::SqliteDriver::new()
        .connect(
            &ConnectionConfig::new("preview context", DriverId::sqlite())
                .with_environment(Environment::Local)
                .with_param("path", path),
            &Credentials::new(),
            &CancelToken::new(),
        )
        .await
        .expect("own memory session")
}

async fn sql(session: &dyn Session, text: &str) {
    let mut cursor = session
        .execute(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), text)
                .with_limits(ExecLimits::unbounded()),
            &CancelToken::new(),
        )
        .await
        .expect("fixture statement");
    while cursor.next_batch().await.expect("drain").is_some() {}
}

#[tokio::test]
async fn attached_files_are_reopened_read_only_under_the_same_quoted_name() {
    let directory = tempfile::tempdir().expect("own fixture directory");
    let file = directory.path().join("file?mode=rw#%.sqlite");
    let session = open().await;
    let escaped = file.to_string_lossy().replace('\'', "''");
    sql(&*session, &format!("ATTACH DATABASE '{escaped}' AS \"a\"\"b\"; CREATE TABLE \"a\"\"b\".items (id INTEGER); INSERT INTO \"a\"\"b\".items VALUES (1),(2); CREATE VIEW \"a\"\"b\".records AS SELECT * FROM items")).await;
    let path = CatalogPath::for_relation(None, Some("a\"b"), "records").expect("quoted alias");
    let prepared = session
        .prepare_preview(&path, 50, &PreviewShape::default(), &CancelToken::new())
        .await
        .expect("prepared");
    assert_eq!(prepared.mode, PreviewMode::Dedicated);
    let sibling = session
        .open_preview_session(&path, &CancelToken::new())
        .await
        .expect("same attachment");
    let mut cursor = sibling
        .execute_preview(prepared.request, &CancelToken::new())
        .await
        .expect("read");
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch().await.expect("batch") {
        rows += batch.num_rows();
    }
    assert_eq!(rows, 2);
    drop(cursor);
    let write = ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Sqlite),
        "DELETE FROM \"a\"\"b\".items",
    )
    .with_limits(ExecLimits::unbounded());
    assert!(sibling.execute(write, &CancelToken::new()).await.is_err());
    sibling.close().await.expect("private close");
    session.close().await.expect("source close");
}

#[tokio::test]
async fn temporary_or_attached_memory_state_never_substitutes_another_object() {
    for setup in [
        "CREATE TEMP TABLE shadow (id INTEGER)",
        "ATTACH DATABASE ':memory:' AS attached",
    ] {
        let session = open().await;
        sql(&*session, "CREATE TABLE items (id INTEGER); INSERT INTO items VALUES (1); CREATE VIEW records AS SELECT * FROM items").await;
        sql(&*session, setup).await;
        let path = CatalogPath::for_relation(None, Some("main"), "records").expect("path");
        let prepared = session
            .prepare_preview(&path, 50, &PreviewShape::default(), &CancelToken::new())
            .await
            .expect("original session preview");
        assert_eq!(prepared.mode, PreviewMode::Original);
        assert!(!prepared.request.text.contains("LIMIT 50"));
        assert!(
            session
                .open_preview_session(&path, &CancelToken::new())
                .await
                .is_err()
        );
        session.close().await.expect("close");
    }
}

#[path = "preview_uri_tests.rs"]
mod uri;
