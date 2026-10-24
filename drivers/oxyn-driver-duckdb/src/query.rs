//! The driver's own queries — catalog, probes — read through the same
//! conversion as the user's.
//!
//! `duckdb-rs`'s row interface decodes values through `Row::value_ref`, whose
//! match ends in `unreachable!` for an Arrow carrier it does not expect; a
//! `SET GLOBAL produce_arrow_string_view = true` typed in a console would make
//! every `VARCHAR` of every session such a carrier, and release builds abort
//! on a panic. These queries therefore never call it: they fetch chunks with
//! [`duckdb::Statement::step`], rebuild them as Arrow 59 ([`crate::convert`])
//! and read cells with Arrow's checked accessors, text cast to `Utf8` first.

use arrow::array::{Array, AsArray, RecordBatch};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Int64Type};
use duckdb::Connection;
use duckdb::types::Value;
use oxyn_core::{ErrorClass, Result};

use crate::convert;
use crate::error::{self, DuckDbError, Effect};
use crate::logical;

/// A small result, read in full: catalog listings and probes only, never a
/// user's statement.
#[derive(Debug, Default)]
pub(crate) struct Table {
    batches: Vec<RecordBatch>,
}

impl Table {
    /// Every row, as positions into the batches.
    pub(crate) fn rows(&self) -> impl Iterator<Item = Row<'_>> {
        self.batches
            .iter()
            .flat_map(|batch| (0..batch.num_rows()).map(move |index| Row { batch, index }))
    }

    /// The number of rows.
    pub(crate) fn len(&self) -> usize {
        self.batches.iter().map(RecordBatch::num_rows).sum()
    }
}

/// One row of a [`Table`].
#[derive(Clone, Copy)]
pub(crate) struct Row<'a> {
    batch: &'a RecordBatch,
    index: usize,
}

impl Row<'_> {
    /// The cell as text, `None` for `NULL`, an absent column or a value that
    /// does not cast to text.
    pub(crate) fn text(&self, column: usize) -> Option<String> {
        let array = self.batch.columns().get(column)?;
        if array.is_null(self.index) {
            return None;
        }
        let single = array.slice(self.index, 1);
        let text = cast(&single, &DataType::Utf8).ok()?;
        let strings = text.as_string_opt::<i32>()?;
        if strings.is_null(0) {
            return None;
        }
        Some(strings.value(0).to_owned())
    }

    /// The cell as a signed 64-bit integer, `None` for `NULL` or out of range.
    pub(crate) fn int(&self, column: usize) -> Option<i64> {
        let array = self.batch.columns().get(column)?;
        if array.is_null(self.index) {
            return None;
        }
        let single = array.slice(self.index, 1);
        let number = cast(&single, &DataType::Int64).ok()?;
        let numbers = number.as_primitive_opt::<Int64Type>()?;
        (!numbers.is_null(0)).then(|| numbers.value(0))
    }

    /// The cell as a boolean.
    pub(crate) fn flag(&self, column: usize) -> Option<bool> {
        let array = self.batch.columns().get(column)?;
        if array.is_null(self.index) {
            return None;
        }
        let single = array.slice(self.index, 1);
        let flag = cast(&single, &DataType::Boolean).ok()?;
        let flags = flag.as_boolean_opt()?;
        (!flags.is_null(0)).then(|| flags.value(0))
    }
}

/// The most rows one of the driver's queries reads: the catalog cache keeps
/// 50,000 metadata objects per connection (PERFORMANCE.md, memory budgets),
/// and a listing past it would be evicted as soon as read.
pub(crate) const MAX_ROWS: usize = 50_000;

/// The most bytes one of the driver's queries reads, definitions included.
pub(crate) const MAX_BYTES: usize = 64 * 1024 * 1024;

/// Runs one of the driver's read queries, values bound, and reads it whole —
/// up to [`MAX_ROWS`] and [`MAX_BYTES`]: a file of a million tables must fail
/// to expand, not exhaust the application's memory ([I-06](../../../CLAUDE.md#i-06)).
///
/// # Errors
/// The engine's, classified as a read, a conversion error, or
/// [`DuckDbError::MetadataTooLarge`] past the ceilings.
pub(crate) fn read(connection: &Connection, sql: &str, params: &[Value]) -> Result<Table> {
    read_within(connection, sql, params, MAX_ROWS, MAX_BYTES)
}

/// [`read`], with explicit ceilings.
fn read_within(
    connection: &Connection,
    sql: &str,
    params: &[Value],
    max_rows: usize,
    max_bytes: usize,
) -> Result<Table> {
    let engine = |err: duckdb::Error| error::engine(&err, Effect::ReadOnly);
    let mut statement = connection.prepare(sql).map_err(engine)?;
    if statement.parameter_count() != params.len() {
        return Err(error::driver(
            DuckDbError::ParameterCount {
                expected: statement.parameter_count(),
                given: params.len(),
            },
            ErrorClass::Permanent,
        ));
    }
    drop(
        statement
            .stream_arrow(duckdb::params_from_iter(params.iter()))
            .map_err(engine)?,
    );
    let source = statement.schema();
    let types: Vec<_> = (0..source.fields().len())
        .map(|index| logical::describe(&statement.column_logical_type(index)))
        .collect();
    let decode =
        |message: String| error::driver(DuckDbError::Decode(message), ErrorClass::Permanent);
    let schema = convert::target_schema(&source, &types).map_err(decode)?;
    let mut table = Table::default();
    let (mut rows, mut bytes) = (0_usize, 0_usize);
    while let Some(chunk) = statement.step().map_err(engine)? {
        let batch = convert::convert_chunk(&chunk, &schema, &types).map_err(decode)?;
        rows = rows.saturating_add(batch.num_rows());
        bytes = bytes.saturating_add(batch.get_array_memory_size());
        if rows > max_rows || bytes > max_bytes {
            // Dropping the statement ends the query; nothing read is kept.
            return Err(error::driver(
                DuckDbError::MetadataTooLarge {
                    rows: max_rows,
                    bytes: max_bytes,
                },
                ErrorClass::Permanent,
            ));
        }
        table.batches.push(batch);
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_past_the_ceiling_fails_instead_of_growing() {
        let connection = Connection::open_in_memory().expect("an in-memory engine");
        let within = read_within(
            &connection,
            "SELECT i FROM range(100) t(i)",
            &[],
            100,
            MAX_BYTES,
        )
        .expect("exactly the ceiling");
        assert_eq!(within.len(), 100);
        let past = read_within(
            &connection,
            "SELECT i FROM range(101) t(i)",
            &[],
            100,
            MAX_BYTES,
        )
        .expect_err("one row past the ceiling");
        assert_eq!(past.class(), ErrorClass::Permanent);
        assert!(past.to_string().contains("more than 100 objects"), "{past}");
        let heavy = read_within(
            &connection,
            "SELECT repeat('x', 4096) FROM range(64)",
            &[],
            MAX_ROWS,
            64 * 1024,
        )
        .expect_err("bytes past the ceiling");
        assert_eq!(heavy.class(), ErrorClass::Permanent);
    }
}
