//! The tests that need a real PostgreSQL server.
//!
//! All marked `#[ignore]`: `cargo test` without a server must neither fail nor
//! wait. They live in `src/` rather than `tests/` because they rely on internal
//! details — the execution registry, the batch assembler — that an external
//! test crate would not see.
//!
//! # Running them
//!
//! ```sh
//! docker run --rm -d -p 5433:5432 \
//!   -e POSTGRES_PASSWORD=oxyn --name oxyn-pg postgres:17
//!
//! OXYN_PG_TEST_URL='postgres://postgres:oxyn@localhost:5433/postgres' \
//!   cargo test -p oxyn-driver-postgres -- --ignored --test-threads=1
//!
//! docker rm -f oxyn-pg
//! ```
//!
//! `--test-threads=1` is not decorative: several of these tests create and
//! drop objects in the `public` schema and would step on each other.
//!
//! Without `OXYN_PG_TEST_URL`, each test **stops without failing** and says so.
//! Reading an environment variable is the test's doing here, never the
//! driver's: [`ConnectSpec`](crate::ConnectSpec) overrides anything `sqlx` could
//! have picked up from it.
//!
//! # What these tests must cover, and why
//!
//! The review checklist imposes two that cannot be worked around:
//! **cancellation that proves the server-side stop** and **streaming a volume
//! that would not fit in memory**. The others cover the type table,
//! introspection, and the read-only mode enforced by the server.
//!
//! The session context ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md))
//! adds a third of the same kind: `search_path` is a **per-connection** state,
//! and an Oxyn session is a pool of four. These tests therefore saturate the
//! pool and record `pg_backend_pid()` inside the execution itself — a test that
//! exercised only one connection would be green without proving anything.

use std::time::Duration;

use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, Environment, ExecLimits, ExecRequest,
    OxynError, PreviewShape, PreviewSort, QueryLanguage, SqlDialect, StatementIntent,
};
use oxyn_driver::{Credentials, Cursor, Driver as _, ParsedDsn, Session, SessionContext};

use crate::PostgresDriver;
use crate::options::MAX_CONNECTIONS;

/// The variable that carries the test server URL.
const VARIABLE: &str = "OXYN_PG_TEST_URL";

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn incoming_keys_cross_schemas_preserve_composite_order_and_declared_cardinality() {
    use oxyn_catalog::CatalogPath;
    let Some(session) = session().await else {
        return;
    };
    for sql in [
        "CREATE SCHEMA oxyn_incoming_source",
        "CREATE SCHEMA oxyn_incoming_target",
        "CREATE TABLE oxyn_incoming_target.parent (a integer, b text, PRIMARY KEY (b,a))",
        "CREATE TABLE oxyn_incoming_source.child (id integer PRIMARY KEY, x text, y integer, FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a) ON DELETE CASCADE)",
        "CREATE TABLE oxyn_incoming_source.unique_child (x text, y integer, payload text, FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a))",
        "CREATE UNIQUE INDEX incoming_unique ON oxyn_incoming_source.unique_child(x,y) INCLUDE(payload)",
        "CREATE TABLE oxyn_incoming_source.expression_child (x text, y integer, FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a))",
        "CREATE UNIQUE INDEX incoming_expression ON oxyn_incoming_source.expression_child(lower(x),y)",
        "CREATE TABLE oxyn_incoming_source.partial_child (x text, y integer, FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a))",
        "CREATE UNIQUE INDEX incoming_partial ON oxyn_incoming_source.partial_child(x,y) WHERE x IS NOT NULL",
        "CREATE TABLE oxyn_incoming_source.coercion_child (x varchar, y integer, UNIQUE(x,y), FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a))",
        "CREATE TABLE oxyn_incoming_source.parent (a integer PRIMARY KEY)",
        "CREATE TABLE oxyn_incoming_source.other_child (a integer REFERENCES oxyn_incoming_source.parent)",
    ] {
        apply(&*session, sql).await;
    }
    let path =
        CatalogPath::for_relation(None, Some("oxyn_incoming_target"), "parent").expect("path");
    let keys = session
        .catalog()
        .list_incoming_foreign_keys(&path, &CancelToken::new())
        .await
        .expect("incoming metadata");
    assert_eq!(keys.len(), 5);
    let child = keys
        .iter()
        .find(|key| key.source.relation() == Some("child"))
        .expect("child");
    assert_eq!(child.source.namespace(), Some("oxyn_incoming_source"));
    assert_eq!(child.key.fields, ["x", "y"]);
    assert_eq!(child.key.references.fields, ["b", "a"]);
    assert_eq!(child.source_unique, Some(false));
    assert_eq!(
        child.key.on_delete,
        oxyn_catalog::ReferentialAction::Cascade
    );
    assert_eq!(
        keys.iter()
            .find(|key| key.source.relation() == Some("unique_child"))
            .expect("unique")
            .source_unique,
        Some(true)
    );
    assert_eq!(
        keys.iter()
            .find(|key| key.source.relation() == Some("expression_child"))
            .expect("expression")
            .source_unique,
        None
    );
    for name in ["partial_child", "coercion_child"] {
        assert_eq!(
            keys.iter()
                .find(|key| key.source.relation() == Some(name))
                .expect("uncertain comparison")
                .source_unique,
            None
        );
    }
    apply(&*session, "DROP SCHEMA oxyn_incoming_source CASCADE").await;
    apply(&*session, "DROP SCHEMA oxyn_incoming_target CASCADE").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn constraints_report_validation_status_from_the_server() {
    use oxyn_catalog::CatalogPath;
    let Some(session) = session().await else {
        return;
    };
    apply(
        &*session,
        "CREATE TABLE oxyn_constraint_validation (id integer)",
    )
    .await;
    apply(
        &*session,
        "INSERT INTO oxyn_constraint_validation VALUES (-1)",
    )
    .await;
    apply(
        &*session,
        "ALTER TABLE oxyn_constraint_validation ADD CONSTRAINT positive CHECK (id > 0) NOT VALID",
    )
    .await;
    let path = CatalogPath::for_relation(None, Some("public"), "oxyn_constraint_validation")
        .expect("path");
    let constraints = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect("read");
    assert_eq!(
        constraints.first().expect("constraint").validated,
        Some(false)
    );
    apply(&*session, "UPDATE oxyn_constraint_validation SET id = 1").await;
    apply(
        &*session,
        "ALTER TABLE oxyn_constraint_validation VALIDATE CONSTRAINT positive",
    )
    .await;
    let constraints = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect("read");
    assert_eq!(
        constraints.first().expect("constraint").validated,
        Some(true)
    );
    apply(&*session, "DROP TABLE oxyn_constraint_validation").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn constraints_reject_oversized_catalogs_without_publishing_partial_lists() {
    use oxyn_catalog::CatalogPath;
    let Some(session) = session().await else {
        return;
    };
    let clauses = std::iter::repeat_n("CHECK (id >= 0)", 1025)
        .collect::<Vec<_>>()
        .join(", ");
    apply(
        &*session,
        &format!("CREATE TABLE oxyn_many_constraints (id integer, {clauses})"),
    )
    .await;
    let path =
        CatalogPath::for_relation(None, Some("public"), "oxyn_many_constraints").expect("path");
    let error = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect_err("never silently truncate constraints");
    assert!(error.to_string().contains("1024"));
    apply(&*session, "DROP TABLE oxyn_many_constraints").await;
    let literal = "x".repeat(16385);
    apply(
        &*session,
        &format!("CREATE TABLE oxyn_large_constraint (id text CHECK (id <> '{literal}'))"),
    )
    .await;
    let path =
        CatalogPath::for_relation(None, Some("public"), "oxyn_large_constraint").expect("path");
    let error = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect_err("definition size is bounded before transfer");
    assert!(error.to_string().contains("16 KiB"));
    apply(&*session, "DROP TABLE oxyn_large_constraint").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn constraints_preserve_names_column_order_and_engine_definitions() {
    use oxyn_catalog::{CatalogPath, ConstraintKind};
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "CREATE TABLE \"oxyn_constraints\"\";--\" (b integer, a integer NOT NULL, score integer, CONSTRAINT \"key\"\";--\" PRIMARY KEY (b, a), CONSTRAINT score_positive CHECK (score > 0), UNIQUE (a), FOREIGN KEY (a) REFERENCES \"oxyn_constraints\"\";--\" (a))").await;
    let path =
        CatalogPath::for_relation(None, Some("public"), "oxyn_constraints\";--").expect("path");
    let constraints = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect("constraints");
    let primary = constraints
        .iter()
        .find(|c| c.kind == ConstraintKind::PrimaryKey)
        .expect("primary");
    assert_eq!(primary.name, "key\";--");
    assert_eq!(primary.fields, ["b", "a"]);
    let check = constraints
        .iter()
        .find(|c| c.kind == ConstraintKind::Check)
        .expect("check");
    assert_eq!(check.name, "score_positive");
    assert!(
        check
            .expression
            .as_deref()
            .expect("definition")
            .contains("score > 0")
    );
    assert!(
        constraints
            .iter()
            .any(|c| c.kind == ConstraintKind::ForeignKey)
    );
    assert!(constraints.iter().any(|c| c.kind == ConstraintKind::Unique));
    assert!(
        constraints
            .iter()
            .any(|c| c.kind == ConstraintKind::NotNull && c.fields == ["a"])
    );
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(
        session.catalog().list_constraints(&path, &cancel).await,
        Err(OxynError::Cancelled)
    ));
    apply(&*session, "DROP TABLE \"oxyn_constraints\"\";--\"").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn previews_handle_system_types_and_preserve_native_columns() {
    use arrow::array::{Array as _, AsArray as _};
    use arrow::datatypes::{DataType, TimeUnit};
    use oxyn_catalog::CatalogPath;

    let Some(session) = session().await else {
        return;
    };
    let token = CancelToken::new();
    // Dropped before being recreated, like the other fixtures of this file.
    // Without that the test passes **only once**: the second run fails on
    // `type "oxyn_preview_acl" already exists`, and the failure looks like a
    // driver defect when it is the test that did not clean up after itself.
    // An integration test that cannot be rerun only serves on a fresh CI.
    for sql in [
        "DROP TABLE IF EXISTS oxyn_preview_types CASCADE",
        "DROP DOMAIN IF EXISTS oxyn_preview_acl CASCADE",
    ] {
        apply(&*session, sql).await;
    }
    apply(&*session, "CREATE DOMAIN oxyn_preview_acl AS aclitem[]").await;
    apply(
        &*session,
        "CREATE TABLE oxyn_preview_types (\
        id bigint DEFAULT 1, at timestamptz, acl oxyn_preview_acl, function regproc, \
        \"a\"\"; --\" text)",
    )
    .await;
    apply(
        &*session,
        // `=r/<role>` must name a role **that exists**, and the repository's
        // test protocol creates `oxyn_test`, not `postgres`. Hard-coding the
        // name made the test fail on `role "postgres" does not exist` — a
        // dependency on an environment nothing guarantees. `current_user` says
        // the same thing without assuming it.
        "INSERT INTO oxyn_preview_types VALUES \
        (1, '2021-01-01 00:00:00.123456+00', \
         ARRAY[('=r/' || current_user)::aclitem, NULL], \
         'pg_catalog.int4in'::regproc, 'quoted'), \
        (2, NULL, NULL, NULL, NULL)",
    )
    .await;

    let path = CatalogPath::for_relation(None, Some("public"), "oxyn_preview_types").expect("path");
    let request = session
        .preview_request(&path, 200, &PreviewShape::unordered(), &token)
        .await
        .expect("metadata");
    let mut cursor = session.execute(request, &token).await.expect("preview");
    assert_eq!(cursor.schema().field(0).data_type(), &DataType::Int64);
    assert_eq!(
        cursor.schema().field(1).data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(cursor.schema().field(4).name(), "a\"; --");
    let batch = cursor.next_batch().await.expect("stream").expect("rows");
    assert_eq!(batch.num_rows(), 2);
    let functions = batch.column(3).as_string_opt::<i32>().expect("server text");
    let acls = batch.column(2).as_string_opt::<i32>().expect("server text");
    assert!(functions.iter().flatten().any(|value| value == "int4in"));
    // What is tested here is that an `aclitem[]` crosses the wire losslessly:
    // the `=r/` privilege and the `NULL` of the second element. The **role
    // name** is not part of it, and hard-coding it tied the test to a cluster
    // created with a `postgres` superuser — not the one the repository's
    // protocol describes.
    assert!(
        acls.iter()
            .flatten()
            .any(|value| value.contains("=r/") && value.contains("NULL")),
        "an aclitem array must arrive with its privilege and its null element"
    );
    assert_eq!(acls.null_count(), 1);
    assert!(cursor.next_batch().await.expect("end").is_none());

    let mut raw = session
        .execute(read_request("SELECT 52::regproc AS function"), &token)
        .await
        .expect("unmodified user SQL");
    let schema = raw.schema();
    // The user's own SQL is not rewritten: a `regproc` is the OID the server
    // sent, as a number. The preview above composes `::text` and gets the name.
    assert_eq!(schema.field(0).data_type(), &DataType::UInt32);
    assert!(
        schema
            .field(0)
            .metadata()
            .get(crate::META_FALLBACK)
            .is_none()
    );
    assert!(
        schema
            .field(0)
            .metadata()
            .get(crate::META_PG_TYPE)
            .is_some_and(|name| name.eq_ignore_ascii_case("regproc"))
    );
    let batch = raw
        .next_batch()
        .await
        .expect("raw stream")
        .expect("raw row");
    assert_eq!(
        batch
            .column(0)
            .as_primitive::<arrow::datatypes::UInt32Type>()
            .value(0),
        52
    );
    assert!(raw.next_batch().await.expect("end").is_none());

    for relation in ["pg_database", "pg_attrdef", "pg_aggregate"] {
        let path = CatalogPath::for_relation(None, Some("pg_catalog"), relation).expect("path");
        let request = session
            .preview_request(&path, 1, &PreviewShape::unordered(), &token)
            .await
            .expect("system metadata");
        let mut cursor = session
            .execute(request, &token)
            .await
            .expect("system preview");
        assert_eq!(drain(&mut cursor).await.0, 1, "{relation}");
    }
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert!(matches!(
        session
            .preview_request(&path, 200, &PreviewShape::unordered(), &cancelled)
            .await,
        Err(OxynError::Cancelled)
    ));
    apply(&*session, "DROP TABLE oxyn_preview_types").await;
    apply(&*session, "DROP DOMAIN oxyn_preview_acl").await;
    session.close().await.expect("close");
}

/// An unlikely value, so that a test looking for it only finds it if it
/// really crossed.
const SENTINEL: &str = "S3NT1N3L-42";

/// The error of a failing execution, whether at preparation or in the
/// stream.
///
/// An invalid cast passes preparation — the type of `$1` is `text` — and only
/// fails at execution: the error then arrives through the cursor.
async fn fail(session: &dyn Session, exec_request: ExecRequest) -> OxynError {
    match session.execute(exec_request, &CancelToken::new()).await {
        Err(error) => error,
        Ok(mut cursor) => loop {
            match cursor.next_batch().await {
                Ok(Some(_)) => {}
                Ok(None) => panic!("the statement should have been refused"),
                Err(error) => break error,
            }
        },
    }
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_bound_value_never_comes_out_of_the_server_message() {
    // I-03: `invalid input syntax for type integer: "…"` copies the bound
    // value. This message is displayed, and persisted by `HistoryRecord::failed`
    // and `JournalRecord::failed` — which both call `error.to_string()`.
    let Some(session) = session().await else {
        return;
    };

    let error = fail(
        &*session,
        read_request("SELECT ($1::text)::integer")
            .with_params(vec![oxyn_core::ScalarValue::Text(SENTINEL.to_owned())]),
    )
    .await;
    for rendered in [format!("{error}"), format!("{error:?}")] {
        assert!(
            !rendered.contains(SENTINEL),
            "bound value rendered: {rendered}"
        );
        assert!(!rendered.contains("invalid input syntax"), "{rendered}");
    }
    assert!(
        error.to_string().contains("withheld"),
        "the withholding must be stated: {error}"
    );
    // `invalid_text_representation`: the SQLSTATE survives, it is a code.
    assert!(error.to_string().contains("22P02"), "{error}");
    assert_eq!(error.class(), oxyn_core::ErrorClass::Permanent);

    // Without a bound value, the same refusal keeps PostgreSQL's message:
    // Oxyn's audience reads it, and a paraphrase would be a defect.
    let integer_error = fail(
        &*session,
        read_request(&format!("SELECT ('{SENTINEL}'::text)::integer")),
    )
    .await;
    assert!(
        integer_error
            .to_string()
            .contains("invalid input syntax for type integer"),
        "{integer_error}"
    );

    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_trigger_that_copies_the_value_does_not_leak_it() {
    // The other form of the leak: `RAISE` concatenates `NEW.column`, hence
    // the value that was just bound.
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "CREATE TABLE oxyn_withheld (note text)").await;
    apply(
        &*session,
        "CREATE FUNCTION oxyn_withheld_guard() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'balance: %', NEW.note; END $$",
    )
    .await;
    apply(
        &*session,
        "CREATE TRIGGER oxyn_withheld_trigger BEFORE INSERT ON oxyn_withheld \
         FOR EACH ROW EXECUTE FUNCTION oxyn_withheld_guard()",
    )
    .await;

    let error = fail(
        &*session,
        write_request("INSERT INTO oxyn_withheld(note) VALUES ($1)")
            .with_params(vec![oxyn_core::ScalarValue::Text(SENTINEL.to_owned())]),
    )
    .await;
    for rendered in [format!("{error}"), format!("{error:?}")] {
        assert!(
            !rendered.contains(SENTINEL),
            "bound value rendered: {rendered}"
        );
        assert!(
            !rendered.contains("balance"),
            "server message rendered: {rendered}"
        );
    }
    assert!(error.to_string().contains("withheld"), "{error}");
    assert_eq!(error.class(), oxyn_core::ErrorClass::Permanent);

    apply(
        &*session,
        "DROP TRIGGER oxyn_withheld_trigger ON oxyn_withheld",
    )
    .await;
    apply(&*session, "DROP FUNCTION oxyn_withheld_guard()").await;
    apply(&*session, "DROP TABLE oxyn_withheld").await;
    session.close().await.expect("close");
}

/// The test configuration, or `None` when no server is declared.
pub(super) fn target() -> Option<(ConnectionConfig, Credentials)> {
    let url = std::env::var(VARIABLE).ok()?;
    let read_back = ParsedDsn::parse(&url).expect("OXYN_PG_TEST_URL must be a `postgres://` URL");
    let (parts, credentials) = read_back.into_parts();
    let config = parts
        .to_config("trial", DriverId::postgres())
        .with_environment(Environment::Local);
    Some((config, credentials))
}

/// The error of an execution that should have been refused.
///
/// `Result::expect_err` requires `Debug` on the `Ok` variant, hence on
/// `dyn Cursor`. The cursor holds the session, which holds the connection
/// credentials: giving it `Debug` would put a secret one `{:?}` away
/// ([I-03]). Going through `match` requires nothing of `T`.
///
/// [I-03]: ../../../CLAUDE.md#i-03
fn refusal<T>(issue: Result<T, OxynError>, expected: &str) -> OxynError {
    match issue {
        Ok(_) => panic!("{expected}"),
        Err(err) => err,
    }
}

/// Opens a session, or returns `None` and says so.
pub(super) async fn session() -> Option<Box<dyn Session>> {
    let Some((config, credentials)) = target() else {
        eprintln!("{VARIABLE} is not set: test skipped");
        return None;
    };
    let driver = PostgresDriver::new();
    let session = driver
        .connect(&config, &credentials, &CancelToken::new())
        .await
        .expect("the test server must be reachable");
    Some(session)
}

/// A read request, with wide limits.
fn read_request(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(None))
}

/// A write request, allowed to write.
fn write_request(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_intent(StatementIntent::Write)
        .with_limits(ExecLimits::default().writable().with_max_rows(None))
}

/// Applies a statement and drains its cursor, expecting nothing from it.
///
/// Test setups and cleanups go through here: what matters is that the
/// statement is accepted, not what it returns.
pub(super) async fn apply(session: &dyn Session, sql: &str) {
    let mut cursor = session
        .execute(write_request(sql), &CancelToken::new())
        .await
        .unwrap_or_else(|error| panic!("`{sql}` must be accepted: {error}"));
    let _ = drain(&mut cursor).await;
}

/// Drains a cursor and returns (rows, batches).
async fn drain(cursor: &mut Box<dyn Cursor>) -> (usize, usize) {
    let mut rows = 0;
    let mut batches = 0;
    while let Some(batch) = cursor.next_batch().await.expect("stream without error") {
        rows += batch.num_rows();
        batches += 1;
    }
    (rows, batches)
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn the_connection_detects_the_variant_and_its_capabilities() {
    let Some(session) = session().await else {
        return;
    };
    let capabilities = session.capabilities();
    assert!(capabilities.contains(Capabilities::SQL));
    assert!(capabilities.contains(Capabilities::SERVER_SIDE_CANCEL));
    assert!(capabilities.contains(Capabilities::STREAMING));

    let info = session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .expect("the server identity");
    assert!(info.product.contains("PostgreSQL"), "{}", info.product);
    assert!(!info.version.is_empty(), "the version must be read");

    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn the_schema_is_known_before_the_first_row() {
    // This is what lets the grid draw its columns while the data arrives
    // (PERFORMANCE: first display under 100 ms).
    let Some(session) = session().await else {
        return;
    };
    let mut cursor = session
        .execute(
            read_request("SELECT 1 AS one, 'two'::text AS two"),
            &CancelToken::new(),
        )
        .await
        .expect("execution");

    assert_eq!(cursor.schema().fields().len(), 2);
    assert_eq!(cursor.schema().field(0).name(), "one");

    let (rows, _) = drain(&mut cursor).await;
    assert_eq!(rows, 1);
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_volume_that_would_not_fit_in_memory_arrives_in_batches() {
    // The test the review checklist imposes: the stream never materialises the
    // whole result (I-06). Two million rows of two columns are about a hundred
    // megabytes server side; above all we check that several batches arrive,
    // so that nothing waits for the end.
    let Some(session) = session().await else {
        return;
    };
    let mut cursor = session
        .execute(
            read_request("SELECT i, repeat('x', 100) FROM generate_series(1, 2000000) AS s(i)"),
            &CancelToken::new(),
        )
        .await
        .expect("execution");

    let (rows, batches) = drain(&mut cursor).await;
    assert_eq!(rows, 2_000_000);
    assert!(batches > 10, "the result must arrive in batches: {batches}");
    assert_eq!(cursor.stats().rows, 2_000_000);
    assert!(!cursor.stats().truncated);

    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn cancellation_really_stops_the_query_server_side() {
    // The second test the checklist imposes. Dropping the future is not
    // enough: we check that the server process no longer works
    // (DRIVER-CONTRACT §2).
    let Some(session) = session().await else {
        return;
    };
    let token = CancelToken::new();
    let mut cursor = session
        .execute(read_request("SELECT pg_sleep(30)"), &token)
        .await
        .expect("execution");

    let handle = cursor.handle();
    session
        .cancel(handle)
        .await
        .expect("cancellation requested");

    let issue = cursor.next_batch().await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "the cursor must return a cancellation: {issue:?}"
    );

    // The proof: no `pg_sleep` runs any more for this database.
    drop(cursor);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut remaining = session
        .execute(
            read_request(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE query LIKE '%pg_sleep%' AND state = 'active' AND pid <> pg_backend_pid()",
            ),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    let (rows, _) = drain(&mut remaining).await;
    assert_eq!(rows, 1, "one count row");

    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn closing_a_cursor_stops_the_query_without_being_asked() {
    // Closing a tab destroys the cursor. At the tenth closed tab, the database
    // must still accept connections.
    let Some(session) = session().await else {
        return;
    };
    for _ in 0..10 {
        let cursor = session
            .execute(read_request("SELECT pg_sleep(30)"), &CancelToken::new())
            .await
            .expect("execution");
        drop(cursor);
    }
    tokio::time::sleep(Duration::from_secs(1)).await;

    let mut cursor = session
        .execute(read_request("SELECT 1"), &CancelToken::new())
        .await
        .expect("the database still accepts connections");
    let (rows, _) = drain(&mut cursor).await;
    assert_eq!(rows, 1);

    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn the_row_bound_truncates_and_says_so() {
    // A truncated result that looks complete leads to wrong conclusions on
    // real data.
    let Some(session) = session().await else {
        return;
    };
    let exec_request = ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Postgres),
        "SELECT i FROM generate_series(1, 100000) AS s(i)",
    )
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default().with_max_rows(Some(1_000)));

    let mut cursor = session
        .execute(exec_request, &CancelToken::new())
        .await
        .expect("execution");
    let (rows, _) = drain(&mut cursor).await;

    assert_eq!(rows, 1_000);
    assert!(cursor.stats().truncated, "truncation must be known");

    session.close().await.expect("close");
}

/// Issue #122: exactly N rows end the stream like N - 1 do; only a row N + 1
/// makes the result truncated, and it is never handed out.
#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn the_row_bound_reads_one_row_ahead_before_saying_truncated() {
    let Some(session) = session().await else {
        return;
    };
    for (count, truncated) in [(999, false), (1_000, false), (1_001, true)] {
        let exec_request = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Postgres),
            format!("SELECT i FROM generate_series(1, {count}) AS s(i)"),
        )
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(Some(1_000)));

        let mut cursor = session
            .execute(exec_request, &CancelToken::new())
            .await
            .expect("execution");
        let (rows, _) = drain(&mut cursor).await;

        assert_eq!(rows, count.min(1_000), "{count} rows");
        assert_eq!(cursor.stats().truncated, truncated, "{count} rows");
    }

    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn read_only_is_enforced_by_the_server() {
    // Default `ExecLimits` forbids writing, and it is the server that refuses
    // — not a client-side filter.
    let Some(session) = session().await else {
        return;
    };
    apply(
        &*session,
        "CREATE TABLE IF NOT EXISTS oxyn_trial_ro (id int)",
    )
    .await;

    // Preparation passes — `PREPARE` does not check read-only — and it is the
    // execution the server refuses. The refusal therefore arrives through the
    // stream, not through `execute`.
    let mut cursor = session
        .execute(
            read_request("INSERT INTO oxyn_trial_ro VALUES (1)"),
            &CancelToken::new(),
        )
        .await
        .expect("preparing an INSERT is accepted");
    let refusal = cursor
        .next_batch()
        .await
        .expect_err("the server must refuse the write");
    assert!(
        refusal.to_string().contains("bounded to read-only"),
        "the message must name the bounds, not the privileges: {refusal}"
    );
    drop(cursor);

    apply(&*session, "DROP TABLE oxyn_trial_ro").await;

    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn the_type_table_crosses_the_wire_losslessly() {
    use arrow::array::AsArray as _;
    use arrow::datatypes::{DataType, TimeUnit};

    let Some(session) = session().await else {
        return;
    };
    let mut cursor = session
        .execute(
            read_request(
                "SELECT true::bool, 1::int2, 2::int4, 3::int8, 1.5::float4, 2.5::float8, \
                 12345678901234567890.12345678::numeric, 'text'::text, \
                 '\\x00ff'::bytea, '67e55044-10b1-426f-9d0c-451f8ad05b1a'::uuid, \
                 '2026-09-05'::date, '14:30:00'::time, \
                 '2026-09-05 14:30:00'::timestamp, '2026-09-05 14:30:00+02'::timestamptz, \
                 '{\"a\": 1}'::jsonb, ARRAY[1, NULL, 3]::int4[]",
            ),
            &CancelToken::new(),
        )
        .await
        .expect("execution");

    let schema = cursor.schema();
    let expected_all = [
        DataType::Boolean,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::Float32,
        DataType::Float64,
        // A `numeric` never becomes a float.
        DataType::Utf8,
        DataType::Utf8,
        DataType::Binary,
        DataType::Utf8,
        DataType::Date32,
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Timestamp(TimeUnit::Microsecond, None),
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        DataType::Utf8,
    ];
    for (rank, expected) in expected_all.iter().enumerate() {
        assert_eq!(schema.field(rank).data_type(), expected, "column {rank}");
    }
    assert!(matches!(schema.field(15).data_type(), DataType::List(_)));

    let batch = cursor.next_batch().await.expect("stream").expect("one row");
    assert_eq!(batch.num_rows(), 1);

    // The exact decimal: 26 significant digits, out of reach of an f64.
    let numeric_column = batch
        .column(6)
        .as_string_opt::<i32>()
        .expect("a text column");
    assert_eq!(numeric_column.value(0), "12345678901234567890.12345678");

    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn an_unknown_type_does_not_fail_the_query() {
    use arrow::array::AsArray as _;

    // A user `enum`, whose OID exists in no built-in table.
    let Some(session) = session().await else {
        return;
    };
    for sql in [
        "DROP TYPE IF EXISTS oxyn_trial_state",
        "CREATE TYPE oxyn_trial_state AS ENUM ('draft', 'shipped')",
    ] {
        apply(&*session, sql).await;
    }

    let mut cursor = session
        .execute(
            read_request("SELECT 'shipped'::oxyn_trial_state"),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    let batch = cursor.next_batch().await.expect("stream").expect("one row");
    let values = batch
        .column(0)
        .as_string_opt::<i32>()
        .expect("a text fallback");
    assert_eq!(values.value(0), "shipped");

    // The PostgreSQL type name survives in the field metadata.
    let field = cursor.schema();
    let meta = field.field(0).metadata();
    assert_eq!(
        meta.get(crate::META_PG_TYPE).map(String::as_str),
        Some("oxyn_trial_state")
    );
    drop(cursor);

    apply(&*session, "DROP TYPE oxyn_trial_state").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn introspection_walks_down_the_hierarchy_level_by_level() {
    let Some(session) = session().await else {
        return;
    };
    let token = CancelToken::new();

    for sql in [
        "DROP TABLE IF EXISTS oxyn_trial_line",
        "DROP TABLE IF EXISTS oxyn_trial_order",
        "CREATE TABLE oxyn_trial_order (id bigserial PRIMARY KEY, \
         amount numeric(12,2) NOT NULL, created timestamptz DEFAULT now())",
        "CREATE TABLE oxyn_trial_line (id bigserial PRIMARY KEY, \
         order_id bigint NOT NULL REFERENCES oxyn_trial_order(id) ON DELETE CASCADE)",
        "CREATE INDEX oxyn_trial_line_order ON oxyn_trial_line (order_id)",
        "COMMENT ON TABLE oxyn_trial_order IS 'orders of the shop'",
    ] {
        apply(&*session, sql).await;
    }

    let catalog = session.catalog();
    let namespaces = catalog
        .list_namespaces(None, &token)
        .await
        .expect("the schemas");
    assert!(namespaces.iter().any(|e| e.name() == "public"));
    assert!(
        namespaces
            .iter()
            .any(|e| e.name() == "pg_catalog" && e.is_system),
        "system schemas are marked, not hidden"
    );

    let public = oxyn_catalog::CatalogPath::for_namespace(None, "public").expect("path");
    let relations = catalog
        .list_relations(&public, &token)
        .await
        .expect("the relations");
    assert!(relations.iter().any(|r| r.name() == "oxyn_trial_order"));

    let order = public.with_relation("oxyn_trial_order").expect("path");
    let described_relation = catalog
        .describe_relation(&order, &token)
        .await
        .expect("the description");
    assert_eq!(
        described_relation.comment.as_deref(),
        Some("orders of the shop")
    );
    let amount = described_relation
        .field("amount")
        .expect("the amount column");
    assert!(!amount.nullable);
    assert_eq!(
        amount.logical_type,
        oxyn_catalog::LogicalType::Decimal {
            precision: Some(12),
            scale: Some(2)
        }
    );
    assert_eq!(described_relation.primary_key().len(), 1);

    let row = public.with_relation("oxyn_trial_line").expect("path");
    let index = catalog
        .list_indexes(&row, &token)
        .await
        .expect("the indexes");
    assert!(index.iter().any(|i| i.name == "oxyn_trial_line_order"));

    let keys = catalog
        .list_foreign_keys(&row, &token)
        .await
        .expect("the foreign keys");
    let key = keys.first().expect("a foreign key");
    assert!(key.is_well_formed());
    assert_eq!(key.fields, ["order_id"]);
    assert_eq!(key.references.relation.relation(), Some("oxyn_trial_order"));
    assert_eq!(key.on_delete, oxyn_catalog::ReferentialAction::Cascade);

    for sql in ["DROP TABLE oxyn_trial_line", "DROP TABLE oxyn_trial_order"] {
        apply(&*session, sql).await;
    }
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn another_database_is_not_introspectable_and_says_so() {
    // Returning an empty list would suggest the database is empty.
    let Some(session) = session().await else {
        return;
    };
    let error = session
        .catalog()
        .list_namespaces(Some("another_database"), &CancelToken::new())
        .await
        .expect_err("refusal expected");
    assert!(
        matches!(error, OxynError::CatalogUnavailable(_)),
        "{error:?}"
    );
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_syntax_error_is_permanent_and_carries_its_sqlstate() {
    let Some(session) = session().await else {
        return;
    };
    let error = refusal(
        session
            .execute(read_request("SELECT FROM WHERE"), &CancelToken::new())
            .await,
        "rejection expected",
    );

    assert!(!error.is_retryable(), "a wrong syntax is not retried");
    if let OxynError::Driver { source, .. } = &error {
        let postgres = source
            .downcast_ref::<crate::PostgresError>()
            .expect("the driver wraps its errors");
        assert_eq!(postgres.sqlstate(), Some("42601"));
    }
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_failing_preparation_keeps_the_server_message_even_with_a_parameter() {
    // Counterpart of `without_bound_value_the_server_message_passes_unchanged`,
    // on the protocol side: `prepare` only sends Parse and Describe, which carry
    // the text and the parameter OIDs — never their content. The server can
    // therefore quote nothing, and withholding its message would remove the
    // most frequent diagnostic of a parameterised query: the faulty object name.
    let Some(session) = session().await else {
        return;
    };
    let error = refusal(
        session
            .execute(
                read_request("SELECT * FROM oxyn_invoicess WHERE client_id = $1")
                    .with_params(vec![oxyn_core::ScalarValue::Int64(42)]),
                &CancelToken::new(),
            )
            .await,
        "a missing table must be refused at preparation",
    );

    assert!(
        error.to_string().contains("oxyn_invoicess"),
        "the faulty object name must survive: {error}"
    );
    assert!(
        !error.to_string().contains("withheld"),
        "no value reached the server: {error}"
    );
    assert_eq!(error.class(), oxyn_core::ErrorClass::Permanent);
    if let OxynError::Driver { source, .. } = &error {
        let postgres = source
            .downcast_ref::<crate::PostgresError>()
            .expect("the driver wraps its errors");
        assert_eq!(postgres.sqlstate(), Some("42P01"));
    }

    session.close().await.expect("close");
}

// ---------------------------------------------------------------------------
// Session context (ADR-0019)
// ---------------------------------------------------------------------------

/// The two twin schemas these tests rely on.
///
/// The **same** table name in both: it is the only fixture that makes a wrong
/// `search_path` visible. Two distinct names would merely have failed, which is
/// the easy case.
const SCHEMA_A: &str = "oxyn_ctx_a";
/// The twin of [`SCHEMA_A`], which no context of these tests declares.
const SCHEMA_B: &str = "oxyn_ctx_b";
/// The same-named table, never present in `public`.
const SHARED_TABLE: &str = "shared_target";
/// A table only the server's default `search_path` reaches.
const DEFAULT_TABLE: &str = "oxyn_ctx_default_only";

/// A context that names only a namespace.
fn context(namespace: &str) -> SessionContext {
    SessionContext::new(None, Some(namespace.to_owned()))
}

/// Creates the two twin schemas and the table only the default reaches.
async fn prepare_twins(session: &dyn Session) {
    drop_twins(session).await;
    for (schema, marker) in [(SCHEMA_A, "a"), (SCHEMA_B, "b")] {
        apply(session, &format!("CREATE SCHEMA {schema}")).await;
        apply(
            session,
            &format!("CREATE TABLE {schema}.{SHARED_TABLE} (marker text)"),
        )
        .await;
        apply(
            session,
            &format!("INSERT INTO {schema}.{SHARED_TABLE} VALUES ('{marker}')"),
        )
        .await;
    }
    apply(
        session,
        &format!("CREATE TABLE public.{DEFAULT_TABLE} (marker text)"),
    )
    .await;
    apply(
        session,
        &format!("INSERT INTO public.{DEFAULT_TABLE} VALUES ('default_value')"),
    )
    .await;
}

/// Undoes what [`prepare_twins`] created.
async fn drop_twins(session: &dyn Session) {
    for schema in [SCHEMA_A, SCHEMA_B] {
        apply(session, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE")).await;
    }
    apply(
        session,
        &format!("DROP TABLE IF EXISTS public.{DEFAULT_TABLE}"),
    )
    .await;
}

/// The first text of the first row of a read, with the stream **drained**.
///
/// Draining is not politeness: a cursor dropped mid-stream leaves unread bytes,
/// so its connection is closed instead of returning to the pool. Tests that
/// compare pids from one execution to the next would then never get the same
/// connection twice, and would prove nothing about the state it carries.
async fn first_text(session: &dyn Session, sql: &str) -> String {
    use arrow::array::AsArray as _;
    let mut cursor = session
        .execute(read_request(sql), &CancelToken::new())
        .await
        .unwrap_or_else(|error| panic!("`{sql}` must execute: {error}"));
    let mut first = None;
    while let Some(batch) = cursor
        .next_batch()
        .await
        .unwrap_or_else(|error| panic!("`{sql}` must return a batch: {error}"))
    {
        if first.is_none() && batch.num_rows() > 0 {
            first = Some(
                batch
                    .column(0)
                    .as_string_opt::<i32>()
                    .expect("a text column")
                    .value(0)
                    .to_owned(),
            );
        }
    }
    first.unwrap_or_else(|| panic!("`{sql}` must return a row"))
}

/// What `SELECT marker FROM shared_target` resolves, unqualified.
const BARE_READ: &str = "SELECT marker FROM shared_target";

/// What [`resolution_query`] returns when the bare name designates nothing.
const NOT_FOUND: &str = "not_found";

/// The query that reads the marker of the same-named table, unqualified.
///
/// The `CROSS JOIN` is not decorative: it makes the result long enough for the
/// stream task to stay blocked on its connection until we drain. This is what
/// forces the pool to open another one for the next execution.
const MARKER_QUERY: &str = "SELECT marker || '@' || pg_catalog.pg_backend_pid()::text \
                                   FROM shared_target CROSS JOIN generate_series(1, 50000)";

/// The query that asks the server what it resolves for a bare name.
///
/// Two reasons to go through `to_regclass` rather than a failing query: it
/// uses exactly the `search_path` of an ordinary statement, and it returns
/// `NULL` instead of raising. The connection therefore stays healthy and
/// returns to the pool — a condition without which two successive calls never
/// hit the same connections, and prove nothing about the state they keep.
fn resolution_query(name: &str) -> String {
    format!(
        "SELECT coalesce(pg_catalog.to_regclass('{name}')::text, '{NOT_FOUND}') \
         || '@' || pg_catalog.pg_backend_pid()::text FROM generate_series(1, 50000)"
    )
}

/// What **each** connection of the pool answers, keyed by pid.
///
/// The query must return `value@pid` in its first column, and enough rows to
/// keep its connection busy — see [`MARKER_QUERY`].
///
/// The four executions are alive at the same time: `execute` holds its
/// connection from start to end and the cursor channel is bounded to one batch,
/// so until we drain, each cursor ties one up. The pool has four
/// ([`MAX_CONNECTIONS`]), so all four are exercised. The final drain leaves
/// them healthy: a dropped cursor leaves unread bytes, its connection is
/// closed, and the next call would not find the same ones.
async fn on_whole_pool(
    session: &dyn Session,
    sql: &str,
) -> std::collections::BTreeMap<String, String> {
    use arrow::array::AsArray as _;

    let mut cursors = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        cursors.push(
            session
                .execute(read_request(sql), &CancelToken::new())
                .await
                .expect("execution"),
        );
    }

    let mut seen_count = std::collections::BTreeMap::new();
    for cursor in &mut cursors {
        let mut first = None;
        while let Some(batch) = cursor.next_batch().await.expect("stream") {
            if first.is_none() && batch.num_rows() > 0 {
                first = Some(
                    batch
                        .column(0)
                        .as_string_opt::<i32>()
                        .expect("a text column")
                        .value(0)
                        .to_owned(),
                );
            }
        }
        let raw_type = first.expect("one row");
        let (what, process) = raw_type
            .split_once('@')
            .expect("the template composes both fields");
        seen_count.insert(process.to_owned(), what.to_owned());
    }
    seen_count
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn the_context_applies_to_every_connection_of_the_pool() {
    // The test that decides the design (ADR-0019). An Oxyn PostgreSQL session
    // is a pool of four connections and `search_path` is a **per-connection**
    // state: a `SET` issued once would only apply to the connection that
    // received it, and every other query would resolve in another schema with
    // nothing to signal it. That is the worst possible outcome — a control that
    // looks like it works —, and the fixture is built to make it visible:
    // `shared_target` exists in both schemas, with a different marker. A wrong
    // resolution returns `b`, not an error.
    let Some(session) = session().await else {
        return;
    };
    prepare_twins(&*session).await;
    session
        .set_context(&context(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    let seen_count = on_whole_pool(&*session, MARKER_QUERY).await;
    assert_eq!(
        seen_count.len(),
        usize::try_from(MAX_CONNECTIONS).expect("four fits in a usize"),
        "the test did not exercise four distinct connections ({seen_count:?}): \
         it then proves nothing about per-connection state"
    );
    for (process, marker) in &seen_count {
        assert_eq!(
            marker, "a",
            "connection {process} resolved outside {SCHEMA_A}"
        );
    }

    drop_twins(&*session).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_context_change_survives_the_prepared_statement_cache() {
    // The corollary of the pool, and the most silent one. `sqlx` keeps a
    // **per-connection** prepared statement cache, keyed on the text: after a
    // context change, the same query does not go through a client-side
    // preparation again. If the server did not redo its analysis, it would keep
    // reading the old schema — and since `shared_target` exists in both, it
    // would return rows, just the wrong ones.
    let Some(session) = session().await else {
        return;
    };
    prepare_twins(&*session).await;

    session
        .set_context(&context(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");
    let before = on_whole_pool(&*session, MARKER_QUERY).await;
    assert!(before.values().all(|seen| seen == "a"), "{before:?}");

    session
        .set_context(&context(SCHEMA_B), &CancelToken::new())
        .await
        .expect("the second context must be accepted");
    let after = on_whole_pool(&*session, MARKER_QUERY).await;
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>(),
        "the pool must have reused the connections that carried the prepared \
         statement: otherwise the test proves nothing about the cache"
    );
    for (process, marker) in &after {
        assert_eq!(
            marker, "b",
            "connection {process} still reads the old schema"
        );
    }

    drop_twins(&*session).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn going_back_to_the_default_undoes_the_context_on_the_connection_that_carried_it() {
    // What this test proves: end to end, on **the same** server processes,
    // resolution goes back to the server default. This is what the user
    // observes. Hence going through the whole pool, saturated and left healthy
    // between the two phases: without the pid identity, the test would only
    // observe that a fresh connection starts from the default, which teaches
    // nothing.
    //
    // What it **no longer** proves, since the connection is reset to the
    // default before going back to the pool: that `context_statement()` does
    // emit a `SET search_path TO DEFAULT` rather than nothing. Both mechanisms
    // aim at the same effect, and this one masks the other. That assertion now
    // lives in the unit test
    // `session::tests::going_back_to_the_default_sets_a_statement_rather_than_nothing`,
    // which compares the composed text. Both are needed.
    let Some(session) = session().await else {
        return;
    };
    prepare_twins(&*session).await;

    session
        .set_context(&context(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");
    assert_eq!(first_text(&*session, BARE_READ).await, "a");

    let sub_context = on_whole_pool(&*session, &resolution_query(SHARED_TABLE)).await;
    assert_eq!(
        sub_context.len(),
        usize::try_from(MAX_CONNECTIONS).expect("four fits in a usize"),
        "the test did not exercise four distinct connections: {sub_context:?}"
    );
    assert!(
        sub_context.values().all(|seen| seen == SHARED_TABLE),
        "every connection must resolve the bare name: {sub_context:?}"
    );

    session
        .set_context(&SessionContext::server_default(), &CancelToken::new())
        .await
        .expect("going back to the default must be accepted");

    let after = on_whole_pool(&*session, &resolution_query(SHARED_TABLE)).await;
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        sub_context.keys().collect::<Vec<_>>(),
        "the pool did not reuse the connections that carried the context: \
         the test then does not prove that the `SET … TO DEFAULT` is emitted"
    );
    assert!(
        after.values().all(|seen| seen == NOT_FOUND),
        "outside the context, `shared_target` must no longer resolve: {after:?}"
    );

    let visible = on_whole_pool(&*session, &resolution_query(DEFAULT_TABLE)).await;
    assert!(
        visible.values().all(|seen| seen == DEFAULT_TABLE),
        "the default `search_path` must become resolvable again: {visible:?}"
    );
    assert_eq!(
        first_text(&*session, &format!("SELECT marker FROM {DEFAULT_TABLE}")).await,
        "default_value"
    );

    // And the real query fails the way the server says.
    //
    // Through `fail` and not `refusal`: `sqlx` keeps a **per-connection**
    // prepared statement cache, keyed on the text. The same text having already
    // been prepared above under `oxyn_ctx_a`, preparation does not go back to
    // the server and returns without error. It is the server that redoes the
    // analysis at execution, because `search_path` changed — the refusal thus
    // arrives through the stream. What this test checks here is precisely that
    // the cache does not keep the old resolution alive.
    let gone = fail(&*session, read_request(BARE_READ)).await;
    assert!(gone.to_string().contains(SHARED_TABLE), "{gone}");

    drop_twins(&*session).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_missing_schema_is_refused_and_keeps_the_previous_context() {
    // PostgreSQL silently accepts a `SET search_path` to a missing schema.
    // Without the prior check, Oxyn would display a context the server does
    // not apply — and a half-applied context would be worse still: the
    // interface would show one, the server would resolve the other.
    let Some(session) = session().await else {
        return;
    };
    prepare_twins(&*session).await;
    session
        .set_context(&context(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    let error = refusal(
        session
            .set_context(&context("oxyn_ctx_absent"), &CancelToken::new())
            .await,
        "a missing schema must be refused",
    );
    assert!(matches!(error, OxynError::Config(_)), "{error:?}");
    assert!(error.to_string().contains("oxyn_ctx_absent"), "{error}");
    assert!(error.is_user_error(), "{error}");

    assert_eq!(
        session
            .context()
            .and_then(|seen| seen.namespace().map(str::to_owned)),
        Some(SCHEMA_A.to_owned()),
        "the confirmed context must not move on a refusal"
    );
    assert_eq!(first_text(&*session, BARE_READ).await, "a");

    drop_twins(&*session).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn another_database_is_refused_at_the_catalog_level() {
    // A PostgreSQL session does not change database. Pretending otherwise
    // would be the fake control the ADR seeks to avoid: the interface would say
    // `other_db / public`, the server would stay where it is.
    let Some(session) = session().await else {
        return;
    };
    prepare_twins(&*session).await;

    let error = refusal(
        session
            .set_context(
                &SessionContext::new(
                    Some("oxyn_another_database".to_owned()),
                    Some(SCHEMA_A.to_owned()),
                ),
                &CancelToken::new(),
            )
            .await,
        "another database must be refused",
    );
    assert!(matches!(error, OxynError::Config(_)), "{error:?}");
    assert!(error.to_string().contains("database"), "{error}");
    assert!(session.context().is_none(), "no context must be kept");

    // The connection's own database is a legitimate `catalog` level.
    let base = first_text(&*session, "SELECT current_database()::text").await;
    session
        .set_context(
            &SessionContext::new(Some(base), Some(SCHEMA_A.to_owned())),
            &CancelToken::new(),
        )
        .await
        .expect("the connection database must be accepted");
    assert_eq!(first_text(&*session, BARE_READ).await, "a");

    drop_twins(&*session).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn user_sql_is_not_rewritten_by_the_context() {
    // The context changes what the **server** resolves, not the submitted
    // text. An identifier qualified by hand therefore keeps targeting what it
    // names.
    let Some(session) = session().await else {
        return;
    };
    prepare_twins(&*session).await;
    session
        .set_context(&context(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    assert_eq!(first_text(&*session, BARE_READ).await, "a");
    assert_eq!(
        first_text(
            &*session,
            &format!("SELECT marker FROM {SCHEMA_B}.{SHARED_TABLE}")
        )
        .await,
        "b",
        "a qualification written by hand wins over the context"
    );

    // The most direct proof: ask the server for the text it received.
    const READ_BACK: &str = "SELECT query FROM pg_catalog.pg_stat_activity \
                        WHERE pid = pg_catalog.pg_backend_pid()";
    assert_eq!(
        first_text(&*session, READ_BACK).await,
        READ_BACK,
        "the text received by the server must be exactly the one submitted"
    );

    drop_twins(&*session).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_hostile_schema_name_is_quoted_and_works() {
    use oxyn_catalog::path::{QuoteStyle, quote_identifier};

    // I-10: a schema named with a double quote or a dot is legal in
    // PostgreSQL. Concatenated, `SET search_path TO "oxyn_ctx"weird"` does not
    // even compile server side — and a better chosen name would execute
    // something else.
    let Some(session) = session().await else {
        return;
    };
    let hostiles = [
        (r#"oxyn_ctx"weird"#, "guillemet"),
        ("oxyn_ctx.dotted", "point"),
    ];
    let quote = |name: &str| quote_identifier(name, QuoteStyle::for_dialect(SqlDialect::Postgres));

    for (name, marker) in hostiles {
        let quoted = quote(name);
        apply(
            &*session,
            &format!("DROP SCHEMA IF EXISTS {quoted} CASCADE"),
        )
        .await;
        apply(&*session, &format!("CREATE SCHEMA {quoted}")).await;
        apply(
            &*session,
            &format!("CREATE TABLE {quoted}.{SHARED_TABLE} (marker text)"),
        )
        .await;
        apply(
            &*session,
            &format!("INSERT INTO {quoted}.{SHARED_TABLE} VALUES ('{marker}')"),
        )
        .await;
    }

    for (name, marker) in hostiles {
        session
            .set_context(&context(name), &CancelToken::new())
            .await
            .unwrap_or_else(|error| panic!("`{name}` must be accepted: {error}"));
        assert_eq!(
            first_text(&*session, BARE_READ).await,
            marker,
            "`{name}` was not quoted correctly"
        );
    }

    session
        .set_context(&SessionContext::server_default(), &CancelToken::new())
        .await
        .expect("back to the default");
    for (name, _) in hostiles {
        apply(
            &*session,
            &format!("DROP SCHEMA IF EXISTS {} CASCADE", quote(name)),
        )
        .await;
    }
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_declared_context_does_not_disarm_read_only() {
    // The `SET` is issued **before** `BEGIN READ ONLY` — otherwise the
    // `ROLLBACK` that closes the read-only transaction would undo it. It remains
    // to check that this order does not reopen writing: it is the server that
    // refuses, not a client filter.
    let Some(session) = session().await else {
        return;
    };
    prepare_twins(&*session).await;
    session
        .set_context(&context(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    let mut cursor = session
        .execute(
            read_request(&format!("INSERT INTO {SHARED_TABLE} VALUES ('intruder')")),
            &CancelToken::new(),
        )
        .await
        .expect("preparing an INSERT is accepted");
    let refusal = cursor
        .next_batch()
        .await
        .expect_err("the server must refuse the write");
    assert!(
        refusal.to_string().contains("bounded to read-only"),
        "the message must name the bounds, not the privileges: {refusal}"
    );
    drop(cursor);

    // And reading still works in the declared context.
    assert_eq!(first_text(&*session, BARE_READ).await, "a");
    assert_eq!(
        first_text(&*session, "SELECT count(*)::text FROM shared_target").await,
        "1",
        "the refused write must have left nothing"
    );

    drop_twins(&*session).await;
    session.close().await.expect("close");
}

/// The table whose definition is read, in [`SCHEMA_A`].
///
/// Distinct from [`SHARED_TABLE`]: what is tested here is not the resolution
/// of a bare name, but the **rendering** of a definition.
const DEFINED_TABLE: &str = "ctx_defined";

/// The query that keeps a connection busy working in [`DEFINED_TABLE`].
const DEFINED_QUERY: &str = "SELECT marker || '@' || pg_catalog.pg_backend_pid()::text \
                               FROM ctx_defined CROSS JOIN generate_series(1, 50000)";

/// Creates in [`SCHEMA_A`] an object whose definition **renders** differently
/// depending on the `search_path`.
///
/// A domain and a function, both in the schema: `format_type`,
/// `pg_get_constraintdef` and `pg_get_expr` qualify their output when the
/// schema is not on the path, and omit it when it is. This is exactly the
/// channel through which a console's context could leak into introspection.
async fn prepare_defined_object(session: &dyn Session) {
    apply(
        session,
        &format!("DROP SCHEMA IF EXISTS {SCHEMA_A} CASCADE"),
    )
    .await;
    for sql in [
        format!("CREATE SCHEMA {SCHEMA_A}"),
        format!("CREATE DOMAIN {SCHEMA_A}.ctx_amount AS numeric"),
        format!(
            "CREATE FUNCTION {SCHEMA_A}.ctx_positive(v numeric) RETURNS boolean \
             LANGUAGE sql IMMUTABLE AS $$ SELECT v > 0 $$"
        ),
        format!(
            "CREATE TABLE {SCHEMA_A}.{DEFINED_TABLE} (\
             marker text, amount {SCHEMA_A}.ctx_amount, \
             CONSTRAINT ctx_defined_positive CHECK ({SCHEMA_A}.ctx_positive(amount)))"
        ),
        format!(
            "CREATE INDEX ctx_defined_partial ON {SCHEMA_A}.{DEFINED_TABLE} (marker) \
             WHERE {SCHEMA_A}.ctx_positive(amount)"
        ),
        format!("INSERT INTO {SCHEMA_A}.{DEFINED_TABLE} VALUES ('a', 1)"),
    ] {
        apply(session, &sql).await;
    }
}

/// Everything the catalog returns that is sensitive to `search_path`, as one
/// string.
///
/// The three sources named by the finding: `format_type` for `raw_type`,
/// `pg_get_constraintdef` for a constraint expression, `pg_get_expr` for the
/// predicate of a partial index.
async fn definition_fingerprint(session: &dyn Session, path: &oxyn_catalog::CatalogPath) -> String {
    let token = CancelToken::new();
    let relation = session
        .catalog()
        .describe_relation(path, &token)
        .await
        .expect("the description must succeed");
    let constraints = session
        .catalog()
        .list_constraints(path, &token)
        .await
        .expect("the constraints must succeed");
    let index = session
        .catalog()
        .list_indexes(path, &token)
        .await
        .expect("the indexes must succeed");

    let types: Vec<&str> = relation
        .fields
        .iter()
        .map(|field| field.raw_type.as_str())
        .collect();
    let expressions: Vec<&str> = constraints
        .iter()
        .filter_map(|constraint| constraint.expression.as_deref())
        .collect();
    let predicates: Vec<&str> = index
        .iter()
        .filter_map(|index_def| index_def.predicate.as_deref())
        .collect();
    format!("{types:?} | {expressions:?} | {predicates:?}")
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn introspection_does_not_depend_on_a_console_context() {
    use oxyn_catalog::CatalogPath;

    // The catalog shares the session pool, and it issues **no** `SET`: it
    // therefore cannot protect itself. Before the connection was reset to the
    // default on its way back to the pool, an object's definition was rendered
    // sometimes qualified, sometimes not, depending on the connection drawn — a
    // discrepancy no error signals and that a user would blame on the server.
    //
    // The rendering is indeed sensitive to `search_path`; observed on 17.11:
    //
    // | rendering              | off the path                | on the path         |
    // |------------------------|-----------------------------|---------------------|
    // | `format_type`          | `oxyn_ctx_a.ctx_amount`     | `ctx_amount`        |
    // | `pg_get_constraintdef` | `CHECK (oxyn_ctx_a.ctx_…)`  | `CHECK (ctx_…)`     |
    // | `pg_get_expr`          | `oxyn_ctx_a.ctx_positive(…)`| `ctx_positive(…)`   |
    //
    // This test is therefore not tautological: without the reset to the
    // default, the read forced onto a recycled connection returns the right
    // column.
    let Some(session) = session().await else {
        return;
    };
    prepare_defined_object(&*session).await;
    let path = CatalogPath::for_relation(None, Some(SCHEMA_A), DEFINED_TABLE).expect("path");

    // The reference: what the server returns when no context was ever
    // declared. The three forms must be qualified there, otherwise the fixture
    // would no longer exercise anything.
    let reference = definition_fingerprint(&*session, &path).await;
    for expected in [
        format!("{SCHEMA_A}.ctx_amount"),
        format!("{SCHEMA_A}.ctx_positive"),
    ] {
        assert!(
            reference.contains(&expected),
            "the fixture no longer exercises the qualified rendering ({expected}): {reference}"
        );
    }

    session
        .set_context(&context(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    // The four pool connections carried the `SET`, then gave it back.
    let seen_count = on_whole_pool(&*session, DEFINED_QUERY).await;
    assert_eq!(
        seen_count.len(),
        usize::try_from(MAX_CONNECTIONS).expect("four fits in a usize"),
        "the test did not exercise four distinct connections ({seen_count:?}): \
         it then proves nothing about what the pool keeps"
    );

    // The hardest case, and it is **deterministic**: the pool is capped at
    // four, all four just served an execution under a context, and three stay
    // tied up by live cursors. Introspection can therefore only borrow a
    // recycled connection — it has no choice.
    let mut held = Vec::new();
    for _ in 1..MAX_CONNECTIONS {
        held.push(
            session
                .execute(read_request(DEFINED_QUERY), &CancelToken::new())
                .await
                .expect("execution"),
        );
    }
    assert_eq!(
        definition_fingerprint(&*session, &path).await,
        reference,
        "introspection followed the console context"
    );
    drop(held);

    // Then alternating, to cover the connections as the pool rotates them.
    for round in 0..4 {
        let _ = on_whole_pool(&*session, DEFINED_QUERY).await;
        assert_eq!(
            definition_fingerprint(&*session, &path).await,
            reference,
            "the definition changed at round {round}"
        );
    }

    apply(&*session, &format!("DROP SCHEMA {SCHEMA_A} CASCADE")).await;
    session.close().await.expect("close");
}

/// The row count of the preview fixtures: enough for three pages.
///
/// Pagination that skips a row or shows the same one twice does not show on
/// ten rows; it shows on five hundred read page by page.
const PREVIEW_ROWS: i64 = 500;

/// Prepares a preview table, its ties and a canary table.
///
/// `bucket` is `id % 7`: sorting on it leaves dozens of ties, hence a non-total
/// order until the primary key completes it.
async fn prepare_preview(session: &dyn Session) {
    apply(
        session,
        "CREATE TABLE oxyn_preview_page (\
         id bigint PRIMARY KEY, bucket bigint, name text)",
    )
    .await;
    apply(
        session,
        &format!(
            "INSERT INTO oxyn_preview_page(id, bucket, name) \
             SELECT i, i % 7, 'row-' || i FROM generate_series(1, {PREVIEW_ROWS}) AS s(i)"
        ),
    )
    .await;
    apply(session, "CREATE TABLE oxyn_preview_witness (guard text)").await;
    apply(
        session,
        "INSERT INTO oxyn_preview_witness VALUES ('intact')",
    )
    .await;
}

/// Composes then executes a preview, and returns the integers of its first column.
async fn preview_ids(
    session: &dyn Session,
    relation: &str,
    limit: u32,
    shape: &PreviewShape,
) -> Vec<i64> {
    use arrow::array::AsArray as _;
    use oxyn_catalog::CatalogPath;

    let token = CancelToken::new();
    let path = CatalogPath::for_relation(None, Some("public"), relation).expect("valid path");
    let exec_request = session
        .preview_request(&path, limit, shape, &token)
        .await
        .unwrap_or_else(|error| panic!("composing the preview of `{relation}`: {error}"));
    let mut cursor = session
        .execute(exec_request, &token)
        .await
        .unwrap_or_else(|error| panic!("executing the preview of `{relation}`: {error}"));
    let mut ids = Vec::new();
    while let Some(batch) = cursor.next_batch().await.expect("stream without error") {
        let column = batch
            .column(0)
            .as_primitive_opt::<arrow::datatypes::Int64Type>()
            .expect("integer column");
        ids.extend(column.values().iter().copied());
    }
    ids
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn pages_of_a_sorted_preview_neither_overlap_nor_omit_a_row() {
    use oxyn_catalog::CatalogPath;

    let Some(session) = session().await else {
        return;
    };
    prepare_preview(&*session).await;

    // Simple sort, in both directions.
    let ascending = PreviewShape {
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    assert_eq!(
        preview_ids(&*session, "oxyn_preview_page", 10, &ascending).await,
        (1..=10).collect::<Vec<_>>()
    );
    let descending = PreviewShape {
        sort: vec![PreviewSort::descending("id")],
        ..PreviewShape::default()
    };
    assert_eq!(
        preview_ids(&*session, "oxyn_preview_page", 10, &descending).await,
        (PREVIEW_ROWS - 9..=PREVIEW_ROWS).rev().collect::<Vec<_>>()
    );

    // Three consecutive pages on a column full of ties.
    let size = 200_u32;
    let mut seen_count = Vec::new();
    let mut sizes = Vec::new();
    for page in 0..3_u64 {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("bucket")],
            offset: page * u64::from(size),
            ..PreviewShape::default()
        };
        let ids = preview_ids(&*session, "oxyn_preview_page", size, &shape).await;
        sizes.push(ids.len());
        seen_count.extend(ids);
    }
    assert_eq!(sizes, vec![200, 200, 100], "three pages, 500 rows");
    let mut sorted_ones = seen_count.clone();
    sorted_ones.sort_unstable();
    sorted_ones.dedup();
    assert_eq!(
        sorted_ones.len(),
        seen_count.len(),
        "no row must appear on two pages"
    );
    assert_eq!(
        sorted_ones,
        (1..=PREVIEW_ROWS).collect::<Vec<_>>(),
        "the union of the pages is exactly the table"
    );

    // A sort or projection column the relation does not declare: refused
    // here, never sent to the server, and permanent — retrying will not make
    // it appear.
    let path = CatalogPath::for_relation(None, Some("public"), "oxyn_preview_page").expect("path");
    let sorted = PreviewShape {
        sort: vec![PreviewSort::ascending("missing_column")],
        ..PreviewShape::default()
    };
    let projected = PreviewShape {
        columns: Some(vec!["missing_column".into()]),
        ..PreviewShape::default()
    };
    for unknown in [sorted, projected] {
        let error = session
            .preview_request(&path, 10, &unknown, &CancelToken::new())
            .await
            .expect_err("an unknown column is neither read nor sorted");
        assert!(
            matches!(&error, OxynError::Query(message)
                if message.contains("missing_column")),
            "{error}"
        );
        assert!(!error.is_retryable(), "{error}");
    }

    // A page on a relation without a unique key: refused, saying why.
    apply(&*session, "CREATE TABLE oxyn_preview_keyless (x text)").await;
    let without_key =
        CatalogPath::for_relation(None, Some("public"), "oxyn_preview_keyless").expect("path");
    let page = PreviewShape {
        offset: 1,
        ..PreviewShape::default()
    };
    let error = session
        .preview_request(&without_key, 10, &page, &CancelToken::new())
        .await
        .expect_err("a page without a unique key makes no sense");
    assert!(
        matches!(&error, OxynError::NotSupported { capability }
            if capability.contains("unique key")),
        "{error}"
    );
    // Its first page stays readable: it is today's preview.
    assert!(
        session
            .preview_request(
                &without_key,
                10,
                &PreviewShape::unordered(),
                &CancelToken::new()
            )
            .await
            .is_ok()
    );

    apply(&*session, "DROP TABLE oxyn_preview_keyless").await;
    apply(&*session, "DROP TABLE oxyn_preview_page").await;
    apply(&*session, "DROP TABLE oxyn_preview_witness").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_preview_predicate_is_sent_as_is_without_reaching_a_second_statement() {
    use arrow::array::{Array as _, AsArray as _};
    use oxyn_catalog::CatalogPath;

    let Some(session) = session().await else {
        return;
    };
    prepare_preview(&*session).await;
    apply(
        &*session,
        "INSERT INTO oxyn_preview_page(id, bucket, name) \
         VALUES (1001, 0, '100%'), (1002, 0, '100 percent')",
    )
    .await;
    let token = CancelToken::new();
    let path = CatalogPath::for_relation(None, Some("public"), "oxyn_preview_page").expect("path");

    // The `%` is not a metacharacter: the driver composes no pattern, it
    // passes the user's text through.
    async fn names(
        session: &dyn Session,
        path: &CatalogPath,
        token: &CancelToken,
        predicate: &str,
    ) -> Vec<String> {
        let shape = PreviewShape {
            predicate: Some(predicate.to_owned()),
            ..PreviewShape::default()
        };
        let exec_request = session
            .preview_request(path, 200, &shape, token)
            .await
            .expect("composition");
        let mut cursor = session
            .execute(exec_request, token)
            .await
            .expect("execution");
        let mut names = Vec::new();
        while let Some(batch) = cursor.next_batch().await.expect("stream") {
            let column = batch.column(2).as_string_opt::<i32>().expect("text column");
            for rank in 0..column.len() {
                names.push(column.value(rank).to_owned());
            }
        }
        names
    }
    assert_eq!(
        names(&*session, &path, &token, "name = '100%'").await,
        vec!["100%".to_owned()],
        "an equality returns only the literal row"
    );
    let mut pattern = names(&*session, &path, &token, "name LIKE '100%'").await;
    pattern.sort();
    assert_eq!(pattern, vec!["100 percent".to_owned(), "100%".to_owned()]);

    for hostile in [
        // A second statement: the extended protocol prepares only one
        // statement, so it cannot reach the server.
        "name = 'row-1'; DROP TABLE oxyn_preview_witness",
        "name = 'row-1'; DELETE FROM oxyn_preview_witness",
        // An unbalanced quote: syntax error, nothing more.
        "name = 'row-1",
        // An end-of-line comment: it must not swallow the LIMIT.
        "name LIKE 'row-%' -- ; DROP TABLE oxyn_preview_witness",
    ] {
        let shape = PreviewShape {
            predicate: Some(hostile.to_owned()),
            ..PreviewShape::default()
        };
        let exec_request = session
            .preview_request(&path, 3, &shape, &token)
            .await
            .expect("composition does not judge the predicate");
        assert!(
            exec_request.text.contains(hostile),
            "the predicate is sent as is: {}",
            exec_request.text
        );
        match session.execute(exec_request, &token).await {
            Err(_) => {}
            Ok(mut cursor) => {
                let (rows, _) = drain(&mut cursor).await;
                assert!(rows <= 3, "{hostile}: {rows} rows despite LIMIT 3");
            }
        }
        let mut cursor = session
            .execute(
                read_request("SELECT guard FROM oxyn_preview_witness"),
                &token,
            )
            .await
            .unwrap_or_else(|error| panic!("the canary table must survive `{hostile}`: {error}"));
        assert_eq!(drain(&mut cursor).await.0, 1, "canary after `{hostile}`");
    }

    // The two guards of the clause, tested here as on SQLite: the line break
    // for `--`, the parentheses for the `/*` no line break ends. PostgreSQL
    // already refused the second; it must keep doing so.
    let block = PreviewShape {
        predicate: Some("name IS NOT NULL /*".into()),
        ..PreviewShape::default()
    };
    let exec_request = session
        .preview_request(&path, 3, &block, &token)
        .await
        .expect("composition does not judge the predicate");
    assert!(
        session.execute(exec_request, &token).await.is_err(),
        "an unclosed block comment must be refused, not executed unbounded"
    );
    // The session survives this refusal.
    assert_eq!(
        preview_ids(
            &*session,
            "oxyn_preview_page",
            3,
            &PreviewShape::unordered()
        )
        .await
        .len(),
        3
    );

    let row = PreviewShape {
        predicate: Some("id > 0 -- this is a comment".into()),
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    assert_eq!(
        preview_ids(&*session, "oxyn_preview_page", 3, &row).await,
        vec![1, 2, 3]
    );

    // A predicate that already carries its parentheses returns exactly what
    // the same text would return without the wrapping.
    let parenthesis = PreviewShape {
        predicate: Some("(id > 0 AND bucket < 2) OR name IS NULL".into()),
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    let envelope = preview_ids(&*session, "oxyn_preview_page", 5, &parenthesis).await;
    let mut cursor = session
        .execute(
            read_request(
                "SELECT * FROM \"public\".\"oxyn_preview_page\" \
                 WHERE (id > 0 AND bucket < 2) OR name IS NULL \
                 ORDER BY \"id\" ASC LIMIT 5",
            ),
            &token,
        )
        .await
        .expect("the same text, without wrapping");
    let mut unwrapped = Vec::new();
    while let Some(batch) = cursor.next_batch().await.expect("stream") {
        let column = batch
            .column(0)
            .as_primitive_opt::<arrow::datatypes::Int64Type>()
            .expect("integer column");
        unwrapped.extend(column.values().iter().copied());
    }
    assert_eq!(envelope, unwrapped);
    assert_eq!(envelope, vec![1, 7, 8, 14, 15]);

    apply(&*session, "DROP TABLE oxyn_preview_page").await;
    apply(&*session, "DROP TABLE oxyn_preview_witness").await;
    session.close().await.expect("close");
}
