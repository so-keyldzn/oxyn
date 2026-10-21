//! A batch whose later statement fails after an earlier one applied is never
//! told as retryable (issue #221, [I-13](../../../CLAUDE.md#i-13)).
//!
//! The failure is a real lock, not a simulated code. The session writes `main`
//! freely and attaches a second file, `aux`, that a plain `rusqlite` connection
//! holds in a write transaction: the first statement of a batch can apply while
//! a later one aimed at `aux` is refused with `SQLITE_BUSY`. `PRAGMA
//! busy_timeout = 0` makes the refusal immediate — the driver leaves `rusqlite`'s
//! default of five seconds in place (see the module note of `session`).

use oxyn_core::{CancelToken, ConnectionConfig, DriverId, Environment, ErrorClass, OxynError};
use oxyn_driver::{Credentials, Driver as _, Session};
use rusqlite::Connection;

use crate::SqliteDriver;
use crate::stopped_write_tests::{count, run, writable};

/// The statement the lock on `aux` refuses.
const BLOCKED: &str = "UPDATE aux.u SET v = v + 1";

/// A session on `main.db`, with `aux.db` attached and locked by someone else.
struct Workshop {
    session: Box<dyn Session>,
    /// Keeps `aux.db` in a write transaction: a second write to it is refused.
    _holder: Connection,
    /// Declared last, hence dropped last: the files outlive both connections.
    _dir: tempfile::TempDir,
}

impl Workshop {
    async fn open() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory can be created");
        let aux = dir.path().join("aux.db");
        let main = dir.path().join("main.db");

        // The holder creates the attached file's content, then locks it: the
        // session must find the table where it attaches.
        let holder = Connection::open(&aux).expect("a plain connection opens a fresh file");
        holder
            .execute_batch(
                "CREATE TABLE u(id INTEGER PRIMARY KEY, v INTEGER); INSERT INTO u VALUES (1, 0);",
            )
            .expect("the attached file's fixture is valid SQL");

        let config = ConnectionConfig::new("busy-batch", DriverId::sqlite())
            .with_environment(Environment::Local)
            .with_param(
                SqliteDriver::PATH,
                main.to_str().expect("a temporary path is UTF-8"),
            );
        let session = SqliteDriver::new()
            .connect(&config, &Credentials::new(), &CancelToken::new())
            .await
            .expect("a file in a fresh directory opens");

        // Typed as the user would: the driver never sets it behind their back.
        run(session.as_ref(), "PRAGMA busy_timeout = 0").await;
        let attach = format!(
            "ATTACH DATABASE '{}' AS aux",
            aux.to_str()
                .expect("a temporary path is UTF-8")
                .replace('\'', "''")
        );
        run(session.as_ref(), &attach).await;
        run(
            session.as_ref(),
            "CREATE TABLE main.t(id INTEGER PRIMARY KEY, v INTEGER)",
        )
        .await;

        holder
            .execute_batch("BEGIN IMMEDIATE")
            .expect("the holder takes the write lock of the attached file");
        Self {
            session,
            _holder: holder,
            _dir: dir,
        }
    }

    fn session(&self) -> &dyn Session {
        self.session.as_ref()
    }
}

/// The error a batch ends with, whether `execute` or a later batch request
/// reports it.
async fn failure_of(session: &dyn Session, sql: &str) -> OxynError {
    let mut cursor = match session.execute(writable(sql), &CancelToken::new()).await {
        Ok(cursor) => cursor,
        Err(err) => return err,
    };
    loop {
        match cursor.next_batch().await {
            Ok(Some(_)) => {}
            Ok(None) => panic!("the batch was meant to fail: {sql}"),
            Err(err) => return err,
        }
    }
}

#[tokio::test]
async fn a_busy_first_statement_stays_transient() {
    // Nothing ran before it: replaying is safe, and `SQLITE_BUSY` is exactly the
    // case a retry exists for. It is also the precondition of every test below —
    // the holder really blocks the statement.
    let workshop = Workshop::open().await;

    let err = failure_of(workshop.session(), BLOCKED).await;

    assert_eq!(err.class(), ErrorClass::Transient, "{err:?}");
    assert!(err.is_retryable());
    assert!(err.to_string().contains("locked"), "{err}");
    assert!(
        !err.to_string().contains("already applied"),
        "nothing was applied before it: {err}"
    );
}

#[tokio::test]
async fn a_busy_update_after_an_applied_insert_is_not_retryable_and_names_the_applied_statements() {
    let workshop = Workshop::open().await;

    let err = failure_of(
        workshop.session(),
        &format!("INSERT INTO main.t VALUES (1, 0); {BLOCKED}"),
    )
    .await;

    assert!(!err.is_retryable(), "replaying would insert twice: {err:?}");
    assert!(!err.is_cancelled(), "{err:?}");
    assert_eq!(
        err.class(),
        ErrorClass::Permanent,
        "the update itself applied nothing: {err:?}"
    );
    let shown = err.to_string();
    assert!(shown.contains("already applied"), "{shown}");
    assert!(
        shown.contains("locked"),
        "the cause stays readable: {shown}"
    );
    assert_eq!(
        count(workshop.session()).await,
        1,
        "the insert committed before the update was refused"
    );
}

#[tokio::test]
async fn a_busy_update_after_a_returning_insert_is_not_retryable() {
    // A statement with columns that is not the last one is run and its rows are
    // thrown away (`discard`): that path records what applied as well.
    let workshop = Workshop::open().await;

    let err = failure_of(
        workshop.session(),
        &format!("INSERT INTO main.t VALUES (1, 0) RETURNING id; {BLOCKED}"),
    )
    .await;

    assert!(!err.is_retryable(), "{err:?}");
    assert!(!err.is_cancelled(), "{err:?}");
    assert!(err.to_string().contains("already applied"), "{err}");
    assert_eq!(count(workshop.session()).await, 1);
}

#[tokio::test]
async fn a_busy_update_after_a_read_stays_transient() {
    // A `SELECT` applies nothing: the batch can still be replayed as a whole.
    let workshop = Workshop::open().await;

    let err = failure_of(workshop.session(), &format!("SELECT 1; {BLOCKED}")).await;

    assert_eq!(err.class(), ErrorClass::Transient, "{err:?}");
    assert!(err.is_retryable());
}

#[tokio::test]
async fn a_busy_update_inside_a_transaction_leaves_the_insert_to_the_transaction() {
    // Inside an explicit transaction the earlier statement is applied but not
    // committed: the message must claim neither a commit nor a pending state it
    // cannot know.
    let workshop = Workshop::open().await;
    run(workshop.session(), "BEGIN").await;

    let err = failure_of(
        workshop.session(),
        &format!("INSERT INTO main.t VALUES (1, 0); {BLOCKED}"),
    )
    .await;

    assert!(!err.is_retryable(), "{err:?}");
    assert!(!err.is_cancelled(), "{err:?}");
    let shown = err.to_string();
    assert!(shown.contains("already applied"), "{shown}");
    assert!(shown.contains("follow its fate"), "{shown}");
    assert_eq!(
        count(workshop.session()).await,
        1,
        "visible in the transaction"
    );

    run(workshop.session(), "ROLLBACK").await;
    assert_eq!(
        count(workshop.session()).await,
        0,
        "it followed the transaction's fate, as the message said"
    );
}

#[tokio::test]
async fn a_permanent_failure_after_an_applied_insert_stays_permanent() {
    // The class is not inflated to ambiguous: a constraint or a missing table is
    // what it says it is. Only the statement that applied is reported.
    for (name, batch) in [
        (
            "constraint, at execution",
            "INSERT INTO main.t VALUES (1, 0); INSERT INTO main.t VALUES (1, 0)",
        ),
        (
            "missing table, at preparation",
            "INSERT INTO main.t VALUES (1, 0); SELECT * FROM missing_table",
        ),
    ] {
        let workshop = Workshop::open().await;

        let err = failure_of(workshop.session(), batch).await;

        assert_eq!(err.class(), ErrorClass::Permanent, "{name}: {err:?}");
        assert!(!err.is_cancelled(), "{name}: {err:?}");
        assert!(err.to_string().contains("already applied"), "{name}: {err}");
        assert_eq!(count(workshop.session()).await, 1, "{name}");
    }
}
