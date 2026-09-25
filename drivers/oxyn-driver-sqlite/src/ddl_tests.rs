//! What the destructive review may say about SQLite, proven against the
//! engine the driver embeds
//! ([ADR-0042](../../../docs/adr/0042-revue-sur-place-des-operations-destructrices.md)).
//!
//! One flag is declared, `TRANSACTIONAL_DDL`, and two are not: the tests of
//! their absence matter as much, since the review writes « the server
//! refuses » only under `RESTRICT_DEPENDENTS`.

use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, Environment, ExecLimits, ExecRequest,
    QueryLanguage,
};
use oxyn_driver::{Credentials, Driver as _, Session};

use crate::SqliteDriver;

async fn workshop() -> Box<dyn Session> {
    let config = ConnectionConfig::new("ddl", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
    SqliteDriver::new()
        .connect(&config, &Credentials::new(), &CancelToken::new())
        .await
        .expect("an in-memory database always opens")
}

fn writable(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::SQL, sql).with_limits(ExecLimits::unbounded())
}

/// Runs `sql`; `Err` with the engine's message when it refuses.
async fn run(session: &dyn Session, sql: &str) -> Result<(), String> {
    let mut cursor = session
        .execute(writable(sql), &CancelToken::new())
        .await
        .map_err(|error| error.to_string())?;
    while cursor
        .next_batch()
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {}
    Ok(())
}

async fn apply(session: &dyn Session, sql: &str) {
    if let Err(error) = run(session, sql).await {
        panic!("`{sql}` must be accepted: {error}");
    }
}

async fn exists(session: &dyn Session, table: &str) -> bool {
    run(session, &format!("SELECT 1 FROM {table} LIMIT 0"))
        .await
        .is_ok()
}

#[tokio::test]
async fn a_drop_follows_the_enclosing_transaction() {
    let session = workshop().await;
    assert!(
        session
            .capabilities()
            .contains(Capabilities::TRANSACTIONAL_DDL)
    );
    apply(&*session, "CREATE TABLE kept (id INTEGER)").await;
    apply(&*session, "BEGIN").await;
    apply(&*session, "DROP TABLE kept").await;
    assert!(!exists(&*session, "kept").await);
    apply(&*session, "ROLLBACK").await;
    assert!(exists(&*session, "kept").await, "ROLLBACK gives it back");

    apply(&*session, "BEGIN").await;
    apply(&*session, "ALTER TABLE kept RENAME TO renamed").await;
    apply(&*session, "ROLLBACK").await;
    assert!(exists(&*session, "kept").await, "a rename is undone too");
    session.close().await.expect("close");
}

#[tokio::test]
async fn there_is_no_truncate_statement() {
    let session = workshop().await;
    assert!(!session.capabilities().contains(Capabilities::TRUNCATE));
    apply(&*session, "CREATE TABLE emptied (id INTEGER)").await;
    let refusal = run(&*session, "TRUNCATE TABLE emptied")
        .await
        .expect_err("SQLite has no TRUNCATE");
    assert!(refusal.contains("syntax error"), "{refusal}");
    session.close().await.expect("close");
}

#[tokio::test]
async fn a_drop_goes_through_whatever_depends_on_the_table() {
    let session = workshop().await;
    assert!(
        !session
            .capabilities()
            .contains(Capabilities::RESTRICT_DEPENDENTS)
    );
    for sql in [
        "CREATE TABLE parent (id INTEGER PRIMARY KEY)",
        "CREATE TABLE child (parent INTEGER REFERENCES parent)",
        "CREATE TABLE viewed (id INTEGER)",
        "CREATE VIEW over_viewed AS SELECT id FROM viewed",
    ] {
        apply(&*session, sql).await;
    }
    // A declared key and a view hold nothing back. Only rows would: the
    // bundled engine is built with `SQLITE_DEFAULT_FOREIGN_KEYS=1`, and the
    // implicit `DELETE` of a `DROP` then fails on a child row. That is a
    // check of the data, not a refusal because of a dependent object.
    apply(&*session, "DROP TABLE parent").await;
    apply(&*session, "DROP TABLE viewed").await;
    assert!(!exists(&*session, "parent").await);
    assert!(!exists(&*session, "viewed").await);
    session.close().await.expect("close");
}

#[tokio::test]
async fn foreign_keys_are_enforced_by_the_bundled_engine() {
    // Pinned because the session's module header and ADR-0042 said the
    // opposite: the review must not tell a SQLite user that nothing checks
    // their child rows.
    let session = workshop().await;
    for sql in [
        "CREATE TABLE parent (id INTEGER PRIMARY KEY)",
        "CREATE TABLE child (parent INTEGER REFERENCES parent)",
        "INSERT INTO parent VALUES (1)",
        "INSERT INTO child VALUES (1)",
    ] {
        apply(&*session, sql).await;
    }
    let refusal = run(&*session, "DROP TABLE parent")
        .await
        .expect_err("the child row still references it");
    assert!(refusal.contains("FOREIGN KEY"), "{refusal}");
    session.close().await.expect("close");
}
