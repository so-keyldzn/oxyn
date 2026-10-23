//! What the crate's tests share: opening sessions on real files, running a
//! statement to its end.

use std::path::Path;

use arrow::record_batch::RecordBatch;
use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, Environment, ExecLimits, ExecRequest, OxynError,
    QueryLanguage, SqlDialect,
};
use oxyn_driver::{Credentials, Driver, Session};

use crate::{BatchLimits, DuckDbDriver};

/// A connection configuration on `path`, local, writable.
pub(crate) fn config(path: &str) -> ConnectionConfig {
    ConnectionConfig::new("duckdb-test", DriverId::duckdb())
        .with_environment(Environment::Local)
        .with_param(DuckDbDriver::PATH, path)
}

/// A session on a file of `dir`, created if needed.
pub(crate) async fn open_file(dir: &Path, name: &str) -> Box<dyn Session> {
    let path = dir.join(name);
    open(&config(path.to_str().expect("UTF-8 temporary path"))).await
}

/// A session on a private in-memory database: each configuration gets an
/// identity of its own, hence a database of its own.
pub(crate) async fn memory() -> Box<dyn Session> {
    open(&config(DuckDbDriver::MEMORY)).await
}

/// A session opened through the driver, as the executor gets it.
pub(crate) async fn open(connection: &ConnectionConfig) -> Box<dyn Session> {
    open_with(connection, BatchLimits::default()).await
}

/// A session with explicit batch bounds.
pub(crate) async fn open_with(
    connection: &ConnectionConfig,
    limits: BatchLimits,
) -> Box<dyn Session> {
    match DuckDbDriver::new()
        .with_batch_limits(limits)
        .connect(connection, &Credentials::new(), &CancelToken::new())
        .await
    {
        Ok(session) => session,
        Err(err) => panic!("open: {err}"),
    }
}

/// A request without bounds, as an export or a script runs.
pub(crate) fn unbounded(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::DuckDb), sql)
        .with_limits(ExecLimits::unbounded())
}

/// Runs a request to its end and returns its batches.
pub(crate) async fn run(
    session: &dyn Session,
    request: ExecRequest,
) -> Result<Vec<RecordBatch>, OxynError> {
    let mut cursor = session.execute(request, &CancelToken::new()).await?;
    let mut batches = Vec::new();
    while let Some(batch) = cursor.next_batch().await? {
        batches.push(batch);
    }
    Ok(batches)
}

/// Runs a statement to its end, panicking on failure.
pub(crate) async fn run_ok(session: &dyn Session, sql: &str) -> Vec<RecordBatch> {
    match run(session, unbounded(sql)).await {
        Ok(batches) => batches,
        Err(err) => panic!("`{sql}`: {err}"),
    }
}

/// The error of a request that had to fail.
///
/// `expect_err` would need `Debug` on `dyn Cursor`, which the cursor does not
/// give: it holds the session.
pub(crate) async fn run_err(session: &dyn Session, request: ExecRequest) -> OxynError {
    match run(session, request).await {
        Ok(_) => panic!("the request was expected to fail"),
        Err(err) => err,
    }
}

/// The only cell of a one-row, one-column result, as Arrow displays it.
pub(crate) fn single(batches: &[RecordBatch]) -> String {
    let batch = batches
        .iter()
        .find(|batch| batch.num_rows() > 0)
        .expect("a row");
    arrow::util::display::array_value_to_string(batch.column(0), 0).expect("displayable")
}

/// The total row count of batches.
pub(crate) fn rows(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}
