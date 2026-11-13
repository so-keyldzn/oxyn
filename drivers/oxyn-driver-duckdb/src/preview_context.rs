//! Whether a sibling connection can reproduce the catalog session's namespace.

use duckdb::Connection;
use oxyn_core::{OxynError, Result};

pub(crate) fn reproducible(connection: &Connection) -> Result<bool> {
    let result = crate::query::read(
        connection,
        "SELECT NOT EXISTS(SELECT 1 FROM duckdb_tables() WHERE temporary AND NOT internal \
         UNION ALL SELECT 1 FROM duckdb_views() WHERE temporary AND NOT internal)",
        &[],
    )?;
    result
        .rows()
        .next()
        .and_then(|row| row.flag(0))
        .ok_or_else(|| {
            OxynError::CatalogUnavailable("DuckDB did not describe its temporary namespace".into())
        })
}
