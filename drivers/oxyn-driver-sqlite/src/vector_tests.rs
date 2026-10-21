//! `vec0` tables through the driver's ordinary paths
//! ([ADR-0054](../../../docs/adr/0054-bundle-sqlite-vec-in-the-sqlite-driver.md)).
//!
//! What is proven: the module exists only on a connection where the user
//! turned sqlite-vec on, and there on every session, read-only ones included;
//! without the switch, a `vec0` table is `no such module: vec0`; a vector comes out as the bytes SQLite stores, in a `Binary`
//! column; a KNN query is a read for the engine; and a malformed vector is an
//! error, never a panic.

use arrow::array::{Array, BinaryArray, Int64Array, StringArray};
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use oxyn_catalog::path::CatalogPath;
use std::time::Duration;

use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits, ExecRequest,
    OxynError, PreviewShape, QueryLanguage,
};
use oxyn_driver::{Credentials, Driver as _, Session};

use crate::{MAIN, SqliteDriver};

/// The table a vector tool writes: `semantiq` names it `chunks_vec`.
const TABLE: &str = "CREATE VIRTUAL TABLE chunks_vec USING vec0(embedding float[4])";
const ROWS: &str = "INSERT INTO chunks_vec(rowid, embedding) VALUES \
     (1, '[1.0, 0.0, 0.0, 0.0]'), (2, '[0.0, 1.0, 0.0, 0.0]'), (3, '[0.9, 0.1, 0.0, 0.0]')";

async fn connect(config: &ConnectionConfig) -> Box<dyn Session> {
    SqliteDriver::new()
        .connect(config, &Credentials::new(), &CancelToken::new())
        .await
        .expect("the database opens")
}

/// A connection to `path` with the sqlite-vec switch set to `switch`, or
/// without the parameter at all — a workspace saved before it existed.
fn config(path: &str, switch: Option<&str>) -> ConnectionConfig {
    let config = ConnectionConfig::new("vectors", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, path);
    match switch {
        Some(value) => config.with_param(SqliteDriver::VECTOR_EXTENSION, value),
        None => config,
    }
}

/// An in-memory database with sqlite-vec turned on.
async fn in_memory() -> Box<dyn Session> {
    connect(&config(SqliteDriver::MEMORY, Some("true"))).await
}

/// The modules among `vec0` and `vec_each` that the session sees.
async fn vector_modules(session: &dyn Session) -> Vec<String> {
    let batches = run(
        session,
        read(
            "SELECT name FROM pragma_module_list WHERE name IN ('vec0', 'vec_each') ORDER BY name",
        ),
    )
    .await
    .expect("the module list is readable");
    batches
        .iter()
        .flat_map(|batch| {
            let names = column::<StringArray>(batch, 0);
            (0..names.len()).map(|row| names.value(row).to_owned())
        })
        .collect()
}

/// Runs `request` to the end; `Err` with the driver's error when it refuses.
async fn run(session: &dyn Session, request: ExecRequest) -> Result<Vec<RecordBatch>, OxynError> {
    let mut cursor = session.execute(request, &CancelToken::new()).await?;
    let mut batches = Vec::new();
    while let Some(batch) = cursor.next_batch().await? {
        batches.push(batch);
    }
    Ok(batches)
}

/// The error of a request that must be refused.
async fn refusal(session: &dyn Session, request: ExecRequest) -> OxynError {
    match run(session, request).await {
        Ok(_) => panic!("the request must be refused"),
        Err(error) => error,
    }
}

/// The number of rows of `chunks_vec`.
async fn count(session: &dyn Session) -> i64 {
    let batches = run(session, read("SELECT count(*) FROM chunks_vec"))
        .await
        .expect("counting the rows");
    column::<Int64Array>(&batches[0], 0).value(0)
}

fn write(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::SQL, sql).with_limits(ExecLimits::unbounded())
}

/// The default limits: read-only, refused by the engine if the statement writes.
fn read(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::SQL, sql)
}

async fn apply(session: &dyn Session, sql: &str) {
    if let Err(error) = run(session, write(sql)).await {
        panic!("`{sql}` must be accepted: {error}");
    }
}

fn column<T: 'static>(batch: &RecordBatch, index: usize) -> &T {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<T>()
        .unwrap_or_else(|| panic!("column {index}: {:?}", batch.column(index).data_type()))
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    let (words, rest) = bytes.as_chunks::<4>();
    assert!(rest.is_empty(), "a float vector is a whole number of f32");
    words.iter().map(|word| f32::from_le_bytes(*word)).collect()
}

#[tokio::test]
async fn the_module_list_names_vec0_when_turned_on() {
    let session = in_memory().await;
    assert_eq!(vector_modules(session.as_ref()).await, ["vec0", "vec_each"]);
}

#[tokio::test]
async fn sqlite_vec_stays_off_unless_turned_on() {
    // Absent — a workspace saved before the switch existed — or `false`.
    for switch in [None, Some("false")] {
        let session = connect(&config(SqliteDriver::MEMORY, switch)).await;
        assert!(
            vector_modules(session.as_ref()).await.is_empty(),
            "{switch:?}: no sqlite-vec module without the user's choice"
        );
        let refused = refusal(session.as_ref(), read("SELECT vec_version()")).await;
        assert_eq!(refused.class(), ErrorClass::Permanent, "{refused}");
    }
}

#[tokio::test]
async fn a_vec0_table_on_a_connection_without_the_switch_says_no_such_module() {
    // The file was written with sqlite-vec on, then opened by a connection
    // that did not turn it on: the C code is never reached, and the engine
    // names the missing module.
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("semantiq.sqlite");
    let path = path.to_str().expect("UTF-8 path");
    let writer = connect(&config(path, Some("true"))).await;
    apply(writer.as_ref(), TABLE).await;
    apply(writer.as_ref(), ROWS).await;
    writer.close().await.expect("close the writer");

    let reader = connect(&config(path, None)).await;
    let refused = refusal(reader.as_ref(), read("SELECT rowid FROM chunks_vec")).await;
    assert_eq!(refused.class(), ErrorClass::Permanent, "{refused}");
    assert!(
        refused.to_string().contains("no such module: vec0"),
        "{refused}"
    );
    reader.close().await.expect("close the reader");
}

#[tokio::test]
async fn a_switch_value_other_than_true_or_false_is_refused() {
    let issue = SqliteDriver::new()
        .connect(
            &config(SqliteDriver::MEMORY, Some("yes")),
            &Credentials::new(),
            &CancelToken::new(),
        )
        .await;
    match issue {
        Ok(_) => panic!("`yes` is neither on nor off"),
        Err(err) => assert!(matches!(err, OxynError::Config(_)), "{err}"),
    }
}

#[tokio::test]
async fn a_vector_comes_back_as_its_stored_bytes() {
    let session = in_memory().await;
    apply(session.as_ref(), TABLE).await;
    apply(session.as_ref(), ROWS).await;

    let batches = run(
        session.as_ref(),
        read("SELECT rowid, embedding, vec_to_json(embedding) FROM chunks_vec ORDER BY rowid"),
    )
    .await
    .expect("reading a vec0 table");
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 3);
    assert_eq!(batch.column(1).data_type(), &DataType::Binary);
    let vectors = column::<BinaryArray>(batch, 1);
    // float[4]: four little-endian f32, the format sqlite-vec stores.
    assert_eq!(vectors.value(0).len(), 16);
    assert_eq!(floats(vectors.value(1)), vec![0.0, 1.0, 0.0, 0.0]);
    // Readable text is the user's choice, through the extension's own function.
    assert_eq!(
        column::<StringArray>(batch, 2).value(0),
        "[1.000000,0.000000,0.000000,0.000000]"
    );
}

#[tokio::test]
async fn a_knn_query_is_a_read() {
    let session = in_memory().await;
    apply(session.as_ref(), TABLE).await;
    apply(session.as_ref(), ROWS).await;

    // Default limits: the engine itself would refuse a statement that writes.
    let batches = run(
        session.as_ref(),
        read(
            "SELECT rowid, distance FROM chunks_vec \
             WHERE embedding MATCH '[1.0, 0.0, 0.0, 0.0]' AND k = 2 ORDER BY distance",
        ),
    )
    .await
    .expect("a KNN query is accepted as a read");
    let rowids = column::<Int64Array>(&batches[0], 0);
    assert_eq!(rowids.values().to_vec(), vec![1, 3]);

    // A write into vec0 stays a write: the read-only gate refuses it before
    // the engine runs it.
    let refused = refusal(
        session.as_ref(),
        read("INSERT INTO chunks_vec(rowid, embedding) VALUES (9, '[0, 0, 0, 1]')"),
    )
    .await;
    assert!(
        matches!(refused, OxynError::PolicyDenied { .. }),
        "{refused}"
    );
    assert_eq!(count(session.as_ref()).await, 3);
}

#[tokio::test]
async fn a_malformed_vector_is_an_error_not_a_panic() {
    let session = in_memory().await;
    apply(session.as_ref(), TABLE).await;
    apply(session.as_ref(), ROWS).await;

    for sql in [
        // A blob whose length is no multiple of four bytes.
        "INSERT INTO chunks_vec(rowid, embedding) VALUES (10, x'010203')",
        // The wrong dimension.
        "INSERT INTO chunks_vec(rowid, embedding) VALUES (11, '[1.0, 2.0]')",
        // Neither JSON nor a vector.
        "INSERT INTO chunks_vec(rowid, embedding) VALUES (12, 'not a vector')",
        "SELECT vec_to_json(x'010203')",
        "SELECT rowid FROM chunks_vec WHERE embedding MATCH x'00' AND k = 1",
    ] {
        let refused = refusal(session.as_ref(), write(sql)).await;
        assert_eq!(refused.class(), ErrorClass::Permanent, "`{sql}`: {refused}");
    }
    assert_eq!(
        count(session.as_ref()).await,
        3,
        "nothing malformed was kept"
    );
}

#[tokio::test]
async fn a_corrupted_shadow_table_is_an_error_not_a_crash() {
    // The threat is the opened file, not the typed SQL: a database whose
    // shadow table holds a truncated chunk.
    let session = in_memory().await;
    apply(session.as_ref(), TABLE).await;
    apply(session.as_ref(), ROWS).await;
    apply(
        session.as_ref(),
        "UPDATE chunks_vec_vector_chunks00 SET vectors = x'0102'",
    )
    .await;

    for sql in [
        "SELECT rowid, embedding FROM chunks_vec",
        "SELECT rowid FROM chunks_vec WHERE embedding MATCH '[1, 0, 0, 0]' AND k = 2",
    ] {
        assert!(
            run(session.as_ref(), read(sql)).await.is_err(),
            "`{sql}` over a truncated chunk must be refused"
        );
    }
}

#[tokio::test]
async fn a_read_only_session_previews_a_vec0_table() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("semantiq.sqlite");
    let mut config = config(path.to_str().expect("UTF-8 path"), Some("true"));
    let writer = connect(&config).await;
    apply(writer.as_ref(), TABLE).await;
    apply(writer.as_ref(), ROWS).await;
    writer.close().await.expect("close the writer");

    config.read_only = true;
    let reader = connect(&config).await;
    let relation = CatalogPath::for_relation(None, Some(MAIN), "chunks_vec").expect("path");
    let cancel = CancelToken::new();
    let request = reader
        .preview_request(&relation, 100, &PreviewShape::default(), &cancel)
        .await
        .expect("the preview of a vec0 table is composed");
    let batches = run(reader.as_ref(), request)
        .await
        .expect("the preview of a vec0 table runs");
    let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    assert_eq!(rows, 3);

    let nearest = run(
        reader.as_ref(),
        read("SELECT rowid FROM chunks_vec WHERE embedding MATCH '[1, 0, 0, 0]' AND k = 1"),
    )
    .await
    .expect("a KNN query on a read-only session");
    assert_eq!(column::<Int64Array>(&nearest[0], 0).value(0), 1);
    // Even with unbounded limits: a read-only session forces read-only
    // requests, and the gate refuses the write before the engine sees it.
    let refused = refusal(reader.as_ref(), write("DELETE FROM chunks_vec")).await;
    assert!(
        matches!(refused, OxynError::PolicyDenied { .. }),
        "{refused}"
    );
    assert_eq!(count(reader.as_ref()).await, 3);
    reader.close().await.expect("close the reader");
}

/// `vec0_metadata_filter_text` only `assert`s the size of the `rowids` blob,
/// and the assert stays active in release: reaching it would abort the
/// process. The KNN scan checks the same blob first and refuses it; this test
/// keeps that order honest across a bump.
#[tokio::test]
async fn a_corrupted_rowids_blob_under_a_text_filter_is_an_error_not_an_abort() {
    let session = in_memory().await;
    apply(
        session.as_ref(),
        "CREATE VIRTUAL TABLE m USING vec0(embedding float[2], label text)",
    )
    .await;
    apply(
        session.as_ref(),
        "INSERT INTO m(rowid, embedding, label) VALUES (1, '[1,0]', 'a'), (2, '[0,1]', 'b')",
    )
    .await;
    apply(session.as_ref(), "UPDATE m_chunks SET rowids = x'0102'").await;
    for label in ["a", "a label longer than twelve bytes"] {
        let sql = format!(
            "SELECT rowid FROM m WHERE embedding MATCH '[1,0]' AND k = 2 AND label = '{label}'"
        );
        assert!(
            run(session.as_ref(), read(&sql)).await.is_err(),
            "`{sql}` over a truncated rowids blob must be refused"
        );
    }
}

/// sqlite-vec turns the `SQLITE_INTERRUPT` of its inner statements into a plain
/// `SQLITE_ERROR` ("Could not find latest chunk"): the result code alone would
/// call a stopped write a refusal. What it applied is unknown — ambiguous, and
/// never retried ([I-13](../../../CLAUDE.md#i-13)).
#[tokio::test]
async fn a_stopped_write_into_vec0_is_ambiguous() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("vectors.sqlite");
    let journal = dir.path().join("vectors.sqlite-journal");
    let session = connect(&config(path.to_str().expect("UTF-8 path"), Some("true"))).await;
    apply(session.as_ref(), TABLE).await;

    let cancel = CancelToken::new();
    let endless = write(
        "INSERT INTO chunks_vec(rowid, embedding) SELECT x, '[1.0, 0.0, 0.0, 0.0]' FROM \
         (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c)",
    );
    let mut execution = Box::pin(session.execute(endless, &cancel));
    assert!(futures::poll!(execution.as_mut()).is_pending());
    // The journal appears on the first page written: the INSERT is then inside
    // the extension's own statements. A condition, not a delay.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !journal.exists() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the endless INSERT starts writing");
    // Past the first chunk, so that the stop lands in sqlite-vec's statements.
    tokio::time::sleep(Duration::from_millis(50)).await;

    cancel.cancel();
    match execution.await {
        Ok(_) => panic!("an endless INSERT only ends interrupted"),
        Err(err) => assert_eq!(err.class(), ErrorClass::Ambiguous, "{err}"),
    }
    session.close().await.expect("close");
}
