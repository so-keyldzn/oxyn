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
        appliquer(&*session, sql).await;
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
    appliquer(&*session, "DROP SCHEMA oxyn_incoming_source CASCADE").await;
    appliquer(&*session, "DROP SCHEMA oxyn_incoming_target CASCADE").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn constraints_report_validation_status_from_the_server() {
    use oxyn_catalog::CatalogPath;
    let Some(session) = session().await else {
        return;
    };
    appliquer(
        &*session,
        "CREATE TABLE oxyn_constraint_validation (id integer)",
    )
    .await;
    appliquer(
        &*session,
        "INSERT INTO oxyn_constraint_validation VALUES (-1)",
    )
    .await;
    appliquer(
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
    appliquer(&*session, "UPDATE oxyn_constraint_validation SET id = 1").await;
    appliquer(
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
    appliquer(&*session, "DROP TABLE oxyn_constraint_validation").await;
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
    appliquer(
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
    appliquer(&*session, "DROP TABLE oxyn_many_constraints").await;
    let literal = "x".repeat(16385);
    appliquer(
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
    appliquer(&*session, "DROP TABLE oxyn_large_constraint").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn constraints_preserve_names_column_order_and_engine_definitions() {
    use oxyn_catalog::{CatalogPath, ConstraintKind};
    let Some(session) = session().await else {
        return;
    };
    appliquer(&*session, "CREATE TABLE \"oxyn_constraints\"\";--\" (b integer, a integer NOT NULL, score integer, CONSTRAINT \"key\"\";--\" PRIMARY KEY (b, a), CONSTRAINT score_positive CHECK (score > 0), UNIQUE (a), FOREIGN KEY (a) REFERENCES \"oxyn_constraints\"\";--\" (a))").await;
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
    appliquer(&*session, "DROP TABLE \"oxyn_constraints\"\";--\"").await;
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
        appliquer(&*session, sql).await;
    }
    appliquer(&*session, "CREATE DOMAIN oxyn_preview_acl AS aclitem[]").await;
    appliquer(
        &*session,
        "CREATE TABLE oxyn_preview_types (\
        id bigint DEFAULT 1, at timestamptz, acl oxyn_preview_acl, function regproc, \
        \"a\"\"; --\" text)",
    )
    .await;
    appliquer(
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
        .execute(lecture("SELECT 52::regproc AS function"), &token)
        .await
        .expect("unmodified user SQL");
    let schema = raw.schema();
    assert_eq!(schema.field(0).data_type(), &DataType::Binary);
    assert_eq!(
        schema
            .field(0)
            .metadata()
            .get(crate::META_FALLBACK)
            .map(String::as_str),
        Some("opaque")
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
            .as_binary_opt::<i32>()
            .expect("binary")
            .value(0),
        52_u32.to_be_bytes()
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
        assert_eq!(drainer(&mut cursor).await.0, 1, "{relation}");
    }
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert!(matches!(
        session
            .preview_request(&path, 200, &PreviewShape::unordered(), &cancelled)
            .await,
        Err(OxynError::Cancelled)
    ));
    appliquer(&*session, "DROP TABLE oxyn_preview_types").await;
    appliquer(&*session, "DROP DOMAIN oxyn_preview_acl").await;
    session.close().await.expect("close");
}

/// An unlikely value, so that a test looking for it only finds it if it
/// really crossed.
const SENTINELLE: &str = "S3NT1NELLE-42";

/// The error of a failing execution, whether at preparation or in the
/// stream.
///
/// An invalid cast passes preparation — the type of `$1` is `text` — and only
/// fails at execution: the error then arrives through the cursor.
async fn echouer(session: &dyn Session, demande: ExecRequest) -> OxynError {
    match session.execute(demande, &CancelToken::new()).await {
        Err(erreur) => erreur,
        Ok(mut curseur) => loop {
            match curseur.next_batch().await {
                Ok(Some(_)) => {}
                Ok(None) => panic!("the statement should have been refused"),
                Err(erreur) => break erreur,
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

    let erreur = echouer(
        &*session,
        lecture("SELECT ($1::text)::integer")
            .with_params(vec![oxyn_core::ScalarValue::Text(SENTINELLE.to_owned())]),
    )
    .await;
    for rendu in [format!("{erreur}"), format!("{erreur:?}")] {
        assert!(!rendu.contains(SENTINELLE), "bound value rendered: {rendu}");
        assert!(!rendu.contains("invalid input syntax"), "{rendu}");
    }
    assert!(
        erreur.to_string().contains("withheld"),
        "the withholding must be stated: {erreur}"
    );
    // `invalid_text_representation`: the SQLSTATE survives, it is a code.
    assert!(erreur.to_string().contains("22P02"), "{erreur}");
    assert_eq!(erreur.class(), oxyn_core::ErrorClass::Permanent);

    // Without a bound value, the same refusal keeps PostgreSQL's message:
    // Oxyn's audience reads it, and a paraphrase would be a defect.
    let entier = echouer(
        &*session,
        lecture(&format!("SELECT ('{SENTINELLE}'::text)::integer")),
    )
    .await;
    assert!(
        entier
            .to_string()
            .contains("invalid input syntax for type integer"),
        "{entier}"
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
    appliquer(&*session, "CREATE TABLE oxyn_withheld (note text)").await;
    appliquer(
        &*session,
        "CREATE FUNCTION oxyn_withheld_guard() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'solde : %', NEW.note; END $$",
    )
    .await;
    appliquer(
        &*session,
        "CREATE TRIGGER oxyn_withheld_trigger BEFORE INSERT ON oxyn_withheld \
         FOR EACH ROW EXECUTE FUNCTION oxyn_withheld_guard()",
    )
    .await;

    let erreur = echouer(
        &*session,
        ecriture("INSERT INTO oxyn_withheld(note) VALUES ($1)")
            .with_params(vec![oxyn_core::ScalarValue::Text(SENTINELLE.to_owned())]),
    )
    .await;
    for rendu in [format!("{erreur}"), format!("{erreur:?}")] {
        assert!(!rendu.contains(SENTINELLE), "bound value rendered: {rendu}");
        assert!(!rendu.contains("solde"), "server message rendered: {rendu}");
    }
    assert!(erreur.to_string().contains("withheld"), "{erreur}");
    assert_eq!(erreur.class(), oxyn_core::ErrorClass::Permanent);

    appliquer(
        &*session,
        "DROP TRIGGER oxyn_withheld_trigger ON oxyn_withheld",
    )
    .await;
    appliquer(&*session, "DROP FUNCTION oxyn_withheld_guard()").await;
    appliquer(&*session, "DROP TABLE oxyn_withheld").await;
    session.close().await.expect("close");
}

/// The test configuration, or `None` when no server is declared.
pub(super) fn cible() -> Option<(ConnectionConfig, Credentials)> {
    let url = std::env::var(VARIABLE).ok()?;
    let relue = ParsedDsn::parse(&url).expect("OXYN_PG_TEST_URL must be a `postgres://` URL");
    let (parts, identifiants) = relue.into_parts();
    let config = parts
        .to_config("essai", DriverId::postgres())
        .with_environment(Environment::Local);
    Some((config, identifiants))
}

/// The error of an execution that should have been refused.
///
/// `Result::expect_err` requires `Debug` on the `Ok` variant, hence on
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

/// Opens a session, or returns `None` and says so.
pub(super) async fn session() -> Option<Box<dyn Session>> {
    let Some((config, identifiants)) = cible() else {
        eprintln!("{VARIABLE} is not set: test skipped");
        return None;
    };
    let driver = PostgresDriver::new();
    let session = driver
        .connect(&config, &identifiants, &CancelToken::new())
        .await
        .expect("the test server must be reachable");
    Some(session)
}

/// A read request, with wide limits.
fn lecture(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(None))
}

/// A write request, allowed to write.
fn ecriture(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_intent(StatementIntent::Write)
        .with_limits(ExecLimits::default().writable().with_max_rows(None))
}

/// Applies a statement and drains its cursor, expecting nothing from it.
///
/// Test setups and cleanups go through here: what matters is that the
/// statement is accepted, not what it returns.
pub(super) async fn appliquer(session: &dyn Session, sql: &str) {
    let mut curseur = session
        .execute(ecriture(sql), &CancelToken::new())
        .await
        .unwrap_or_else(|erreur| panic!("`{sql}` must be accepted: {erreur}"));
    let _ = drainer(&mut curseur).await;
}

/// Drains a cursor and returns (rows, batches).
async fn drainer(curseur: &mut Box<dyn Cursor>) -> (usize, usize) {
    let mut lignes = 0;
    let mut lots = 0;
    while let Some(lot) = curseur.next_batch().await.expect("stream without error") {
        lignes += lot.num_rows();
        lots += 1;
    }
    (lignes, lots)
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn the_connection_detects_the_variant_and_its_capabilities() {
    let Some(session) = session().await else {
        return;
    };
    let capacites = session.capabilities();
    assert!(capacites.contains(Capabilities::SQL));
    assert!(capacites.contains(Capabilities::SERVER_SIDE_CANCEL));
    assert!(capacites.contains(Capabilities::STREAMING));

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
    let mut curseur = session
        .execute(
            lecture("SELECT 1 AS un, 'deux'::text AS deux"),
            &CancelToken::new(),
        )
        .await
        .expect("execution");

    assert_eq!(curseur.schema().fields().len(), 2);
    assert_eq!(curseur.schema().field(0).name(), "un");

    let (lignes, _) = drainer(&mut curseur).await;
    assert_eq!(lignes, 1);
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
    let mut curseur = session
        .execute(
            lecture("SELECT i, repeat('x', 100) FROM generate_series(1, 2000000) AS s(i)"),
            &CancelToken::new(),
        )
        .await
        .expect("execution");

    let (lignes, lots) = drainer(&mut curseur).await;
    assert_eq!(lignes, 2_000_000);
    assert!(lots > 10, "the result must arrive in batches: {lots}");
    assert_eq!(curseur.stats().rows, 2_000_000);
    assert!(!curseur.stats().truncated);

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
    let jeton = CancelToken::new();
    let mut curseur = session
        .execute(lecture("SELECT pg_sleep(30)"), &jeton)
        .await
        .expect("execution");

    let poignee = curseur.handle();
    session
        .cancel(poignee)
        .await
        .expect("cancellation requested");

    let issue = curseur.next_batch().await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "the cursor must return a cancellation: {issue:?}"
    );

    // The proof: no `pg_sleep` runs any more for this database.
    drop(curseur);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut restants = session
        .execute(
            lecture(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE query LIKE '%pg_sleep%' AND state = 'active' AND pid <> pg_backend_pid()",
            ),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    let (lignes, _) = drainer(&mut restants).await;
    assert_eq!(lignes, 1, "one count row");

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
        let curseur = session
            .execute(lecture("SELECT pg_sleep(30)"), &CancelToken::new())
            .await
            .expect("execution");
        drop(curseur);
    }
    tokio::time::sleep(Duration::from_secs(1)).await;

    let mut curseur = session
        .execute(lecture("SELECT 1"), &CancelToken::new())
        .await
        .expect("the database still accepts connections");
    let (lignes, _) = drainer(&mut curseur).await;
    assert_eq!(lignes, 1);

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
    let demande = ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Postgres),
        "SELECT i FROM generate_series(1, 100000) AS s(i)",
    )
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default().with_max_rows(Some(1_000)));

    let mut curseur = session
        .execute(demande, &CancelToken::new())
        .await
        .expect("execution");
    let (lignes, _) = drainer(&mut curseur).await;

    assert_eq!(lignes, 1_000);
    assert!(curseur.stats().truncated, "truncation must be known");

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
    appliquer(
        &*session,
        "CREATE TABLE IF NOT EXISTS oxyn_essai_ro (id int)",
    )
    .await;

    // Preparation passes — `PREPARE` does not check read-only — and it is the
    // execution the server refuses. The refusal therefore arrives through the
    // stream, not through `execute`.
    let mut curseur = session
        .execute(
            lecture("INSERT INTO oxyn_essai_ro VALUES (1)"),
            &CancelToken::new(),
        )
        .await
        .expect("preparing an INSERT is accepted");
    let refus = curseur
        .next_batch()
        .await
        .expect_err("the server must refuse the write");
    assert!(
        refus.to_string().contains("bounded to read-only"),
        "the message must name the bounds, not the privileges: {refus}"
    );
    drop(curseur);

    appliquer(&*session, "DROP TABLE oxyn_essai_ro").await;

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
    let mut curseur = session
        .execute(
            lecture(
                "SELECT true::bool, 1::int2, 2::int4, 3::int8, 1.5::float4, 2.5::float8, \
                 12345678901234567890.12345678::numeric, 'texte'::text, \
                 '\\x00ff'::bytea, '67e55044-10b1-426f-9d0c-451f8ad05b1a'::uuid, \
                 '2026-09-05'::date, '14:30:00'::time, \
                 '2026-09-05 14:30:00'::timestamp, '2026-09-05 14:30:00+02'::timestamptz, \
                 '{\"a\": 1}'::jsonb, ARRAY[1, NULL, 3]::int4[]",
            ),
            &CancelToken::new(),
        )
        .await
        .expect("execution");

    let schema = curseur.schema();
    let attendus = [
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
    for (rang, attendu) in attendus.iter().enumerate() {
        assert_eq!(schema.field(rang).data_type(), attendu, "column {rang}");
    }
    assert!(matches!(schema.field(15).data_type(), DataType::List(_)));

    let lot = curseur
        .next_batch()
        .await
        .expect("stream")
        .expect("one row");
    assert_eq!(lot.num_rows(), 1);

    // The exact decimal: 26 significant digits, out of reach of an f64.
    let numerique = lot.column(6).as_string_opt::<i32>().expect("a text column");
    assert_eq!(numerique.value(0), "12345678901234567890.12345678");

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
        "DROP TYPE IF EXISTS oxyn_essai_etat",
        "CREATE TYPE oxyn_essai_etat AS ENUM ('brouillon', 'expedie')",
    ] {
        appliquer(&*session, sql).await;
    }

    let mut curseur = session
        .execute(
            lecture("SELECT 'expedie'::oxyn_essai_etat"),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    let lot = curseur
        .next_batch()
        .await
        .expect("stream")
        .expect("one row");
    let valeurs = lot
        .column(0)
        .as_string_opt::<i32>()
        .expect("a text fallback");
    assert_eq!(valeurs.value(0), "expedie");

    // The PostgreSQL type name survives in the field metadata.
    let champ = curseur.schema();
    let meta = champ.field(0).metadata();
    assert_eq!(
        meta.get(crate::META_PG_TYPE).map(String::as_str),
        Some("oxyn_essai_etat")
    );
    drop(curseur);

    appliquer(&*session, "DROP TYPE oxyn_essai_etat").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn introspection_walks_down_the_hierarchy_level_by_level() {
    let Some(session) = session().await else {
        return;
    };
    let jeton = CancelToken::new();

    for sql in [
        "DROP TABLE IF EXISTS oxyn_essai_ligne",
        "DROP TABLE IF EXISTS oxyn_essai_commande",
        "CREATE TABLE oxyn_essai_commande (id bigserial PRIMARY KEY, \
         montant numeric(12,2) NOT NULL, cree timestamptz DEFAULT now())",
        "CREATE TABLE oxyn_essai_ligne (id bigserial PRIMARY KEY, \
         commande_id bigint NOT NULL REFERENCES oxyn_essai_commande(id) ON DELETE CASCADE)",
        "CREATE INDEX oxyn_essai_ligne_commande ON oxyn_essai_ligne (commande_id)",
        "COMMENT ON TABLE oxyn_essai_commande IS 'commandes de la caisse'",
    ] {
        appliquer(&*session, sql).await;
    }

    let catalogue = session.catalog();
    let espaces = catalogue
        .list_namespaces(None, &jeton)
        .await
        .expect("the schemas");
    assert!(espaces.iter().any(|e| e.name() == "public"));
    assert!(
        espaces
            .iter()
            .any(|e| e.name() == "pg_catalog" && e.is_system),
        "system schemas are marked, not hidden"
    );

    let public = oxyn_catalog::CatalogPath::for_namespace(None, "public").expect("path");
    let relations = catalogue
        .list_relations(&public, &jeton)
        .await
        .expect("the relations");
    assert!(relations.iter().any(|r| r.name() == "oxyn_essai_commande"));

    let commande = public.with_relation("oxyn_essai_commande").expect("path");
    let decrite = catalogue
        .describe_relation(&commande, &jeton)
        .await
        .expect("the description");
    assert_eq!(decrite.comment.as_deref(), Some("commandes de la caisse"));
    let montant = decrite.field("montant").expect("the montant column");
    assert!(!montant.nullable);
    assert_eq!(
        montant.logical_type,
        oxyn_catalog::LogicalType::Decimal {
            precision: Some(12),
            scale: Some(2)
        }
    );
    assert_eq!(decrite.primary_key().len(), 1);

    let ligne = public.with_relation("oxyn_essai_ligne").expect("path");
    let index = catalogue
        .list_indexes(&ligne, &jeton)
        .await
        .expect("the indexes");
    assert!(index.iter().any(|i| i.name == "oxyn_essai_ligne_commande"));

    let cles = catalogue
        .list_foreign_keys(&ligne, &jeton)
        .await
        .expect("the foreign keys");
    let cle = cles.first().expect("a foreign key");
    assert!(cle.is_well_formed());
    assert_eq!(cle.fields, ["commande_id"]);
    assert_eq!(
        cle.references.relation.relation(),
        Some("oxyn_essai_commande")
    );
    assert_eq!(cle.on_delete, oxyn_catalog::ReferentialAction::Cascade);

    for sql in [
        "DROP TABLE oxyn_essai_ligne",
        "DROP TABLE oxyn_essai_commande",
    ] {
        appliquer(&*session, sql).await;
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
    let erreur = session
        .catalog()
        .list_namespaces(Some("une_autre_base"), &CancelToken::new())
        .await
        .expect_err("refusal expected");
    assert!(
        matches!(erreur, OxynError::CatalogUnavailable(_)),
        "{erreur:?}"
    );
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the module documentation"]
async fn a_syntax_error_is_permanent_and_carries_its_sqlstate() {
    let Some(session) = session().await else {
        return;
    };
    let erreur = refus(
        session
            .execute(lecture("SELECT FROM WHERE"), &CancelToken::new())
            .await,
        "rejection expected",
    );

    assert!(!erreur.is_retryable(), "a wrong syntax is not retried");
    if let OxynError::Driver { source, .. } = &erreur {
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
    let erreur = refus(
        session
            .execute(
                lecture("SELECT * FROM oxyn_facturse WHERE client_id = $1")
                    .with_params(vec![oxyn_core::ScalarValue::Int64(42)]),
                &CancelToken::new(),
            )
            .await,
        "a missing table must be refused at preparation",
    );

    assert!(
        erreur.to_string().contains("oxyn_facturse"),
        "the faulty object name must survive: {erreur}"
    );
    assert!(
        !erreur.to_string().contains("withheld"),
        "no value reached the server: {erreur}"
    );
    assert_eq!(erreur.class(), oxyn_core::ErrorClass::Permanent);
    if let OxynError::Driver { source, .. } = &erreur {
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
const TABLE_PARTAGEE: &str = "shared_target";
/// A table only the server's default `search_path` reaches.
const TABLE_PAR_DEFAUT: &str = "oxyn_ctx_default_only";

/// A context that names only a namespace.
fn contexte(namespace: &str) -> SessionContext {
    SessionContext::new(None, Some(namespace.to_owned()))
}

/// Creates the two twin schemas and the table only the default reaches.
async fn preparer_jumeaux(session: &dyn Session) {
    nettoyer_jumeaux(session).await;
    for (schema, marqueur) in [(SCHEMA_A, "a"), (SCHEMA_B, "b")] {
        appliquer(session, &format!("CREATE SCHEMA {schema}")).await;
        appliquer(
            session,
            &format!("CREATE TABLE {schema}.{TABLE_PARTAGEE} (marker text)"),
        )
        .await;
        appliquer(
            session,
            &format!("INSERT INTO {schema}.{TABLE_PARTAGEE} VALUES ('{marqueur}')"),
        )
        .await;
    }
    appliquer(
        session,
        &format!("CREATE TABLE public.{TABLE_PAR_DEFAUT} (marker text)"),
    )
    .await;
    appliquer(
        session,
        &format!("INSERT INTO public.{TABLE_PAR_DEFAUT} VALUES ('defaut')"),
    )
    .await;
}

/// Undoes what [`preparer_jumeaux`] created.
async fn nettoyer_jumeaux(session: &dyn Session) {
    for schema in [SCHEMA_A, SCHEMA_B] {
        appliquer(session, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE")).await;
    }
    appliquer(
        session,
        &format!("DROP TABLE IF EXISTS public.{TABLE_PAR_DEFAUT}"),
    )
    .await;
}

/// The first text of the first row of a read, with the stream **drained**.
///
/// Draining is not politeness: a cursor dropped mid-stream leaves unread bytes,
/// so its connection is closed instead of returning to the pool. Tests that
/// compare pids from one execution to the next would then never get the same
/// connection twice, and would prove nothing about the state it carries.
async fn premier_texte(session: &dyn Session, sql: &str) -> String {
    use arrow::array::AsArray as _;
    let mut curseur = session
        .execute(lecture(sql), &CancelToken::new())
        .await
        .unwrap_or_else(|erreur| panic!("`{sql}` must execute: {erreur}"));
    let mut premier = None;
    while let Some(lot) = curseur
        .next_batch()
        .await
        .unwrap_or_else(|erreur| panic!("`{sql}` must return a batch: {erreur}"))
    {
        if premier.is_none() && lot.num_rows() > 0 {
            premier = Some(
                lot.column(0)
                    .as_string_opt::<i32>()
                    .expect("a text column")
                    .value(0)
                    .to_owned(),
            );
        }
    }
    premier.unwrap_or_else(|| panic!("`{sql}` must return a row"))
}

/// What `SELECT marker FROM shared_target` resolves, unqualified.
const LECTURE_NUE: &str = "SELECT marker FROM shared_target";

/// What [`requete_de_resolution`] returns when the bare name designates nothing.
const INTROUVABLE: &str = "introuvable";

/// The query that reads the marker of the same-named table, unqualified.
///
/// The `CROSS JOIN` is not decorative: it makes the result long enough for the
/// stream task to stay blocked on its connection until we drain. This is what
/// forces the pool to open another one for the next execution.
const REQUETE_DE_MARQUEUR: &str = "SELECT marker || '@' || pg_catalog.pg_backend_pid()::text \
                                   FROM shared_target CROSS JOIN generate_series(1, 50000)";

/// The query that asks the server what it resolves for a bare name.
///
/// Two reasons to go through `to_regclass` rather than a failing query: it
/// uses exactly the `search_path` of an ordinary statement, and it returns
/// `NULL` instead of raising. The connection therefore stays healthy and
/// returns to the pool — a condition without which two successive calls never
/// hit the same connections, and prove nothing about the state they keep.
fn requete_de_resolution(nom: &str) -> String {
    format!(
        "SELECT coalesce(pg_catalog.to_regclass('{nom}')::text, '{INTROUVABLE}') \
         || '@' || pg_catalog.pg_backend_pid()::text FROM generate_series(1, 50000)"
    )
}

/// What **each** connection of the pool answers, keyed by pid.
///
/// The query must return `value@pid` in its first column, and enough rows to
/// keep its connection busy — see [`REQUETE_DE_MARQUEUR`].
///
/// The four executions are alive at the same time: `execute` holds its
/// connection from start to end and the cursor channel is bounded to one batch,
/// so until we drain, each cursor ties one up. The pool has four
/// ([`MAX_CONNECTIONS`]), so all four are exercised. The final drain leaves
/// them healthy: a dropped cursor leaves unread bytes, its connection is
/// closed, and the next call would not find the same ones.
async fn sur_tout_le_bassin(
    session: &dyn Session,
    sql: &str,
) -> std::collections::BTreeMap<String, String> {
    use arrow::array::AsArray as _;

    let mut curseurs = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        curseurs.push(
            session
                .execute(lecture(sql), &CancelToken::new())
                .await
                .expect("execution"),
        );
    }

    let mut vues = std::collections::BTreeMap::new();
    for curseur in &mut curseurs {
        let mut premier = None;
        while let Some(lot) = curseur.next_batch().await.expect("stream") {
            if premier.is_none() && lot.num_rows() > 0 {
                premier = Some(
                    lot.column(0)
                        .as_string_opt::<i32>()
                        .expect("a text column")
                        .value(0)
                        .to_owned(),
                );
            }
        }
        let brut = premier.expect("one row");
        let (quoi, processus) = brut
            .split_once('@')
            .expect("the template composes both fields");
        vues.insert(processus.to_owned(), quoi.to_owned());
    }
    vues
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
    preparer_jumeaux(&*session).await;
    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    let vues = sur_tout_le_bassin(&*session, REQUETE_DE_MARQUEUR).await;
    assert_eq!(
        vues.len(),
        usize::try_from(MAX_CONNECTIONS).expect("four fits in a usize"),
        "the test did not exercise four distinct connections ({vues:?}): \
         it then proves nothing about per-connection state"
    );
    for (processus, marqueur) in &vues {
        assert_eq!(
            marqueur, "a",
            "connection {processus} resolved outside {SCHEMA_A}"
        );
    }

    nettoyer_jumeaux(&*session).await;
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
    preparer_jumeaux(&*session).await;

    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");
    let avant = sur_tout_le_bassin(&*session, REQUETE_DE_MARQUEUR).await;
    assert!(avant.values().all(|vu| vu == "a"), "{avant:?}");

    session
        .set_context(&contexte(SCHEMA_B), &CancelToken::new())
        .await
        .expect("the second context must be accepted");
    let apres = sur_tout_le_bassin(&*session, REQUETE_DE_MARQUEUR).await;
    assert_eq!(
        apres.keys().collect::<Vec<_>>(),
        avant.keys().collect::<Vec<_>>(),
        "the pool must have reused the connections that carried the prepared \
         statement: otherwise the test proves nothing about the cache"
    );
    for (processus, marqueur) in &apres {
        assert_eq!(
            marqueur, "b",
            "connection {processus} still reads the old schema"
        );
    }

    nettoyer_jumeaux(&*session).await;
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
    preparer_jumeaux(&*session).await;

    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");
    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");

    let sous_contexte = sur_tout_le_bassin(&*session, &requete_de_resolution(TABLE_PARTAGEE)).await;
    assert_eq!(
        sous_contexte.len(),
        usize::try_from(MAX_CONNECTIONS).expect("four fits in a usize"),
        "the test did not exercise four distinct connections: {sous_contexte:?}"
    );
    assert!(
        sous_contexte.values().all(|vu| vu == TABLE_PARTAGEE),
        "every connection must resolve the bare name: {sous_contexte:?}"
    );

    session
        .set_context(&SessionContext::server_default(), &CancelToken::new())
        .await
        .expect("going back to the default must be accepted");

    let apres = sur_tout_le_bassin(&*session, &requete_de_resolution(TABLE_PARTAGEE)).await;
    assert_eq!(
        apres.keys().collect::<Vec<_>>(),
        sous_contexte.keys().collect::<Vec<_>>(),
        "the pool did not reuse the connections that carried the context: \
         the test then does not prove that the `SET … TO DEFAULT` is emitted"
    );
    assert!(
        apres.values().all(|vu| vu == INTROUVABLE),
        "outside the context, `shared_target` must no longer resolve: {apres:?}"
    );

    let visible = sur_tout_le_bassin(&*session, &requete_de_resolution(TABLE_PAR_DEFAUT)).await;
    assert!(
        visible.values().all(|vu| vu == TABLE_PAR_DEFAUT),
        "the default `search_path` must become resolvable again: {visible:?}"
    );
    assert_eq!(
        premier_texte(&*session, &format!("SELECT marker FROM {TABLE_PAR_DEFAUT}")).await,
        "defaut"
    );

    // And the real query fails the way the server says.
    //
    // Through `echouer` and not `refus`: `sqlx` keeps a **per-connection**
    // prepared statement cache, keyed on the text. The same text having already
    // been prepared above under `oxyn_ctx_a`, preparation does not go back to
    // the server and returns without error. It is the server that redoes the
    // analysis at execution, because `search_path` changed — the refusal thus
    // arrives through the stream. What this test checks here is precisely that
    // the cache does not keep the old resolution alive.
    let disparue = echouer(&*session, lecture(LECTURE_NUE)).await;
    assert!(disparue.to_string().contains(TABLE_PARTAGEE), "{disparue}");

    nettoyer_jumeaux(&*session).await;
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
    preparer_jumeaux(&*session).await;
    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    let erreur = refus(
        session
            .set_context(&contexte("oxyn_ctx_absent"), &CancelToken::new())
            .await,
        "a missing schema must be refused",
    );
    assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
    assert!(erreur.to_string().contains("oxyn_ctx_absent"), "{erreur}");
    assert!(erreur.is_user_error(), "{erreur}");

    assert_eq!(
        session
            .context()
            .and_then(|vu| vu.namespace().map(str::to_owned)),
        Some(SCHEMA_A.to_owned()),
        "the confirmed context must not move on a refusal"
    );
    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");

    nettoyer_jumeaux(&*session).await;
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
    preparer_jumeaux(&*session).await;

    let erreur = refus(
        session
            .set_context(
                &SessionContext::new(
                    Some("oxyn_une_autre_base".to_owned()),
                    Some(SCHEMA_A.to_owned()),
                ),
                &CancelToken::new(),
            )
            .await,
        "another database must be refused",
    );
    assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
    assert!(erreur.to_string().contains("database"), "{erreur}");
    assert!(session.context().is_none(), "no context must be kept");

    // The connection's own database is a legitimate `catalog` level.
    let base = premier_texte(&*session, "SELECT current_database()::text").await;
    session
        .set_context(
            &SessionContext::new(Some(base), Some(SCHEMA_A.to_owned())),
            &CancelToken::new(),
        )
        .await
        .expect("the connection database must be accepted");
    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");

    nettoyer_jumeaux(&*session).await;
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
    preparer_jumeaux(&*session).await;
    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");
    assert_eq!(
        premier_texte(
            &*session,
            &format!("SELECT marker FROM {SCHEMA_B}.{TABLE_PARTAGEE}")
        )
        .await,
        "b",
        "a qualification written by hand wins over the context"
    );

    // The most direct proof: ask the server for the text it received.
    const RELU: &str = "SELECT query FROM pg_catalog.pg_stat_activity \
                        WHERE pid = pg_catalog.pg_backend_pid()";
    assert_eq!(
        premier_texte(&*session, RELU).await,
        RELU,
        "the text received by the server must be exactly the one submitted"
    );

    nettoyer_jumeaux(&*session).await;
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
    let citer = |nom: &str| quote_identifier(nom, QuoteStyle::for_dialect(SqlDialect::Postgres));

    for (nom, marqueur) in hostiles {
        let cite = citer(nom);
        appliquer(&*session, &format!("DROP SCHEMA IF EXISTS {cite} CASCADE")).await;
        appliquer(&*session, &format!("CREATE SCHEMA {cite}")).await;
        appliquer(
            &*session,
            &format!("CREATE TABLE {cite}.{TABLE_PARTAGEE} (marker text)"),
        )
        .await;
        appliquer(
            &*session,
            &format!("INSERT INTO {cite}.{TABLE_PARTAGEE} VALUES ('{marqueur}')"),
        )
        .await;
    }

    for (nom, marqueur) in hostiles {
        session
            .set_context(&contexte(nom), &CancelToken::new())
            .await
            .unwrap_or_else(|erreur| panic!("`{nom}` must be accepted: {erreur}"));
        assert_eq!(
            premier_texte(&*session, LECTURE_NUE).await,
            marqueur,
            "`{nom}` was not quoted correctly"
        );
    }

    session
        .set_context(&SessionContext::server_default(), &CancelToken::new())
        .await
        .expect("back to the default");
    for (nom, _) in hostiles {
        appliquer(
            &*session,
            &format!("DROP SCHEMA IF EXISTS {} CASCADE", citer(nom)),
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
    preparer_jumeaux(&*session).await;
    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    let mut curseur = session
        .execute(
            lecture(&format!("INSERT INTO {TABLE_PARTAGEE} VALUES ('intrus')")),
            &CancelToken::new(),
        )
        .await
        .expect("preparing an INSERT is accepted");
    let refus = curseur
        .next_batch()
        .await
        .expect_err("the server must refuse the write");
    assert!(
        refus.to_string().contains("bounded to read-only"),
        "the message must name the bounds, not the privileges: {refus}"
    );
    drop(curseur);

    // And reading still works in the declared context.
    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");
    assert_eq!(
        premier_texte(&*session, "SELECT count(*)::text FROM shared_target").await,
        "1",
        "the refused write must have left nothing"
    );

    nettoyer_jumeaux(&*session).await;
    session.close().await.expect("close");
}

/// The table whose definition is read, in [`SCHEMA_A`].
///
/// Distinct from [`TABLE_PARTAGEE`]: what is tested here is not the resolution
/// of a bare name, but the **rendering** of a definition.
const TABLE_DEFINIE: &str = "ctx_defined";

/// The query that keeps a connection busy working in [`TABLE_DEFINIE`].
const REQUETE_DEFINIE: &str = "SELECT marker || '@' || pg_catalog.pg_backend_pid()::text \
                               FROM ctx_defined CROSS JOIN generate_series(1, 50000)";

/// Creates in [`SCHEMA_A`] an object whose definition **renders** differently
/// depending on the `search_path`.
///
/// A domain and a function, both in the schema: `format_type`,
/// `pg_get_constraintdef` and `pg_get_expr` qualify their output when the
/// schema is not on the path, and omit it when it is. This is exactly the
/// channel through which a console's context could leak into introspection.
async fn preparer_objet_defini(session: &dyn Session) {
    appliquer(
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
            "CREATE TABLE {SCHEMA_A}.{TABLE_DEFINIE} (\
             marker text, amount {SCHEMA_A}.ctx_amount, \
             CONSTRAINT ctx_defined_positive CHECK ({SCHEMA_A}.ctx_positive(amount)))"
        ),
        format!(
            "CREATE INDEX ctx_defined_partial ON {SCHEMA_A}.{TABLE_DEFINIE} (marker) \
             WHERE {SCHEMA_A}.ctx_positive(amount)"
        ),
        format!("INSERT INTO {SCHEMA_A}.{TABLE_DEFINIE} VALUES ('a', 1)"),
    ] {
        appliquer(session, &sql).await;
    }
}

/// Everything the catalog returns that is sensitive to `search_path`, as one
/// string.
///
/// The three sources named by the finding: `format_type` for `raw_type`,
/// `pg_get_constraintdef` for a constraint expression, `pg_get_expr` for the
/// predicate of a partial index.
async fn empreinte_de_definition(
    session: &dyn Session,
    path: &oxyn_catalog::CatalogPath,
) -> String {
    let jeton = CancelToken::new();
    let relation = session
        .catalog()
        .describe_relation(path, &jeton)
        .await
        .expect("the description must succeed");
    let contraintes = session
        .catalog()
        .list_constraints(path, &jeton)
        .await
        .expect("the constraints must succeed");
    let index = session
        .catalog()
        .list_indexes(path, &jeton)
        .await
        .expect("the indexes must succeed");

    let types: Vec<&str> = relation
        .fields
        .iter()
        .map(|champ| champ.raw_type.as_str())
        .collect();
    let expressions: Vec<&str> = contraintes
        .iter()
        .filter_map(|contrainte| contrainte.expression.as_deref())
        .collect();
    let predicats: Vec<&str> = index
        .iter()
        .filter_map(|un_index| un_index.predicate.as_deref())
        .collect();
    format!("{types:?} | {expressions:?} | {predicats:?}")
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
    preparer_objet_defini(&*session).await;
    let path = CatalogPath::for_relation(None, Some(SCHEMA_A), TABLE_DEFINIE).expect("path");

    // The reference: what the server returns when no context was ever
    // declared. The three forms must be qualified there, otherwise the fixture
    // would no longer exercise anything.
    let reference = empreinte_de_definition(&*session, &path).await;
    for attendu in [
        format!("{SCHEMA_A}.ctx_amount"),
        format!("{SCHEMA_A}.ctx_positive"),
    ] {
        assert!(
            reference.contains(&attendu),
            "the fixture no longer exercises the qualified rendering ({attendu}): {reference}"
        );
    }

    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("the context must be accepted");

    // The four pool connections carried the `SET`, then gave it back.
    let vues = sur_tout_le_bassin(&*session, REQUETE_DEFINIE).await;
    assert_eq!(
        vues.len(),
        usize::try_from(MAX_CONNECTIONS).expect("four fits in a usize"),
        "the test did not exercise four distinct connections ({vues:?}): \
         it then proves nothing about what the pool keeps"
    );

    // The hardest case, and it is **deterministic**: the pool is capped at
    // four, all four just served an execution under a context, and three stay
    // tied up by live cursors. Introspection can therefore only borrow a
    // recycled connection — it has no choice.
    let mut occupees = Vec::new();
    for _ in 1..MAX_CONNECTIONS {
        occupees.push(
            session
                .execute(lecture(REQUETE_DEFINIE), &CancelToken::new())
                .await
                .expect("execution"),
        );
    }
    assert_eq!(
        empreinte_de_definition(&*session, &path).await,
        reference,
        "introspection followed the console context"
    );
    drop(occupees);

    // Then alternating, to cover the connections as the pool rotates them.
    for tour in 0..4 {
        let _ = sur_tout_le_bassin(&*session, REQUETE_DEFINIE).await;
        assert_eq!(
            empreinte_de_definition(&*session, &path).await,
            reference,
            "the definition changed at round {tour}"
        );
    }

    appliquer(&*session, &format!("DROP SCHEMA {SCHEMA_A} CASCADE")).await;
    session.close().await.expect("close");
}

/// The row count of the preview fixtures: enough for three pages.
///
/// Pagination that skips a row or shows the same one twice does not show on
/// ten rows; it shows on five hundred read page by page.
const LIGNES_APERCU: i64 = 500;

/// Prepares a preview table, its ties and a canary table.
///
/// `seau` is `id % 7`: sorting on it leaves dozens of ties, hence a non-total
/// order until the primary key completes it.
async fn preparer_apercu(session: &dyn Session) {
    appliquer(
        session,
        "CREATE TABLE oxyn_preview_page (\
         id bigint PRIMARY KEY, seau bigint, nom text)",
    )
    .await;
    appliquer(
        session,
        &format!(
            "INSERT INTO oxyn_preview_page(id, seau, nom) \
             SELECT i, i % 7, 'ligne-' || i FROM generate_series(1, {LIGNES_APERCU}) AS s(i)"
        ),
    )
    .await;
    appliquer(session, "CREATE TABLE oxyn_preview_temoin (garde text)").await;
    appliquer(
        session,
        "INSERT INTO oxyn_preview_temoin VALUES ('intacte')",
    )
    .await;
}

/// Composes then executes a preview, and returns the integers of its first column.
async fn ids_apercu(
    session: &dyn Session,
    relation: &str,
    limit: u32,
    shape: &PreviewShape,
) -> Vec<i64> {
    use arrow::array::AsArray as _;
    use oxyn_catalog::CatalogPath;

    let jeton = CancelToken::new();
    let chemin = CatalogPath::for_relation(None, Some("public"), relation).expect("valid path");
    let demande = session
        .preview_request(&chemin, limit, shape, &jeton)
        .await
        .unwrap_or_else(|erreur| panic!("composing the preview of `{relation}`: {erreur}"));
    let mut curseur = session
        .execute(demande, &jeton)
        .await
        .unwrap_or_else(|erreur| panic!("executing the preview of `{relation}`: {erreur}"));
    let mut ids = Vec::new();
    while let Some(lot) = curseur.next_batch().await.expect("stream without error") {
        let colonne = lot
            .column(0)
            .as_primitive_opt::<arrow::datatypes::Int64Type>()
            .expect("integer column");
        ids.extend(colonne.values().iter().copied());
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
    preparer_apercu(&*session).await;

    // Simple sort, in both directions.
    let croissant = PreviewShape {
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    assert_eq!(
        ids_apercu(&*session, "oxyn_preview_page", 10, &croissant).await,
        (1..=10).collect::<Vec<_>>()
    );
    let decroissant = PreviewShape {
        sort: vec![PreviewSort::descending("id")],
        ..PreviewShape::default()
    };
    assert_eq!(
        ids_apercu(&*session, "oxyn_preview_page", 10, &decroissant).await,
        (LIGNES_APERCU - 9..=LIGNES_APERCU)
            .rev()
            .collect::<Vec<_>>()
    );

    // Three consecutive pages on a column full of ties.
    let taille = 200_u32;
    let mut vues = Vec::new();
    let mut tailles = Vec::new();
    for page in 0..3_u64 {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("seau")],
            offset: page * u64::from(taille),
            ..PreviewShape::default()
        };
        let ids = ids_apercu(&*session, "oxyn_preview_page", taille, &shape).await;
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

    // A sort or projection column the relation does not declare: refused
    // here, never sent to the server, and permanent — retrying will not make
    // it appear.
    let chemin =
        CatalogPath::for_relation(None, Some("public"), "oxyn_preview_page").expect("path");
    let triee = PreviewShape {
        sort: vec![PreviewSort::ascending("colonne_absente")],
        ..PreviewShape::default()
    };
    let projetee = PreviewShape {
        columns: Some(vec!["colonne_absente".into()]),
        ..PreviewShape::default()
    };
    for inconnue in [triee, projetee] {
        let erreur = session
            .preview_request(&chemin, 10, &inconnue, &CancelToken::new())
            .await
            .expect_err("an unknown column is neither read nor sorted");
        assert!(
            matches!(&erreur, OxynError::Query(message)
                if message.contains("colonne_absente")),
            "{erreur}"
        );
        assert!(!erreur.is_retryable(), "{erreur}");
    }

    // A page on a relation without a unique key: refused, saying why.
    appliquer(&*session, "CREATE TABLE oxyn_preview_sans_cle (x text)").await;
    let sans_cle =
        CatalogPath::for_relation(None, Some("public"), "oxyn_preview_sans_cle").expect("path");
    let page = PreviewShape {
        offset: 1,
        ..PreviewShape::default()
    };
    let erreur = session
        .preview_request(&sans_cle, 10, &page, &CancelToken::new())
        .await
        .expect_err("a page without a unique key makes no sense");
    assert!(
        matches!(&erreur, OxynError::NotSupported { capability }
            if capability.contains("unique key")),
        "{erreur}"
    );
    // Its first page stays readable: it is today's preview.
    assert!(
        session
            .preview_request(
                &sans_cle,
                10,
                &PreviewShape::unordered(),
                &CancelToken::new()
            )
            .await
            .is_ok()
    );

    appliquer(&*session, "DROP TABLE oxyn_preview_sans_cle").await;
    appliquer(&*session, "DROP TABLE oxyn_preview_page").await;
    appliquer(&*session, "DROP TABLE oxyn_preview_temoin").await;
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
    preparer_apercu(&*session).await;
    appliquer(
        &*session,
        "INSERT INTO oxyn_preview_page(id, seau, nom) \
         VALUES (1001, 0, '100%'), (1002, 0, '100 pour cent')",
    )
    .await;
    let jeton = CancelToken::new();
    let chemin =
        CatalogPath::for_relation(None, Some("public"), "oxyn_preview_page").expect("path");

    // The `%` is not a metacharacter: the driver composes no pattern, it
    // passes the user's text through.
    async fn noms(
        session: &dyn Session,
        chemin: &CatalogPath,
        jeton: &CancelToken,
        predicate: &str,
    ) -> Vec<String> {
        let shape = PreviewShape {
            predicate: Some(predicate.to_owned()),
            ..PreviewShape::default()
        };
        let demande = session
            .preview_request(chemin, 200, &shape, jeton)
            .await
            .expect("composition");
        let mut curseur = session.execute(demande, jeton).await.expect("execution");
        let mut noms = Vec::new();
        while let Some(lot) = curseur.next_batch().await.expect("stream") {
            let colonne = lot.column(2).as_string_opt::<i32>().expect("colonne texte");
            for rang in 0..colonne.len() {
                noms.push(colonne.value(rang).to_owned());
            }
        }
        noms
    }
    assert_eq!(
        noms(&*session, &chemin, &jeton, "nom = '100%'").await,
        vec!["100%".to_owned()],
        "an equality returns only the literal row"
    );
    let mut motif = noms(&*session, &chemin, &jeton, "nom LIKE '100%'").await;
    motif.sort();
    assert_eq!(motif, vec!["100 pour cent".to_owned(), "100%".to_owned()]);

    for hostile in [
        // A second statement: the extended protocol prepares only one
        // statement, so it cannot reach the server.
        "nom = 'ligne-1'; DROP TABLE oxyn_preview_temoin",
        "nom = 'ligne-1'; DELETE FROM oxyn_preview_temoin",
        // An unbalanced quote: syntax error, nothing more.
        "nom = 'ligne-1",
        // An end-of-line comment: it must not swallow the LIMIT.
        "nom LIKE 'ligne-%' -- ; DROP TABLE oxyn_preview_temoin",
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
            "the predicate is sent as is: {}",
            demande.text
        );
        match session.execute(demande, &jeton).await {
            Err(_) => {}
            Ok(mut curseur) => {
                let (lignes, _) = drainer(&mut curseur).await;
                assert!(lignes <= 3, "{hostile}: {lignes} rows despite LIMIT 3");
            }
        }
        let mut curseur = session
            .execute(lecture("SELECT garde FROM oxyn_preview_temoin"), &jeton)
            .await
            .unwrap_or_else(|erreur| panic!("the canary table must survive `{hostile}`: {erreur}"));
        assert_eq!(drainer(&mut curseur).await.0, 1, "canary after `{hostile}`");
    }

    // The two guards of the clause, tested here as on SQLite: the line break
    // for `--`, the parentheses for the `/*` no line break ends. PostgreSQL
    // already refused the second; it must keep doing so.
    let bloc = PreviewShape {
        predicate: Some("nom IS NOT NULL /*".into()),
        ..PreviewShape::default()
    };
    let demande = session
        .preview_request(&chemin, 3, &bloc, &jeton)
        .await
        .expect("composition does not judge the predicate");
    assert!(
        session.execute(demande, &jeton).await.is_err(),
        "an unclosed block comment must be refused, not executed unbounded"
    );
    // The session survives this refusal.
    assert_eq!(
        ids_apercu(
            &*session,
            "oxyn_preview_page",
            3,
            &PreviewShape::unordered()
        )
        .await
        .len(),
        3
    );

    let ligne = PreviewShape {
        predicate: Some("id > 0 -- ceci est un commentaire".into()),
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    assert_eq!(
        ids_apercu(&*session, "oxyn_preview_page", 3, &ligne).await,
        vec![1, 2, 3]
    );

    // A predicate that already carries its parentheses returns exactly what
    // the same text would return without the wrapping.
    let parenthese = PreviewShape {
        predicate: Some("(id > 0 AND seau < 2) OR nom IS NULL".into()),
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    let enveloppe = ids_apercu(&*session, "oxyn_preview_page", 5, &parenthese).await;
    let mut curseur = session
        .execute(
            lecture(
                "SELECT * FROM \"public\".\"oxyn_preview_page\" \
                 WHERE (id > 0 AND seau < 2) OR nom IS NULL \
                 ORDER BY \"id\" ASC LIMIT 5",
            ),
            &jeton,
        )
        .await
        .expect("the same text, without wrapping");
    let mut sans_enveloppe = Vec::new();
    while let Some(lot) = curseur.next_batch().await.expect("stream") {
        let colonne = lot
            .column(0)
            .as_primitive_opt::<arrow::datatypes::Int64Type>()
            .expect("integer column");
        sans_enveloppe.extend(colonne.values().iter().copied());
    }
    assert_eq!(enveloppe, sans_enveloppe);
    assert_eq!(enveloppe, vec![1, 7, 8, 14, 15]);

    appliquer(&*session, "DROP TABLE oxyn_preview_page").await;
    appliquer(&*session, "DROP TABLE oxyn_preview_temoin").await;
    session.close().await.expect("close");
}
