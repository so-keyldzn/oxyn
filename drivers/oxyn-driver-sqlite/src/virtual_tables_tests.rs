//! Virtual tables as the listing reports them, against the engine the driver
//! embeds: a built-in module (`fts5`), and a module this connection does not
//! load — the case of a database written by a tool that loads `sqlite-vec`.

use oxyn_catalog::model::RelationRef;
use oxyn_catalog::path::CatalogPath;
use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits, ExecRequest,
    OxynError, PreviewShape, QueryLanguage,
};
use oxyn_driver::{Credentials, Driver as _, Session};

use crate::{MAIN, SqliteDriver};

async fn open(path: &str) -> Box<dyn Session> {
    let config = ConnectionConfig::new("virtual", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, path);
    SqliteDriver::new()
        .connect(&config, &Credentials::new(), &CancelToken::new())
        .await
        .unwrap_or_else(|err| panic!("open: {err}"))
}

async fn run(session: &dyn Session, sql: &str) -> Result<(), OxynError> {
    let request = ExecRequest::new(QueryLanguage::SQL, sql).with_limits(ExecLimits::unbounded());
    let mut cursor = session.execute(request, &CancelToken::new()).await?;
    while cursor.next_batch().await?.is_some() {}
    Ok(())
}

async fn relations(session: &dyn Session) -> Vec<RelationRef> {
    session
        .catalog()
        .list_relations(&CatalogPath::empty(), &CancelToken::new())
        .await
        .unwrap_or_else(|err| panic!("listing: {err}"))
}

fn named<'a>(listed: &'a [RelationRef], name: &str) -> &'a RelationRef {
    listed
        .iter()
        .find(|relation| relation.name() == name)
        .unwrap_or_else(|| panic!("`{name}` is listed"))
}

#[tokio::test]
async fn a_built_in_module_is_reported_available_with_its_shadow_tables() {
    let session = open(SqliteDriver::MEMORY).await;
    run(&*session, "CREATE VIRTUAL TABLE docs USING fts5(body)")
        .await
        .expect("fts5 is built in");
    run(&*session, "CREATE TABLE docs_archive (id INTEGER)")
        .await
        .expect("a plain table");

    let listed = relations(&*session).await;
    let docs = named(&listed, "docs");
    let module = docs.virtual_table.as_ref().expect("a virtual table");
    assert_eq!(module.module, "fts5");
    assert_eq!(module.available, Some(true));
    assert_eq!(docs.shadow_of, None);

    for shadow in [
        "docs_config",
        "docs_content",
        "docs_data",
        "docs_docsize",
        "docs_idx",
    ] {
        assert_eq!(
            named(&listed, shadow).shadow_of.as_deref(),
            Some("docs"),
            "{shadow}"
        );
    }
    let archive = named(&listed, "docs_archive");
    assert_eq!(
        archive.shadow_of, None,
        "with fts5 loaded, only the engine names shadow tables"
    );
    assert_eq!(archive.virtual_table, None);
    session.close().await.expect("close");
}

#[tokio::test]
async fn a_hostile_virtual_table_name_stays_data() {
    // I-10: the name and its shadow tables are compared, never joined to SQL.
    let session = open(SqliteDriver::MEMORY).await;
    run(
        &*session,
        r#"CREATE VIRTUAL TABLE "x""; DROP TABLE audit; --" USING fts5(body)"#,
    )
    .await
    .expect("a legal name");
    run(&*session, "CREATE TABLE audit (id INTEGER)")
        .await
        .expect("witness");

    let listed = relations(&*session).await;
    let hostile = r#"x"; DROP TABLE audit; --"#;
    assert_eq!(
        named(&listed, hostile)
            .virtual_table
            .as_ref()
            .map(|table| table.module.as_str()),
        Some("fts5")
    );
    assert_eq!(
        named(&listed, &format!("{hostile}_data"))
            .shadow_of
            .as_deref(),
        Some(hostile)
    );
    named(&listed, "audit");
    session.close().await.expect("close");
}

/// A database holding `CREATE VIRTUAL TABLE chunks_vec USING vec0(…)`, written
/// as a tool that loads `sqlite-vec` would leave it. The schema row is written
/// directly: the module needed to create it is precisely the one missing.
fn database_with_a_missing_module(path: &std::path::Path) {
    let connection = rusqlite::Connection::open(path).expect("temporary file");
    connection
        .execute_batch(
            "CREATE TABLE chunks_vec_info (key TEXT PRIMARY KEY, value ANY);
             CREATE TABLE chunks_vec_rowids (rowid INTEGER PRIMARY KEY, id);
             CREATE TABLE chunks_vec_vector_chunks00 (rowid INTEGER PRIMARY KEY, vectors BLOB);
             CREATE TABLE chunks (id INTEGER PRIMARY KEY, body TEXT);
             PRAGMA writable_schema = ON;
             INSERT INTO sqlite_schema (type, name, tbl_name, rootpage, sql) VALUES
               ('table', 'chunks_vec', 'chunks_vec', 0,
                'CREATE VIRTUAL TABLE chunks_vec USING vec0(embedding float[4])');
             PRAGMA writable_schema = OFF;",
        )
        .expect("schema written");
}

#[tokio::test]
async fn a_missing_module_is_reported_unavailable_and_its_tables_attributed() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("semantiq.db");
    database_with_a_missing_module(&path);
    let session = open(path.to_str().expect("utf-8 path")).await;

    let listed = relations(&*session).await;
    let vec = named(&listed, "chunks_vec");
    let module = vec.virtual_table.as_ref().expect("a virtual table");
    assert_eq!(module.module, "vec0");
    // Nothing names vec0 in the driver: once a build loads it, the module list
    // says so and this becomes `Some(true)`.
    assert_eq!(module.available, Some(false));

    for shadow in [
        "chunks_vec_info",
        "chunks_vec_rowids",
        "chunks_vec_vector_chunks00",
    ] {
        assert_eq!(
            named(&listed, shadow).shadow_of.as_deref(),
            Some("chunks_vec"),
            "{shadow}"
        );
    }
    assert_eq!(named(&listed, "chunks").shadow_of, None);

    // The read the Data tab makes: refused by the engine, and permanent —
    // never retried (I-13).
    let relation = CatalogPath::for_relation(None, Some(MAIN), "chunks_vec").expect("valid path");
    let request = session
        .preview_request(
            &relation,
            100,
            &PreviewShape::default(),
            &CancelToken::new(),
        )
        .await
        .expect("composing needs no module");
    let mut failure = None;
    match session.execute(request, &CancelToken::new()).await {
        Err(err) => failure = Some(err),
        Ok(mut cursor) => {
            if let Err(err) = cursor.next_batch().await {
                failure = Some(err);
            }
        }
    }
    let failure = failure.expect("reading without the module fails");
    assert_eq!(failure.class(), ErrorClass::Permanent);
    assert!(
        failure.to_string().contains("no such module: vec0"),
        "the engine's words are kept: {failure}"
    );
    assert_eq!(
        failure.missing_module(),
        Some("vec0"),
        "the module travels as data, not as text to parse"
    );

    // The structure read fails the same way, with the same data.
    let structure = session
        .catalog()
        .describe_relation(&relation, &CancelToken::new())
        .await;
    let Err(structure) = structure else {
        panic!("describing without the module fails");
    };
    assert_eq!(structure.missing_module(), Some("vec0"));

    // Any other failure on the same connection carries no module.
    let other = run(&*session, "SELECT * FROM no_such_table")
        .await
        .expect_err("a missing table fails");
    assert_eq!(other.missing_module(), None);
    session.close().await.expect("close");
}
