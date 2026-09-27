//! From PostgreSQL's binary format to an Arrow `RecordBatch`.
//!
//! This is the product's **per-row, per-value** path: what is written here runs
//! once per cell of a ten-million-row result. One allocation per value here is
//! a design defect, not an optimization to postpone
//! ([rust.md](../../../.claude/rules/rust.md)).
//!
//! # The three rules this module holds
//!
//! **Nothing panics.** The bytes come from the network. No `unwrap`, no slice
//! indexing, no `as` on an integer that can overflow; a buffer that is too
//! short returns a named error ([I-09](../../../CLAUDE.md#i-09)). The reader of
//! the `numeric` module exists exactly for that: `bytes::Buf` panics on a short
//! buffer.
//!
//! **No value enters an error message.** Errors name the column's rank, the
//! PostgreSQL type — sanitized — and what was wrong. Never the data
//! ([I-03](../../../CLAUDE.md#i-03)).
//!
//! **An unknown type does not make the query fail.** It becomes text if it is
//! valid UTF-8, a hexadecimal transcription otherwise. The only values that
//! really make a row fail are those Arrow cannot represent *without lying*: an
//! infinite date, an interval longer than 292 years in microseconds, a
//! multi-dimensional array. Rendering them `NULL` would be a silent lie about
//! real data.
//!
//! # Batch sizing
//!
//! [`BatchAssembler::bytes`] counts the **data bytes** seen on the wire, not
//! the rows. A thousand rows each carrying a one-megabyte BLOB make a gigabyte:
//! a batch sized by row count works on demo tables and triggers the OOM on real
//! ones.

use std::fmt::Write as _;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryBuilder, BooleanBuilder, Date32Builder, Float32Builder, Float64Builder,
    Int16Builder, Int32Builder, Int64Builder, IntervalMonthDayNanoBuilder, ListArray,
    StringBuilder, Time64MicrosecondBuilder, TimestampMicrosecondBuilder, UInt32Builder,
};
use arrow::buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow::datatypes::{Field, FieldRef, IntervalMonthDayNano, SchemaRef};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use sqlx::Column as _;
use sqlx::Row as _;
use sqlx::TypeInfo as _;
use sqlx::postgres::{PgRow, PgValueFormat};

use crate::numeric::{Lecteur, render_binary};
use crate::types::PgDecoding;

/// Microseconds between the Unix epoch and PostgreSQL's (2000-01-01).
const EPOCH_SHIFT_MICROS: i64 = 946_684_800_000_000;
/// Days between the Unix epoch and PostgreSQL's.
const EPOCH_SHIFT_DAYS: i32 = 10_957;
/// Bytes of a `uuid` on the wire.
const UUID_LEN: usize = 16;
/// Version expected at the head of a `jsonb`.
const JSONB_VERSION: u8 = 1;

/// What can go wrong decoding a server response.
///
/// No variant carries a value coming from the database: only a column rank, a
/// sanitized type name, and a `&'static str` detail.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// A value could not be represented.
    #[error("column {ordinal} of type `{pg_type}`: {detail}")]
    Value {
        /// Rank of the column in the result, from zero.
        ordinal: usize,
        /// The PostgreSQL type, sanitized.
        pg_type: String,
        /// What was wrong, without repeating the value.
        detail: &'static str,
    },

    /// The row could not be read: rank out of bounds, missing metadata.
    #[error("unreadable row: {detail}")]
    Row {
        /// What was wrong.
        detail: String,
    },

    /// Arrow refused the built batch. It is a driver bug: the schema and the
    /// column builders have diverged.
    #[error("invalid Arrow batch: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

/// Makes a type name showable in an error message.
///
/// A type name comes from the server — hence from untrusted input: nothing
/// prevents a user type from being named with terminal control sequences. What
/// comes out of here is bounded and restricted to a harmless alphabet.
fn sanitize_type(name: &str) -> String {
    const MAX: usize = 64;
    let acceptable = !name.is_empty()
        && name.len() <= MAX
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '[' | ']' | '"'));
    if acceptable {
        name.to_owned()
    } else {
        "<unrepresentable type>".to_owned()
    }
}

/// Accumulates PostgreSQL rows into Arrow batches.
///
/// Reusable: [`BatchAssembler::finish`] empties the builders and returns the
/// batch, the assembler starts over for the next one. That is what lets a
/// stream never materialize more than one batch at a time
/// ([I-06](../../../CLAUDE.md#i-06)).
#[derive(Debug)]
pub struct BatchAssembler {
    schema: SchemaRef,
    columns: Vec<ColumnBuilder>,
    rows: usize,
    bytes: usize,
}

impl BatchAssembler {
    /// Prepares an assembler for this schema and this decoding plan.
    ///
    /// Both come from [`crate::types::schema_for`] and are aligned by
    /// construction; a divergence would be a bug, and [`Self::finish`] would
    /// report it as such.
    #[must_use]
    pub fn new(schema: SchemaRef, decodings: &[PgDecoding]) -> Self {
        let columns = decodings.iter().map(ColumnBuilder::for_decoding).collect();
        Self {
            schema,
            columns,
            rows: 0,
            bytes: 0,
        }
    }

    /// Rows accumulated since the last batch.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Data bytes accumulated since the last batch.
    ///
    /// It is the measure that must decide when to cut a batch — not
    /// [`Self::rows`].
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// Nothing has been accumulated since the last batch.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// Adds a row.
    ///
    /// # Errors
    /// [`DecodeError::Row`] if the row does not have the expected number of
    /// columns, [`DecodeError::Value`] if a value is not representable.
    pub fn push(&mut self, row: &PgRow) -> Result<(), DecodeError> {
        if row.columns().len() != self.columns.len() {
            return Err(DecodeError::Row {
                detail: format!(
                    "{} columns received, {} announced by the schema",
                    row.columns().len(),
                    self.columns.len()
                ),
            });
        }

        for (ordinal, colonne) in self.columns.iter_mut().enumerate() {
            let brute = row.try_get_raw(ordinal).map_err(|err| DecodeError::Row {
                detail: err.to_string(),
            })?;
            let format = brute.format();
            // `as_bytes` returns an error on a missing value; that is how `sqlx`
            // distinguishes NULL, not through a variant.
            let octets = brute.as_bytes().ok();
            self.bytes = self.bytes.saturating_add(octets.map_or(0, <[u8]>::len));

            // The type name is composed only on the error path: here, one
            // allocation per cell would cost the budget of a whole result.
            if let Err(detail) = colonne.append(format, octets) {
                return Err(DecodeError::Value {
                    ordinal,
                    pg_type: sanitize_type(
                        row.columns()
                            .get(ordinal)
                            .map_or("unknown", |c| c.type_info().name()),
                    ),
                    detail,
                });
            }
        }

        self.rows = self.rows.saturating_add(1);
        Ok(())
    }

    /// Closes the current batch and starts over empty.
    ///
    /// # Errors
    /// [`DecodeError::Arrow`] if the built batch does not match the schema —
    /// which would be a driver bug, not faulty data.
    pub fn finish(&mut self) -> Result<RecordBatch, DecodeError> {
        let mut tableaux = Vec::with_capacity(self.columns.len());
        for colonne in &mut self.columns {
            tableaux.push(colonne.finish()?);
        }
        let lignes = self.rows;
        self.rows = 0;
        self.bytes = 0;

        // A result without columns — an `INSERT`, a `SET` — still has a row
        // count, which `try_new` cannot infer from zero columns.
        if tableaux.is_empty() {
            let options = RecordBatchOptions::new().with_row_count(Some(lignes));
            return Ok(RecordBatch::try_new_with_options(
                Arc::clone(&self.schema),
                tableaux,
                &options,
            )?);
        }
        Ok(RecordBatch::try_new(Arc::clone(&self.schema), tableaux)?)
    }
}

/// An Arrow column builder, with what it takes to pour PostgreSQL binary into
/// it.
#[derive(Debug)]
enum ColumnBuilder {
    Bool(BooleanBuilder),
    Int16(Int16Builder),
    Int32(Int32Builder),
    Int64(Int64Builder),
    UInt32(UInt32Builder),
    Float32(Float32Builder),
    Float64(Float64Builder),
    /// All `Utf8` columns, distinguished by how the text is obtained.
    Text(StringBuilder, TextSource),
    Binary(BinaryBuilder),
    Date(Date32Builder),
    Time(Time64MicrosecondBuilder),
    Timestamp(TimestampMicrosecondBuilder),
    Interval(IntervalMonthDayNanoBuilder),
    List(Box<ListColumn>),
}

/// Where the text of a `Utf8` column comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextSource {
    /// The bytes **are** the text.
    Raw,
    /// A version byte, then the text.
    Jsonb,
    /// The `numeric` binary format, rendered as exact decimal.
    Numeric,
    /// Sixteen bytes, rendered in canonical form.
    Uuid,
    /// Time and offset, rendered in ISO 8601.
    TimeTz,
}

/// A column of lists, built by hand.
///
/// Arrow's `ListBuilder` would require going through `Box<dyn ArrayBuilder>`
/// and back down with `downcast_mut` for every element; assembling the offsets
/// oneself avoids that detour and keeps the child's type.
#[derive(Debug)]
struct ListColumn {
    field: FieldRef,
    child: ColumnBuilder,
    /// Cumulative offsets, starting at zero.
    offsets: Vec<i32>,
    /// One entry per row: is the list itself non-null?
    validity: Vec<bool>,
    /// Elements poured into the child since the last batch.
    elements: i32,
}

impl ColumnBuilder {
    /// The builder matching a decoding plan.
    fn for_decoding(decoding: &PgDecoding) -> Self {
        match decoding {
            PgDecoding::Bool => Self::Bool(BooleanBuilder::new()),
            PgDecoding::Int16 => Self::Int16(Int16Builder::new()),
            PgDecoding::Int32 => Self::Int32(Int32Builder::new()),
            PgDecoding::Int64 => Self::Int64(Int64Builder::new()),
            PgDecoding::UInt32 => Self::UInt32(UInt32Builder::new()),
            PgDecoding::Float32 => Self::Float32(Float32Builder::new()),
            PgDecoding::Float64 => Self::Float64(Float64Builder::new()),
            PgDecoding::Text => Self::Text(StringBuilder::new(), TextSource::Raw),
            PgDecoding::Jsonb => Self::Text(StringBuilder::new(), TextSource::Jsonb),
            PgDecoding::Numeric => Self::Text(StringBuilder::new(), TextSource::Numeric),
            PgDecoding::Uuid => Self::Text(StringBuilder::new(), TextSource::Uuid),
            PgDecoding::TimeTz => Self::Text(StringBuilder::new(), TextSource::TimeTz),
            PgDecoding::Opaque | PgDecoding::Bytes => Self::Binary(BinaryBuilder::new()),
            PgDecoding::Date => Self::Date(Date32Builder::new()),
            PgDecoding::Time => Self::Time(Time64MicrosecondBuilder::new()),
            PgDecoding::Timestamp => Self::Timestamp(TimestampMicrosecondBuilder::new()),
            PgDecoding::TimestampTz => {
                Self::Timestamp(TimestampMicrosecondBuilder::new().with_timezone("UTC"))
            }
            PgDecoding::Interval => Self::Interval(IntervalMonthDayNanoBuilder::new()),
            PgDecoding::List(element) => Self::List(Box::new(ListColumn {
                field: Arc::new(Field::new("item", element.arrow_type(), true)),
                child: Self::for_decoding(element),
                offsets: vec![0],
                validity: Vec::new(),
                elements: 0,
            })),
        }
    }

    /// Adds a value, or `NULL` when `bytes` is absent.
    fn append(&mut self, format: PgValueFormat, bytes: Option<&[u8]>) -> Result<(), &'static str> {
        let Some(bytes) = bytes else {
            self.append_null();
            return Ok(());
        };

        // The driver uses only the extended protocol, where the server always
        // answers in binary. The text format can only come from a path that
        // does not exist yet; textual columns support it for free, the others
        // refuse it rather than guess.
        if format == PgValueFormat::Text {
            return match self {
                Self::Text(builder, _) => {
                    let texte =
                        std::str::from_utf8(bytes).map_err(|_| "non-UTF-8 text from the server")?;
                    builder.append_value(texte);
                    Ok(())
                }
                _ => Err("the simple protocol's text format is not decoded"),
            };
        }

        match self {
            Self::Bool(builder) => {
                let octet = bytes.first().ok_or("empty boolean")?;
                builder.append_value(*octet != 0);
            }
            Self::Int16(builder) => builder.append_value(read_i16(bytes)?),
            Self::Int32(builder) => builder.append_value(read_i32(bytes)?),
            Self::Int64(builder) => builder.append_value(read_i64(bytes)?),
            Self::UInt32(builder) => builder.append_value(read_u32(bytes)?),
            Self::Float32(builder) => {
                let octets: [u8; 4] = bytes.try_into().map_err(|_| "float4 of unexpected size")?;
                builder.append_value(f32::from_be_bytes(octets));
            }
            Self::Float64(builder) => {
                let octets: [u8; 8] = bytes.try_into().map_err(|_| "float8 of unexpected size")?;
                builder.append_value(f64::from_be_bytes(octets));
            }
            Self::Text(builder, source) => {
                render_text(builder, *source, bytes)?;
            }
            Self::Binary(builder) => builder.append_value(bytes),
            Self::Date(builder) => builder.append_value(read_date(bytes)?),
            Self::Time(builder) => builder.append_value(read_time(bytes)?),
            Self::Timestamp(builder) => builder.append_value(read_timestamp(bytes)?),
            Self::Interval(builder) => builder.append_value(read_interval(bytes)?),
            Self::List(colonne) => colonne.append(bytes)?,
        }
        Ok(())
    }

    /// Adds a missing value.
    fn append_null(&mut self) {
        match self {
            Self::Bool(builder) => builder.append_null(),
            Self::Int16(builder) => builder.append_null(),
            Self::Int32(builder) => builder.append_null(),
            Self::Int64(builder) => builder.append_null(),
            Self::UInt32(builder) => builder.append_null(),
            Self::Float32(builder) => builder.append_null(),
            Self::Float64(builder) => builder.append_null(),
            Self::Text(builder, _) => builder.append_null(),
            Self::Binary(builder) => builder.append_null(),
            Self::Date(builder) => builder.append_null(),
            Self::Time(builder) => builder.append_null(),
            Self::Timestamp(builder) => builder.append_null(),
            Self::Interval(builder) => builder.append_null(),
            Self::List(colonne) => colonne.append_null(),
        }
    }

    /// Closes the column and starts over empty.
    fn finish(&mut self) -> Result<ArrayRef, DecodeError> {
        let tableau: ArrayRef = match self {
            Self::Bool(builder) => Arc::new(builder.finish()),
            Self::Int16(builder) => Arc::new(builder.finish()),
            Self::Int32(builder) => Arc::new(builder.finish()),
            Self::Int64(builder) => Arc::new(builder.finish()),
            Self::UInt32(builder) => Arc::new(builder.finish()),
            Self::Float32(builder) => Arc::new(builder.finish()),
            Self::Float64(builder) => Arc::new(builder.finish()),
            Self::Text(builder, _) => Arc::new(builder.finish()),
            Self::Binary(builder) => Arc::new(builder.finish()),
            Self::Date(builder) => Arc::new(builder.finish()),
            Self::Time(builder) => Arc::new(builder.finish()),
            Self::Timestamp(builder) => Arc::new(builder.finish()),
            Self::Interval(builder) => Arc::new(builder.finish()),
            Self::List(colonne) => colonne.finish()?,
        };
        Ok(tableau)
    }
}

impl ListColumn {
    /// Adds a one-dimensional PostgreSQL array.
    fn append(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        let elements = split_array(bytes)?;
        for element in elements {
            self.child.append(PgValueFormat::Binary, element)?;
            self.elements = self
                .elements
                .checked_add(1)
                .ok_or("more than 2 billion elements in one batch")?;
        }
        self.offsets.push(self.elements);
        self.validity.push(true);
        Ok(())
    }

    /// Adds a missing list. `NULL` and `{}` are not the same: the first has no
    /// elements, the second has zero.
    fn append_null(&mut self) {
        self.offsets.push(self.elements);
        self.validity.push(false);
    }

    /// Closes the column and starts over empty.
    fn finish(&mut self) -> Result<ArrayRef, DecodeError> {
        let valeurs = self.child.finish()?;
        let decalages = std::mem::replace(&mut self.offsets, vec![0]);
        let validite = std::mem::take(&mut self.validity);
        self.elements = 0;

        let tableau = ListArray::try_new(
            Arc::clone(&self.field),
            OffsetBuffer::new(ScalarBuffer::from(decalages)),
            valeurs,
            Some(NullBuffer::from(validite)),
        )?;
        Ok(Arc::new(tableau))
    }
}

/// Writes the textual representation of a value into the builder.
fn render_text(
    builder: &mut StringBuilder,
    source: TextSource,
    bytes: &[u8],
) -> Result<(), &'static str> {
    match source {
        TextSource::Raw => {
            let texte = std::str::from_utf8(bytes).map_err(|_| "non-UTF-8 text from the server")?;
            builder.append_value(texte);
        }
        TextSource::Jsonb => {
            let (version, suite) = bytes.split_first().ok_or("empty jsonb")?;
            if *version != JSONB_VERSION {
                return Err("unknown jsonb version");
            }
            let texte = std::str::from_utf8(suite).map_err(|_| "non-UTF-8 jsonb")?;
            builder.append_value(texte);
        }
        TextSource::Numeric => {
            let rendu = render_binary(bytes).ok_or("unreadable numeric")?;
            builder.append_value(&rendu);
        }
        TextSource::Uuid => {
            let octets: [u8; UUID_LEN] = bytes.try_into().map_err(|_| "uuid of unexpected size")?;
            let identifiant = uuid::Uuid::from_bytes(octets);
            let mut tampon = uuid::Uuid::encode_buffer();
            let canonique: &str = identifiant.hyphenated().encode_lower(&mut tampon);
            builder.append_value(canonique);
        }
        TextSource::TimeTz => {
            builder.append_value(&render_timetz(bytes)?);
        }
    }
    Ok(())
}

/// Renders a `timetz` in ISO 8601: `14:30:00.250000+02:00`.
///
/// PostgreSQL transmits the offset in **seconds west** of UTC; ISO notation
/// counts it east. The sign flips, and that is exactly the kind of inversion
/// that shifts data by two hours without anyone seeing it
/// ([DRIVER-CONTRACT §7](../../../docs/DRIVER-CONTRACT.md)).
fn render_timetz(bytes: &[u8]) -> Result<String, &'static str> {
    let mut lecteur = Lecteur::new(bytes);
    let micros = lecteur.i64().ok_or("truncated timetz")?;
    let ouest = lecteur.i32().ok_or("timetz without an offset")?;

    if !(0..=86_400_000_000).contains(&micros) {
        return Err("timetz outside the bounds of a day");
    }
    let secondes = micros / 1_000_000;
    let reste = micros % 1_000_000;
    let heures = secondes / 3_600;
    let minutes = (secondes % 3_600) / 60;
    let sec = secondes % 60;

    let est = ouest.checked_neg().ok_or("nonsensical timetz offset")?;
    let signe = if est < 0 { '-' } else { '+' };
    let absolu = est.unsigned_abs();
    let heures_offset = absolu / 3_600;
    let minutes_offset = (absolu % 3_600) / 60;

    let mut sortie = String::with_capacity(24);
    let _ = write!(sortie, "{heures:02}:{minutes:02}:{sec:02}");
    if reste != 0 {
        let _ = write!(sortie, ".{reste:06}");
    }
    let _ = write!(sortie, "{signe}{heures_offset:02}:{minutes_offset:02}");
    Ok(sortie)
}

/// A big-endian `int2`.
fn read_i16(bytes: &[u8]) -> Result<i16, &'static str> {
    let octets: [u8; 2] = bytes.try_into().map_err(|_| "int2 of unexpected size")?;
    Ok(i16::from_be_bytes(octets))
}

/// A big-endian `int4`.
fn read_i32(bytes: &[u8]) -> Result<i32, &'static str> {
    let octets: [u8; 4] = bytes.try_into().map_err(|_| "int4 of unexpected size")?;
    Ok(i32::from_be_bytes(octets))
}

/// A big-endian `int8`.
fn read_i64(bytes: &[u8]) -> Result<i64, &'static str> {
    let octets: [u8; 8] = bytes.try_into().map_err(|_| "int8 of unexpected size")?;
    Ok(i64::from_be_bytes(octets))
}

/// A big-endian `oid`.
fn read_u32(bytes: &[u8]) -> Result<u32, &'static str> {
    let octets: [u8; 4] = bytes.try_into().map_err(|_| "oid of unexpected size")?;
    Ok(u32::from_be_bytes(octets))
}

/// A `date`: days since 2000-01-01, brought back to the Unix epoch.
fn read_date(bytes: &[u8]) -> Result<i32, &'static str> {
    let jours = read_i32(bytes)?;
    if jours == i32::MIN || jours == i32::MAX {
        // `infinity` and `-infinity` are legal dates in PostgreSQL. No `Date32`
        // value represents them; rendering them as the extreme date would be
        // wrong, and `NULL` would be a lie.
        return Err("infinite date, not representable as Date32 — cast it to text");
    }
    jours
        .checked_add(EPOCH_SHIFT_DAYS)
        .ok_or("date outside the bounds of Date32")
}

/// A `time`: microseconds since midnight.
fn read_time(bytes: &[u8]) -> Result<i64, &'static str> {
    let micros = read_i64(bytes)?;
    if !(0..=86_400_000_000).contains(&micros) {
        return Err("time outside the bounds of a day");
    }
    Ok(micros)
}

/// A `timestamp` or `timestamptz`: microseconds since 2000-01-01, brought back
/// to the Unix epoch.
fn read_timestamp(bytes: &[u8]) -> Result<i64, &'static str> {
    let micros = read_i64(bytes)?;
    if micros == i64::MIN || micros == i64::MAX {
        return Err("infinite timestamp, not representable — cast it to text");
    }
    micros
        .checked_add(EPOCH_SHIFT_MICROS)
        .ok_or("timestamp out of bounds")
}

/// An `interval`: microseconds, days, months — in that order on the wire.
fn read_interval(bytes: &[u8]) -> Result<IntervalMonthDayNano, &'static str> {
    let mut lecteur = Lecteur::new(bytes);
    let micros = lecteur.i64().ok_or("truncated interval")?;
    let jours = lecteur.i32().ok_or("interval without days")?;
    let mois = lecteur.i32().ok_or("interval without months")?;
    let nanos = micros
        .checked_mul(1_000)
        .ok_or("interval longer than 292 years in microseconds, not representable")?;
    Ok(IntervalMonthDayNano::new(mois, jours, nanos))
}

/// Splits a binary PostgreSQL array into elements.
///
/// Returns one slice per element, `None` for a missing element. The element
/// type announced in the header is **ignored**: the decoding plan comes from the
/// `RowDescription`, which is authoritative, and following the header would let
/// a server have anything decoded as anything.
fn split_array(bytes: &[u8]) -> Result<Vec<Option<&[u8]>>, &'static str> {
    let mut lecteur = Lecteur::new(bytes);
    let dimensions = lecteur.i32().ok_or("truncated array header")?;
    let _drapeaux = lecteur.i32().ok_or("truncated array header")?;
    let _type_element = lecteur.u32().ok_or("truncated array header")?;

    if dimensions == 0 {
        return Ok(Vec::new());
    }
    if dimensions != 1 {
        // An Arrow list is one-dimensional. Flattening would lose the shape
        // without saying so; refusing names it.
        return Err("multi-dimensional array, not representable as an Arrow list");
    }

    let longueur = lecteur.i32().ok_or("truncated array dimension")?;
    let _borne_basse = lecteur.i32().ok_or("truncated array dimension")?;
    let longueur = usize::try_from(longueur).map_err(|_| "negative array length")?;

    // Each element costs at least its four length bytes: a value larger than
    // what the buffer can hold is hostile, and `with_capacity` on such a value
    // would exhaust memory.
    if longueur > lecteur.reste().len() / 4 + 1 {
        return Err("array length inconsistent with the buffer received");
    }

    let mut elements = Vec::with_capacity(longueur);
    for _ in 0..longueur {
        let taille = lecteur.i32().ok_or("truncated array element")?;
        if taille == -1 {
            elements.push(None);
            continue;
        }
        let taille = usize::try_from(taille).map_err(|_| "negative element size")?;
        elements.push(Some(
            lecteur.prendre(taille).ok_or("truncated array element")?,
        ));
    }
    Ok(elements)
}

#[cfg(test)]
mod tests {
    use arrow::array::{Array, AsArray, StringArray};
    use arrow::datatypes::{Int32Type, Schema};

    use super::*;

    /// Builds a one-dimensional binary PostgreSQL array.
    fn tableau(elements: &[Option<&[u8]>]) -> Vec<u8> {
        let mut octets = Vec::new();
        octets.extend_from_slice(&1_i32.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        octets.extend_from_slice(&23_u32.to_be_bytes());
        octets.extend_from_slice(
            &i32::try_from(elements.len())
                .expect("short test case")
                .to_be_bytes(),
        );
        octets.extend_from_slice(&1_i32.to_be_bytes());
        for element in elements {
            match element {
                Some(valeur) => {
                    octets.extend_from_slice(
                        &i32::try_from(valeur.len())
                            .expect("short test case")
                            .to_be_bytes(),
                    );
                    octets.extend_from_slice(valeur);
                }
                None => octets.extend_from_slice(&(-1_i32).to_be_bytes()),
            }
        }
        octets
    }

    fn colonne(decoding: &PgDecoding) -> ColumnBuilder {
        ColumnBuilder::for_decoding(decoding)
    }

    fn texte_rendu(source: TextSource, bytes: &[u8]) -> Result<String, &'static str> {
        let mut builder = StringBuilder::new();
        render_text(&mut builder, source, bytes)?;
        let tableau: StringArray = builder.finish();
        Ok(tableau.value(0).to_owned())
    }

    #[test]
    fn integers_read_back_big_endian() {
        assert_eq!(read_i16(&1234_i16.to_be_bytes()), Ok(1234));
        assert_eq!(read_i32(&(-7_i32).to_be_bytes()), Ok(-7));
        assert_eq!(read_i64(&i64::MAX.to_be_bytes()), Ok(i64::MAX));
        assert_eq!(
            read_u32(&4_000_000_000_u32.to_be_bytes()),
            Ok(4_000_000_000)
        );
    }

    #[test]
    fn a_badly_sized_buffer_returns_an_error_not_a_panic() {
        // The bytes come from the network: a server can lie about sizes.
        assert!(read_i32(&[0, 1]).is_err());
        assert!(read_i64(&[]).is_err());
        assert!(read_i16(&[0, 0, 0]).is_err());
    }

    #[test]
    fn a_date_is_brought_back_to_the_unix_epoch() {
        // 2000-01-01 is 0 on the PostgreSQL side and 10,957 on the Arrow side.
        assert_eq!(read_date(&0_i32.to_be_bytes()), Ok(EPOCH_SHIFT_DAYS));
        // 1970-01-01: -10,957 days before the PostgreSQL epoch.
        assert_eq!(read_date(&(-EPOCH_SHIFT_DAYS).to_be_bytes()), Ok(0));
    }

    #[test]
    fn an_infinite_date_is_refused_rather_than_rendered_wrong() {
        // PostgreSQL accepts `infinity`; Date32 does not. Rendering it as the
        // extreme date or as NULL would be a lie about real data.
        let erreur = read_date(&i32::MAX.to_be_bytes()).expect_err("refusal expected");
        assert!(erreur.contains("infinite"), "{erreur}");
        assert!(read_date(&i32::MIN.to_be_bytes()).is_err());
    }

    #[test]
    fn a_timestamp_is_brought_back_to_the_unix_epoch() {
        assert_eq!(read_timestamp(&0_i64.to_be_bytes()), Ok(EPOCH_SHIFT_MICROS));
        assert!(read_timestamp(&i64::MAX.to_be_bytes()).is_err());
    }

    #[test]
    fn a_time_outside_the_bounds_of_a_day_is_refused() {
        assert_eq!(read_time(&0_i64.to_be_bytes()), Ok(0));
        assert_eq!(
            read_time(&86_400_000_000_i64.to_be_bytes()),
            Ok(86_400_000_000)
        );
        assert!(read_time(&(-1_i64).to_be_bytes()).is_err());
        assert!(read_time(&86_400_000_001_i64.to_be_bytes()).is_err());
    }

    #[test]
    fn an_interval_keeps_its_three_components_distinct() {
        // A month is not 30 days and a day is not 24 hours: merging them would
        // distort any calendar computation.
        let mut octets = Vec::new();
        octets.extend_from_slice(&3_600_000_000_i64.to_be_bytes());
        octets.extend_from_slice(&2_i32.to_be_bytes());
        octets.extend_from_slice(&14_i32.to_be_bytes());
        let intervalle = read_interval(&octets).expect("valid interval");
        assert_eq!(intervalle.months, 14);
        assert_eq!(intervalle.days, 2);
        assert_eq!(intervalle.nanoseconds, 3_600_000_000_000);
    }

    #[test]
    fn an_interval_that_overflows_in_nanoseconds_is_refused() {
        let mut octets = Vec::new();
        octets.extend_from_slice(&i64::MAX.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        assert!(read_interval(&octets).is_err());
    }

    #[test]
    fn a_timetz_offset_changes_sign() {
        // PostgreSQL counts seconds west; ISO 8601 east. A botched inversion
        // shifts the data without anything reporting it.
        let mut octets = Vec::new();
        octets.extend_from_slice(&52_200_000_000_i64.to_be_bytes()); // 14:30:00
        octets.extend_from_slice(&(-7_200_i32).to_be_bytes()); // 7200 s east
        assert_eq!(render_timetz(&octets).as_deref(), Ok("14:30:00+02:00"));

        let mut ouest = Vec::new();
        ouest.extend_from_slice(&52_200_000_000_i64.to_be_bytes());
        ouest.extend_from_slice(&18_000_i32.to_be_bytes()); // 5 h west
        assert_eq!(render_timetz(&ouest).as_deref(), Ok("14:30:00-05:00"));
    }

    #[test]
    fn a_timetz_with_microseconds_keeps_them() {
        let mut octets = Vec::new();
        octets.extend_from_slice(&52_200_250_000_i64.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        assert_eq!(
            render_timetz(&octets).as_deref(),
            Ok("14:30:00.250000+00:00")
        );
    }

    #[test]
    fn a_jsonb_loses_its_version_byte_and_nothing_else() {
        let mut octets = vec![JSONB_VERSION];
        octets.extend_from_slice(br#"{"a": 1}"#);
        assert_eq!(
            texte_rendu(TextSource::Jsonb, &octets).as_deref(),
            Ok(r#"{"a": 1}"#)
        );
        // An unknown version is refused: the rest is no longer JSON.
        assert!(texte_rendu(TextSource::Jsonb, &[9, b'{']).is_err());
    }

    #[test]
    fn a_uuid_is_rendered_in_canonical_form() {
        let octets: [u8; 16] = [
            0x67, 0xe5, 0x50, 0x44, 0x10, 0xb1, 0x42, 0x6f, 0x9d, 0x0c, 0x45, 0x1f, 0x8a, 0xd0,
            0x5b, 0x1a,
        ];
        assert_eq!(
            texte_rendu(TextSource::Uuid, &octets).as_deref(),
            Ok("67e55044-10b1-426f-9d0c-451f8ad05b1a")
        );
        assert!(texte_rendu(TextSource::Uuid, &octets[..8]).is_err());
    }

    #[test]
    fn opaque_binary_is_preserved_even_when_it_is_valid_utf8() {
        let mut column = colonne(&PgDecoding::Opaque);
        let bytes = b"\x00\x00\x004";
        column
            .append(PgValueFormat::Binary, Some(bytes))
            .expect("opaque bytes");
        let large = vec![0xab; 9000];
        column
            .append(PgValueFormat::Binary, Some(&large))
            .expect("large opaque value");
        column.append(PgValueFormat::Binary, None).expect("null");
        let array = column.finish().expect("binary column");
        let values = array
            .as_binary_opt::<i32>()
            .expect("raw bytes, not guessed text");
        assert_eq!(values.value(0), bytes);
        assert_eq!(values.value(1), large);
        assert!(values.is_null(2));
    }

    #[test]
    fn an_empty_array_and_a_missing_array_are_not_confused() {
        let mut colonne = colonne(&PgDecoding::List(Box::new(PgDecoding::Int32)));
        // `{}`: zero elements, but the list exists.
        let vide = tableau(&[]);
        colonne
            .append(PgValueFormat::Binary, Some(vide.as_slice()))
            .expect("valid empty array");
        // `NULL`: no list at all.
        colonne.append(PgValueFormat::Binary, None).expect("null");

        let tableau_arrow = colonne.finish().expect("list built");
        assert_eq!(tableau_arrow.len(), 2);
        assert!(!tableau_arrow.is_null(0), "{{}} is not NULL");
        assert!(tableau_arrow.is_null(1));
    }

    #[test]
    fn an_integer_array_keeps_its_elements_and_its_holes() {
        let mut colonne = colonne(&PgDecoding::List(Box::new(PgDecoding::Int32)));
        let un = 1_i32.to_be_bytes();
        let trois = 3_i32.to_be_bytes();
        let octets = tableau(&[Some(&un), None, Some(&trois)]);
        colonne
            .append(PgValueFormat::Binary, Some(octets.as_slice()))
            .expect("valid array");

        let tableau_arrow = colonne.finish().expect("list built");
        let listes = tableau_arrow
            .as_list_opt::<i32>()
            .expect("a column of lists");
        let premiere = listes.value(0);
        let entiers = premiere
            .as_primitive_opt::<Int32Type>()
            .expect("32-bit integers");
        assert_eq!(entiers.len(), 3);
        assert_eq!(entiers.value(0), 1);
        assert!(entiers.is_null(1), "a NULL element stays NULL");
        assert_eq!(entiers.value(2), 3);
    }

    #[test]
    fn a_multi_dimensional_array_is_refused_not_flattened() {
        let mut octets = Vec::new();
        octets.extend_from_slice(&2_i32.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        octets.extend_from_slice(&23_u32.to_be_bytes());
        let erreur = split_array(&octets).expect_err("refusal expected");
        assert!(erreur.contains("dimensional"), "{erreur}");
    }

    #[test]
    fn a_hostile_array_length_does_not_exhaust_memory() {
        let mut octets = Vec::new();
        octets.extend_from_slice(&1_i32.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        octets.extend_from_slice(&23_u32.to_be_bytes());
        octets.extend_from_slice(&i32::MAX.to_be_bytes());
        octets.extend_from_slice(&1_i32.to_be_bytes());
        assert!(split_array(&octets).is_err());
    }

    #[test]
    fn a_hostile_type_name_does_not_go_out_as_is_in_an_error() {
        // A user type can carry terminal control sequences.
        assert_eq!(sanitize_type("int4"), "int4");
        assert_eq!(sanitize_type("BYTEA[]"), "BYTEA[]");
        assert_eq!(sanitize_type("\u{1b}[2J"), "<unrepresentable type>");
        assert_eq!(sanitize_type(""), "<unrepresentable type>");
        assert_eq!(sanitize_type(&"a".repeat(200)), "<unrepresentable type>");
    }

    #[test]
    fn a_result_without_columns_keeps_its_row_count() {
        // An `INSERT` has no column but has affected rows: without
        // `with_row_count`, Arrow would build a zero-row batch.
        let schema = Arc::new(Schema::empty());
        let mut assembleur = BatchAssembler::new(schema, &[]);
        assembleur.rows = 3;
        let lot = assembleur.finish().expect("batch without columns");
        assert_eq!(lot.num_rows(), 3);
        assert_eq!(lot.num_columns(), 0);
    }

    #[test]
    fn the_assembler_starts_over_empty_after_each_batch() {
        let schema = Arc::new(Schema::empty());
        let mut assembleur = BatchAssembler::new(schema, &[]);
        assembleur.rows = 5;
        assembleur.bytes = 4_096;
        let _ = assembleur.finish().expect("batch without columns");
        assert!(assembleur.is_empty());
        assert_eq!(assembleur.bytes(), 0);
    }
}
