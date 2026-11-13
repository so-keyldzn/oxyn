//! Whether a sibling connection can reproduce the catalog session's namespace.

use duckdb::Connection;
use oxyn_core::{OxynError, Result};

pub(crate) fn reproducible(connection: &Connection) -> Result<bool> {
    let result = crate::query::read(
        connection,
        "SELECT NOT EXISTS(SELECT 1 FROM duckdb_tables() WHERE temporary AND NOT internal \
         UNION ALL SELECT 1 FROM duckdb_views() WHERE temporary AND NOT internal \
         UNION ALL SELECT 1 FROM duckdb_variables())",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variable_without_temporary_objects_requires_the_original_session() {
        let connection = Connection::open_in_memory().expect("isolated engine");
        assert!(reproducible(&connection).expect("initial state"));
        connection
            .execute_batch("SET VARIABLE threshold = 400")
            .expect("local variable");
        assert!(!reproducible(&connection).expect("variable changes predicate results"));
        connection
            .execute_batch("RESET VARIABLE threshold")
            .expect("reset");
        assert!(reproducible(&connection).expect("reproducible again"));
    }
}
