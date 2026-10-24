//! Documents to Arrow: a schema inferred from a sample, and a lossless column.
//!
//! MongoDB declares no columns. A result's schema is **inferred** from the
//! first documents of the stream, and said to be ([DRIVER-CONTRACT §3](../../../docs/DRIVER-CONTRACT.md)):
//! every field carries `oxyn:inferred`, the schema carries the sample's size.
//! Two guarantees make the inference safe
//! ([ADR-0063](../../../docs/adr/0063-mongodb-driver-database-commands-in-extended-json.md)):
//!
//! * **nothing is dropped.** The last column, [`DOCUMENT_COLUMN`], holds each
//!   whole document in canonical Extended JSON. A field the sample never saw,
//!   a key repeated inside one document, a field beyond the column cap: all are
//!   there, typed;
//! * **nothing is coerced.** A field column holds one BSON type. A field the
//!   sample saw with two types — an `Int32` here, a document there — is a
//!   *mixed* column of relaxed Extended JSON, each value written as its own
//!   type. A value met after the sample with a type its column does not hold
//!   ends the stream with an error naming the field, both types and the
//!   document: the rows before it are delivered, none after is invented.
//!
//! # The type table, both ways
//!
//! | BSON | Arrow column | Loss, or why |
//! |---|---|---|
//! | Double | `Float64` | none; NaN and ±∞ kept |
//! | String | `Utf8` | none; a string that is not UTF-8 ends the stream, never replaced |
//! | Int32 | `Int32` | none |
//! | Int64 | `Int64` | none; beyond 2^53 kept exactly |
//! | Boolean | `Boolean` | none |
//! | Date | `Timestamp(ms, "UTC")` | none: BSON dates are UTC milliseconds by definition |
//! | ObjectId | `Utf8`, 24 hex digits | none; the creation time inside it is not decoded |
//! | Decimal128 | `Utf8`, the decimal's text | none; IEEE 754 decimal fits no fixed-scale Arrow decimal, and never a double |
//! | Binary (one subtype) | `Binary`, subtype in `oxyn:bson_subtype` | none; a UUID is subtype 4, shown as bytes |
//! | Timestamp, Regex, JavaScript (with scope), Symbol, DBPointer, MinKey, MaxKey, Undefined, document, array | `Utf8`, relaxed Extended JSON | the cell is text; the type is in its wrapper (`{"$timestamp": …}`) and in the canonical column. A Timestamp is the replication clock, never shown as a date |
//! | Null, missing | null | the canonical column tells a BSON null from a missing field |
//! | two types or more in the sample | `Utf8`, relaxed Extended JSON, `oxyn:bson_type = mixed` | an `Int32` and an `Int64` both read `1` here; the canonical column keeps them apart |
//! | an element type `bson` cannot frame | — | the stream ends with the library's message, which names the type byte: BSON gives an unknown element no length, so nothing after it can be read |
//!
//! The other direction — what a console command can carry — is the Extended
//! JSON reader of `crate::command`: every row of this table can be written
//! back in canonical form, DBPointer excepted (deprecated, refused).

use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryBuilder, BooleanBuilder, Float64Builder, Int32Builder, Int64Builder,
    StringBuilder, TimestampMillisecondBuilder,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use arrow::record_batch::RecordBatch;
use bson::raw::{RawBsonRef, RawDocument};
use bson::spec::ElementType;

use crate::ejson::{self, Mode};

/// The name of the lossless column. A document key may take it; the plan then
/// appends `#` until no field of the sample does.
pub const DOCUMENT_COLUMN: &str = "$document";

/// The field metadata key carrying the BSON type of a column, in the spelling
/// of the server's `$type` operator.
pub const META_BSON_TYPE: &str = "oxyn:bson_type";
/// The field metadata key carrying a binary column's subtype, two hex digits.
pub const META_BSON_SUBTYPE: &str = "oxyn:bson_subtype";
/// The field metadata key marking a column inferred from a sample.
pub const META_INFERRED: &str = "oxyn:inferred";
/// The field metadata key naming the text encoding of a text column.
pub const META_ENCODING: &str = "oxyn:encoding";
/// The schema metadata key carrying how many documents the inference read.
pub const META_SAMPLE_DOCUMENTS: &str = "oxyn:inferred_sample_documents";
/// The schema metadata key carrying how many sampled fields got no column:
/// a count, or a lower bound ending in `+` ([`OMITTED_COUNTED`]).
pub const META_OMITTED_FIELDS: &str = "oxyn:omitted_fields";

/// Field columns beyond which a sampled field only lives in the canonical
/// column: a hostile sample of a million keys would otherwise be a million
/// columns.
pub const MAX_FIELD_COLUMNS: usize = 1024;

/// What a column holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Double,
    String,
    Int32,
    Int64,
    Boolean,
    Date,
    ObjectId,
    Decimal,
    Binary(u8),
    /// Relaxed Extended JSON of whatever the value is. `alias` is the single
    /// BSON type the sample saw, `None` when it saw several or none.
    Json(Option<&'static str>),
}

impl Kind {
    /// The kind a single BSON type gets when it is a column's only one.
    fn of(element: ElementType, subtype: Option<u8>) -> Self {
        match element {
            ElementType::Double => Self::Double,
            ElementType::String => Self::String,
            ElementType::Int32 => Self::Int32,
            ElementType::Int64 => Self::Int64,
            ElementType::Boolean => Self::Boolean,
            ElementType::DateTime => Self::Date,
            ElementType::ObjectId => Self::ObjectId,
            ElementType::Decimal128 => Self::Decimal,
            ElementType::Binary => Self::Binary(subtype.unwrap_or(0)),
            other => Self::Json(Some(alias(other))),
        }
    }

    fn data_type(self) -> DataType {
        match self {
            Self::Double => DataType::Float64,
            Self::Int32 => DataType::Int32,
            Self::Int64 => DataType::Int64,
            Self::Boolean => DataType::Boolean,
            Self::Date => DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            Self::Binary(_) => DataType::Binary,
            Self::String | Self::ObjectId | Self::Decimal | Self::Json(_) => DataType::Utf8,
        }
    }

    fn bson_type(self) -> &'static str {
        match self {
            Self::Double => "double",
            Self::String => "string",
            Self::Int32 => "int",
            Self::Int64 => "long",
            Self::Boolean => "bool",
            Self::Date => "date",
            Self::ObjectId => "objectId",
            Self::Decimal => "decimal",
            Self::Binary(_) => "binData",
            Self::Json(alias) => alias.unwrap_or("mixed"),
        }
    }
}

/// The server's `$type` alias of an element type.
fn alias(element: ElementType) -> &'static str {
    match element {
        ElementType::Double => "double",
        ElementType::String => "string",
        ElementType::EmbeddedDocument => "object",
        ElementType::Array => "array",
        ElementType::Binary => "binData",
        ElementType::Undefined => "undefined",
        ElementType::ObjectId => "objectId",
        ElementType::Boolean => "bool",
        ElementType::DateTime => "date",
        ElementType::Null => "null",
        ElementType::RegularExpression => "regex",
        ElementType::DbPointer => "dbPointer",
        ElementType::JavaScriptCode => "javascript",
        ElementType::Symbol => "symbol",
        ElementType::JavaScriptCodeWithScope => "javascriptWithScope",
        ElementType::Int32 => "int",
        ElementType::Timestamp => "timestamp",
        ElementType::Int64 => "long",
        ElementType::Decimal128 => "decimal",
        ElementType::MinKey => "minKey",
        ElementType::MaxKey => "maxKey",
    }
}

/// Why a document cannot become a row.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// `bson` could not frame or decode the document.
    #[error("document {row} of the result cannot be read: {reason}")]
    Malformed {
        /// The document's ordinal in the result, from 1.
        row: u64,
        /// What `bson` or the Extended JSON writer said.
        reason: String,
    },
    /// A value whose type its inferred column does not hold.
    #[error(
        "document {row} has a {found} in field {field:?}, whose column was inferred as {expected} \
         from the first {sample} documents; the rows before it are shown. The whole document is \
         in the `{DOCUMENT_COLUMN}` column of a result that reaches it; to read past it, project \
         the field to one type ({{\"$toString\": \"$field\"}})"
    )]
    TypeOutsideSample {
        /// The document's ordinal in the result, from 1.
        row: u64,
        /// The field's name.
        field: String,
        /// The column's inferred BSON type.
        expected: &'static str,
        /// The value's BSON type.
        found: &'static str,
        /// How many documents the inference read.
        sample: usize,
    },
}

/// Distinct sampled keys past the column cap that are counted, by their
/// 64-bit hash; past them, the count is a lower bound declared with a
/// trailing `+`. About a MiB of hashes at most, whatever the sample.
pub const OMITTED_COUNTED: usize = 1 << 16;

/// The sampled keys that got no column, counted without being kept.
#[derive(Default)]
struct Omitted {
    seen: std::collections::HashSet<u64>,
    saturated: bool,
    hasher: std::hash::RandomState,
}

impl Omitted {
    fn count(&mut self, key: &str) {
        if self.seen.len() >= OMITTED_COUNTED {
            self.saturated = true;
            return;
        }
        self.seen
            .insert(std::hash::BuildHasher::hash_one(&self.hasher, key));
    }

    /// The value of [`META_OMITTED_FIELDS`]: `12`, or `65536+` past the
    /// count.
    fn declared(&self) -> String {
        if self.saturated {
            format!("{}+", self.seen.len())
        } else {
            self.seen.len().to_string()
        }
    }
}

/// What the sample says about one field.
#[derive(Debug, Clone)]
struct Observed {
    name: String,
    kind: Option<(ElementType, Option<u8>)>,
    mixed: bool,
}

/// The columns of a result, decided once from its sample.
#[derive(Debug, Clone)]
pub struct Plan {
    columns: Vec<(String, Kind)>,
    index: HashMap<String, usize>,
    schema: SchemaRef,
    sample: usize,
}

impl Plan {
    /// Infers the columns from the sampled documents, in the order their
    /// fields first appear.
    ///
    /// # Errors
    /// [`DecodeError::Malformed`] for a sampled document `bson` cannot read.
    pub fn infer<'a>(
        sample: impl IntoIterator<Item = &'a RawDocument>,
    ) -> Result<Self, DecodeError> {
        let mut observed: Vec<Observed> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();
        let mut omitted = Omitted::default();
        let mut documents = 0_usize;
        for (ordinal, document) in sample.into_iter().enumerate() {
            documents = documents.saturating_add(1);
            let row = u64::try_from(ordinal).unwrap_or(u64::MAX).saturating_add(1);
            for element in document.iter_elements() {
                let element = element.map_err(|error| DecodeError::Malformed {
                    row,
                    reason: error.to_string(),
                })?;
                let key = element.key().as_str();
                let position = match positions.get(key) {
                    Some(&position) => position,
                    // Past the cap a new key gets no column: it is counted,
                    // never kept, so a sample of a million distinct keys
                    // costs the cap, not a million strings.
                    None if observed.len() >= MAX_FIELD_COLUMNS => {
                        omitted.count(key);
                        continue;
                    }
                    None => {
                        let position = observed.len();
                        positions.insert(key.to_owned(), position);
                        observed.push(Observed {
                            name: key.to_owned(),
                            kind: None,
                            mixed: false,
                        });
                        position
                    }
                };
                let element_type = element.element_type();
                if element_type == ElementType::Null {
                    continue;
                }
                let subtype = if element_type == ElementType::Binary {
                    match element.value() {
                        Ok(RawBsonRef::Binary(binary)) => Some(ejson::subtype_code(binary.subtype)),
                        Ok(_) => None,
                        Err(error) => {
                            return Err(DecodeError::Malformed {
                                row,
                                reason: error.to_string(),
                            });
                        }
                    }
                } else {
                    None
                };
                if let Some(field) = observed.get_mut(position) {
                    match field.kind {
                        None => field.kind = Some((element_type, subtype)),
                        Some(seen) if seen != (element_type, subtype) => field.mixed = true,
                        Some(_) => {}
                    }
                }
            }
        }
        Ok(Self::from_observed(observed, documents, &omitted))
    }

    fn from_observed(observed: Vec<Observed>, sample: usize, omitted: &Omitted) -> Self {
        let mut columns = Vec::with_capacity(observed.len().min(MAX_FIELD_COLUMNS));
        let mut index = HashMap::new();
        for field in observed.into_iter().take(MAX_FIELD_COLUMNS) {
            let kind = match (field.kind, field.mixed) {
                (Some((element, subtype)), false) => Kind::of(element, subtype),
                (Some(_), true) => Kind::Json(None),
                (None, _) => Kind::Json(Some("null")),
            };
            index.insert(field.name.clone(), columns.len());
            columns.push((field.name, kind));
        }
        let mut document_name = DOCUMENT_COLUMN.to_owned();
        while index.contains_key(&document_name) {
            document_name.push('#');
        }
        let mut fields: Vec<Field> = columns
            .iter()
            .map(|(name, kind)| {
                let mut metadata = HashMap::from([
                    (META_BSON_TYPE.to_owned(), kind.bson_type().to_owned()),
                    (META_INFERRED.to_owned(), "true".to_owned()),
                ]);
                if let Kind::Binary(subtype) = kind {
                    metadata.insert(META_BSON_SUBTYPE.to_owned(), format!("{subtype:02x}"));
                }
                if matches!(kind, Kind::Json(_)) {
                    metadata.insert(META_ENCODING.to_owned(), "relaxed-extjson".to_owned());
                }
                Field::new(name, kind.data_type(), true).with_metadata(metadata)
            })
            .collect();
        fields.push(
            Field::new(&document_name, DataType::Utf8, false).with_metadata(HashMap::from([
                (META_BSON_TYPE.to_owned(), "object".to_owned()),
                (META_ENCODING.to_owned(), "canonical-extjson".to_owned()),
            ])),
        );
        let schema = Schema::new(fields).with_metadata(HashMap::from([
            (META_SAMPLE_DOCUMENTS.to_owned(), sample.to_string()),
            (META_OMITTED_FIELDS.to_owned(), omitted.declared()),
        ]));
        Self {
            columns,
            index,
            schema: Arc::new(schema),
            sample,
        }
    }

    /// The field columns, in order: name, BSON type alias, binary subtype.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &'static str, Option<u8>)> {
        self.columns.iter().map(|(name, kind)| {
            let subtype = match kind {
                Kind::Binary(subtype) => Some(*subtype),
                _ => None,
            };
            (name.as_str(), kind.bson_type(), subtype)
        })
    }

    /// How many documents the inference read.
    #[must_use]
    pub const fn sample_size(&self) -> usize {
        self.sample
    }

    /// The schema every batch of the result follows.
    #[must_use]
    pub fn schema(&self) -> SchemaRef {
        SchemaRef::clone(&self.schema)
    }

    /// An empty assembler for this plan.
    #[must_use]
    pub fn assembler(&self) -> Assembler {
        Assembler {
            plan: self.clone(),
            columns: self
                .columns
                .iter()
                .map(|(_, kind)| Builder::new(*kind))
                .collect(),
            document: StringBuilder::new(),
            filled: vec![false; self.columns.len()],
            rows: 0,
            bytes: 0,
        }
    }
}

/// One column under construction.
#[derive(Debug)]
enum Builder {
    Double(Float64Builder),
    Text(StringBuilder),
    Int32(Int32Builder),
    Int64(Int64Builder),
    Boolean(BooleanBuilder),
    Date(TimestampMillisecondBuilder),
    Binary(BinaryBuilder),
}

impl Builder {
    fn new(kind: Kind) -> Self {
        match kind {
            Kind::Double => Self::Double(Float64Builder::new()),
            Kind::Int32 => Self::Int32(Int32Builder::new()),
            Kind::Int64 => Self::Int64(Int64Builder::new()),
            Kind::Boolean => Self::Boolean(BooleanBuilder::new()),
            Kind::Date => Self::Date(TimestampMillisecondBuilder::new().with_timezone("UTC")),
            Kind::Binary(_) => Self::Binary(BinaryBuilder::new()),
            Kind::String | Kind::ObjectId | Kind::Decimal | Kind::Json(_) => {
                Self::Text(StringBuilder::new())
            }
        }
    }

    fn append_null(&mut self) {
        match self {
            Self::Double(b) => b.append_null(),
            Self::Text(b) => b.append_null(),
            Self::Int32(b) => b.append_null(),
            Self::Int64(b) => b.append_null(),
            Self::Boolean(b) => b.append_null(),
            Self::Date(b) => b.append_null(),
            Self::Binary(b) => b.append_null(),
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            Self::Double(b) => Arc::new(b.finish()),
            Self::Text(b) => Arc::new(b.finish()),
            Self::Int32(b) => Arc::new(b.finish()),
            Self::Int64(b) => Arc::new(b.finish()),
            Self::Boolean(b) => Arc::new(b.finish()),
            Self::Date(b) => Arc::new(b.finish()),
            Self::Binary(b) => Arc::new(b.finish()),
        }
    }
}

/// Accumulates documents into one batch, counting its bytes.
#[derive(Debug)]
pub struct Assembler {
    plan: Plan,
    columns: Vec<Builder>,
    document: StringBuilder,
    filled: Vec<bool>,
    rows: usize,
    bytes: usize,
}

impl Assembler {
    /// Appends one document as a row. `row` is its ordinal in the result,
    /// from 1, for the error message.
    ///
    /// # Errors
    /// [`DecodeError`] naming the document. Everything is checked before
    /// anything is appended: a refused document leaves the rows already
    /// accumulated whole, and they can still be finished into a batch.
    pub fn push(&mut self, document: &RawDocument, row: u64) -> Result<(), DecodeError> {
        let malformed = |reason: String| DecodeError::Malformed { row, reason };
        self.filled.iter_mut().for_each(|filled| *filled = false);
        let mut cells: Vec<(usize, Cell<'_>)> = Vec::new();
        for element in document.iter_elements() {
            let element = element.map_err(|error| malformed(error.to_string()))?;
            let Some(&position) = self.plan.index.get(element.key().as_str()) else {
                continue;
            };
            // A key repeated inside one document: the first value is the
            // column's, both are in the canonical column.
            if self.filled.get(position).copied().unwrap_or(true) {
                continue;
            }
            let value = element
                .value()
                .map_err(|error| malformed(error.to_string()))?;
            let Some((name, kind)) = self.plan.columns.get(position) else {
                continue;
            };
            let cell = cell(*kind, value).map_err(|failure| match failure {
                Failure::Type(found) => DecodeError::TypeOutsideSample {
                    row,
                    field: name.clone(),
                    expected: kind.bson_type(),
                    found,
                    sample: self.plan.sample,
                },
                Failure::Text(reason) => malformed(reason),
            })?;
            cells.push((position, cell));
            if let Some(filled) = self.filled.get_mut(position) {
                *filled = true;
            }
        }
        let text = ejson::document(document, Mode::Canonical)
            .map_err(|error| malformed(error.to_string()))?;

        for (position, cell) in cells {
            if let Some(builder) = self.columns.get_mut(position) {
                self.bytes = self.bytes.saturating_add(cell.append_to(builder));
            }
        }
        for (builder, filled) in self.columns.iter_mut().zip(&self.filled) {
            if !filled {
                builder.append_null();
            }
        }
        self.bytes = self.bytes.saturating_add(text.len());
        self.document.append_value(text);
        self.rows = self.rows.saturating_add(1);
        Ok(())
    }

    /// Rows accumulated since the last batch.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Approximate bytes accumulated since the last batch.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// Closes the batch and starts the next one.
    ///
    /// # Errors
    /// Arrow's, if the columns disagree with the schema — a driver bug.
    pub fn finish(&mut self) -> Result<RecordBatch, arrow::error::ArrowError> {
        let mut arrays: Vec<ArrayRef> = self.columns.iter_mut().map(Builder::finish).collect();
        arrays.push(Arc::new(self.document.finish()));
        self.rows = 0;
        self.bytes = 0;
        RecordBatch::try_new(self.plan.schema(), arrays)
    }
}

/// Why a value did not fit its column.
enum Failure {
    /// The value's BSON type, which the column does not hold.
    Type(&'static str),
    /// The value could not be written as text.
    Text(String),
}

/// One value, checked against its column and ready to append.
enum Cell<'a> {
    Null,
    Double(f64),
    Int32(i32),
    Int64(i64),
    Boolean(bool),
    Date(i64),
    Text(std::borrow::Cow<'a, str>),
    Bytes(&'a [u8]),
}

impl Cell<'_> {
    /// Appends the value; returns the bytes it added. A builder of another
    /// kind than the cell's is a driver bug the plan rules out: the value is
    /// then appended as null rather than panicking.
    fn append_to(self, builder: &mut Builder) -> usize {
        match (builder, self) {
            (Builder::Double(b), Self::Double(v)) => b.append_value(v),
            (Builder::Int32(b), Self::Int32(v)) => b.append_value(v),
            (Builder::Int64(b), Self::Int64(v)) => b.append_value(v),
            (Builder::Boolean(b), Self::Boolean(v)) => b.append_value(v),
            (Builder::Date(b), Self::Date(v)) => b.append_value(v),
            (Builder::Text(b), Self::Text(v)) => {
                b.append_value(&v);
                return v.len();
            }
            (Builder::Binary(b), Self::Bytes(v)) => {
                b.append_value(v);
                return v.len();
            }
            (builder, _) => builder.append_null(),
        }
        8
    }
}

/// Checks one value against its column's kind.
fn cell(kind: Kind, value: RawBsonRef<'_>) -> Result<Cell<'_>, Failure> {
    let found = alias(value.element_type());
    Ok(match (kind, value) {
        (_, RawBsonRef::Null) => Cell::Null,
        (Kind::Double, RawBsonRef::Double(v)) => Cell::Double(v),
        (Kind::Int32, RawBsonRef::Int32(v)) => Cell::Int32(v),
        (Kind::Int64, RawBsonRef::Int64(v)) => Cell::Int64(v),
        (Kind::Boolean, RawBsonRef::Boolean(v)) => Cell::Boolean(v),
        (Kind::Date, RawBsonRef::DateTime(v)) => Cell::Date(v.timestamp_millis()),
        (Kind::String, RawBsonRef::String(v)) => Cell::Text(v.into()),
        (Kind::ObjectId, RawBsonRef::ObjectId(v)) => Cell::Text(v.to_hex().into()),
        (Kind::Decimal, RawBsonRef::Decimal128(v)) => Cell::Text(v.to_string().into()),
        (Kind::Binary(subtype), RawBsonRef::Binary(v)) => {
            if ejson::subtype_code(v.subtype) != subtype {
                return Err(Failure::Type("binData of another subtype"));
            }
            Cell::Bytes(v.bytes)
        }
        (Kind::Json(_), value) => Cell::Text(
            ejson::value(value, Mode::Relaxed)
                .map_err(|error| Failure::Text(error.to_string()))?
                .into(),
        ),
        _ => return Err(Failure::Type(found)),
    })
}

#[cfg(test)]
mod tests;
