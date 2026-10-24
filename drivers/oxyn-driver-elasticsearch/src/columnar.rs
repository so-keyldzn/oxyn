//! The tabular answers: ES|QL, Elasticsearch `_sql`, OpenSearch SQL.
//!
//! These answers declare their columns and a type for each, so the grid gets
//! typed Arrow columns rather than text:
//!
//! | Server type | Arrow | Loss |
//! |---|---|---|
//! | `boolean` | `Boolean` | none |
//! | `byte`, `short`, `integer` | `Int32` | none |
//! | `long` | `Int64` | none |
//! | `unsigned_long` | `UInt64` | none: up to 2^64 − 1, never through `f64` |
//! | `double`, `float`, `half_float`, `scaled_float` | `Float64` | none: the server computes them as doubles |
//! | `date`, `datetime` with an offset in the value | `Timestamp(ms, UTC)` | none: the value names its instant |
//! | `date_nanos` with an offset in the value | `Timestamp(ns, UTC)` | none |
//! | anything else — `keyword`, `text`, `ip`, `version`, geo types, `unsupported` | `Utf8`, the value's JSON text | none |
//!
//! A column keeps its server type in the field metadata (`elasticsearch.type`).
//! When a value does not fit the column's type — a multi-valued field, which
//! ES|QL returns as an array; a timestamp without an offset, whose zone the
//! driver would have to invent — the **whole column** is its JSON text: a
//! column typed by its first rows would have to drop the others.
//!
//! The endpoint names the API, never the keys of its answer: an `_sql` answer
//! asked `columnar` holds `columns` and `values`, like ES|QL, yet carries a
//! cursor ES|QL does not have. With `"columnar": true`, `values` holds one
//! array per column; it is read in that orientation, and an `_sql` cursor
//! asks for it again on every later page, since the server does not remember
//! it.
//!
//! Every page is bounded before its first Arrow allocation: at most
//! [`MAX_COLUMNS`] columns, and at most [`DECODED_LIMIT`] of Arrow buffers
//! for the whole page — each cell counted at its Arrow cost, eight bytes for
//! a `long` whatever its JSON length, a text cell's bytes and its offset, a
//! validity bit for every cell — whatever the number of batches it is cut
//! into.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arrow::array::{
    ArrayRef, BooleanBuilder, Float64Builder, Int32Builder, Int64Builder, StringBuilder,
    TimestampMillisecondBuilder, TimestampNanosecondBuilder, UInt64Builder,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use arrow::record_batch::RecordBatch;
use indexmap::IndexMap;
use oxyn_core::{OxynError, Result};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::cells::{BATCH_BYTES, DECODED_LIMIT, MAX_COLUMNS, META_TYPE, cell_text};
use crate::decode::{Decoded, PageState};

/// A column as the answer declares it.
#[derive(Deserialize)]
struct Declared {
    name: String,
    #[serde(rename = "type")]
    raw_type: String,
}

/// How a column's values become Arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Boolean,
    Int32,
    Int64,
    UInt64,
    Float64,
    Millis,
    Nanos,
    Text,
}

impl Kind {
    fn of(raw_type: &str) -> Self {
        match raw_type {
            "boolean" => Self::Boolean,
            "byte" | "short" | "integer" => Self::Int32,
            "long" => Self::Int64,
            "unsigned_long" => Self::UInt64,
            "double" | "float" | "half_float" | "scaled_float" => Self::Float64,
            "date" | "datetime" => Self::Millis,
            "date_nanos" => Self::Nanos,
            _ => Self::Text,
        }
    }

    fn data_type(self) -> DataType {
        match self {
            Self::Boolean => DataType::Boolean,
            Self::Int32 => DataType::Int32,
            Self::Int64 => DataType::Int64,
            Self::UInt64 => DataType::UInt64,
            Self::Float64 => DataType::Float64,
            Self::Millis => DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            Self::Nanos => DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
            Self::Text => DataType::Utf8,
        }
    }

    /// Whether `value` fits this kind; a JSON `null` fits every kind.
    fn fits(self, value: &RawValue) -> bool {
        let text = value.get();
        text == "null"
            || match self {
                Self::Boolean => matches!(text, "true" | "false"),
                Self::Int32 => text.parse::<i32>().is_ok(),
                Self::Int64 => text.parse::<i64>().is_ok(),
                Self::UInt64 => text.parse::<u64>().is_ok(),
                Self::Float64 => text.parse::<f64>().is_ok_and(f64::is_finite),
                Self::Millis => instant(value).is_some(),
                // Beyond 1677–2262 an instant has no 64-bit nanosecond count.
                Self::Nanos => instant(value).is_some_and(|at| at.timestamp_nanos_opt().is_some()),
                Self::Text => true,
            }
    }

    /// What a cell of this kind costs in Arrow buffers, in bits: its slot —
    /// a bit for a boolean, the width of a number or an instant, a text's
    /// bytes and its `i32` offset — and its validity bit. A JSON string's
    /// text is counted at its quoted length less the quotes, which no
    /// unescaping exceeds.
    fn cell_bits(self, value: &RawValue) -> usize {
        let slot = match self {
            Self::Boolean => 1,
            Self::Int32 => 32,
            Self::Int64 | Self::UInt64 | Self::Float64 | Self::Millis | Self::Nanos => 64,
            Self::Text => 32_usize.saturating_add(text_bytes(value).saturating_mul(8)),
        };
        slot.saturating_add(1)
    }
}

/// The bytes a text cell's value takes, at most.
fn text_bytes(value: &RawValue) -> usize {
    let text = value.get();
    match text.as_bytes().first() {
        Some(b'"') => text.len().saturating_sub(2),
        Some(b'n') if text == "null" => 0,
        _ => text.len(),
    }
}

/// What a batch's buffers cost beyond their cells, per column: each of its
/// validity, offset and data buffers rounded up to Arrow's 64-byte padding,
/// and the offset a text column has past its last row.
const COLUMN_OVERHEAD: usize = 3 * 64 + 4;

/// The instant a timestamp value names, when it carries its offset.
fn instant(value: &RawValue) -> Option<chrono::DateTime<chrono::Utc>> {
    let text: String = serde_json::from_str(value.get()).ok()?;
    chrono::DateTime::parse_from_rfc3339(&text)
        .ok()
        .map(|instant| instant.with_timezone(&chrono::Utc))
}

/// The columns of a tabular answer, kept for its later pages.
#[derive(Debug, Clone)]
pub(crate) struct Columns {
    kinds: Vec<Kind>,
    pub(crate) schema: SchemaRef,
}

/// Which tabular API answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tabular {
    /// ES|QL: `columns` and `values`.
    Esql,
    /// Elasticsearch `_sql`: `columns`, `rows`, `cursor`.
    Sql,
    /// OpenSearch SQL: `schema`, `datarows`, `cursor`.
    OpenSearchSql,
}

/// The tabular APIs, by the path prefix that reaches them.
const APIS: [(&str, Tabular); 6] = [
    ("/_query", Tabular::Esql),
    ("/_sql", Tabular::Sql),
    ("/_plugins/_sql", Tabular::OpenSearchSql),
    ("/_plugins/_ppl", Tabular::OpenSearchSql),
    ("/_opendistro/_sql", Tabular::OpenSearchSql),
    ("/_opendistro/_ppl", Tabular::OpenSearchSql),
];

impl Tabular {
    /// The tabular API a request path reaches, if any — `/_query` and its
    /// `async` routes are ES|QL.
    pub(crate) fn of_path(path: &str) -> Option<Self> {
        let path = path.split('?').next().unwrap_or_default();
        APIS.into_iter().find_map(|(api, tabular)| {
            path.strip_prefix(api)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                .then_some(tabular)
        })
    }

    fn declared_key(self) -> &'static str {
        match self {
            Self::Esql | Self::Sql => "columns",
            Self::OpenSearchSql => "schema",
        }
    }
}

/// How a tabular answer arrives: its API, and whether `values` holds one
/// array per column (`"columnar": true` in the request) rather than per row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shape {
    pub(crate) tabular: Tabular,
    pub(crate) columnar: bool,
}

impl Shape {
    /// The shape a request asks for. OpenSearch SQL documents no columnar
    /// mode: its `datarows` are rows whatever the body says.
    pub(crate) fn new(tabular: Tabular, columnar: bool) -> Self {
        Self {
            tabular,
            columnar: columnar && tabular != Tabular::OpenSearchSql,
        }
    }

    fn rows_key(self) -> &'static str {
        match self.tabular {
            Tabular::Esql => "values",
            Tabular::Sql if self.columnar => "values",
            Tabular::Sql => "rows",
            Tabular::OpenSearchSql => "datarows",
        }
    }
}

/// Decodes a first page, if the answer has the shape its API answers with.
pub(crate) fn decode(
    object: &IndexMap<String, Box<RawValue>>,
    shape: Shape,
) -> Result<Option<Decoded>> {
    let (Some(declared), true) = (
        object.get(shape.tabular.declared_key()),
        object.contains_key(shape.rows_key()),
    ) else {
        return Ok(None);
    };
    let Ok(declared) = serde_json::from_str::<Vec<Declared>>(declared.get()) else {
        // A `columns` key that is not a list of columns: not this shape.
        return Ok(None);
    };
    if declared.len() > MAX_COLUMNS {
        return Err(OxynError::Query(format!(
            "the answer declares {} columns, more than the {MAX_COLUMNS} a table takes: name \
             the columns to return (`KEEP` in ES|QL, the select list in SQL)",
            declared.len()
        )));
    }
    let rows = rows(object, shape, declared.len())?;
    let kinds: Vec<Kind> = declared
        .iter()
        .enumerate()
        .map(|(position, column)| {
            let kind = Kind::of(&column.raw_type);
            let fits = rows
                .iter()
                .all(|row| row.get(position).is_none_or(|value| kind.fits(value)));
            if fits { kind } else { Kind::Text }
        })
        .collect();
    let fields: Vec<Field> = declared
        .iter()
        .zip(&kinds)
        .map(|(column, kind)| {
            Field::new(&column.name, kind.data_type(), true).with_metadata(HashMap::from([(
                META_TYPE.to_owned(),
                column.raw_type.clone(),
            )]))
        })
        .collect();
    let columns = Columns {
        kinds,
        schema: Arc::new(Schema::new(fields)),
    };
    let decoded = page(&columns, object, shape, &rows)?;
    Ok(Some(decoded))
}

/// Decodes a later page of `_sql`, in its first page's columns and shape.
pub(crate) fn next_page(
    columns: &Columns,
    object: &IndexMap<String, Box<RawValue>>,
    shape: Shape,
) -> Result<Decoded> {
    let rows = rows(object, shape, columns.kinds.len())?;
    for row in &rows {
        for (value, (kind, field)) in row
            .iter()
            .zip(columns.kinds.iter().zip(columns.schema.fields()))
        {
            if !kind.fits(value) {
                return Err(OxynError::Query(format!(
                    "a later page holds a value of column `{}` that its first page did not \
                     announce: the stream stops rather than drop it",
                    field.name()
                )));
            }
        }
    }
    page(columns, object, shape, &rows)
}

/// The rows of a page, each one value per declared column. A columnar page
/// is turned into rows: `values` then holds `width` arrays of one length.
fn rows(
    object: &IndexMap<String, Box<RawValue>>,
    shape: Shape,
    width: usize,
) -> Result<Vec<Vec<&RawValue>>> {
    let Some(values) = object.get(shape.rows_key()) else {
        return Ok(Vec::new());
    };
    let arrays: Vec<Vec<&RawValue>> = serde_json::from_str(values.get())
        .map_err(|_| OxynError::Query("the rows of a tabular answer are not arrays".to_owned()))?;
    let rows = if shape.columnar && !arrays.is_empty() {
        if arrays.len() != width {
            return Err(OxynError::Query(format!(
                "a columnar answer holds {} columns of values for {width} declared columns",
                arrays.len()
            )));
        }
        let length = arrays.first().map_or(0, Vec::len);
        if arrays.iter().any(|column| column.len() != length) {
            return Err(OxynError::Query(
                "the columns of a columnar answer do not hold as many values each".to_owned(),
            ));
        }
        let mut rows: Vec<Vec<&RawValue>> =
            (0..length).map(|_| Vec::with_capacity(width)).collect();
        for column in arrays {
            for (row, value) in rows.iter_mut().zip(column) {
                row.push(value);
            }
        }
        rows
    } else {
        arrays
    };
    if let Some(row) = rows.iter().find(|row| row.len() != width) {
        return Err(OxynError::Query(format!(
            "a row of the answer holds {} values for {width} columns",
            row.len()
        )));
    }
    Ok(rows)
}

fn page(
    columns: &Columns,
    object: &IndexMap<String, Box<RawValue>>,
    shape: Shape,
    rows: &[Vec<&RawValue>],
) -> Result<Decoded> {
    let overhead = columns.kinds.len().saturating_mul(COLUMN_OVERHEAD);
    let mut batches = Vec::new();
    let mut start = 0;
    let mut bits = 0_usize;
    // The whole page's Arrow cost: never reset when a batch is cut.
    let mut total = overhead;
    for (index, row) in rows.iter().enumerate() {
        let row_bits = row
            .iter()
            .zip(&columns.kinds)
            .map(|(value, kind)| kind.cell_bits(value))
            .fold(0_usize, usize::saturating_add);
        bits = bits.saturating_add(row_bits);
        if total.saturating_add(bits.div_ceil(8)) > DECODED_LIMIT {
            return Err(OxynError::Query(
                "the answer decodes to more than the 64 MiB a page may take: lower `LIMIT` or \
                 `fetch_size`, or return fewer columns"
                    .to_owned(),
            ));
        }
        if bits.div_ceil(8) >= BATCH_BYTES {
            batches.push(batch(columns, rows.get(start..=index).unwrap_or_default())?);
            total = total
                .saturating_add(bits.div_ceil(8))
                .saturating_add(overhead);
            start = index + 1;
            bits = 0;
        }
    }
    if start < rows.len() || batches.is_empty() {
        batches.push(batch(columns, rows.get(start..).unwrap_or_default())?);
    }
    let took = object
        .get("took")
        .and_then(|took| took.get().parse::<u64>().ok())
        .map(Duration::from_millis);
    let sql_cursor = object
        .get("cursor")
        .and_then(|cursor| serde_json::from_str::<String>(cursor.get()).ok())
        .filter(|cursor| !cursor.is_empty());
    Ok(Decoded {
        schema: Arc::clone(&columns.schema),
        batches: batches
            .into_iter()
            .filter(|batch| batch.num_rows() > 0)
            .collect(),
        took,
        page: PageState {
            rows: rows.len(),
            sql_cursor,
            ..PageState::default()
        },
        layout: None,
        tabular: Some((columns.clone(), shape)),
    })
}

fn batch(columns: &Columns, rows: &[Vec<&RawValue>]) -> Result<RecordBatch> {
    let arrays: Vec<ArrayRef> = columns
        .kinds
        .iter()
        .enumerate()
        .map(|(position, kind)| {
            let values: Vec<&RawValue> = rows
                .iter()
                .filter_map(|row| row.get(position).copied())
                .collect();
            column(*kind, &values)
        })
        .collect();
    RecordBatch::try_new(Arc::clone(&columns.schema), arrays)
        .map_err(|error| OxynError::Internal(format!("a tabular batch is invalid: {error}")))
}

/// One column, its buffers sized to its values before the first append.
/// Values were checked against the kind before; one that still does not
/// parse is a null only if it is a JSON `null`.
fn column(kind: Kind, values: &[&RawValue]) -> ArrayRef {
    let null = |value: &RawValue| value.get() == "null";
    let rows = values.len();
    match kind {
        Kind::Boolean => {
            let mut builder = BooleanBuilder::with_capacity(rows);
            for value in values {
                builder.append_option((!null(value)).then(|| value.get() == "true"));
            }
            Arc::new(builder.finish())
        }
        Kind::Int32 => {
            let mut builder = Int32Builder::with_capacity(rows);
            values
                .iter()
                .for_each(|value| builder.append_option(value.get().parse().ok()));
            Arc::new(builder.finish())
        }
        Kind::Int64 => {
            let mut builder = Int64Builder::with_capacity(rows);
            values
                .iter()
                .for_each(|value| builder.append_option(value.get().parse().ok()));
            Arc::new(builder.finish())
        }
        Kind::UInt64 => {
            let mut builder = UInt64Builder::with_capacity(rows);
            values
                .iter()
                .for_each(|value| builder.append_option(value.get().parse().ok()));
            Arc::new(builder.finish())
        }
        Kind::Float64 => {
            let mut builder = Float64Builder::with_capacity(rows);
            values
                .iter()
                .for_each(|value| builder.append_option(value.get().parse().ok()));
            Arc::new(builder.finish())
        }
        Kind::Millis => {
            let mut builder = TimestampMillisecondBuilder::with_capacity(rows).with_timezone("UTC");
            values.iter().for_each(|value| {
                builder.append_option(instant(value).map(|instant| instant.timestamp_millis()));
            });
            Arc::new(builder.finish())
        }
        Kind::Nanos => {
            let mut builder = TimestampNanosecondBuilder::with_capacity(rows).with_timezone("UTC");
            values.iter().for_each(|value| {
                builder.append_option(
                    instant(value).and_then(|instant| instant.timestamp_nanos_opt()),
                );
            });
            Arc::new(builder.finish())
        }
        Kind::Text => {
            let bytes = values
                .iter()
                .map(|value| text_bytes(value))
                .fold(0_usize, usize::saturating_add);
            let mut builder = StringBuilder::with_capacity(rows, bytes);
            values
                .iter()
                .for_each(|value| builder.append_option(cell_text(value)));
            Arc::new(builder.finish())
        }
    }
}

#[cfg(test)]
#[path = "columnar_tests.rs"]
mod tests;
