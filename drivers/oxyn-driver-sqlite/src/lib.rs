//! Oxyn's SQLite driver: embedded, synchronous, without network.
//!
//! It is the simple case of the driver layer, and that is why it must be
//! beyond reproach: the PostgreSQL driver will be reviewed against it. The
//! underlying contract is authoritative in
//! [DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md); it is not copied here.
//!
//! | Module | Subject |
//! |---|---|
//! | [`driver`] | [`SqliteDriver`]: identity, connection form, capabilities |
//! | [`session`] | [`SqliteSession`]: execution, transactions, closing |
//! | [`cursor`] | [`SqliteCursor`]: Arrow batches, and the cancellation that cuts |
//! | [`catalog`] | [`SqliteCatalog`]: `sqlite_master` and the `PRAGMA`s |
//! | [`convert`] | SQLite's dynamic typing brought down to Arrow columns |
//! | [`params`] | `ScalarValue` to the five storage classes |
//! | [`error`] | the transient / permanent / ambiguous classification |
//! | [`options`] | [`BatchLimits`]: a batch bounded in rows **and** in bytes |
//!
//! # The four decisions that govern this crate
//!
//! **The connection lives on a thread of its own.** `rusqlite::Connection` is
//! not `Sync`, and a `Statement` borrows the connection: streaming a result batch
//! by batch requires keeping this borrow alive between two `await`s, which no
//! future can do without `unsafe`. The connection therefore stays on a dedicated
//! thread, and what crosses the channel is already converted `RecordBatch`. The
//! full reasoning is in [`crate::worker`].
//!
//! **Cancellation is local, and it is stated as such.** SQLite has no server:
//! `sqlite3_interrupt` stops a statement in **our** process. The session
//! therefore never declares `SERVER_SIDE_CANCEL`, and `SqliteSession::cancel`
//! refuses — but interruption does exist, through the
//! [`CancelToken`](oxyn_core::CancelToken) handed to `execute`, checked between
//! two batches and watched while waiting for a batch.
//!
//! **A column's type is decided once, from the data.** In SQLite the type
//! belongs to the value, not to the column; Arrow requires the opposite. The
//! first batch is set aside as raw values while deciding, and a column mixing
//! storage classes falls back to text — or to bytes — **declaring it** in the
//! metadata of its field. See [`convert`].
//!
//! **A batch is measured in bytes as much as in rows.** A thousand rows each
//! carrying a one-megabyte BLOB make a gigabyte ([`BatchLimits`]).
//!
//! # Example
//!
//! ```no_run
//! use oxyn_core::{CancelToken, ConnectionConfig, DriverId, Environment, ExecRequest,
//!                 QueryLanguage};
//! use oxyn_driver::{Credentials, Cursor, Driver, Session};
//! use oxyn_driver_sqlite::SqliteDriver;
//!
//! # async fn example() -> oxyn_core::Result<()> {
//! let driver = SqliteDriver::new();
//! let connection = ConnectionConfig::new("workshop", DriverId::sqlite())
//!     .with_environment(Environment::Local)
//!     .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
//!
//! let token = CancelToken::new();
//! let session = driver.connect(&connection, &Credentials::new(), &token).await?;
//!
//! // The default limits are cautious: bounded and read-only.
//! let mut cursor = session
//!     .execute(ExecRequest::new(QueryLanguage::SQL, "SELECT 1 AS n"), &token)
//!     .await?;
//! while let Some(batch) = cursor.next_batch().await? {
//!     println!("{} row(s)", batch.num_rows());
//! }
//! # Ok(())
//! # }
//! ```

pub mod catalog;
mod constraints;
pub mod convert;
pub mod cursor;
#[cfg(test)]
mod ddl_tests;
mod definition;
pub mod driver;
pub mod error;
mod incoming_keys;
mod interrupt;
pub mod options;
pub mod params;
mod preview;
mod schema_sql;
pub mod session;
#[cfg(test)]
mod stopped_write_tests;
pub mod stream;
mod virtual_tables;
#[cfg(test)]
mod virtual_tables_tests;
pub mod worker;

pub use catalog::{MAIN, SqliteCatalog, logical_type, partial_predicate};
pub use convert::{
    ColumnKind, METADATA_DECLARED_TYPE, METADATA_INFERRED, METADATA_STORAGE_CLASSES, Observed,
    affinity,
};
pub use cursor::SqliteCursor;
pub use driver::SqliteDriver;
pub use error::{SqliteError, classify};
pub use options::BatchLimits;
pub use session::SqliteSession;

#[cfg(test)]
mod tests {
    use arrow::array::{Array, BinaryArray, Float64Array, Int64Array, StringArray};
    use arrow::record_batch::RecordBatch;
    use oxyn_catalog::model::{LogicalType, RelationKind};
    use oxyn_catalog::path::CatalogPath;
    use oxyn_core::{
        CancelToken, Capabilities, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits,
        ExecRequest, OxynError, PreviewShape, PreviewSort, QueryLanguage, ScalarValue,
    };
    use oxyn_driver::{Credentials, Cursor, Driver, Session};

    use super::*;

    #[tokio::test]
    async fn constraint_introspection_crosses_the_worker_and_honors_session_capabilities() {
        let session = workshop().await;
        assert!(session.capabilities().contains(Capabilities::CONSTRAINTS));
        run_to_end(&*session, "CREATE TABLE constraints_fixture (id INTEGER PRIMARY KEY, value TEXT CONSTRAINT value_present NOT NULL CHECK(length(value) > 0))").await;
        let path =
            CatalogPath::for_relation(None, Some("main"), "constraints_fixture").expect("path");
        let constraints = session
            .catalog()
            .list_constraints(&path, &CancelToken::new())
            .await
            .expect("worker metadata");
        assert_eq!(constraints.len(), 3);
        assert!(
            constraints
                .iter()
                .any(|constraint| constraint.name == "value_present")
        );
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            session.catalog().list_constraints(&path, &cancel).await,
            Err(OxynError::Cancelled)
        ));
        let wrong_catalog = CatalogPath::for_relation(
            Some("not_a_sqlite_level"),
            Some("main"),
            "constraints_fixture",
        )
        .expect("structured path");
        assert!(matches!(
            session
                .catalog()
                .list_constraints(&wrong_catalog, &CancelToken::new())
                .await,
            Err(OxynError::CatalogUnavailable(_))
        ));
        session.close().await.expect("close fixture");
    }

    /// A session on an in-memory database, private to the test.
    async fn session(limits: BatchLimits) -> Box<dyn Session> {
        let driver = SqliteDriver::new().with_batch_limits(limits);
        let conn_config = ConnectionConfig::new("workshop", DriverId::sqlite())
            .with_environment(Environment::Local)
            .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
        driver
            .connect(&conn_config, &Credentials::new(), &CancelToken::new())
            .await
            .expect("an in-memory database always opens")
    }

    async fn workshop() -> Box<dyn Session> {
        session(BatchLimits::default()).await
    }

    /// The error of an execution that had to be refused.
    ///
    /// `Result::expect_err` requires `Debug` on the `Ok` variant, hence here on
    /// `dyn Cursor`. The cursor holds the session, which holds the connection
    /// credentials: giving it `Debug` would put a secret one `{:?}` away
    /// ([I-03]). Going through `match` requires nothing of `T`.
    ///
    /// [I-03]: ../../../CLAUDE.md#i-03
    fn expect_refusal<T>(issue: Result<T, OxynError>, expected_ids: &str) -> OxynError {
        match issue {
            Ok(_) => panic!("{expected_ids}"),
            Err(err) => err,
        }
    }

    /// A write request: the default limits are read-only.
    fn write_request(sql: &str) -> ExecRequest {
        ExecRequest::new(QueryLanguage::SQL, sql).with_limits(ExecLimits::unbounded())
    }

    /// A read request, with the cautious default limits.
    fn read_request(sql: &str) -> ExecRequest {
        ExecRequest::new(QueryLanguage::SQL, sql)
    }

    /// Executes a write and returns the number of affected rows.
    async fn run_to_end(session: &dyn Session, sql: &str) -> u64 {
        let cancel_token = CancelToken::new();
        let batch_cursor = session
            .execute(write_request(sql), &cancel_token)
            .await
            .unwrap_or_else(|err| panic!("execution of `{sql}`: {err}"));
        batch_cursor.stats().rows
    }

    /// Drains a cursor and returns its batches.
    async fn drain_all(batch_cursor: &mut Box<dyn Cursor>) -> Vec<RecordBatch> {
        let mut all_batches = Vec::new();
        while let Some(one_batch) = batch_cursor.next_batch().await.expect("next batch") {
            all_batches.push(one_batch);
        }
        all_batches
    }

    /// The first batch of a read.
    async fn first_batch(session: &dyn Session, sql: &str) -> RecordBatch {
        let cancel_token = CancelToken::new();
        let mut batch_cursor = session
            .execute(read_request(sql), &cancel_token)
            .await
            .unwrap_or_else(|err| panic!("read of `{sql}`: {err}"));
        batch_cursor
            .next_batch()
            .await
            .expect("first batch")
            .expect("at least one row")
    }

    #[tokio::test]
    async fn the_full_journey_create_insert_read() {
        let session = workshop().await;
        run_to_end(
            session.as_ref(),
            "CREATE TABLE clients(id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        )
        .await;
        let affected_rows = run_to_end(
            session.as_ref(),
            "INSERT INTO clients(id, name) VALUES (1, 'Ada'), (2, 'Grace')",
        )
        .await;
        assert_eq!(affected_rows, 2, "the affected row count must come back");

        let one_batch =
            first_batch(session.as_ref(), "SELECT id, name FROM clients ORDER BY id").await;
        assert_eq!(one_batch.num_rows(), 2);
        assert_eq!(one_batch.num_columns(), 2);

        let ids = one_batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("Int64 column");
        assert_eq!(ids.value(0), 1);
        let column_names = one_batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Utf8 column");
        assert_eq!(column_names.value(1), "Grace");

        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn the_five_storage_classes_become_arrow_columns() {
        let session = workshop().await;
        run_to_end(
            session.as_ref(),
            "CREATE TABLE t(n INTEGER, x REAL, s TEXT, b BLOB, empty_col TEXT)",
        )
        .await;
        run_to_end(
            session.as_ref(),
            "INSERT INTO t VALUES (7, 1.5, 'café', x'00ff', NULL)",
        )
        .await;

        let one_batch = first_batch(session.as_ref(), "SELECT n, x, s, b, empty_col FROM t").await;
        assert_eq!(
            one_batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            7
        );
        assert!(
            (one_batch
                .column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("Float64")
                .value(0)
                - 1.5)
                .abs()
                < f64::EPSILON
        );
        assert_eq!(
            one_batch
                .column(2)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("Utf8")
                .value(0),
            "café"
        );
        assert_eq!(
            one_batch
                .column(3)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .expect("Binary")
                .value(0),
            b"\x00\xff",
            "a BLOB stays opaque, it does not become text \"at best\""
        );
        // The entirely null column falls back on its declared type.
        assert!(one_batch.column(4).is_null(0));
        let schema = one_batch.schema();
        assert_eq!(
            schema.field(4).data_type(),
            &arrow::datatypes::DataType::Utf8
        );
    }

    #[tokio::test]
    async fn a_column_mixing_types_falls_back_to_text_and_declares_it() {
        // SQLite accepts this: the type belongs to the value, not to the column.
        // The interface must be able to say that the rendering is a fallback.
        let session = workshop().await;
        run_to_end(session.as_ref(), "CREATE TABLE m(v INTEGER)").await;
        run_to_end(
            session.as_ref(),
            "INSERT INTO m(v) VALUES (1), ('abc'), (2.5)",
        )
        .await;

        let one_batch = first_batch(session.as_ref(), "SELECT v FROM m ORDER BY rowid").await;
        let field_def = one_batch.schema().field(0).clone();
        assert_eq!(field_def.data_type(), &arrow::datatypes::DataType::Utf8);
        assert_eq!(
            field_def
                .metadata()
                .get(METADATA_INFERRED)
                .map(String::as_str),
            Some("true"),
            "the type does not come from the declaration: it must say it is inferred"
        );
        let classes = field_def
            .metadata()
            .get(METADATA_STORAGE_CLASSES)
            .map(String::as_str)
            .expect("mixed classes must be declared");
        assert!(
            classes.contains("integer") && classes.contains("text"),
            "{classes}"
        );

        let column_values = one_batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Utf8");
        assert_eq!(column_values.value(0), "1");
        assert_eq!(column_values.value(1), "abc");
        assert_eq!(column_values.value(2), "2.5");
    }

    #[tokio::test]
    async fn a_write_is_refused_when_the_request_declares_itself_read_only() {
        // The `ExecLimits` default is cautious: read-only. The question is asked
        // of the engine (`sqlite3_stmt_readonly`), not of the text.
        let session = workshop().await;
        run_to_end(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;

        let cancel_token = CancelToken::new();
        let err = expect_refusal(
            session
                .execute(read_request("INSERT INTO t(v) VALUES (1)"), &cancel_token)
                .await,
            "expected refusal",
        );
        assert!(matches!(err, OxynError::PolicyDenied { .. }), "{err:?}");

        // And nothing was written.
        let one_batch = first_batch(session.as_ref(), "SELECT count(*) FROM t").await;
        assert_eq!(
            one_batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            0
        );
    }

    #[tokio::test]
    async fn a_bound_value_stays_a_value() {
        // I-10: values are bound, not concatenated.
        let session = workshop().await;
        run_to_end(session.as_ref(), "CREATE TABLE audit(note TEXT)").await;

        let cancel_token = CancelToken::new();
        let exec_request = read_request("SELECT ?1 AS v").with_params(vec![ScalarValue::Text(
            "'); DROP TABLE audit; --".to_owned(),
        )]);
        let mut batch_cursor = session
            .execute(exec_request, &cancel_token)
            .await
            .expect("execution");
        let one_batch = batch_cursor
            .next_batch()
            .await
            .expect("batch")
            .expect("one row");
        assert_eq!(
            one_batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("Utf8")
                .value(0),
            "'); DROP TABLE audit; --"
        );
        drop(batch_cursor);

        let remaining = first_batch(
            session.as_ref(),
            "SELECT count(*) FROM sqlite_master WHERE name = 'audit'",
        )
        .await;
        assert_eq!(
            remaining
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            1,
            "the audit table was dropped"
        );
    }

    /// An unlikely value, so that a test looking for it finds it only if it
    /// really went through.
    const SENTINEL: &str = "S3NT1N3L-42";

    /// A table whose trigger copies the inserted value into its message: it is
    /// the leak the fix closes.
    async fn chatty_workshop() -> Box<dyn Session> {
        let session = workshop().await;
        run_to_end(session.as_ref(), "CREATE TABLE accounts(note TEXT)").await;
        run_to_end(
            session.as_ref(),
            "CREATE TRIGGER refuser BEFORE INSERT ON accounts \
             BEGIN SELECT RAISE(ABORT, 'balance: ' || NEW.note); END",
        )
        .await;
        session
    }

    #[tokio::test]
    async fn a_bound_value_comes_out_neither_on_screen_nor_in_history() {
        // I-03: the engine message is displayed by the console, persisted by
        // `HistoryRecord::failed` and `JournalRecord::failed` — which both call
        // `error.to_string()` —, and a `tracing::debug!` would render it through
        // `Debug`. All three renderings count.
        let session = chatty_workshop().await;
        let exec_request = write_request("INSERT INTO accounts(note) VALUES (?1)")
            .with_params(vec![ScalarValue::Text(SENTINEL.to_owned())]);
        let err = expect_refusal(
            session.execute(exec_request, &CancelToken::new()).await,
            "the trigger must refuse the insertion",
        );

        let displayed_text = format!("{err}");
        let persisted_text = err.to_string();
        let debugged = format!("{err:?}");
        for shown in [&displayed_text, &persisted_text, &debugged] {
            assert!(!shown.contains(SENTINEL), "bound value rendered: {shown}");
            assert!(
                !shown.contains("balance"),
                "engine message rendered: {shown}"
            );
        }
        assert!(
            displayed_text.contains("withheld"),
            "the withdrawal must be stated, otherwise the user hunts for a bug: {displayed_text}"
        );
        // The extended result code stays readable: `SQLITE_CONSTRAINT_TRIGGER`.
        assert!(displayed_text.contains("1811"), "{displayed_text}");
        assert_eq!(err.class(), ErrorClass::Permanent, "{err:?}");
    }

    #[tokio::test]
    async fn without_bound_value_the_engine_message_arrives_whole() {
        // The protection must not apply wrongly: Oxyn's audience reads its
        // engine's messages.
        let session = chatty_workshop().await;
        let exec_request =
            write_request(&format!("INSERT INTO accounts(note) VALUES ('{SENTINEL}')"));
        let err = expect_refusal(
            session.execute(exec_request, &CancelToken::new()).await,
            "the trigger must refuse the insertion",
        );
        assert!(
            err.to_string().contains(&format!("balance: {SENTINEL}")),
            "the engine message must pass unchanged: {err}"
        );
    }

    #[tokio::test]
    async fn a_read_failing_on_a_bound_value_also_withholds_its_message() {
        // The read path fails in `sqlite3_step`, far from the request: that is
        // where the information "there were bound values" must be passed, not
        // guessed.
        let session = workshop().await;
        let exec_request =
            read_request("SELECT abs(?1)").with_params(vec![ScalarValue::Int64(i64::MIN)]);
        let err = expect_refusal(
            session.execute(exec_request, &CancelToken::new()).await,
            "`abs` overflows on the smallest integer",
        );
        assert!(
            !err.to_string().contains("integer overflow"),
            "engine message rendered: {err}"
        );
        assert!(err.to_string().contains("withheld"), "{err}");

        // The same overflow written in clear keeps its message.
        let without_binding = expect_refusal(
            session
                .execute(
                    read_request("SELECT abs(-9223372036854775808)"),
                    &CancelToken::new(),
                )
                .await,
            "`abs` overflows on the smallest integer",
        );
        assert!(
            without_binding.to_string().contains("integer overflow"),
            "{without_binding}"
        );
    }

    #[tokio::test]
    async fn cancellation_also_cuts_a_read_with_bound_values() {
        // Withholding a message must not turn a cancellation into a failure.
        let session = session(BatchLimits::new().with_max_rows(10)).await;
        let cancel_token = CancelToken::new();
        let exec_request = ExecRequest::new(
            QueryLanguage::SQL,
            "WITH RECURSIVE series(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM series WHERE n < ?1) \
             SELECT n FROM series",
        )
        .with_limits(ExecLimits::unbounded())
        .with_params(vec![ScalarValue::Int64(50_000)]);

        let mut batch_cursor = session
            .execute(exec_request, &cancel_token)
            .await
            .expect("execution");
        assert!(
            batch_cursor
                .next_batch()
                .await
                .expect("first batch")
                .is_some_and(|one_batch| one_batch.num_rows() == 10)
        );

        cancel_token.cancel();
        let err = expect_refusal(batch_cursor.next_batch().await, "cancellation must cut");
        assert!(err.is_cancelled(), "{err:?}");
        assert!(batch_cursor.stats().truncated);

        drop(batch_cursor);
        session.ping().await.expect("the session stays usable");
    }

    #[tokio::test]
    async fn a_result_arrives_in_several_bounded_batches() {
        // The stream: no result is materialized in full (I-06).
        let session = session(BatchLimits::new().with_max_rows(100)).await;
        run_to_end(session.as_ref(), "CREATE TABLE big(n INTEGER)").await;
        run_to_end(
            session.as_ref(),
            "WITH RECURSIVE series(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM series WHERE n < 5000) \
             INSERT INTO big(n) SELECT n FROM series",
        )
        .await;

        let cancel_token = CancelToken::new();
        let mut batch_cursor = session
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM big ORDER BY n")
                    .with_limits(ExecLimits::unbounded()),
                &cancel_token,
            )
            .await
            .expect("execution");
        let all_batches = drain_all(&mut batch_cursor).await;

        let row_count: usize = all_batches.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(row_count, 5_000);
        assert!(
            all_batches.len() >= 50,
            "{} batch(es) for 5,000 rows",
            all_batches.len()
        );
        assert!(
            all_batches
                .iter()
                .all(|one_batch| one_batch.num_rows() <= 100),
            "a batch exceeds the row bound"
        );
        let stats = batch_cursor.stats();
        assert_eq!(stats.rows, 5_000);
        assert!(!stats.truncated, "the result is complete");
        assert!(stats.server_time.is_none(), "SQLite has no server clock");
    }

    #[tokio::test]
    async fn a_result_truncated_by_the_row_bound_says_so() {
        let session = workshop().await;
        run_to_end(session.as_ref(), "CREATE TABLE t(n INTEGER)").await;
        run_to_end(
            session.as_ref(),
            "INSERT INTO t(n) VALUES (1), (2), (3), (4), (5)",
        )
        .await;

        let cancel_token = CancelToken::new();
        let mut batch_cursor = session
            .execute(
                read_request("SELECT n FROM t ORDER BY n")
                    .with_limits(ExecLimits::default().with_max_rows(2)),
                &cancel_token,
            )
            .await
            .expect("execution");
        let all_batches = drain_all(&mut batch_cursor).await;

        let row_count: usize = all_batches.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(row_count, 2);
        assert!(
            batch_cursor.stats().truncated,
            "a truncated result that looks complete leads to wrong conclusions"
        );
    }

    #[tokio::test]
    async fn a_write_at_the_row_bound_is_not_stepped_past_it() {
        // Exactly N rows: a read steps once more and proves the result whole;
        // a write does not, like the executor's own end probe, and its result
        // stays truncated.
        let session = workshop().await;
        run_to_end(session.as_ref(), "CREATE TABLE t(n INTEGER)").await;
        run_to_end(session.as_ref(), "CREATE TABLE u(n INTEGER)").await;
        run_to_end(session.as_ref(), "INSERT INTO t(n) VALUES (1), (2)").await;

        for (request, truncated) in [
            (
                read_request("SELECT n FROM t").with_limits(ExecLimits::default().with_max_rows(2)),
                false,
            ),
            (
                write_request("INSERT INTO u(n) SELECT n FROM t RETURNING n")
                    .with_limits(ExecLimits::default().writable().with_max_rows(2)),
                true,
            ),
        ] {
            let mut batch_cursor = session
                .execute(request, &CancelToken::new())
                .await
                .expect("execution");
            let all_batches = drain_all(&mut batch_cursor).await;
            let row_count: usize = all_batches.iter().map(RecordBatch::num_rows).sum();
            assert_eq!(row_count, 2);
            assert_eq!(batch_cursor.stats().truncated, truncated);
        }
    }

    #[tokio::test]
    async fn cancellation_cuts_the_read_between_two_batches() {
        let session = session(BatchLimits::new().with_max_rows(10)).await;
        run_to_end(session.as_ref(), "CREATE TABLE big(n INTEGER)").await;
        run_to_end(
            session.as_ref(),
            "WITH RECURSIVE series(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM series WHERE n < 2000) \
             INSERT INTO big(n) SELECT n FROM series",
        )
        .await;

        let cancel_token = CancelToken::new();
        let mut batch_cursor = session
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM big ORDER BY n")
                    .with_limits(ExecLimits::unbounded()),
                &cancel_token,
            )
            .await
            .expect("execution");

        let first_batch_opt = batch_cursor.next_batch().await.expect("first batch");
        assert!(first_batch_opt.is_some_and(|one_batch| one_batch.num_rows() == 10));

        cancel_token.cancel();

        let err = batch_cursor
            .next_batch()
            .await
            .expect_err("cancellation must cut");
        assert!(err.is_cancelled(), "{err:?}");
        assert!(
            batch_cursor.stats().truncated,
            "a cancelled result is incomplete, and must say so"
        );

        // The session stays usable: the cancellation freed the worker thread.
        drop(batch_cursor);
        session
            .ping()
            .await
            .expect("the session survives the cancellation");
    }

    #[tokio::test]
    async fn server_side_cancel_is_refused_not_simulated() {
        // SQLite has no server. Suggesting that a "Cancel" cuts a query on the
        // server side would assert what does not exist.
        let session = workshop().await;
        assert!(
            !session
                .capabilities()
                .contains(Capabilities::SERVER_SIDE_CANCEL),
            "{}",
            session.capabilities()
        );
        let err = session
            .cancel(oxyn_core::StatementHandle::new())
            .await
            .expect_err("expected refusal");
        assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
        assert!(err.is_user_error(), "it is not an incident");
    }

    #[tokio::test]
    async fn a_rolled_back_transaction_leaves_nothing() {
        // The worst case would be to succeed without opening anything: the user
        // would believe a ROLLBACK undid their write.
        let session = workshop().await;
        let cancel_token = CancelToken::new();
        run_to_end(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;

        session.begin(&cancel_token).await.expect("open");
        run_to_end(session.as_ref(), "INSERT INTO t(v) VALUES (1)").await;
        session.rollback(&cancel_token).await.expect("cancellation");

        let one_batch = first_batch(session.as_ref(), "SELECT count(*) FROM t").await;
        assert_eq!(
            one_batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            0,
            "the ROLLBACK must have undone for good"
        );

        // And a committed transaction does stay.
        session.begin(&cancel_token).await.expect("open");
        run_to_end(session.as_ref(), "INSERT INTO t(v) VALUES (2)").await;
        session.commit(&cancel_token).await.expect("validation");
        let one_batch = first_batch(session.as_ref(), "SELECT count(*) FROM t").await;
        assert_eq!(
            one_batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            1
        );
    }

    #[tokio::test]
    async fn a_multi_statement_batch_runs_everything_and_returns_the_last_result() {
        let session = workshop().await;
        let cancel_token = CancelToken::new();
        let mut batch_cursor = session
            .execute(
                write_request(
                    "CREATE TABLE m(x INTEGER); \
                     INSERT INTO m(x) VALUES (1), (2); \
                     SELECT x FROM m ORDER BY x",
                ),
                &cancel_token,
            )
            .await
            .expect("batch execution");
        let all_batches = drain_all(&mut batch_cursor).await;
        let row_count: usize = all_batches.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(
            row_count, 2,
            "the cursor carries the result of the last one"
        );

        // A statement following the row producer runs all the same.
        let affected_rows = run_to_end(
            session.as_ref(),
            "SELECT x FROM m; INSERT INTO m(x) VALUES (3)",
        )
        .await;
        assert_eq!(affected_rows, 1);
        let one_batch = first_batch(session.as_ref(), "SELECT count(*) FROM m").await;
        assert_eq!(
            one_batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            3
        );
    }

    #[tokio::test]
    async fn bound_parameters_with_a_multi_statement_batch_are_refused() {
        let session = workshop().await;
        let cancel_token = CancelToken::new();
        let exec_request =
            write_request("SELECT ?1; SELECT 2").with_params(vec![ScalarValue::Int64(1)]);
        let err = expect_refusal(
            session.execute(exec_request, &cancel_token).await,
            "nothing says which statement they relate to",
        );
        assert!(!err.is_retryable(), "{err:?}");
    }

    #[tokio::test]
    async fn an_undeclared_language_is_refused_not_translated() {
        let session = workshop().await;
        let cancel_token = CancelToken::new();
        let err = expect_refusal(
            session
                .execute(
                    ExecRequest::new(QueryLanguage::Cypher, "MATCH (n) RETURN n"),
                    &cancel_token,
                )
                .await,
            "expected refusal",
        );
        assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_syntax_error_is_permanent_and_showable() {
        let session = workshop().await;
        let cancel_token = CancelToken::new();
        let err = expect_refusal(
            session
                .execute(read_request("SLECT 1"), &cancel_token)
                .await,
            "expected refusal",
        );
        assert!(
            !err.is_retryable(),
            "a syntax error is never retried: {err:?}"
        );
        assert!(err.to_string().contains("sqlite"), "{err}");
    }

    #[tokio::test]
    async fn introspection_describes_what_the_database_contains() {
        let session = workshop().await;
        run_to_end(
            session.as_ref(),
            "CREATE TABLE clients(\
                 id INTEGER PRIMARY KEY, \
                 name TEXT NOT NULL, \
                 balance DECIMAL(10,2) DEFAULT '0.00')",
        )
        .await;
        run_to_end(
            session.as_ref(),
            "CREATE TABLE orders(\
                 id INTEGER PRIMARY KEY, \
                 client_id INTEGER REFERENCES clients(id) ON DELETE CASCADE)",
        )
        .await;
        run_to_end(
            session.as_ref(),
            "CREATE UNIQUE INDEX idx_name ON clients(name)",
        )
        .await;
        run_to_end(
            session.as_ref(),
            "CREATE VIEW v_clients AS SELECT id FROM clients",
        )
        .await;

        let cancel_token = CancelToken::new();
        let catalog_view = session.catalog();

        let info = catalog_view
            .server_info(&cancel_token)
            .await
            .expect("identity");
        assert_eq!(info.product, "SQLite");
        assert!(!info.version.is_empty());

        // SQLite has no catalog tier; its databases occupy the namespace
        // tier.
        assert!(
            catalog_view
                .list_catalogs(&cancel_token)
                .await
                .expect("empty")
                .is_empty()
        );
        let namespaces_seen = catalog_view
            .list_namespaces(None, &cancel_token)
            .await
            .expect("namespaces");
        assert!(
            namespaces_seen.iter().any(|ns| ns.name() == MAIN),
            "`main` must always be there"
        );

        let relations = catalog_view
            .list_relations(&CatalogPath::empty(), &cancel_token)
            .await
            .expect("relations");
        let column_names: Vec<&str> = relations.iter().map(|r| r.name()).collect();
        assert!(column_names.contains(&"clients"), "{column_names:?}");
        assert!(column_names.contains(&"v_clients"), "{column_names:?}");
        assert!(
            relations
                .iter()
                .any(|r| r.name() == "v_clients" && r.kind == RelationKind::View),
            "a view is not a table"
        );

        let rel_path = CatalogPath::for_relation(None, Some(MAIN), "clients").expect("path");
        let described = catalog_view
            .describe_relation(&rel_path, &cancel_token)
            .await
            .expect("description");
        assert_eq!(described.kind, RelationKind::Table);
        assert_eq!(described.fields.len(), 3);
        assert_eq!(
            described
                .primary_key()
                .first()
                .map(|field_def| field_def.name.as_str()),
            Some("id")
        );
        let name = described.field("name").expect("column `name`");
        assert!(!name.nullable, "NOT NULL must come back as is");
        assert_eq!(name.logical_type, LogicalType::Text);
        assert_eq!(name.raw_type, "TEXT", "the server type is kept");
        let balance = described.field("balance").expect("column `balance`");
        assert_eq!(
            balance.logical_type,
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2)
            }
        );
        assert_eq!(balance.default.as_deref(), Some("'0.00'"));
        assert_eq!(
            described.estimated_rows, None,
            "SQLite cannot estimate without counting, and `None` is not zero"
        );

        let index = catalog_view
            .list_indexes(&rel_path, &cancel_token)
            .await
            .expect("index");
        let idx = index
            .iter()
            .find(|index| index.name == "idx_name")
            .expect("the declared index");
        assert!(idx.unique);
        assert_eq!(idx.fields, ["name"]);
        assert!(!idx.is_partial());

        let orders = CatalogPath::for_relation(None, Some(MAIN), "orders").expect("path");
        let fk_list = catalog_view
            .list_foreign_keys(&orders, &cancel_token)
            .await
            .expect("foreign keys");
        let fk = fk_list.first().expect("one key");
        assert!(fk.is_well_formed(), "both sides must pair: {fk:?}");
        assert_eq!(fk.fields, ["client_id"]);
        assert_eq!(fk.references.fields, ["id"]);
        assert_eq!(fk.references.relation.relation(), Some("clients"));
        assert!(
            fk.on_delete.propagates_delete(),
            "an ON DELETE CASCADE must be visible"
        );
    }

    #[tokio::test]
    async fn a_partial_index_carries_its_predicate() {
        let session = workshop().await;
        run_to_end(session.as_ref(), "CREATE TABLE t(a INTEGER, b INTEGER)").await;
        run_to_end(
            session.as_ref(),
            "CREATE INDEX idx_active ON t(a) WHERE b > 0",
        )
        .await;

        let cancel_token = CancelToken::new();
        let rel_path = CatalogPath::for_relation(None, Some(MAIN), "t").expect("path");
        let index = session
            .catalog()
            .list_indexes(&rel_path, &cancel_token)
            .await
            .expect("index");
        let partial_idx = index
            .iter()
            .find(|index| index.name == "idx_active")
            .expect("the declared index");
        assert!(partial_idx.is_partial());
        assert_eq!(partial_idx.predicate.as_deref(), Some("b > 0"));
    }

    #[tokio::test]
    async fn a_missing_relation_is_reported() {
        let session = workshop().await;
        let cancel_token = CancelToken::new();
        let rel_path = CatalogPath::for_relation(None, Some(MAIN), "ghost").expect("path");
        let err = session
            .catalog()
            .describe_relation(&rel_path, &cancel_token)
            .await
            .expect_err("the relation does not exist");
        assert!(!err.is_retryable(), "{err:?}");
    }

    #[tokio::test]
    async fn ping_answers_and_close_releases() {
        let session = workshop().await;
        session.ping().await.expect("the session is alive");
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_hostile_table_name_does_not_execute() {
        // I-10: `"users"; DROP TABLE audit; --` is a legal table name.
        let session = workshop().await;
        run_to_end(session.as_ref(), "CREATE TABLE audit(note TEXT)").await;
        run_to_end(
            session.as_ref(),
            r#"CREATE TABLE "clients""; DROP TABLE audit; --"(id INTEGER)"#,
        )
        .await;

        let cancel_token = CancelToken::new();
        let relations = session
            .catalog()
            .list_relations(&CatalogPath::empty(), &cancel_token)
            .await
            .expect("relations");
        let hostile = relations
            .iter()
            .find(|relation| relation.name().contains("DROP TABLE"))
            .expect("the hostile table does exist");

        // The introspection of this table goes through quoted identifiers and
        // bound values: nothing executes.
        let described = session
            .catalog()
            .describe_relation(&hostile.path(), &cancel_token)
            .await
            .expect("description");
        assert_eq!(described.fields.len(), 1);

        let remaining = first_batch(
            session.as_ref(),
            "SELECT count(*) FROM sqlite_master WHERE name = 'audit'",
        )
        .await;
        assert_eq!(
            remaining
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            1,
            "the audit table was dropped by a preview"
        );
    }
    #[tokio::test]
    async fn shared_memory_sessions_keep_profile_isolation_and_enforce_read_only_requests() {
        let driver = SqliteDriver::new();
        let mut config = ConnectionConfig::new("memory profile", DriverId::sqlite())
            .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
        let cancel = CancelToken::new();
        let writer = driver
            .connect(&config, &Credentials::new(), &cancel)
            .await
            .expect("writer");
        for text in [
            "CREATE TABLE shared_probe(n INTEGER)",
            "INSERT INTO shared_probe VALUES (1)",
        ] {
            let mut cursor = writer
                .execute(write_request(text), &cancel)
                .await
                .expect("write");
            while cursor.next_batch().await.expect("drain").is_some() {}
        }
        config.read_only = true;
        let reader = driver
            .connect(&config, &Credentials::new(), &cancel)
            .await
            .expect("read-only sibling");
        assert!(
            reader
                .capabilities()
                .contains(Capabilities::READ_ONLY_SESSION)
        );
        let mut cursor = reader
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM shared_probe"),
                &cancel,
            )
            .await
            .expect("shared table");
        assert_eq!(
            cursor
                .next_batch()
                .await
                .expect("rows")
                .expect("batch")
                .num_rows(),
            1
        );
        drop(cursor);
        assert!(
            reader
                .execute(
                    write_request("INSERT INTO shared_probe VALUES (2)"),
                    &cancel
                )
                .await
                .is_err(),
            "writable caller limits cannot lift a session restriction"
        );
        reader.close().await.expect("close reader");
        let mut cursor = writer
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM shared_probe"),
                &cancel,
            )
            .await
            .expect("writer survives sibling close");
        assert_eq!(
            cursor
                .next_batch()
                .await
                .expect("rows")
                .expect("batch")
                .num_rows(),
            1
        );
        drop(cursor);
        writer.close().await.expect("last session closes");
        config.read_only = false;
        let empty = driver
            .connect(&config, &Credentials::new(), &cancel)
            .await
            .expect("fresh memory lifetime");
        assert!(
            empty
                .execute(
                    ExecRequest::new(QueryLanguage::SQL, "SELECT * FROM shared_probe"),
                    &cancel
                )
                .await
                .is_err()
        );
    }

    // ── Sorted, filtered, paged previews (ADR-0020) ─────────────────────────────

    /// The number of rows of the preview table, enough to exceed three pages: a
    /// pagination skipping a row does not show on ten.
    const PREVIEW_ROWS: i64 = 500;

    /// A preview table, its ties, and a witness table that must survive.
    ///
    /// `bucket` is `id % 7`: sorting on it leaves seventy-one ties per value, hence
    /// a non-total order if the primary key does not complete it.
    async fn preview_workshop() -> Box<dyn Session> {
        let session = workshop().await;
        run_to_end(
            session.as_ref(),
            "CREATE TABLE preview_rows (id INTEGER PRIMARY KEY, bucket INTEGER, name TEXT)",
        )
        .await;
        run_to_end(
            session.as_ref(),
            &format!(
                "INSERT INTO preview_rows(id, bucket, name) \
                 WITH RECURSIVE seq(i) AS (\
                   SELECT 1 UNION ALL SELECT i + 1 FROM seq WHERE i < {PREVIEW_ROWS}) \
                 SELECT i, i % 7, 'row-' || i FROM seq"
            ),
        )
        .await;
        run_to_end(session.as_ref(), "CREATE TABLE witness (guard TEXT)").await;
        run_to_end(session.as_ref(), "INSERT INTO witness VALUES ('intact')").await;
        session
    }

    /// Composes then executes a preview, and returns its batches.
    async fn preview_rows(
        session: &dyn Session,
        relation: &str,
        limit: u32,
        shape: &PreviewShape,
    ) -> Vec<RecordBatch> {
        let cancel_token = CancelToken::new();
        let rel_path = CatalogPath::for_relation(None, Some(MAIN), relation).expect("valid path");
        let exec_request = session
            .preview_request(&rel_path, limit, shape, &cancel_token)
            .await
            .unwrap_or_else(|err| panic!("composing the preview of `{relation}`: {err}"));
        let mut batch_cursor = session
            .execute(exec_request, &cancel_token)
            .await
            .unwrap_or_else(|err| panic!("executing the preview of `{relation}`: {err}"));
        drain_all(&mut batch_cursor).await
    }

    /// The integers of a column, in the order the batches return them.
    fn ints_of(all_batches: &[RecordBatch], column_index: usize) -> Vec<i64> {
        all_batches
            .iter()
            .flat_map(|one_batch| {
                one_batch
                    .column(column_index)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .expect("integer column")
                    .values()
                    .to_vec()
            })
            .collect()
    }

    /// The texts of a column, in the order the batches return them.
    fn texts_of(all_batches: &[RecordBatch], column_index: usize) -> Vec<String> {
        all_batches
            .iter()
            .flat_map(|one_batch| {
                let column_values = one_batch
                    .column(column_index)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("text column");
                (0..column_values.len())
                    .map(|position| column_values.value(position).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[tokio::test]
    async fn a_sorted_preview_returns_rows_in_the_requested_order() {
        let session = preview_workshop().await;

        let ascending_shape = PreviewShape {
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        let ids = ints_of(
            &preview_rows(&*session, "preview_rows", 10, &ascending_shape).await,
            0,
        );
        assert_eq!(ids, (1..=10).collect::<Vec<_>>());

        let descending_shape = PreviewShape {
            sort: vec![PreviewSort::descending("id")],
            ..PreviewShape::default()
        };
        let ids = ints_of(
            &preview_rows(&*session, "preview_rows", 10, &descending_shape).await,
            0,
        );
        assert_eq!(
            ids,
            (PREVIEW_ROWS - 9..=PREVIEW_ROWS).rev().collect::<Vec<_>>()
        );

        // A sort on a column full of ties: the primary key completes the order,
        // so the result is predictable row by row.
        let by_bucket = PreviewShape {
            sort: vec![PreviewSort::descending("bucket")],
            ..PreviewShape::default()
        };
        let all_batches = preview_rows(&*session, "preview_rows", 10, &by_bucket).await;
        let expected_ids: Vec<i64> = {
            let mut all_ids: Vec<i64> = (1..=PREVIEW_ROWS).collect();
            all_ids.sort_by_key(|id| (-(id % 7), *id));
            all_ids.into_iter().take(10).collect()
        };
        assert_eq!(ints_of(&all_batches, 0), expected_ids);
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn consecutive_pages_neither_overlap_nor_skip_a_row() {
        let session = preview_workshop().await;
        let page_size = 200_u32;
        let mut seen_ids: Vec<i64> = Vec::new();
        let mut sizes = Vec::new();

        for page in 0..3_u64 {
            let shape = PreviewShape {
                // Sorting on the ties is the worst case: without the primary key
                // added by the driver, two pages would show the same row twice and
                // hide another.
                sort: vec![PreviewSort::ascending("bucket")],
                offset: page * u64::from(page_size),
                ..PreviewShape::default()
            };
            let all_batches = preview_rows(&*session, "preview_rows", page_size, &shape).await;
            let ids = ints_of(&all_batches, 0);
            sizes.push(ids.len());
            seen_ids.extend(ids);
        }

        assert_eq!(sizes, vec![200, 200, 100], "three pages, 500 rows");
        let mut sorted_ids = seen_ids.clone();
        sorted_ids.sort_unstable();
        sorted_ids.dedup();
        assert_eq!(
            sorted_ids.len(),
            seen_ids.len(),
            "no row must appear on two pages"
        );
        assert_eq!(
            sorted_ids,
            (1..=PREVIEW_ROWS).collect::<Vec<_>>(),
            "the union of the pages is exactly the table"
        );
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_page_beyond_the_last_is_empty_without_being_an_error() {
        let session = preview_workshop().await;
        let shape = PreviewShape {
            offset: 1_000,
            ..PreviewShape::default()
        };
        let all_batches = preview_rows(&*session, "preview_rows", 200, &shape).await;
        assert!(ints_of(&all_batches, 0).is_empty());
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn the_user_predicate_goes_as_is() {
        let session = preview_workshop().await;
        run_to_end(
            session.as_ref(),
            "INSERT INTO preview_rows(id, bucket, name) VALUES (1001, 0, '100%'), (1002, 0, '100 percent')",
        )
        .await;

        // The `%` is not a metacharacter here: the driver composes no pattern, it
        // passes on the user's text.
        let equality_shape = PreviewShape {
            predicate: Some("name = '100%'".into()),
            ..PreviewShape::default()
        };
        assert_eq!(
            texts_of(
                &preview_rows(&*session, "preview_rows", 200, &equality_shape).await,
                2
            ),
            vec!["100%".to_owned()],
            "an equality must bring back only the literal row"
        );

        // And when the user writes a pattern, it is a pattern indeed: their SQL
        // is neither escaped nor reinterpreted.
        let like_shape = PreviewShape {
            predicate: Some("name LIKE '100%'".into()),
            ..PreviewShape::default()
        };
        let mut found = texts_of(
            &preview_rows(&*session, "preview_rows", 200, &like_shape).await,
            2,
        );
        found.sort();
        assert_eq!(found, vec!["100 percent".to_owned(), "100%".to_owned()]);
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn both_guards_around_the_predicate_hold_on_the_engine() {
        let session = preview_workshop().await;
        let cancel_token = CancelToken::new();
        let rel_path = CatalogPath::for_relation(None, Some(MAIN), "preview_rows").expect("path");

        // 1. The unclosed block comment. No line break ends it: without the
        // parentheses, SQLite read until the end of the text, lost the LIMIT
        // and returned the whole table without anything reporting it. The
        // opening parenthesis makes the statement incomplete, hence refused.
        let sql_block = PreviewShape {
            predicate: Some("id > 0 /*".into()),
            ..PreviewShape::default()
        };
        let exec_request = session
            .preview_request(&rel_path, 3, &sql_block, &cancel_token)
            .await
            .expect("composition does not judge the predicate");
        expect_refusal(
            session.execute(exec_request, &cancel_token).await,
            "an unclosed block comment must be refused, not executed unbounded",
        );
        // The session survives this refusal: the relation stays readable.
        assert_eq!(
            ints_of(
                &preview_rows(&*session, "preview_rows", 3, &PreviewShape::unordered()).await,
                0
            )
            .len(),
            3
        );

        // 2. The end-of-line comment keeps working, and the LIMIT applies: the
        // line break is what allows it, and it stays useful.
        let single_row = PreviewShape {
            predicate: Some("id > 0 -- this is a comment".into()),
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        assert_eq!(
            ints_of(
                &preview_rows(&*session, "preview_rows", 3, &single_row).await,
                0
            ),
            vec![1, 2, 3]
        );

        // 3. A predicate already carrying its parentheses returns exactly what
        // the same text without the wrapper would return.
        let parenthesized = PreviewShape {
            predicate: Some("(id > 0 AND bucket < 2) OR name IS NULL".into()),
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        let wrapped = ints_of(
            &preview_rows(&*session, "preview_rows", 5, &parenthesized).await,
            0,
        );
        let mut batch_cursor = session
            .execute(
                read_request(
                    "SELECT * FROM \"main\".\"preview_rows\" \
                     WHERE (id > 0 AND bucket < 2) OR name IS NULL \
                     ORDER BY \"id\" ASC LIMIT 5",
                ),
                &cancel_token,
            )
            .await
            .expect("the same text, without wrapper");
        assert_eq!(wrapped, ints_of(&drain_all(&mut batch_cursor).await, 0));
        assert_eq!(wrapped, vec![1, 7, 8, 14, 15]);
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_hostile_predicate_destroys_nothing_and_does_not_overflow_its_clause() {
        let session = preview_workshop().await;
        let cancel_token = CancelToken::new();
        let rel_path = CatalogPath::for_relation(None, Some(MAIN), "preview_rows").expect("path");

        for hostile in [
            // A second statement: it must never execute.
            "name = 'row-1'; DROP TABLE witness",
            "name = 'row-1'; DELETE FROM witness",
            // An unbalanced quote: a syntax error, nothing more.
            "name = 'row-1",
            // An end-of-line comment: it must not swallow the LIMIT.
            "name LIKE 'row-%' -- ; DROP TABLE witness",
        ] {
            let shape = PreviewShape {
                predicate: Some(hostile.to_owned()),
                ..PreviewShape::default()
            };
            let exec_request = session
                .preview_request(&rel_path, 3, &shape, &cancel_token)
                .await
                .expect("composition does not judge the predicate");
            assert!(
                exec_request.text.contains(hostile),
                "the predicate goes as is: {}",
                exec_request.text
            );
            match session.execute(exec_request, &cancel_token).await {
                // Either the engine refuses — syntax, or a second statement that
                // writes under read-only limits…
                Err(_) => {}
                // … or it is an ordinary read, and it stays bounded.
                Ok(mut batch_cursor) => {
                    let row_count: usize = drain_all(&mut batch_cursor)
                        .await
                        .iter()
                        .map(RecordBatch::num_rows)
                        .sum();
                    assert!(
                        row_count <= 3,
                        "{hostile}: {row_count} rows despite LIMIT 3"
                    );
                }
            }
            let witness = first_batch(&*session, "SELECT guard FROM witness").await;
            assert_eq!(
                witness.num_rows(),
                1,
                "the witness table must survive `{hostile}`"
            );
        }
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_projection_returns_only_the_named_columns() {
        let session = preview_workshop().await;
        let form_shape = PreviewShape {
            columns: Some(vec!["name".into(), "id".into()]),
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        let all_batches = preview_rows(&*session, "preview_rows", 3, &form_shape).await;
        let schema = all_batches.first().expect("at least one batch").schema();
        let column_names: Vec<&str> = schema
            .fields()
            .iter()
            .map(|field_def| field_def.name().as_str())
            .collect();
        // `bucket`, not requested, is not read from the engine; the order is the
        // projection's, not the table's.
        assert_eq!(column_names, ["name", "id"]);
        assert_eq!(texts_of(&all_batches, 0), ["row-1", "row-2", "row-3"]);
        assert_eq!(ints_of(&all_batches, 1), [1, 2, 3]);

        let cancel_token = CancelToken::new();
        let rel_path = CatalogPath::for_relation(None, Some(MAIN), "preview_rows").expect("path");
        let unknown_shape = PreviewShape {
            columns: Some(vec!["missing_column".into()]),
            ..PreviewShape::default()
        };
        let err = expect_refusal(
            session
                .preview_request(&rel_path, 10, &unknown_shape, &cancel_token)
                .await,
            "an unknown column is not projected",
        );
        assert!(
            matches!(&err, OxynError::Query(message)
                if message.contains("missing_column")),
            "{err}"
        );
        assert!(!err.is_retryable(), "{err}");
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_shape_the_relation_does_not_allow_is_refused_saying_so() {
        let session = preview_workshop().await;
        run_to_end(session.as_ref(), "CREATE TABLE no_key (x TEXT)").await;
        run_to_end(session.as_ref(), "INSERT INTO no_key VALUES ('a'), ('b')").await;
        let cancel_token = CancelToken::new();

        // A sort column the relation does not declare: refused here, not sent to
        // the engine hoping it rejects it.
        let rel_path = CatalogPath::for_relation(None, Some(MAIN), "preview_rows").expect("path");
        let unknown_shape = PreviewShape {
            sort: vec![PreviewSort::ascending("missing_column")],
            ..PreviewShape::default()
        };
        let err = expect_refusal(
            session
                .preview_request(&rel_path, 10, &unknown_shape, &cancel_token)
                .await,
            "an unknown column is not sorted",
        );
        assert!(
            matches!(&err, OxynError::Query(message)
                if message.contains("missing_column")),
            "{err}"
        );
        assert!(!err.is_retryable(), "{err}");

        // A page on a relation without unique key: refused, because an OFFSET
        // without total order returns duplicate rows and skips others.
        let rel_path = CatalogPath::for_relation(None, Some(MAIN), "no_key").expect("path");
        let page = PreviewShape {
            offset: 1,
            ..PreviewShape::default()
        };
        let err = expect_refusal(
            session
                .preview_request(&rel_path, 10, &page, &cancel_token)
                .await,
            "a page without unique key makes no sense",
        );
        assert!(
            matches!(&err, OxynError::NotSupported { capability }
                if capability.contains("unique key")),
            "{err}"
        );

        // The first page of the same relation stays readable: it is today's
        // preview, and it has lost nothing.
        let all_batches = preview_rows(&*session, "no_key", 10, &PreviewShape::unordered()).await;
        assert_eq!(texts_of(&all_batches, 0).len(), 2);
        session.close().await.expect("close");
    }
}
