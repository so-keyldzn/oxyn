//! The columns of a reply, and the batches they fill.
//!
//! # The shape follows the reply, then the command
//!
//! A reply's first byte says what it is, so its columns are known before any
//! element arrives — what lets the grid draw them while the elements stream
//! ([ADR-0002](../../../docs/adr/0002-arrow-result-model.md)). The generic
//! shapes cover every reply:
//!
//! | Reply | Rows |
//! |---|---|
//! | a scalar | one row, typed: `Int64`, `Float64`, `Boolean`, or text and bytes |
//! | an array, a set, a push | one per element: `index`, `type`, `value`, `value_bytes`, `truncated` |
//! | a map | one per entry: `key`, `key_bytes`, `type`, `value`, `value_bytes`, `truncated` |
//!
//! A few commands answer a nesting the generic shape would put in one cell;
//! for them the command names the shape, never the reply alone: the `SCAN`
//! family (`cursor` beside each key, field or member), `ZRANGE … WITHSCORES`
//! (`member`, `score`), `XRANGE` (`id`, `field`, `value`), `INFO` (`section`,
//! `name`, `value`). The list is presentation, not classification: a command
//! missing from it still shows, in the generic shape.
//!
//! # Bounded in bytes
//!
//! A batch closes past [`BATCH_BYTE_BUDGET`] bytes, whatever its row count;
//! one value keeps at most [`MAX_VALUE_BYTES`], and says so in `truncated`
//! ([I-06](../../../CLAUDE.md#i-06)).

use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{
    ArrayBuilder, ArrayRef, BinaryBuilder, BooleanBuilder, Float64Builder, Int64Builder,
    StringBuilder,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use oxyn_core::{OxynError, Result};

use crate::render::Cell;
use crate::resp::{Aggregate, Head};

/// A batch closes once it holds this many bytes.
pub const BATCH_BYTE_BUDGET: usize = 1 << 20;

/// The most bytes one value keeps. Redis allows 512 MB per string; past this
/// bound the value is cut and its row says so.
pub const MAX_VALUE_BYTES: usize = 16 << 20;

/// Field metadata naming the RESP type of a typed scalar column, so an export
/// read without Oxyn still says a `big_number` was a number.
pub const META_REDIS_TYPE: &str = "oxyn:redis_type";

/// How a reply becomes rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// One row, typed by the scalar's RESP type.
    Scalar(ScalarKind),
    /// One row per element of an array, a set or a push.
    Elements,
    /// One row per map entry.
    Pairs,
    /// A `SCAN`-family page.
    Scan(ScanForm),
    /// `member` and `score` pairs.
    Scored,
    /// Stream entries, one row per field.
    StreamEntries,
    /// Time series samples, `[timestamp, value]` each.
    Samples,
    /// `INFO`'s report, one row per line.
    Info,
    /// `SLOWLOG GET`'s entries, one row each.
    Slowlog,
    /// `CLIENT LIST` and `CLIENT INFO`, one row per client.
    Clients,
    /// An array of maps — `XINFO GROUPS`, `ACL LOG`… —, one row per map and
    /// one column per key of the first.
    Records,
}

/// The type of a scalar reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarKind {
    /// A simple, bulk or verbatim string.
    Text,
    /// A 64-bit integer.
    Integer,
    /// A double.
    Double,
    /// A boolean.
    Boolean,
    /// An integer beyond 64 bits.
    BigNumber,
    /// No value.
    Null,
}

/// The element a `SCAN`-family page lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanForm {
    /// `SCAN`: keys.
    Keys,
    /// `HSCAN`: fields and values.
    HashFields,
    /// `HSCAN … NOVALUES`: fields only.
    HashFieldNames,
    /// `SSCAN`: members.
    Members,
    /// `ZSCAN`: members and scores.
    ScoredMembers,
}

impl ScanForm {
    /// How many reply elements make one row.
    #[must_use]
    pub const fn stride(self) -> u64 {
        match self {
            Self::Keys | Self::HashFieldNames | Self::Members => 1,
            Self::HashFields | Self::ScoredMembers => 2,
        }
    }
}

impl Layout {
    /// The layout of a command's reply, from the command and the reply's
    /// first head.
    #[must_use]
    pub fn choose(argv: &[Vec<u8>], head: &Head) -> Self {
        let command = argv
            .first()
            .map(|word| String::from_utf8_lossy(word).to_ascii_lowercase())
            .unwrap_or_default();
        let has = |option: &str| {
            argv.iter()
                .skip(1)
                .any(|word| word.eq_ignore_ascii_case(option.as_bytes()))
        };
        let sub = |name: &str| {
            argv.get(1)
                .is_some_and(|word| word.eq_ignore_ascii_case(name.as_bytes()))
        };
        match head {
            Head::Aggregate(Aggregate::Map, _) => Self::Pairs,
            // A page is `[cursor, elements]`: two elements, or not a page.
            Head::Aggregate(Aggregate::Array, len) => match command.as_str() {
                "scan" if *len == 2 => Self::Scan(ScanForm::Keys),
                "hscan" if *len == 2 && has("novalues") => Self::Scan(ScanForm::HashFieldNames),
                "hscan" if *len == 2 => Self::Scan(ScanForm::HashFields),
                "sscan" if *len == 2 => Self::Scan(ScanForm::Members),
                "zscan" if *len == 2 => Self::Scan(ScanForm::ScoredMembers),
                "zrange" | "zrangebyscore" | "zrevrange" | "zrevrangebyscore"
                    if has("withscores") =>
                {
                    Self::Scored
                }
                "xrange" | "xrevrange" => Self::StreamEntries,
                "ts.range" | "ts.revrange" => Self::Samples,
                "slowlog" if sub("get") => Self::Slowlog,
                "xinfo" if sub("groups") || sub("consumers") => Self::Records,
                "acl" if sub("log") => Self::Records,
                "module" | "function" if sub("list") => Self::Records,
                _ => Self::Elements,
            },
            Head::Aggregate(..) => Self::Elements,
            Head::Bulk(_) | Head::Verbatim(_) if command == "info" => Self::Info,
            Head::Bulk(_) | Head::Verbatim(_)
                if command == "client" && (sub("list") || sub("info")) =>
            {
                Self::Clients
            }
            Head::Simple(_)
            | Head::Bulk(_)
            | Head::Verbatim(_)
            | Head::Error(_)
            | Head::BulkError(_) => Self::Scalar(ScalarKind::Text),
            Head::Integer(_) => Self::Scalar(ScalarKind::Integer),
            Head::Double(_) => Self::Scalar(ScalarKind::Double),
            Head::Boolean(_) => Self::Scalar(ScalarKind::Boolean),
            Head::BigNumber(_) => Self::Scalar(ScalarKind::BigNumber),
            Head::Null => Self::Scalar(ScalarKind::Null),
        }
    }

    /// The columns of this layout.
    #[must_use]
    pub fn schema(self) -> SchemaRef {
        let text = |name: &str| Field::new(name, DataType::Utf8, true);
        let bytes = |name: &str| Field::new(name, DataType::Binary, true);
        let truncated = Field::new("truncated", DataType::Boolean, false);
        let kind = Field::new("type", DataType::Utf8, false);
        let typed = |data_type: DataType, redis: &str| {
            Field::new("value", data_type, true).with_metadata(HashMap::from([(
                META_REDIS_TYPE.to_owned(),
                redis.to_owned(),
            )]))
        };
        let fields = match self {
            Self::Scalar(ScalarKind::Text) => {
                vec![text("value"), bytes("value_bytes"), kind, truncated]
            }
            Self::Scalar(ScalarKind::Integer) => vec![typed(DataType::Int64, "integer")],
            Self::Scalar(ScalarKind::Double) => vec![typed(DataType::Float64, "double")],
            Self::Scalar(ScalarKind::Boolean) => vec![typed(DataType::Boolean, "boolean")],
            Self::Scalar(ScalarKind::BigNumber) => vec![typed(DataType::Utf8, "big_number")],
            Self::Scalar(ScalarKind::Null) => vec![typed(DataType::Utf8, "null")],
            Self::Elements => vec![
                Field::new("index", DataType::Int64, false),
                kind,
                text("value"),
                bytes("value_bytes"),
                truncated,
            ],
            Self::Pairs => vec![
                text("key"),
                bytes("key_bytes"),
                kind,
                text("value"),
                bytes("value_bytes"),
                truncated,
            ],
            Self::Scan(form) => {
                let mut fields = vec![Field::new("cursor", DataType::Utf8, false)];
                let (name, name_bytes) = match form {
                    ScanForm::Keys => ("key", "key_bytes"),
                    ScanForm::HashFields | ScanForm::HashFieldNames => ("field", "field_bytes"),
                    ScanForm::Members | ScanForm::ScoredMembers => ("member", "member_bytes"),
                };
                fields.extend([text(name), bytes(name_bytes)]);
                match form {
                    ScanForm::HashFields => fields.extend([text("value"), bytes("value_bytes")]),
                    ScanForm::ScoredMembers => {
                        fields.push(Field::new("score", DataType::Float64, true))
                    }
                    ScanForm::Keys | ScanForm::HashFieldNames | ScanForm::Members => {}
                }
                fields.push(truncated);
                fields
            }
            Self::Scored => vec![
                text("member"),
                bytes("member_bytes"),
                Field::new("score", DataType::Float64, true),
                truncated,
            ],
            Self::StreamEntries => vec![
                Field::new("id", DataType::Utf8, false),
                text("field"),
                bytes("field_bytes"),
                text("value"),
                bytes("value_bytes"),
                truncated,
            ],
            Self::Info => vec![text("section"), text("name"), text("value")],
            // RedisTimeSeries keeps an integer the application chose — by
            // convention milliseconds since the epoch, in no time zone. It
            // stays that integer, tagged, never a timestamp Oxyn would have
            // to place in a zone (DRIVER-CONTRACT §7).
            Self::Samples => vec![
                Field::new("timestamp", DataType::Int64, true).with_metadata(HashMap::from([(
                    META_REDIS_TYPE.to_owned(),
                    "timestamp_ms".to_owned(),
                )])),
                Field::new("value", DataType::Float64, true),
            ],
            Self::Slowlog => vec![
                Field::new("id", DataType::Int64, true),
                Field::new("time", DataType::Int64, true).with_metadata(HashMap::from([(
                    META_REDIS_TYPE.to_owned(),
                    "unix_time_s".to_owned(),
                )])),
                Field::new("duration_us", DataType::Int64, true),
                text("command"),
                text("client"),
                text("client_name"),
                truncated,
            ],
            // The fields every client line carries; the line itself keeps
            // the others, whatever the server version adds.
            Self::Clients => vec![
                Field::new("id", DataType::Int64, true),
                text("addr"),
                text("name"),
                text("user"),
                Field::new("db", DataType::Int64, true),
                text("cmd"),
                text("flags"),
                Field::new("age_s", DataType::Int64, true),
                Field::new("idle_s", DataType::Int64, true),
                text("line"),
                truncated,
            ],
            // Its columns are the first map's keys, known once it is read.
            Self::Records => return crate::admin::records_schema(&[]),
        };
        Arc::new(Schema::new(fields))
    }
}

/// One cell of a row, in the column's type.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    /// A `Utf8` cell.
    Text(Option<String>),
    /// A `Binary` cell.
    Bytes(Option<Vec<u8>>),
    /// An `Int64` cell.
    Integer(Option<i64>),
    /// A `Float64` cell.
    Float(Option<f64>),
    /// A `Boolean` cell.
    Boolean(Option<bool>),
}

impl Slot {
    /// The two cells of a string column pair: its text and its bytes.
    #[must_use]
    pub fn string_pair(cell: &Cell) -> [Self; 2] {
        [
            Self::Text(cell.text.clone()),
            Self::Bytes(cell.bytes.clone()),
        ]
    }

    fn size(&self) -> usize {
        match self {
            Self::Text(text) => text.as_ref().map_or(0, String::len) + 4,
            Self::Bytes(bytes) => bytes.as_ref().map_or(0, Vec::len) + 4,
            Self::Integer(_) | Self::Float(_) => 8,
            Self::Boolean(_) => 1,
        }
    }
}

enum Column {
    Text(StringBuilder),
    Bytes(BinaryBuilder),
    Integer(Int64Builder),
    Float(Float64Builder),
    Boolean(BooleanBuilder),
}

/// Rows being gathered into the next batch.
pub struct Rows {
    schema: SchemaRef,
    columns: Vec<Column>,
    bytes: usize,
}

impl std::fmt::Debug for Rows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rows")
            .field("columns", &self.schema.fields().len())
            .field("rows", &self.len())
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl Rows {
    /// Empty rows for a schema of `Utf8`, `Binary`, `Int64`, `Float64` and
    /// `Boolean` columns.
    ///
    /// # Errors
    /// [`OxynError::Internal`] for another column type: a layout bug.
    pub fn new(schema: SchemaRef) -> Result<Self> {
        let columns = schema
            .fields()
            .iter()
            .map(|field| match field.data_type() {
                DataType::Utf8 => Ok(Column::Text(StringBuilder::new())),
                DataType::Binary => Ok(Column::Bytes(BinaryBuilder::new())),
                DataType::Int64 => Ok(Column::Integer(Int64Builder::new())),
                DataType::Float64 => Ok(Column::Float(Float64Builder::new())),
                DataType::Boolean => Ok(Column::Boolean(BooleanBuilder::new())),
                other => Err(OxynError::Internal(format!(
                    "no Redis layout has a {other} column"
                ))),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            schema,
            columns,
            bytes: 0,
        })
    }

    /// Appends a row whose cells follow the schema's order.
    ///
    /// # Errors
    /// [`OxynError::Internal`] when the row does not match the schema: a
    /// layout bug, refused rather than shifting every later column.
    pub fn push(&mut self, row: &[Slot]) -> Result<()> {
        if row.len() != self.columns.len() {
            return Err(OxynError::Internal(format!(
                "a Redis row of {} cells for {} columns",
                row.len(),
                self.columns.len()
            )));
        }
        for (column, slot) in self.columns.iter().zip(row) {
            let matched = matches!(
                (column, slot),
                (Column::Text(_), Slot::Text(_))
                    | (Column::Bytes(_), Slot::Bytes(_))
                    | (Column::Integer(_), Slot::Integer(_))
                    | (Column::Float(_), Slot::Float(_))
                    | (Column::Boolean(_), Slot::Boolean(_))
            );
            if !matched {
                return Err(OxynError::Internal(
                    "a Redis row cell does not match its column".to_owned(),
                ));
            }
        }
        for (column, slot) in self.columns.iter_mut().zip(row) {
            self.bytes = self.bytes.saturating_add(slot.size());
            match (column, slot) {
                (Column::Text(builder), Slot::Text(value)) => {
                    builder.append_option(value.as_deref())
                }
                (Column::Bytes(builder), Slot::Bytes(value)) => {
                    builder.append_option(value.as_deref())
                }
                (Column::Integer(builder), Slot::Integer(value)) => builder.append_option(*value),
                (Column::Float(builder), Slot::Float(value)) => builder.append_option(*value),
                (Column::Boolean(builder), Slot::Boolean(value)) => builder.append_option(*value),
                _ => {}
            }
        }
        Ok(())
    }

    /// The rows gathered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.columns.first().map_or(0, |column| match column {
            Column::Text(b) => b.len(),
            Column::Bytes(b) => b.len(),
            Column::Integer(b) => b.len(),
            Column::Float(b) => b.len(),
            Column::Boolean(b) => b.len(),
        })
    }

    /// Whether no row was gathered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the rows reached the batch's byte bound.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.bytes >= BATCH_BYTE_BUDGET
    }

    /// Closes the batch and starts the next one.
    ///
    /// # Errors
    /// [`OxynError::Internal`] if Arrow refuses the columns: a layout bug.
    pub fn take(&mut self) -> Result<RecordBatch> {
        let arrays: Vec<ArrayRef> = self
            .columns
            .iter_mut()
            .map(|column| match column {
                Column::Text(b) => Arc::new(b.finish()) as ArrayRef,
                Column::Bytes(b) => Arc::new(b.finish()),
                Column::Integer(b) => Arc::new(b.finish()),
                Column::Float(b) => Arc::new(b.finish()),
                Column::Boolean(b) => Arc::new(b.finish()),
            })
            .collect();
        self.bytes = 0;
        RecordBatch::try_new(Arc::clone(&self.schema), arrays)
            .map_err(|error| OxynError::Internal(format!("a Redis batch does not hold: {error}")))
    }
}

/// `INFO`'s report as `(section, name, value)` rows.
///
/// `# Server` opens a section, `name:value` is a row, blank lines separate.
/// A line without a colon is kept whole as a name, never dropped.
#[must_use]
pub fn info_rows(report: &str) -> Vec<[Slot; 3]> {
    let mut section: Option<String> = None;
    report
        .lines()
        .filter_map(|line| {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                return None;
            }
            if let Some(name) = line.strip_prefix('#') {
                section = Some(name.trim().to_owned());
                return None;
            }
            let (name, value) = line
                .split_once(':')
                .map_or((line, None), |(n, v)| (n, Some(v.to_owned())));
            Some([
                Slot::Text(section.clone()),
                Slot::Text(Some(name.to_owned())),
                Slot::Text(value),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn argv(line: &str) -> Vec<Vec<u8>> {
        crate::argv::parse(line).expect("valid")
    }

    #[rstest]
    #[case("GET k", Head::Bulk(3), Layout::Scalar(ScalarKind::Text))]
    #[case("INCR k", Head::Integer(1), Layout::Scalar(ScalarKind::Integer))]
    #[case("GET k", Head::Null, Layout::Scalar(ScalarKind::Null))]
    #[case(
        "LRANGE k 0 -1",
        Head::Aggregate(Aggregate::Array, 3),
        Layout::Elements
    )]
    #[case("SMEMBERS k", Head::Aggregate(Aggregate::Set, 3), Layout::Elements)]
    #[case("HGETALL k", Head::Aggregate(Aggregate::Map, 3), Layout::Pairs)]
    #[case(
        "SCAN 0",
        Head::Aggregate(Aggregate::Array, 2),
        Layout::Scan(ScanForm::Keys)
    )]
    #[case(
        "hscan h 0 NOVALUES",
        Head::Aggregate(Aggregate::Array, 2),
        Layout::Scan(ScanForm::HashFieldNames)
    )]
    #[case(
        "HSCAN h 0",
        Head::Aggregate(Aggregate::Array, 2),
        Layout::Scan(ScanForm::HashFields)
    )]
    #[case(
        "ZRANGE z 0 -1 WITHSCORES",
        Head::Aggregate(Aggregate::Array, 2),
        Layout::Scored
    )]
    #[case(
        "ZRANGE z 0 -1",
        Head::Aggregate(Aggregate::Array, 2),
        Layout::Elements
    )]
    #[case(
        "XRANGE s - +",
        Head::Aggregate(Aggregate::Array, 2),
        Layout::StreamEntries
    )]
    #[case("INFO", Head::Verbatim(10), Layout::Info)]
    #[case(
        "SLOWLOG GET 10",
        Head::Aggregate(Aggregate::Array, 2),
        Layout::Slowlog
    )]
    #[case("SLOWLOG LEN", Head::Integer(2), Layout::Scalar(ScalarKind::Integer))]
    #[case("client list", Head::Verbatim(90), Layout::Clients)]
    #[case("CLIENT INFO", Head::Bulk(90), Layout::Clients)]
    #[case("CLIENT GETNAME", Head::Bulk(4), Layout::Scalar(ScalarKind::Text))]
    // A command that is not the shape's own falls back to the generic one.
    #[case("SCAN 0", Head::Null, Layout::Scalar(ScalarKind::Null))]
    fn the_layout_follows_the_reply_then_the_command(
        #[case] line: &str,
        #[case] head: Head,
        #[case] expected: Layout,
    ) {
        assert_eq!(Layout::choose(&argv(line), &head), expected);
    }

    #[test]
    fn rows_fill_a_batch_in_schema_order() {
        let mut rows = Rows::new(Layout::Elements.schema()).expect("supported columns");
        rows.push(&[
            Slot::Integer(Some(0)),
            Slot::Text(Some("bulk".to_owned())),
            Slot::Text(Some("v".to_owned())),
            Slot::Bytes(None),
            Slot::Boolean(Some(false)),
        ])
        .expect("matching row");
        let batch = rows.take().expect("a batch");
        assert_eq!(batch.num_rows(), 1);
        assert!(rows.is_empty(), "the next batch starts empty");
    }

    #[test]
    fn a_row_that_does_not_match_is_refused_not_shifted() {
        let mut rows = Rows::new(Layout::Elements.schema()).expect("supported columns");
        assert!(rows.push(&[Slot::Integer(Some(0))]).is_err());
        assert!(
            rows.push(&[
                Slot::Text(None),
                Slot::Text(None),
                Slot::Text(None),
                Slot::Bytes(None),
                Slot::Boolean(Some(false)),
            ])
            .is_err()
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn a_batch_closes_on_bytes_not_on_rows() {
        let mut rows = Rows::new(Layout::Scalar(ScalarKind::Text).schema()).expect("supported");
        let big = "x".repeat(BATCH_BYTE_BUDGET / 4);
        let mut pushed = 0;
        while !rows.is_full() {
            rows.push(&[
                Slot::Text(Some(big.clone())),
                Slot::Bytes(None),
                Slot::Text(Some("bulk".to_owned())),
                Slot::Boolean(Some(false)),
            ])
            .expect("matching row");
            pushed += 1;
        }
        assert!(pushed <= 4, "{pushed} rows of a quarter budget each");
    }

    #[test]
    fn info_becomes_sections_names_and_values() {
        let rows = info_rows(
            "# Server\r\nredis_version:8.10.2\r\nodd line\r\n\r\n# Keyspace\r\ndb0:keys=1,expires=0\r\n",
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][0], Slot::Text(Some("Server".to_owned())));
        assert_eq!(rows[0][2], Slot::Text(Some("8.10.2".to_owned())));
        assert_eq!(rows[1][1], Slot::Text(Some("odd line".to_owned())));
        assert_eq!(rows[1][2], Slot::Text(None));
        assert_eq!(rows[2][0], Slot::Text(Some("Keyspace".to_owned())));
    }
}
