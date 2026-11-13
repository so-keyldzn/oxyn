//! Temporary table metadata must not come from a persistent homonym.
use super::*;

#[tokio::test]
#[ignore = "requires an isolated MySQL or MariaDB test server"]
async fn original_preview_of_a_select_only_view_does_not_require_show_view() {
    let Some(owner) = session().await else {
        return;
    };
    let Some((mut config, _)) = target() else {
        return;
    };
    let database = first(&*owner, "SELECT DATABASE()").await;
    let path = CatalogPath::for_relation(None, Some(database.as_str()), "oxyn_select_only_view")
        .expect("view path");
    let quoted = path.qualify(oxyn_catalog::path::QuoteStyle::Backtick);
    apply(&*owner, "DROP USER IF EXISTS 'oxyn_preview_reader'@'%'").await;
    apply(&*owner, "CREATE USER 'oxyn_preview_reader'@'%'").await;
    apply(
        &*owner,
        &format!("CREATE OR REPLACE VIEW {quoted} AS SELECT 42 AS id"),
    )
    .await;
    apply(
        &*owner,
        &format!("GRANT SELECT ON {quoted} TO 'oxyn_preview_reader'@'%'"),
    )
    .await;
    config
        .params
        .insert("user".into(), "oxyn_preview_reader".into());
    let reader = MysqlDriver::new()
        .connect(&config, &Credentials::default(), &CancelToken::new())
        .await
        .expect("select-only account");
    assert_eq!(first(&*reader, "SELECT 1").await, "1");
    let prepared = reader
        .prepare_preview(&path, 50, &PreviewShape::default(), &CancelToken::new())
        .await
        .expect("SELECT does not require SHOW VIEW");
    assert_eq!(prepared.mode, oxyn_driver::PreviewMode::Original);
    let mut cursor = reader
        .execute_original_preview(prepared.request, &CancelToken::new())
        .await
        .expect("readable view");
    assert_eq!(
        cursor
            .next_batch()
            .await
            .expect("batch")
            .expect("row")
            .num_rows(),
        1
    );
    drop(cursor);
    reader.close().await.expect("reader close");
    apply(&*owner, &format!("DROP VIEW {quoted}")).await;
    apply(&*owner, "DROP USER 'oxyn_preview_reader'@'%'").await;
    owner.close().await.expect("owner close");
}

#[tokio::test]
#[ignore = "requires an isolated MySQL or MariaDB test server"]
async fn original_preview_reads_the_temporary_shadow_and_keeps_it_after_release() {
    let Some(source) = session().await else {
        return;
    };
    let Some(sibling) = session().await else {
        return;
    };
    let database = first(&*source, "SELECT DATABASE()").await;
    apply(&*sibling, "DROP TABLE IF EXISTS oxyn_original_shadow").await;
    apply(
        &*source,
        "CREATE TABLE oxyn_original_shadow (persistent_column INT PRIMARY KEY)",
    )
    .await;
    apply(
        &*source,
        "CREATE TEMPORARY TABLE oxyn_original_shadow (local_value INT)",
    )
    .await;
    let values = (1..=601)
        .map(|id| format!("({id})"))
        .collect::<Vec<_>>()
        .join(",");
    apply(
        &*source,
        &format!("INSERT INTO oxyn_original_shadow VALUES {values}"),
    )
    .await;
    let path = oxyn_catalog::CatalogPath::for_relation(
        None,
        Some(database.as_str()),
        "oxyn_original_shadow",
    )
    .expect("path");
    let local = source
        .catalog()
        .describe_relation(&path, &CancelToken::new())
        .await
        .expect("temporary metadata");
    let permanent = sibling
        .catalog()
        .describe_relation(&path, &CancelToken::new())
        .await
        .expect("persistent metadata");
    assert!(local.field("local_value").is_some());
    assert!(local.field("persistent_column").is_none());
    assert!(permanent.field("persistent_column").is_some());
    assert!(local.retained_preview);
    assert!(!local.has_preview_key());
    assert!(
        source
            .catalog()
            .list_indexes(&path, &CancelToken::new())
            .await
            .expect("temporary indexes")
            .is_empty()
    );
    let owner = first(&*source, "SELECT CONNECTION_ID()").await;
    for _ in 0..3 {
        let prepared = source
            .prepare_preview(
                &path,
                50,
                &oxyn_core::PreviewShape::default(),
                &CancelToken::new(),
            )
            .await
            .expect("prepared local read");
        assert_eq!(prepared.mode, oxyn_driver::PreviewMode::Original);
        let mut cursor = source
            .execute_original_preview(prepared.request, &CancelToken::new())
            .await
            .expect("original read");
        assert!(cursor.next_batch().await.expect("batch").is_some());
        drop(cursor);
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(10),
                first(&*source, "SELECT COUNT(*) FROM oxyn_original_shadow")
            )
            .await
            .expect("cleanup"),
            "601"
        );
        assert_eq!(first(&*source, "SELECT CONNECTION_ID()").await, owner);
    }
    // Dropping the session-local shadow reveals the persistent object again.
    apply(&*source, "DROP TEMPORARY TABLE oxyn_original_shadow").await;
    apply(&*sibling, "DROP TABLE oxyn_original_shadow").await;
    source.close().await.expect("source close");
    sibling.close().await.expect("sibling close");
}
