//! The server-side proof of the three flags the destructive review reads
//! ([ADR-0042](../../../docs/adr/0042-revue-sur-place-des-operations-destructrices.md)):
//! `TRUNCATE`, `TRANSACTIONAL_DDL`, `RESTRICT_DEPENDENTS`. A flag the review
//! trusts to write « applied whole or not at all » or « the server refuses »
//! must hold against the engine, not against its documentation.
//!
//! All `#[ignore]`: run them as `integration.rs` says. The session's pool lends
//! one connection per execution, so a transaction cannot span two statements
//! here: a `DO` block is the enclosing transaction, and its `RAISE` the
//! rollback.

use arrow::array::{Array, Int64Array};
use oxyn_core::{CancelToken, Capabilities, ExecLimits, ExecRequest, QueryLanguage, SqlDialect};
use oxyn_driver::Session;

use crate::integration::{appliquer, session};

fn writable(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_limits(ExecLimits::default().writable().with_max_rows(None))
}

/// Whether the server refused `sql`, draining it when it did not.
async fn refused(session: &dyn Session, sql: &str) -> bool {
    match session.execute(writable(sql), &CancelToken::new()).await {
        Err(_) => true,
        Ok(mut cursor) => loop {
            match cursor.next_batch().await {
                Ok(Some(_)) => {}
                Ok(None) => break false,
                Err(_) => break true,
            }
        },
    }
}

/// The single `bigint` a `SELECT count(*)` answers.
async fn count(session: &dyn Session, sql: &str) -> i64 {
    let mut cursor = session
        .execute(writable(sql), &CancelToken::new())
        .await
        .unwrap_or_else(|error| panic!("`{sql}`: {error}"));
    let batch = cursor
        .next_batch()
        .await
        .expect("a count streams")
        .expect("a count has one row");
    let column = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count(*) is a bigint");
    assert_eq!(column.len(), 1);
    column.value(0)
}

async fn exists(session: &dyn Session, table: &str) -> bool {
    count(
        session,
        &format!("SELECT count(*) FROM pg_catalog.pg_tables WHERE tablename = '{table}'"),
    )
    .await
        == 1
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn truncate_is_accepted_and_empties_the_table() {
    let Some(session) = session().await else {
        return;
    };
    assert!(session.capabilities().contains(Capabilities::TRUNCATE));
    appliquer(&*session, "CREATE TABLE oxyn_ddl_truncate (id integer)").await;
    appliquer(&*session, "INSERT INTO oxyn_ddl_truncate VALUES (1), (2)").await;
    appliquer(&*session, "TRUNCATE TABLE oxyn_ddl_truncate").await;
    assert_eq!(
        count(&*session, "SELECT count(*) FROM oxyn_ddl_truncate").await,
        0
    );
    appliquer(&*session, "DROP TABLE oxyn_ddl_truncate").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn ddl_follows_the_enclosing_transaction_and_applies_whole_or_not_at_all() {
    let Some(session) = session().await else {
        return;
    };
    assert!(
        session
            .capabilities()
            .contains(Capabilities::TRANSACTIONAL_DDL)
    );
    appliquer(&*session, "CREATE TABLE oxyn_ddl_kept (id integer)").await;
    appliquer(&*session, "INSERT INTO oxyn_ddl_kept VALUES (1), (2)").await;

    // A DROP, then a failure in the same transaction: the table comes back.
    assert!(
        refused(
            &*session,
            "DO $$ BEGIN DROP TABLE oxyn_ddl_kept; RAISE EXCEPTION 'rolled back'; END $$",
        )
        .await
    );
    assert!(exists(&*session, "oxyn_ddl_kept").await);

    // TRUNCATE obeys it too: the rows come back.
    assert!(
        refused(
            &*session,
            "DO $$ BEGIN TRUNCATE TABLE oxyn_ddl_kept; RAISE EXCEPTION 'rolled back'; END $$",
        )
        .await
    );
    assert_eq!(
        count(&*session, "SELECT count(*) FROM oxyn_ddl_kept").await,
        2
    );

    // One statement that fails halfway leaves nothing applied.
    assert!(refused(&*session, "DROP TABLE oxyn_ddl_kept, oxyn_ddl_missing").await);
    assert!(exists(&*session, "oxyn_ddl_kept").await);

    appliquer(&*session, "DROP TABLE oxyn_ddl_kept").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn drop_and_truncate_are_refused_while_another_object_depends_on_the_table() {
    let Some(session) = session().await else {
        return;
    };
    assert!(
        session
            .capabilities()
            .contains(Capabilities::RESTRICT_DEPENDENTS)
    );
    for sql in [
        "CREATE TABLE oxyn_ddl_parent (id integer PRIMARY KEY)",
        "CREATE TABLE oxyn_ddl_child (parent integer REFERENCES oxyn_ddl_parent)",
        "CREATE TABLE oxyn_ddl_viewed (id integer)",
        "CREATE VIEW oxyn_ddl_view AS SELECT id FROM oxyn_ddl_viewed",
    ] {
        appliquer(&*session, sql).await;
    }

    assert!(refused(&*session, "DROP TABLE oxyn_ddl_parent").await);
    assert!(refused(&*session, "TRUNCATE TABLE oxyn_ddl_parent").await);
    assert!(refused(&*session, "DROP TABLE oxyn_ddl_viewed").await);
    assert!(exists(&*session, "oxyn_ddl_parent").await);
    assert!(exists(&*session, "oxyn_ddl_viewed").await);

    appliquer(&*session, "DROP TABLE oxyn_ddl_parent CASCADE").await;
    appliquer(&*session, "DROP TABLE oxyn_ddl_viewed CASCADE").await;
    appliquer(&*session, "DROP TABLE oxyn_ddl_child").await;
    session.close().await.expect("close");
}
