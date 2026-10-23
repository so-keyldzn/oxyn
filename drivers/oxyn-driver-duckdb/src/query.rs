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

/// Runs one of the driver's read queries, values bound, and reads it whole.
///
/// # Errors
/// The engine's, classified as a read, or a conversion error.
pub(crate) fn read(connection: &Connection, sql: &str, params: &[Value]) -> Result<Table> {
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
    while let Some(chunk) = statement.step().map_err(engine)? {
        table
            .batches
            .push(convert::convert_chunk(&chunk, &schema, &types).map_err(decode)?);
    }
    Ok(table)
}
