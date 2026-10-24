//! A JSON value as a grid cell, and rows cut into batches bounded in bytes.
//!
//! A cell is the value's **exact JSON text**: a string unquoted, anything
//! else as the server wrote it. Values are kept as `RawValue`s, never decoded
//! into `serde_json::Number`, so `18446744073709551615`, `0.1000000000000000055`
//! and `1e400` reach the grid as the server sent them instead of through an
//! `f64` (DRIVER-CONTRACT §3). A JSON `null` is a null cell.

use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{ArrayRef, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use oxyn_core::{OxynError, Result};
use serde_json::value::RawValue;

/// The most cell text one batch carries: 4 MiB.
///
/// A batch is sized in bytes, not rows: a page of a hundred 100 KiB documents
/// is cut into three batches, a page of ten thousand tiny ones into one.
pub(crate) const BATCH_BYTES: usize = 4 << 20;

/// The field metadata key carrying the server's type of a column.
pub(crate) const META_TYPE: &str = "elasticsearch.type";

/// The text of a cell, or `None` for a JSON `null`.
pub(crate) fn cell_text(value: &RawValue) -> Option<String> {
    let text = value.get();
    match text.as_bytes().first() {
        Some(b'"') => serde_json::from_str::<String>(text).ok(),
        Some(b'n') if text == "null" => None,
        _ => Some(text.to_owned()),
    }
}

/// A column of text cells.
pub(crate) fn text_field(name: &str, raw_type: Option<&str>) -> Field {
    let field = Field::new(name, DataType::Utf8, true);
    match raw_type {
        Some(raw_type) => {
            field.with_metadata(HashMap::from([(META_TYPE.to_owned(), raw_type.to_owned())]))
        }
        None => field,
    }
}

/// Rows of text cells, cut into batches of at most [`BATCH_BYTES`].
pub(crate) struct TextRows {
    schema: SchemaRef,
    builders: Vec<StringBuilder>,
    bytes: usize,
    batches: Vec<RecordBatch>,
}

impl TextRows {
    /// Rows for a schema whose every column is `Utf8`.
    pub(crate) fn new(schema: SchemaRef) -> Self {
        let builders = schema
            .fields()
            .iter()
            .map(|_| StringBuilder::new())
            .collect();
        Self {
            schema,
            builders,
            bytes: 0,
            batches: Vec::new(),
        }
    }

    /// Appends one row; `cells` has one entry per column, in order.
    pub(crate) fn push(&mut self, cells: impl IntoIterator<Item = Option<String>>) -> Result<()> {
        let mut columns = 0;
        for (builder, cell) in self.builders.iter_mut().zip(cells) {
            self.bytes = self
                .bytes
                .saturating_add(cell.as_ref().map_or(0, String::len));
            builder.append_option(cell);
            columns += 1;
        }
        if columns != self.builders.len() {
            return Err(OxynError::Internal(
                "a row does not have one cell per column".to_owned(),
            ));
        }
        if self.bytes >= BATCH_BYTES {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        let arrays: Vec<ArrayRef> = self
            .builders
            .iter_mut()
            .map(|builder| Arc::new(builder.finish()) as ArrayRef)
            .collect();
        let batch = RecordBatch::try_new(Arc::clone(&self.schema), arrays)
            .map_err(|error| OxynError::Internal(format!("a result batch is invalid: {error}")))?;
        if batch.num_rows() > 0 {
            self.batches.push(batch);
        }
        self.bytes = 0;
        Ok(())
    }

    /// The batches, the last partial one included.
    pub(crate) fn finish(mut self) -> Result<Vec<RecordBatch>> {
        self.flush()?;
        Ok(self.batches)
    }
}

/// A schema of text columns named `names`, made unique: a second `status`
/// becomes `status (2)`, since a grid cannot show two columns under one name
/// and dropping either would lose its values.
pub(crate) fn text_schema<'a>(
    columns: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> SchemaRef {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let fields: Vec<Field> = columns
        .into_iter()
        .map(|(name, raw_type)| {
            let count = seen.entry(name.to_owned()).or_insert(0);
            *count += 1;
            if *count == 1 {
                text_field(name, raw_type)
            } else {
                text_field(&format!("{name} ({count})"), raw_type)
            }
        })
        .collect();
    Arc::new(Schema::new(fields))
}

#[cfg(test)]
mod tests {
    use arrow::array::{Array, StringArray};

    use super::*;

    fn raw(text: &str) -> Box<RawValue> {
        RawValue::from_string(text.to_owned()).expect("valid JSON")
    }

    #[test]
    fn numbers_reach_the_cell_as_the_server_wrote_them() {
        for text in [
            "18446744073709551615",
            "-9223372036854775809",
            "0.1000000000000000055511151231257827",
            "1e400",
            "1.10",
        ] {
            assert_eq!(cell_text(&raw(text)).as_deref(), Some(text));
        }
        assert_eq!(cell_text(&raw(r#""a\"b""#)).as_deref(), Some("a\"b"));
        assert_eq!(cell_text(&raw("null")), None);
        assert_eq!(cell_text(&raw("[1,2]")).as_deref(), Some("[1,2]"));
    }

    #[test]
    fn rows_are_cut_into_batches_bounded_in_bytes() {
        let schema = text_schema([("body", None)]);
        let mut rows = TextRows::new(schema);
        let cell = "x".repeat(1 << 20);
        for _ in 0..10 {
            rows.push([Some(cell.clone())]).expect("a row");
        }
        let batches = rows.finish().expect("batches");
        assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 10);
        assert!(batches.len() >= 3, "{} batches", batches.len());
        for batch in &batches {
            let column = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("utf8");
            assert!(column.value_data().len() <= BATCH_BYTES);
        }
    }

    #[test]
    fn a_repeated_column_name_is_kept_under_a_distinct_name() {
        let schema = text_schema([
            ("status", None),
            ("status", None),
            ("status", Some("keyword")),
        ]);
        let names: Vec<&str> = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        assert_eq!(names, ["status", "status (2)", "status (3)"]);
        assert_eq!(
            schema
                .field(2)
                .metadata()
                .get(META_TYPE)
                .map(String::as_str),
            Some("keyword")
        );
    }
}
