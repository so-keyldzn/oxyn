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

use crate::cells::{BATCH_BYTES, META_TYPE, cell_text};
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
}

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

impl Tabular {
    fn rows_key(self) -> &'static str {
        match self {
            Self::Esql => "values",
            Self::Sql => "rows",
            Self::OpenSearchSql => "datarows",
        }
    }
}

/// Decodes a first page, if the answer is tabular.
pub(crate) fn decode(object: &IndexMap<String, Box<RawValue>>) -> Result<Option<Decoded>> {
    let shapes = [
        ("columns", Tabular::Esql),
        ("columns", Tabular::Sql),
        ("schema", Tabular::OpenSearchSql),
    ];
    let Some((declared, tabular)) = shapes.into_iter().find_map(|(key, tabular)| {
        let declared = object.get(key)?;
        object
            .contains_key(tabular.rows_key())
            .then_some((declared, tabular))
    }) else {
        return Ok(None);
    };
    let Ok(declared) = serde_json::from_str::<Vec<Declared>>(declared.get()) else {
        // A `columns` key that is not a list of columns: not this shape.
        return Ok(None);
    };
    let rows = rows(object, tabular)?;
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
    let decoded = page(&columns, object, tabular, &rows)?;
    Ok(Some(decoded))
}

/// Decodes a later page of `_sql`, in its first page's columns.
pub(crate) fn next_page(
    columns: &Columns,
    object: &IndexMap<String, Box<RawValue>>,
    tabular: Tabular,
) -> Result<Decoded> {
    let rows = rows(object, tabular)?;
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
    page(columns, object, tabular, &rows)
}

fn rows(
    object: &IndexMap<String, Box<RawValue>>,
    tabular: Tabular,
) -> Result<Vec<Vec<Box<RawValue>>>> {
    let Some(rows) = object.get(tabular.rows_key()) else {
        return Ok(Vec::new());
    };
    serde_json::from_str(rows.get())
        .map_err(|_| OxynError::Query("the rows of a tabular answer are not arrays".to_owned()))
}

fn page(
    columns: &Columns,
    object: &IndexMap<String, Box<RawValue>>,
    tabular: Tabular,
    rows: &[Vec<Box<RawValue>>],
) -> Result<Decoded> {
    let width = columns.kinds.len();
    if let Some(row) = rows.iter().find(|row| row.len() != width) {
        return Err(OxynError::Query(format!(
            "a row of the answer holds {} values for {width} columns",
            row.len()
        )));
    }
    let mut batches = Vec::new();
    let mut start = 0;
    let mut bytes = 0;
    for (index, row) in rows.iter().enumerate() {
        bytes += row.iter().map(|value| value.get().len()).sum::<usize>();
        if bytes >= BATCH_BYTES {
            batches.push(batch(columns, rows.get(start..=index).unwrap_or_default())?);
            start = index + 1;
            bytes = 0;
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
        tabular: Some((columns.clone(), tabular)),
    })
}

fn batch(columns: &Columns, rows: &[Vec<Box<RawValue>>]) -> Result<RecordBatch> {
    let arrays: Vec<ArrayRef> = columns
        .kinds
        .iter()
        .enumerate()
        .map(|(position, kind)| {
            let values = rows.iter().filter_map(|row| row.get(position));
            column(*kind, values)
        })
        .collect();
    RecordBatch::try_new(Arc::clone(&columns.schema), arrays)
        .map_err(|error| OxynError::Internal(format!("a tabular batch is invalid: {error}")))
}

/// One column. Values were checked against the kind before; one that still
/// does not parse is a null only if it is a JSON `null`.
fn column<'a>(kind: Kind, values: impl Iterator<Item = &'a Box<RawValue>>) -> ArrayRef {
    let null = |value: &RawValue| value.get() == "null";
    match kind {
        Kind::Boolean => {
            let mut builder = BooleanBuilder::new();
            for value in values {
                builder.append_option((!null(value)).then(|| value.get() == "true"));
            }
            Arc::new(builder.finish())
        }
        Kind::Int32 => {
            let mut builder = Int32Builder::new();
            values.for_each(|value| builder.append_option(value.get().parse().ok()));
            Arc::new(builder.finish())
        }
        Kind::Int64 => {
            let mut builder = Int64Builder::new();
            values.for_each(|value| builder.append_option(value.get().parse().ok()));
            Arc::new(builder.finish())
        }
        Kind::UInt64 => {
            let mut builder = UInt64Builder::new();
            values.for_each(|value| builder.append_option(value.get().parse().ok()));
            Arc::new(builder.finish())
        }
        Kind::Float64 => {
            let mut builder = Float64Builder::new();
            values.for_each(|value| builder.append_option(value.get().parse().ok()));
            Arc::new(builder.finish())
        }
        Kind::Millis => {
            let mut builder = TimestampMillisecondBuilder::new().with_timezone("UTC");
            values.for_each(|value| {
                builder.append_option(instant(value).map(|instant| instant.timestamp_millis()));
            });
            Arc::new(builder.finish())
        }
        Kind::Nanos => {
            let mut builder = TimestampNanosecondBuilder::new().with_timezone("UTC");
            values.for_each(|value| {
                builder.append_option(
                    instant(value).and_then(|instant| instant.timestamp_nanos_opt()),
                );
            });
            Arc::new(builder.finish())
        }
        Kind::Text => {
            let mut builder = StringBuilder::new();
            values.for_each(|value| builder.append_option(cell_text(value)));
            Arc::new(builder.finish())
        }
    }
}
