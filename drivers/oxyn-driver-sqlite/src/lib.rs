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
pub mod stream;
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
        let session = atelier().await;
        assert!(session.capabilities().contains(Capabilities::CONSTRAINTS));
        executer(&*session, "CREATE TABLE constraints_fixture (id INTEGER PRIMARY KEY, value TEXT CONSTRAINT value_present NOT NULL CHECK(length(value) > 0))").await;
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
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local)
            .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
        driver
            .connect(&connexion, &Credentials::new(), &CancelToken::new())
            .await
            .expect("an in-memory database always opens")
    }

    async fn atelier() -> Box<dyn Session> {
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
    fn refus<T>(issue: Result<T, OxynError>, attendu: &str) -> OxynError {
        match issue {
            Ok(_) => panic!("{attendu}"),
            Err(err) => err,
        }
    }

    /// A write request: the default limits are read-only.
    fn ecriture(sql: &str) -> ExecRequest {
        ExecRequest::new(QueryLanguage::SQL, sql).with_limits(ExecLimits::unbounded())
    }

    /// A read request, with the cautious default limits.
    fn lecture(sql: &str) -> ExecRequest {
        ExecRequest::new(QueryLanguage::SQL, sql)
    }

    /// Executes a write and returns the number of affected rows.
    async fn executer(session: &dyn Session, sql: &str) -> u64 {
        let jeton = CancelToken::new();
        let curseur = session
            .execute(ecriture(sql), &jeton)
            .await
            .unwrap_or_else(|err| panic!("execution of `{sql}`: {err}"));
        curseur.stats().rows
    }

    /// Drains a cursor and returns its batches.
    async fn vider(curseur: &mut Box<dyn Cursor>) -> Vec<RecordBatch> {
        let mut lots = Vec::new();
        while let Some(lot) = curseur.next_batch().await.expect("next batch") {
            lots.push(lot);
        }
        lots
    }

    /// The first batch of a read.
    async fn premier_lot(session: &dyn Session, sql: &str) -> RecordBatch {
        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(lecture(sql), &jeton)
            .await
            .unwrap_or_else(|err| panic!("lecture de `{sql}` : {err}"));
        curseur
            .next_batch()
            .await
            .expect("first batch")
            .expect("at least one row")
    }

    #[tokio::test]
    async fn the_full_journey_create_insert_read() {
        let session = atelier().await;
        executer(
            session.as_ref(),
            "CREATE TABLE clients(id INTEGER PRIMARY KEY, nom TEXT NOT NULL)",
        )
        .await;
        let affectees = executer(
            session.as_ref(),
            "INSERT INTO clients(id, nom) VALUES (1, 'Ada'), (2, 'Grace')",
        )
        .await;
        assert_eq!(affectees, 2, "the affected row count must come back");

        let lot = premier_lot(session.as_ref(), "SELECT id, nom FROM clients ORDER BY id").await;
        assert_eq!(lot.num_rows(), 2);
        assert_eq!(lot.num_columns(), 2);

        let ids = lot
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("Int64 column");
        assert_eq!(ids.value(0), 1);
        let noms = lot
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Utf8 column");
        assert_eq!(noms.value(1), "Grace");

        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn the_five_storage_classes_become_arrow_columns() {
        let session = atelier().await;
        executer(
            session.as_ref(),
            "CREATE TABLE t(n INTEGER, x REAL, s TEXT, b BLOB, vide TEXT)",
        )
        .await;
        executer(
            session.as_ref(),
            "INSERT INTO t VALUES (7, 1.5, 'café', x'00ff', NULL)",
        )
        .await;

        let lot = premier_lot(session.as_ref(), "SELECT n, x, s, b, vide FROM t").await;
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            7
        );
        assert!(
            (lot.column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("Float64")
                .value(0)
                - 1.5)
                .abs()
                < f64::EPSILON
        );
        assert_eq!(
            lot.column(2)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("Utf8")
                .value(0),
            "café"
        );
        assert_eq!(
            lot.column(3)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .expect("Binary")
                .value(0),
            b"\x00\xff",
            "a BLOB stays opaque, it does not become text \"at best\""
        );
        // The entirely null column falls back on its declared type.
        assert!(lot.column(4).is_null(0));
        let schema = lot.schema();
        assert_eq!(
            schema.field(4).data_type(),
            &arrow::datatypes::DataType::Utf8
        );
    }

    #[tokio::test]
    async fn a_column_mixing_types_falls_back_to_text_and_declares_it() {
        // SQLite accepts this: the type belongs to the value, not to the column.
        // The interface must be able to say that the rendering is a fallback.
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE m(v INTEGER)").await;
        executer(
            session.as_ref(),
            "INSERT INTO m(v) VALUES (1), ('abc'), (2.5)",
        )
        .await;

        let lot = premier_lot(session.as_ref(), "SELECT v FROM m ORDER BY rowid").await;
        let champ = lot.schema().field(0).clone();
        assert_eq!(champ.data_type(), &arrow::datatypes::DataType::Utf8);
        assert_eq!(
            champ.metadata().get(METADATA_INFERRED).map(String::as_str),
            Some("true"),
            "the type does not come from the declaration: it must say it is inferred"
        );
        let classes = champ
            .metadata()
            .get(METADATA_STORAGE_CLASSES)
            .map(String::as_str)
            .expect("mixed classes must be declared");
        assert!(
            classes.contains("integer") && classes.contains("text"),
            "{classes}"
        );

        let valeurs = lot
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Utf8");
        assert_eq!(valeurs.value(0), "1");
        assert_eq!(valeurs.value(1), "abc");
        assert_eq!(valeurs.value(2), "2.5");
    }

    #[tokio::test]
    async fn a_write_is_refused_when_the_request_declares_itself_read_only() {
        // The `ExecLimits` default is cautious: read-only. The question is asked
        // of the engine (`sqlite3_stmt_readonly`), not of the text.
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;

        let jeton = CancelToken::new();
        let err = refus(
            session
                .execute(lecture("INSERT INTO t(v) VALUES (1)"), &jeton)
                .await,
            "refus attendu",
        );
        assert!(matches!(err, OxynError::PolicyDenied { .. }), "{err:?}");

        // And nothing was written.
        let lot = premier_lot(session.as_ref(), "SELECT count(*) FROM t").await;
        assert_eq!(
            lot.column(0)
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
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE audit(note TEXT)").await;

        let jeton = CancelToken::new();
        let demande = lecture("SELECT ?1 AS v").with_params(vec![ScalarValue::Text(
            "'); DROP TABLE audit; --".to_owned(),
        )]);
        let mut curseur = session.execute(demande, &jeton).await.expect("execution");
        let lot = curseur.next_batch().await.expect("batch").expect("one row");
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("Utf8")
                .value(0),
            "'); DROP TABLE audit; --"
        );
        drop(curseur);

        let reste = premier_lot(
            session.as_ref(),
            "SELECT count(*) FROM sqlite_master WHERE name = 'audit'",
        )
        .await;
        assert_eq!(
            reste
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
    const SENTINELLE: &str = "S3NT1NELLE-42";

    /// A table whose trigger copies the inserted value into its message: it is
    /// the leak the fix closes.
    async fn atelier_bavard() -> Box<dyn Session> {
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE comptes(note TEXT)").await;
        executer(
            session.as_ref(),
            "CREATE TRIGGER refuser BEFORE INSERT ON comptes \
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
        let session = atelier_bavard().await;
        let demande = ecriture("INSERT INTO comptes(note) VALUES (?1)")
            .with_params(vec![ScalarValue::Text(SENTINELLE.to_owned())]);
        let err = refus(
            session.execute(demande, &CancelToken::new()).await,
            "the trigger must refuse the insertion",
        );

        let affiche = format!("{err}");
        let persiste = err.to_string();
        let debogue = format!("{err:?}");
        for rendu in [&affiche, &persiste, &debogue] {
            assert!(!rendu.contains(SENTINELLE), "bound value rendered: {rendu}");
            assert!(
                !rendu.contains("balance"),
                "engine message rendered: {rendu}"
            );
        }
        assert!(
            affiche.contains("withheld"),
            "the withdrawal must be stated, otherwise the user hunts for a bug: {affiche}"
        );
        // The extended result code stays readable: `SQLITE_CONSTRAINT_TRIGGER`.
        assert!(affiche.contains("1811"), "{affiche}");
        assert_eq!(err.class(), ErrorClass::Permanent, "{err:?}");
    }

    #[tokio::test]
    async fn without_bound_value_the_engine_message_arrives_whole() {
        // The protection must not apply wrongly: Oxyn's audience reads its
        // engine's messages.
        let session = atelier_bavard().await;
        let demande = ecriture(&format!(
            "INSERT INTO comptes(note) VALUES ('{SENTINELLE}')"
        ));
        let err = refus(
            session.execute(demande, &CancelToken::new()).await,
            "the trigger must refuse the insertion",
        );
        assert!(
            err.to_string().contains(&format!("balance: {SENTINELLE}")),
            "the engine message must pass unchanged: {err}"
        );
    }

    #[tokio::test]
    async fn a_read_failing_on_a_bound_value_also_withholds_its_message() {
        // The read path fails in `sqlite3_step`, far from the request: that is
        // where the information "there were bound values" must be passed, not
        // guessed.
        let session = atelier().await;
        let demande = lecture("SELECT abs(?1)").with_params(vec![ScalarValue::Int64(i64::MIN)]);
        let err = refus(
            session.execute(demande, &CancelToken::new()).await,
            "`abs` overflows on the smallest integer",
        );
        assert!(
            !err.to_string().contains("integer overflow"),
            "engine message rendered: {err}"
        );
        assert!(err.to_string().contains("withheld"), "{err}");

        // The same overflow written in clear keeps its message.
        let sans_liaison = refus(
            session
                .execute(
                    lecture("SELECT abs(-9223372036854775808)"),
                    &CancelToken::new(),
                )
                .await,
            "`abs` overflows on the smallest integer",
        );
        assert!(
            sans_liaison.to_string().contains("integer overflow"),
            "{sans_liaison}"
        );
    }

    #[tokio::test]
    async fn cancellation_also_cuts_a_read_with_bound_values() {
        // Withholding a message must not turn a cancellation into a failure.
        let session = session(BatchLimits::new().with_max_rows(10)).await;
        let jeton = CancelToken::new();
        let demande = ExecRequest::new(
            QueryLanguage::SQL,
            "WITH RECURSIVE suite(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM suite WHERE n < ?1) \
             SELECT n FROM suite",
        )
        .with_limits(ExecLimits::unbounded())
        .with_params(vec![ScalarValue::Int64(50_000)]);

        let mut curseur = session.execute(demande, &jeton).await.expect("execution");
        assert!(
            curseur
                .next_batch()
                .await
                .expect("first batch")
                .is_some_and(|lot| lot.num_rows() == 10)
        );

        jeton.cancel();
        let err = refus(curseur.next_batch().await, "cancellation must cut");
        assert!(err.is_cancelled(), "{err:?}");
        assert!(curseur.stats().truncated);

        drop(curseur);
        session.ping().await.expect("the session stays usable");
    }

    #[tokio::test]
    async fn a_result_arrives_in_several_bounded_batches() {
        // The stream: no result is materialized in full (I-06).
        let session = session(BatchLimits::new().with_max_rows(100)).await;
        executer(session.as_ref(), "CREATE TABLE grand(n INTEGER)").await;
        executer(
            session.as_ref(),
            "WITH RECURSIVE suite(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM suite WHERE n < 5000) \
             INSERT INTO grand(n) SELECT n FROM suite",
        )
        .await;

        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM grand ORDER BY n")
                    .with_limits(ExecLimits::unbounded()),
                &jeton,
            )
            .await
            .expect("execution");
        let lots = vider(&mut curseur).await;

        let lignes: usize = lots.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(lignes, 5_000);
        assert!(lots.len() >= 50, "{} batch(es) for 5,000 rows", lots.len());
        assert!(
            lots.iter().all(|lot| lot.num_rows() <= 100),
            "a batch exceeds the row bound"
        );
        let stats = curseur.stats();
        assert_eq!(stats.rows, 5_000);
        assert!(!stats.truncated, "the result is complete");
        assert!(stats.server_time.is_none(), "SQLite has no server clock");
    }

    #[tokio::test]
    async fn a_result_truncated_by_the_row_bound_says_so() {
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE t(n INTEGER)").await;
        executer(
            session.as_ref(),
            "INSERT INTO t(n) VALUES (1), (2), (3), (4), (5)",
        )
        .await;

        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(
                lecture("SELECT n FROM t ORDER BY n")
                    .with_limits(ExecLimits::default().with_max_rows(2)),
                &jeton,
            )
            .await
            .expect("execution");
        let lots = vider(&mut curseur).await;

        let lignes: usize = lots.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(lignes, 2);
        assert!(
            curseur.stats().truncated,
            "a truncated result that looks complete leads to wrong conclusions"
        );
    }

    #[tokio::test]
    async fn cancellation_cuts_the_read_between_two_batches() {
        let session = session(BatchLimits::new().with_max_rows(10)).await;
        executer(session.as_ref(), "CREATE TABLE grand(n INTEGER)").await;
        executer(
            session.as_ref(),
            "WITH RECURSIVE suite(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM suite WHERE n < 2000) \
             INSERT INTO grand(n) SELECT n FROM suite",
        )
        .await;

        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM grand ORDER BY n")
                    .with_limits(ExecLimits::unbounded()),
                &jeton,
            )
            .await
            .expect("execution");

        let premier = curseur.next_batch().await.expect("first batch");
        assert!(premier.is_some_and(|lot| lot.num_rows() == 10));

        jeton.cancel();

        let err = curseur
            .next_batch()
            .await
            .expect_err("cancellation must cut");
        assert!(err.is_cancelled(), "{err:?}");
        assert!(
            curseur.stats().truncated,
            "a cancelled result is incomplete, and must say so"
        );

        // The session stays usable: the cancellation freed the worker thread.
        drop(curseur);
        session
            .ping()
            .await
            .expect("the session survives the cancellation");
    }

    #[tokio::test]
    async fn server_side_cancel_is_refused_not_simulated() {
        // SQLite has no server. Suggesting that a "Cancel" cuts a query on the
        // server side would assert what does not exist.
        let session = atelier().await;
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
            .expect_err("refus attendu");
        assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
        assert!(err.is_user_error(), "it is not an incident");
    }

    #[tokio::test]
    async fn a_rolled_back_transaction_leaves_nothing() {
        // The worst case would be to succeed without opening anything: the user
        // would believe a ROLLBACK undid their write.
        let session = atelier().await;
        let jeton = CancelToken::new();
        executer(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;

        session.begin(&jeton).await.expect("open");
        executer(session.as_ref(), "INSERT INTO t(v) VALUES (1)").await;
        session.rollback(&jeton).await.expect("cancellation");

        let lot = premier_lot(session.as_ref(), "SELECT count(*) FROM t").await;
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            0,
            "the ROLLBACK must have undone for good"
        );

        // And a committed transaction does stay.
        session.begin(&jeton).await.expect("open");
        executer(session.as_ref(), "INSERT INTO t(v) VALUES (2)").await;
        session.commit(&jeton).await.expect("validation");
        let lot = premier_lot(session.as_ref(), "SELECT count(*) FROM t").await;
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            1
        );
    }

    #[tokio::test]
    async fn a_multi_statement_batch_runs_everything_and_returns_the_last_result() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(
                ecriture(
                    "CREATE TABLE m(x INTEGER); \
                     INSERT INTO m(x) VALUES (1), (2); \
                     SELECT x FROM m ORDER BY x",
                ),
                &jeton,
            )
            .await
            .expect("batch execution");
        let lots = vider(&mut curseur).await;
        let lignes: usize = lots.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(lignes, 2, "the cursor carries the result of the last one");

        // A statement following the row producer runs all the same.
        let affectees = executer(
            session.as_ref(),
            "SELECT x FROM m; INSERT INTO m(x) VALUES (3)",
        )
        .await;
        assert_eq!(affectees, 1);
        let lot = premier_lot(session.as_ref(), "SELECT count(*) FROM m").await;
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            3
        );
    }

    #[tokio::test]
    async fn bound_parameters_with_a_multi_statement_batch_are_refused() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let demande = ecriture("SELECT ?1; SELECT 2").with_params(vec![ScalarValue::Int64(1)]);
        let err = refus(
            session.execute(demande, &jeton).await,
            "nothing says which statement they relate to",
        );
        assert!(!err.is_retryable(), "{err:?}");
    }

    #[tokio::test]
    async fn an_undeclared_language_is_refused_not_translated() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let err = refus(
            session
                .execute(
                    ExecRequest::new(QueryLanguage::Cypher, "MATCH (n) RETURN n"),
                    &jeton,
                )
                .await,
            "refus attendu",
        );
        assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_syntax_error_is_permanent_and_showable() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let err = refus(
            session.execute(lecture("SLECT 1"), &jeton).await,
            "refus attendu",
        );
        assert!(
            !err.is_retryable(),
            "a syntax error is never retried: {err:?}"
        );
        assert!(err.to_string().contains("sqlite"), "{err}");
    }

    #[tokio::test]
    async fn introspection_describes_what_the_database_contains() {
        let session = atelier().await;
        executer(
            session.as_ref(),
            "CREATE TABLE clients(\
                 id INTEGER PRIMARY KEY, \
                 nom TEXT NOT NULL, \
                 solde DECIMAL(10,2) DEFAULT '0.00')",
        )
        .await;
        executer(
            session.as_ref(),
            "CREATE TABLE commandes(\
                 id INTEGER PRIMARY KEY, \
                 client_id INTEGER REFERENCES clients(id) ON DELETE CASCADE)",
        )
        .await;
        executer(
            session.as_ref(),
            "CREATE UNIQUE INDEX idx_nom ON clients(nom)",
        )
        .await;
        executer(
            session.as_ref(),
            "CREATE VIEW v_clients AS SELECT id FROM clients",
        )
        .await;

        let jeton = CancelToken::new();
        let catalogue = session.catalog();

        let info = catalogue.server_info(&jeton).await.expect("identity");
        assert_eq!(info.product, "SQLite");
        assert!(!info.version.is_empty());

        // SQLite has no catalog tier; its databases occupy the namespace
        // tier.
        assert!(
            catalogue
                .list_catalogs(&jeton)
                .await
                .expect("empty")
                .is_empty()
        );
        let espaces = catalogue
            .list_namespaces(None, &jeton)
            .await
            .expect("namespaces");
        assert!(
            espaces.iter().any(|espace| espace.name() == MAIN),
            "`main` must always be there"
        );

        let relations = catalogue
            .list_relations(&CatalogPath::empty(), &jeton)
            .await
            .expect("relations");
        let noms: Vec<&str> = relations.iter().map(|r| r.name()).collect();
        assert!(noms.contains(&"clients"), "{noms:?}");
        assert!(noms.contains(&"v_clients"), "{noms:?}");
        assert!(
            relations
                .iter()
                .any(|r| r.name() == "v_clients" && r.kind == RelationKind::View),
            "a view is not a table"
        );

        let chemin = CatalogPath::for_relation(None, Some(MAIN), "clients").expect("path");
        let decrite = catalogue
            .describe_relation(&chemin, &jeton)
            .await
            .expect("description");
        assert_eq!(decrite.kind, RelationKind::Table);
        assert_eq!(decrite.fields.len(), 3);
        assert_eq!(
            decrite
                .primary_key()
                .first()
                .map(|champ| champ.name.as_str()),
            Some("id")
        );
        let nom = decrite.field("nom").expect("column `nom`");
        assert!(!nom.nullable, "NOT NULL must come back as is");
        assert_eq!(nom.logical_type, LogicalType::Text);
        assert_eq!(nom.raw_type, "TEXT", "the server type is kept");
        let solde = decrite.field("solde").expect("column `solde`");
        assert_eq!(
            solde.logical_type,
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2)
            }
        );
        assert_eq!(solde.default.as_deref(), Some("'0.00'"));
        assert_eq!(
            decrite.estimated_rows, None,
            "SQLite cannot estimate without counting, and `None` is not zero"
        );

        let index = catalogue
            .list_indexes(&chemin, &jeton)
            .await
            .expect("index");
        let idx = index
            .iter()
            .find(|index| index.name == "idx_nom")
            .expect("the declared index");
        assert!(idx.unique);
        assert_eq!(idx.fields, ["nom"]);
        assert!(!idx.is_partial());

        let commandes = CatalogPath::for_relation(None, Some(MAIN), "commandes").expect("path");
        let cles = catalogue
            .list_foreign_keys(&commandes, &jeton)
            .await
            .expect("foreign keys");
        let cle = cles.first().expect("one key");
        assert!(cle.is_well_formed(), "both sides must pair: {cle:?}");
        assert_eq!(cle.fields, ["client_id"]);
        assert_eq!(cle.references.fields, ["id"]);
        assert_eq!(cle.references.relation.relation(), Some("clients"));
        assert!(
            cle.on_delete.propagates_delete(),
            "an ON DELETE CASCADE must be visible"
        );
    }

    #[tokio::test]
    async fn a_partial_index_carries_its_predicate() {
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE t(a INTEGER, b INTEGER)").await;
        executer(
            session.as_ref(),
            "CREATE INDEX idx_actifs ON t(a) WHERE b > 0",
        )
        .await;

        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, Some(MAIN), "t").expect("path");
        let index = session
            .catalog()
            .list_indexes(&chemin, &jeton)
            .await
            .expect("index");
        let partiel = index
            .iter()
            .find(|index| index.name == "idx_actifs")
            .expect("the declared index");
        assert!(partiel.is_partial());
        assert_eq!(partiel.predicate.as_deref(), Some("b > 0"));
    }

    #[tokio::test]
    async fn a_missing_relation_is_reported() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, Some(MAIN), "fantome").expect("path");
        let err = session
            .catalog()
            .describe_relation(&chemin, &jeton)
            .await
            .expect_err("the relation does not exist");
        assert!(!err.is_retryable(), "{err:?}");
    }

    #[tokio::test]
    async fn ping_answers_and_close_releases() {
        let session = atelier().await;
        session.ping().await.expect("the session is alive");
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_hostile_table_name_does_not_execute() {
        // I-10: `"users"; DROP TABLE audit; --` is a legal table name.
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE audit(note TEXT)").await;
        executer(
            session.as_ref(),
            r#"CREATE TABLE "clients""; DROP TABLE audit; --"(id INTEGER)"#,
        )
        .await;

        let jeton = CancelToken::new();
        let relations = session
            .catalog()
            .list_relations(&CatalogPath::empty(), &jeton)
            .await
            .expect("relations");
        let hostile = relations
            .iter()
            .find(|relation| relation.name().contains("DROP TABLE"))
            .expect("the hostile table does exist");

        // The introspection of this table goes through quoted identifiers and
        // bound values: nothing executes.
        let decrite = session
            .catalog()
            .describe_relation(&hostile.path(), &jeton)
            .await
            .expect("description");
        assert_eq!(decrite.fields.len(), 1);

        let reste = premier_lot(
            session.as_ref(),
            "SELECT count(*) FROM sqlite_master WHERE name = 'audit'",
        )
        .await;
        assert_eq!(
            reste
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
                .execute(ecriture(text), &cancel)
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
                .execute(ecriture("INSERT INTO shared_probe VALUES (2)"), &cancel)
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
    const LIGNES_APERCU: i64 = 500;

    /// A preview table, its ties, and a witness table that must survive.
    ///
    /// `seau` is `id % 7`: sorting on it leaves seventy-one ties per value, hence
    /// a non-total order if the primary key does not complete it.
    async fn atelier_apercu() -> Box<dyn Session> {
        let session = atelier().await;
        executer(
            session.as_ref(),
            "CREATE TABLE apercu (id INTEGER PRIMARY KEY, seau INTEGER, nom TEXT)",
        )
        .await;
        executer(
            session.as_ref(),
            &format!(
                "INSERT INTO apercu(id, seau, nom) \
                 WITH RECURSIVE serie(i) AS (\
                   SELECT 1 UNION ALL SELECT i + 1 FROM serie WHERE i < {LIGNES_APERCU}) \
                 SELECT i, i % 7, 'ligne-' || i FROM serie"
            ),
        )
        .await;
        executer(session.as_ref(), "CREATE TABLE temoin (garde TEXT)").await;
        executer(session.as_ref(), "INSERT INTO temoin VALUES ('intacte')").await;
        session
    }

    /// Composes then executes a preview, and returns its batches.
    async fn apercu(
        session: &dyn Session,
        relation: &str,
        limit: u32,
        shape: &PreviewShape,
    ) -> Vec<RecordBatch> {
        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, Some(MAIN), relation).expect("valid path");
        let demande = session
            .preview_request(&chemin, limit, shape, &jeton)
            .await
            .unwrap_or_else(|err| panic!("composing the preview of `{relation}`: {err}"));
        let mut curseur = session
            .execute(demande, &jeton)
            .await
            .unwrap_or_else(|err| panic!("executing the preview of `{relation}`: {err}"));
        vider(&mut curseur).await
    }

    /// The integers of a column, in the order the batches return them.
    fn entiers(lots: &[RecordBatch], colonne: usize) -> Vec<i64> {
        lots.iter()
            .flat_map(|lot| {
                lot.column(colonne)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .expect("integer column")
                    .values()
                    .to_vec()
            })
            .collect()
    }

    /// The texts of a column, in the order the batches return them.
    fn textes(lots: &[RecordBatch], colonne: usize) -> Vec<String> {
        lots.iter()
            .flat_map(|lot| {
                let valeurs = lot
                    .column(colonne)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("text column");
                (0..valeurs.len())
                    .map(|rang| valeurs.value(rang).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[tokio::test]
    async fn a_sorted_preview_returns_rows_in_the_requested_order() {
        let session = atelier_apercu().await;

        let croissant = PreviewShape {
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        let ids = entiers(&apercu(&*session, "apercu", 10, &croissant).await, 0);
        assert_eq!(ids, (1..=10).collect::<Vec<_>>());

        let decroissant = PreviewShape {
            sort: vec![PreviewSort::descending("id")],
            ..PreviewShape::default()
        };
        let ids = entiers(&apercu(&*session, "apercu", 10, &decroissant).await, 0);
        assert_eq!(
            ids,
            (LIGNES_APERCU - 9..=LIGNES_APERCU)
                .rev()
                .collect::<Vec<_>>()
        );

        // A sort on a column full of ties: the primary key completes the order,
        // so the result is predictable row by row.
        let par_seau = PreviewShape {
            sort: vec![PreviewSort::descending("seau")],
            ..PreviewShape::default()
        };
        let lots = apercu(&*session, "apercu", 10, &par_seau).await;
        let attendu: Vec<i64> = {
            let mut tous: Vec<i64> = (1..=LIGNES_APERCU).collect();
            tous.sort_by_key(|id| (-(id % 7), *id));
            tous.into_iter().take(10).collect()
        };
        assert_eq!(entiers(&lots, 0), attendu);
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn consecutive_pages_neither_overlap_nor_skip_a_row() {
        let session = atelier_apercu().await;
        let taille = 200_u32;
        let mut vues: Vec<i64> = Vec::new();
        let mut tailles = Vec::new();

        for page in 0..3_u64 {
            let shape = PreviewShape {
                // Sorting on the ties is the worst case: without the primary key
                // added by the driver, two pages would show the same row twice and
                // hide another.
                sort: vec![PreviewSort::ascending("seau")],
                offset: page * u64::from(taille),
                ..PreviewShape::default()
            };
            let lots = apercu(&*session, "apercu", taille, &shape).await;
            let ids = entiers(&lots, 0);
            tailles.push(ids.len());
            vues.extend(ids);
        }

        assert_eq!(tailles, vec![200, 200, 100], "three pages, 500 rows");
        let mut triees = vues.clone();
        triees.sort_unstable();
        triees.dedup();
        assert_eq!(triees.len(), vues.len(), "no row must appear on two pages");
        assert_eq!(
            triees,
            (1..=LIGNES_APERCU).collect::<Vec<_>>(),
            "the union of the pages is exactly the table"
        );
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_page_beyond_the_last_is_empty_without_being_an_error() {
        let session = atelier_apercu().await;
        let shape = PreviewShape {
            offset: 1_000,
            ..PreviewShape::default()
        };
        let lots = apercu(&*session, "apercu", 200, &shape).await;
        assert!(entiers(&lots, 0).is_empty());
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn the_user_predicate_goes_as_is() {
        let session = atelier_apercu().await;
        executer(
            session.as_ref(),
            "INSERT INTO apercu(id, seau, nom) VALUES (1001, 0, '100%'), (1002, 0, '100 percent')",
        )
        .await;

        // The `%` is not a metacharacter here: the driver composes no pattern, it
        // passes on the user's text.
        let egalite = PreviewShape {
            predicate: Some("nom = '100%'".into()),
            ..PreviewShape::default()
        };
        assert_eq!(
            textes(&apercu(&*session, "apercu", 200, &egalite).await, 2),
            vec!["100%".to_owned()],
            "an equality must bring back only the literal row"
        );

        // And when the user writes a pattern, it is a pattern indeed: their SQL
        // is neither escaped nor reinterpreted.
        let motif = PreviewShape {
            predicate: Some("nom LIKE '100%'".into()),
            ..PreviewShape::default()
        };
        let mut trouves = textes(&apercu(&*session, "apercu", 200, &motif).await, 2);
        trouves.sort();
        assert_eq!(trouves, vec!["100 percent".to_owned(), "100%".to_owned()]);
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn both_guards_around_the_predicate_hold_on_the_engine() {
        let session = atelier_apercu().await;
        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, Some(MAIN), "apercu").expect("path");

        // 1. The unclosed block comment. No line break ends it: without the
        // parentheses, SQLite read until the end of the text, lost the LIMIT
        // and returned the whole table without anything reporting it. The
        // opening parenthesis makes the statement incomplete, hence refused.
        let bloc = PreviewShape {
            predicate: Some("id > 0 /*".into()),
            ..PreviewShape::default()
        };
        let demande = session
            .preview_request(&chemin, 3, &bloc, &jeton)
            .await
            .expect("composition does not judge the predicate");
        refus(
            session.execute(demande, &jeton).await,
            "an unclosed block comment must be refused, not executed unbounded",
        );
        // The session survives this refusal: the relation stays readable.
        assert_eq!(
            entiers(
                &apercu(&*session, "apercu", 3, &PreviewShape::unordered()).await,
                0
            )
            .len(),
            3
        );

        // 2. The end-of-line comment keeps working, and the LIMIT applies: the
        // line break is what allows it, and it stays useful.
        let ligne = PreviewShape {
            predicate: Some("id > 0 -- this is a comment".into()),
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        assert_eq!(
            entiers(&apercu(&*session, "apercu", 3, &ligne).await, 0),
            vec![1, 2, 3]
        );

        // 3. A predicate already carrying its parentheses returns exactly what
        // the same text without the wrapper would return.
        let parenthese = PreviewShape {
            predicate: Some("(id > 0 AND seau < 2) OR nom IS NULL".into()),
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        let enveloppe = entiers(&apercu(&*session, "apercu", 5, &parenthese).await, 0);
        let mut curseur = session
            .execute(
                lecture(
                    "SELECT * FROM \"main\".\"apercu\" \
                     WHERE (id > 0 AND seau < 2) OR nom IS NULL \
                     ORDER BY \"id\" ASC LIMIT 5",
                ),
                &jeton,
            )
            .await
            .expect("the same text, without wrapper");
        assert_eq!(enveloppe, entiers(&vider(&mut curseur).await, 0));
        assert_eq!(enveloppe, vec![1, 7, 8, 14, 15]);
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_hostile_predicate_destroys_nothing_and_does_not_overflow_its_clause() {
        let session = atelier_apercu().await;
        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, Some(MAIN), "apercu").expect("path");

        for hostile in [
            // A second statement: it must never execute.
            "nom = 'ligne-1'; DROP TABLE temoin",
            "nom = 'ligne-1'; DELETE FROM temoin",
            // An unbalanced quote: a syntax error, nothing more.
            "nom = 'ligne-1",
            // An end-of-line comment: it must not swallow the LIMIT.
            "nom LIKE 'ligne-%' -- ; DROP TABLE temoin",
        ] {
            let shape = PreviewShape {
                predicate: Some(hostile.to_owned()),
                ..PreviewShape::default()
            };
            let demande = session
                .preview_request(&chemin, 3, &shape, &jeton)
                .await
                .expect("composition does not judge the predicate");
            assert!(
                demande.text.contains(hostile),
                "the predicate goes as is: {}",
                demande.text
            );
            match session.execute(demande, &jeton).await {
                // Either the engine refuses — syntax, or a second statement that
                // writes under read-only limits…
                Err(_) => {}
                // … or it is an ordinary read, and it stays bounded.
                Ok(mut curseur) => {
                    let lignes: usize = vider(&mut curseur)
                        .await
                        .iter()
                        .map(RecordBatch::num_rows)
                        .sum();
                    assert!(lignes <= 3, "{hostile}: {lignes} rows despite LIMIT 3");
                }
            }
            let temoin = premier_lot(&*session, "SELECT garde FROM temoin").await;
            assert_eq!(
                temoin.num_rows(),
                1,
                "the witness table must survive `{hostile}`"
            );
        }
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_projection_returns_only_the_named_columns() {
        let session = atelier_apercu().await;
        let forme = PreviewShape {
            columns: Some(vec!["nom".into(), "id".into()]),
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        let lots = apercu(&*session, "apercu", 3, &forme).await;
        let schema = lots.first().expect("at least one batch").schema();
        let noms: Vec<&str> = schema
            .fields()
            .iter()
            .map(|champ| champ.name().as_str())
            .collect();
        // `seau`, not requested, is not read from the engine; the order is the
        // projection's, not the table's.
        assert_eq!(noms, ["nom", "id"]);
        assert_eq!(textes(&lots, 0), ["ligne-1", "ligne-2", "ligne-3"]);
        assert_eq!(entiers(&lots, 1), [1, 2, 3]);

        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, Some(MAIN), "apercu").expect("path");
        let inconnue = PreviewShape {
            columns: Some(vec!["colonne_absente".into()]),
            ..PreviewShape::default()
        };
        let err = refus(
            session
                .preview_request(&chemin, 10, &inconnue, &jeton)
                .await,
            "an unknown column is not projected",
        );
        assert!(
            matches!(&err, OxynError::Query(message)
                if message.contains("colonne_absente")),
            "{err}"
        );
        assert!(!err.is_retryable(), "{err}");
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_shape_the_relation_does_not_allow_is_refused_saying_so() {
        let session = atelier_apercu().await;
        executer(session.as_ref(), "CREATE TABLE sans_cle (x TEXT)").await;
        executer(session.as_ref(), "INSERT INTO sans_cle VALUES ('a'), ('b')").await;
        let jeton = CancelToken::new();

        // A sort column the relation does not declare: refused here, not sent to
        // the engine hoping it rejects it.
        let chemin = CatalogPath::for_relation(None, Some(MAIN), "apercu").expect("path");
        let inconnue = PreviewShape {
            sort: vec![PreviewSort::ascending("colonne_absente")],
            ..PreviewShape::default()
        };
        let err = refus(
            session
                .preview_request(&chemin, 10, &inconnue, &jeton)
                .await,
            "an unknown column is not sorted",
        );
        assert!(
            matches!(&err, OxynError::Query(message)
                if message.contains("colonne_absente")),
            "{err}"
        );
        assert!(!err.is_retryable(), "{err}");

        // A page on a relation without unique key: refused, because an OFFSET
        // without total order returns duplicate rows and skips others.
        let chemin = CatalogPath::for_relation(None, Some(MAIN), "sans_cle").expect("path");
        let page = PreviewShape {
            offset: 1,
            ..PreviewShape::default()
        };
        let err = refus(
            session.preview_request(&chemin, 10, &page, &jeton).await,
            "a page without unique key makes no sense",
        );
        assert!(
            matches!(&err, OxynError::NotSupported { capability }
                if capability.contains("unique key")),
            "{err}"
        );

        // The first page of the same relation stays readable: it is today's
        // preview, and it has lost nothing.
        let lots = apercu(&*session, "sans_cle", 10, &PreviewShape::unordered()).await;
        assert_eq!(textes(&lots, 0).len(), 2);
        session.close().await.expect("close");
    }
}
