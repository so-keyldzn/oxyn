//! From Hrana values to Arrow columns.
//!
//! Turso is SQLite, so the same fact governs this module as the SQLite
//! driver's: **the type belongs to the value, not to the column.** A column
//! declared `INTEGER` can hold text. Arrow wants one type per column, known
//! before the first batch.
//!
//! What differs from the SQLite driver is the moment it can be decided. The
//! answer of Hrana over HTTP is whole before the first batch is built, so the
//! type of a column is chosen from **every** value of the result, not from the
//! first batch, and there is no "residual" value that arrives later and does
//! not fit.
//!
//! # How a column's type is decided
//!
//! | Storage classes seen | Arrow type | Why |
//! |---|---|---|
//! | none (empty or all `NULL`) | by the declared affinity, else `Utf8` | nothing else to go on |
//! | integers | `Int64` | exact |
//! | floats | `Float64` | exact |
//! | integers **and** floats | `Utf8` | no `f64` holds every `i64`: beyond 2⁵³ it corrupts |
//! | text, with or without numbers | `Utf8` | a number renders as text without loss |
//! | a blob, with anything | `Binary` | only bytes contain everything |
//!
//! A column whose type was inferred says so in its field metadata
//! ([`META_INFERRED`]), the type its table declared is kept
//! ([`META_DECLARED_TYPE`]), and a column that mixes classes names them
//! ([`META_STORAGE_CLASSES`]). A `DATETIME` is **not** converted to an Arrow
//! timestamp: SQLite stores text or a number there depending on what the
//! application wrote, and guessing would shift the data
//! ([DRIVER-CONTRACT §7](../../../docs/DRIVER-CONTRACT.md)).
//!
//! # Batches are bounded in bytes
//!
//! A batch closes at [`BATCH_BYTES`] of values or [`BATCH_ROWS`] rows,
//! whichever comes first: a thousand rows of one-megabyte blobs are a
//! gigabyte ([I-06](../../../CLAUDE.md#i-06)).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use arrow::array::{ArrayRef, BinaryBuilder, Float64Builder, Int64Builder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use oxyn_core::{OxynError, Result};

use crate::wire::{Cell, Col};

/// Field metadata: the column type was inferred from the values.
pub const META_INFERRED: &str = "oxyn.inferred";
/// Field metadata: the type the table declared (`INTEGER`, `VARCHAR(255)`…).
pub const META_DECLARED_TYPE: &str = "oxyn.turso.declared_type";
/// Field metadata: the storage classes found, `|`-separated, when they mix.
pub const META_STORAGE_CLASSES: &str = "oxyn.turso.storage_classes";

/// A batch closes at this many bytes of values.
pub const BATCH_BYTES: usize = 1024 * 1024;
/// A batch closes at this many rows, so narrow rows reach the grid early.
pub const BATCH_ROWS: usize = 8192;

const INTEGER: u8 = 1;
const FLOAT: u8 = 2;
const TEXT: u8 = 4;
const BLOB: u8 = 8;

/// The Arrow type a column gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Signed 64-bit integer.
    Int64,
    /// Double-precision float.
    Float64,
    /// UTF-8 text.
    Utf8,
    /// Opaque bytes.
    Binary,
}

impl Kind {
    fn data_type(self) -> DataType {
        match self {
            Self::Int64 => DataType::Int64,
            Self::Float64 => DataType::Float64,
            Self::Utf8 => DataType::Utf8,
            Self::Binary => DataType::Binary,
        }
    }

    /// The kind of a column that holds exactly the classes in `seen`.
    const fn of_classes(seen: u8) -> Option<Self> {
        match seen {
            0 => None,
            INTEGER => Some(Self::Int64),
            FLOAT => Some(Self::Float64),
            _ if seen & BLOB != 0 => Some(Self::Binary),
            _ => Some(Self::Utf8),
        }
    }

    /// The kind SQLite's affinity rules give a declared type, for a column
    /// with no value to look at.
    fn of_affinity(declared: &str) -> Self {
        let upper = declared.to_ascii_uppercase();
        if upper.contains("INT") {
            Self::Int64
        } else if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
            Self::Utf8
        } else if upper.contains("BLOB") {
            Self::Binary
        } else if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
            Self::Float64
        } else {
            Self::Utf8
        }
    }
}

fn class_of(cell: &Cell) -> u8 {
    match cell {
        Cell::Null => 0,
        Cell::Integer(_) => INTEGER,
        Cell::Float(_) => FLOAT,
        Cell::Text(_) => TEXT,
        Cell::Blob(_) => BLOB,
    }
}

fn class_names(seen: u8) -> String {
    [
        (INTEGER, "integer"),
        (FLOAT, "real"),
        (TEXT, "text"),
        (BLOB, "blob"),
    ]
    .iter()
    .filter(|(bit, _)| seen & bit != 0)
    .map(|(_, name)| *name)
    .collect::<Vec<_>>()
    .join("|")
}

/// The decided schema of a result, and the kind of each column.
#[derive(Debug, Clone)]
pub struct Plan {
    schema: SchemaRef,
    kinds: Vec<Kind>,
}

impl Plan {
    /// Decides the columns of a result from all its rows.
    ///
    /// # Errors
    /// [`OxynError::Query`] if a row does not have one value per column: the
    /// server's answer contradicts itself.
    pub(crate) fn of(cols: &[Col], rows: &[Vec<Cell>]) -> Result<Self> {
        let mut seen = vec![0_u8; cols.len()];
        for row in rows {
            if row.len() != cols.len() {
                return Err(OxynError::Query(
                    "the answer holds a row that does not have one value per column".to_owned(),
                ));
            }
            for (classes, cell) in seen.iter_mut().zip(row) {
                *classes |= class_of(cell);
            }
        }
        let mut fields = Vec::with_capacity(cols.len());
        let mut kinds = Vec::with_capacity(cols.len());
        for (index, (col, classes)) in cols.iter().zip(&seen).enumerate() {
            let declared = col.decltype.as_deref().unwrap_or_default();
            let kind = Kind::of_classes(*classes).unwrap_or_else(|| Kind::of_affinity(declared));
            let mut metadata = HashMap::from([(META_INFERRED.to_owned(), "true".to_owned())]);
            if !declared.trim().is_empty() {
                metadata.insert(META_DECLARED_TYPE.to_owned(), declared.to_owned());
            }
            if classes.count_ones() > 1 {
                metadata.insert(META_STORAGE_CLASSES.to_owned(), class_names(*classes));
            }
            let name = col
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("column_{}", index + 1));
            fields.push(Field::new(name, kind.data_type(), true).with_metadata(metadata));
            kinds.push(kind);
        }
        Ok(Self {
            schema: Arc::new(Schema::new(fields)),
            kinds,
        })
    }

    /// The Arrow schema of every batch.
    #[must_use]
    pub fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// The weight of a value, as the batch counts it.
fn weight(cell: &Cell) -> usize {
    match cell {
        Cell::Null => 1,
        Cell::Integer(_) | Cell::Float(_) => 8,
        Cell::Text(text) => text.len(),
        Cell::Blob(bytes) => bytes.len(),
    }
}

/// The rows of a result, handed out as batches closed by size.
#[derive(Debug)]
pub struct Batches {
    plan: Plan,
    rows: std::vec::IntoIter<Vec<Cell>>,
}

impl Batches {
    /// Rows to hand out under `plan`, which decided their columns.
    #[must_use]
    pub(crate) fn new(plan: Plan, rows: Vec<Vec<Cell>>) -> Self {
        // A result without columns (an `INSERT`) has no batch to give.
        let rows = if plan.kinds.is_empty() {
            Vec::new()
        } else {
            rows
        };
        Self {
            plan,
            rows: rows.into_iter(),
        }
    }

    /// The next batch, or `None` once every row was handed out.
    ///
    /// # Errors
    /// [`OxynError::Internal`] if a value does not fit the column the plan
    /// chose for it — a programming error, since the plan read every value.
    pub fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        let mut taken: Vec<Vec<Cell>> = Vec::new();
        let mut bytes = 0_usize;
        while taken.len() < BATCH_ROWS && bytes < BATCH_BYTES {
            let Some(row) = self.rows.next() else { break };
            bytes = bytes.saturating_add(row.iter().map(weight).sum::<usize>());
            taken.push(row);
        }
        if taken.is_empty() {
            return Ok(None);
        }
        self.plan.build(&taken).map(Some)
    }
}

impl Plan {
    fn build(&self, rows: &[Vec<Cell>]) -> Result<RecordBatch> {
        let mut columns: Vec<ArrayRef> = Vec::with_capacity(self.kinds.len());
        for (index, kind) in self.kinds.iter().enumerate() {
            columns.push(column(*kind, rows.iter().map(|row| row.get(index)))?);
        }
        RecordBatch::try_new_with_options(
            self.schema(),
            columns,
            &RecordBatchOptions::new().with_row_count(Some(rows.len())),
        )
        .map_err(|_| OxynError::Internal("the columns of a batch disagree".to_owned()))
    }
}

fn misfit() -> OxynError {
    OxynError::Internal("a value does not fit the column the plan chose for it".to_owned())
}

fn column<'a>(kind: Kind, cells: impl Iterator<Item = Option<&'a Cell>>) -> Result<ArrayRef> {
    let mut scratch = String::new();
    Ok(match kind {
        Kind::Int64 => {
            let mut builder = Int64Builder::new();
            for cell in cells {
                match cell {
                    Some(Cell::Integer(value)) => builder.append_value(*value),
                    Some(Cell::Null) => builder.append_null(),
                    _ => return Err(misfit()),
                }
            }
            Arc::new(builder.finish())
        }
        Kind::Float64 => {
            let mut builder = Float64Builder::new();
            for cell in cells {
                match cell {
                    Some(Cell::Float(value)) => builder.append_value(*value),
                    Some(Cell::Null) => builder.append_null(),
                    _ => return Err(misfit()),
                }
            }
            Arc::new(builder.finish())
        }
        Kind::Utf8 => {
            let mut builder = StringBuilder::new();
            for cell in cells {
                match cell {
                    Some(Cell::Text(text)) => builder.append_value(text),
                    Some(Cell::Integer(value)) => {
                        scratch.clear();
                        let _ = write!(scratch, "{value}");
                        builder.append_value(&scratch);
                    }
                    Some(Cell::Float(value)) => {
                        scratch.clear();
                        let _ = write!(scratch, "{value}");
                        builder.append_value(&scratch);
                    }
                    Some(Cell::Null) => builder.append_null(),
                    _ => return Err(misfit()),
                }
            }
            Arc::new(builder.finish())
        }
        Kind::Binary => {
            let mut builder = BinaryBuilder::new();
            for cell in cells {
                match cell {
                    Some(Cell::Blob(bytes)) => builder.append_value(bytes),
                    Some(Cell::Text(text)) => builder.append_value(text.as_bytes()),
                    Some(Cell::Integer(value)) => builder.append_value(value.to_string()),
                    Some(Cell::Float(value)) => builder.append_value(value.to_string()),
                    Some(Cell::Null) => builder.append_null(),
                    None => return Err(misfit()),
                }
            }
            Arc::new(builder.finish())
        }
    })
}

#[cfg(test)]
mod tests {
    use arrow::array::{Array, AsArray as _};
    use arrow::datatypes::{Float64Type, Int64Type};

    use super::*;

    fn col(name: &str, decltype: Option<&str>) -> Col {
        Col {
            name: Some(name.to_owned()),
            decltype: decltype.map(str::to_owned),
        }
    }

    fn one_column(rows: Vec<Cell>, decltype: Option<&str>) -> (Plan, RecordBatch) {
        let rows: Vec<Vec<Cell>> = rows.into_iter().map(|cell| vec![cell]).collect();
        let plan = Plan::of(&[col("v", decltype)], &rows).expect("plan");
        let batch = Batches::new(plan.clone(), rows)
            .next_batch()
            .expect("batch")
            .expect("rows");
        (plan, batch)
    }

    #[test]
    fn a_column_of_integers_stays_exact() {
        let (plan, batch) = one_column(
            vec![Cell::Integer(i64::MAX), Cell::Null, Cell::Integer(-3)],
            Some("INTEGER"),
        );
        assert_eq!(plan.schema().field(0).data_type(), &DataType::Int64);
        let values = batch.column(0).as_primitive::<Int64Type>();
        assert_eq!(values.value(0), i64::MAX);
        assert!(values.is_null(1));
    }

    #[test]
    fn a_column_of_floats_stays_a_float() {
        let (plan, batch) = one_column(vec![Cell::Float(0.1), Cell::Float(-2.5)], Some("REAL"));
        assert_eq!(plan.schema().field(0).data_type(), &DataType::Float64);
        assert_eq!(batch.column(0).as_primitive::<Float64Type>().value(0), 0.1);
    }

    #[test]
    fn integers_and_floats_together_become_text_and_lose_nothing() {
        // 2^53 + 1 does not exist as an f64: a float column would show 2^53.
        let big = (1_i64 << 53) + 1;
        let (plan, batch) = one_column(
            vec![Cell::Integer(big), Cell::Float(10.5), Cell::Integer(10)],
            Some("NUMERIC(10,2)"),
        );
        let field = plan.schema().field(0).clone();
        assert_eq!(field.data_type(), &DataType::Utf8);
        assert_eq!(
            field
                .metadata()
                .get(META_STORAGE_CLASSES)
                .map(String::as_str),
            Some("integer|real")
        );
        assert_eq!(
            field.metadata().get(META_DECLARED_TYPE).map(String::as_str),
            Some("NUMERIC(10,2)")
        );
        let text = batch.column(0).as_string::<i32>();
        assert_eq!(text.value(0), "9007199254740993");
        assert_eq!(text.value(1), "10.5");
        assert_eq!(text.value(2), "10");
    }

    #[test]
    fn a_column_declared_integer_that_holds_text_is_text() {
        let (plan, batch) = one_column(
            vec![Cell::Integer(1), Cell::Text("abc".to_owned())],
            Some("INTEGER"),
        );
        assert_eq!(plan.schema().field(0).data_type(), &DataType::Utf8);
        assert_eq!(batch.column(0).as_string::<i32>().value(1), "abc");
    }

    #[test]
    fn a_blob_with_anything_is_bytes() {
        let (plan, batch) = one_column(
            vec![
                Cell::Blob(vec![0, 255]),
                Cell::Text("é".to_owned()),
                Cell::Integer(7),
            ],
            None,
        );
        assert_eq!(plan.schema().field(0).data_type(), &DataType::Binary);
        let bytes = batch.column(0).as_binary::<i32>();
        assert_eq!(bytes.value(0), [0, 255]);
        assert_eq!(bytes.value(1), "é".as_bytes());
        assert_eq!(bytes.value(2), b"7");
    }

    #[test]
    fn a_column_without_a_value_follows_its_declared_affinity() {
        for (declared, expected) in [
            (Some("INTEGER"), DataType::Int64),
            (Some("BIGINT"), DataType::Int64),
            (Some("VARCHAR(20)"), DataType::Utf8),
            (Some("DOUBLE"), DataType::Float64),
            (Some("BLOB"), DataType::Binary),
            (Some("DATETIME"), DataType::Utf8),
            (None, DataType::Utf8),
        ] {
            let plan = Plan::of(&[col("v", declared)], &[vec![Cell::Null]]).expect("plan");
            assert_eq!(
                plan.schema().field(0).data_type(),
                &expected,
                "{declared:?}"
            );
        }
    }

    #[test]
    fn a_datetime_is_not_given_a_time_zone_nor_converted() {
        let (plan, batch) = one_column(
            vec![Cell::Text("2024-01-15 10:00:00".to_owned())],
            Some("DATETIME"),
        );
        assert_eq!(plan.schema().field(0).data_type(), &DataType::Utf8);
        assert_eq!(
            batch.column(0).as_string::<i32>().value(0),
            "2024-01-15 10:00:00"
        );
    }

    #[test]
    fn the_columns_keep_their_names_and_an_unnamed_one_gets_a_position() {
        let cols = [
            col("a", None),
            Col {
                name: None,
                decltype: None,
            },
            col("a", None),
        ];
        let plan = Plan::of(&cols, &[]).expect("plan");
        let schema = plan.schema();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, ["a", "column_2", "a"]);
    }

    #[test]
    fn a_row_with_the_wrong_number_of_values_is_refused() {
        let error =
            Plan::of(&[col("a", None), col("b", None)], &[vec![Cell::Null]]).expect_err("ragged");
        assert!(matches!(error, OxynError::Query(_)), "{error:?}");
    }

    #[test]
    fn a_statement_without_columns_has_no_batch() {
        let plan = Plan::of(&[], &[]).expect("plan");
        assert_eq!(plan.schema().fields().len(), 0);
        assert!(
            Batches::new(plan, vec![])
                .next_batch()
                .expect("ok")
                .is_none()
        );
    }

    #[test]
    fn batches_close_by_bytes_not_by_rows() {
        // Five rows of 600 KiB: a row-counted batch would hold all of them.
        let blob = vec![7_u8; 600 * 1024];
        let rows: Vec<Vec<Cell>> = (0..5).map(|_| vec![Cell::Blob(blob.clone())]).collect();
        let plan = Plan::of(&[col("b", Some("BLOB"))], &rows).expect("plan");
        let mut batches = Batches::new(plan, rows);
        let mut sizes = Vec::new();
        while let Some(batch) = batches.next_batch().expect("batch") {
            sizes.push(batch.num_rows());
        }
        assert_eq!(sizes, [2, 2, 1]);
    }

    #[test]
    fn narrow_rows_close_by_count() {
        let rows: Vec<Vec<Cell>> = (0..BATCH_ROWS as i64 + 5)
            .map(|n| vec![Cell::Integer(n)])
            .collect();
        let plan = Plan::of(&[col("n", None)], &rows).expect("plan");
        let mut batches = Batches::new(plan, rows);
        let first = batches.next_batch().expect("batch").expect("rows");
        let second = batches.next_batch().expect("batch").expect("rows");
        assert_eq!((first.num_rows(), second.num_rows()), (BATCH_ROWS, 5));
        assert!(batches.next_batch().expect("ok").is_none());
    }
}
