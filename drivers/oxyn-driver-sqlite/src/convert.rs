//! From SQLite storage classes to Arrow columns.
//!
//! It is the hard part of the driver, and it comes from a fact other databases do
//! not have: **in SQLite, the type belongs to the value, not to the column.** A
//! `CREATE TABLE t(n INTEGER)` does not prevent `INSERT INTO t VALUES('abc')`
//! from storing text. The declared type is only an *affinity*, that is, a
//! conversion preference.
//!
//! Arrow, however, requires one type per column, **known before the first batch
//! and stable for the whole duration of the stream**
//! ([`Cursor::schema`](oxyn_driver::Cursor::schema)). The two models do not
//! overlap; this module is where the gap is paid for, once, explicitly.
//!
//! # How a column's type is decided
//!
//! 1. **What the first batch saw.** The cursor sets the first batch aside as raw
//!    values and notes, column by column, the storage classes encountered. It is
//!    the most reliable source: it is the data.
//! 2. **The declared affinity**, when no value was seen — an empty column, or
//!    entirely `NULL` in the first batch.
//! 3. **`Utf8`** as a last resort: it is the rendering that loses the least.
//!
//! The merge lattice, when several classes coexist:
//!
//! | Classes seen | Type chosen | Why |
//! |---|---|---|
//! | INTEGER alone | `Int64` | exact |
//! | REAL alone | `Float64` | exact |
//! | TEXT (or anything + TEXT) | `Utf8` | an integer and a float render as text without loss |
//! | INTEGER **and** REAL | `Utf8` | **no `f64` holds every `i64`**: beyond 2⁵³, the conversion corrupts |
//! | INTEGER and/or REAL, **declared NUMERIC** (`DECIMAL(10,2)`) | `Utf8` | SQLite stores `10.00` as INTEGER and `10.50` as REAL: the first batch cannot know which comes later |
//! | anything + BLOB | `Binary` | only bytes contain everything: a blob has no textual rendering |
//!
//! The `INTEGER + REAL → Utf8` case is the one that gets missed: it is exactly
//! "a `u64` beyond 2⁵³ does not survive going through a float"
//! ([drivers.md](../../../.claude/rules/drivers.md)).
//!
//! # What the column says about itself
//!
//! A column whose type was inferred, or whose classes were mixed, **declares** it
//! in the metadata of its Arrow `Field` — see [`METADATA_INFERRED`] and
//! [`METADATA_STORAGE_CLASSES`]. Presenting an inference as a truth from the
//! server makes people write wrong queries with confidence
//! ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)).
//!
//! # The residual case
//!
//! A value appearing **after** the first batch may not fit the chosen type: a
//! BLOB at row ten thousand of a column seen as textual. The cursor then returns
//! [`SqliteError::ColumnConflict`], a permanent error naming the column and the
//! two types. It is noisy — and that is the point: the alternative would be a
//! silently wrong value on screen.

use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

use arrow::array::{ArrayRef, BinaryBuilder, Float64Builder, Int64Builder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use rusqlite::types::ValueRef;

use crate::error::SqliteError;

/// Metadata key of an Arrow `Field`: the column type was **inferred** from the
/// values, not read from a declaration in the schema.
pub const METADATA_INFERRED: &str = "oxyn.inferred";

/// Metadata key of an Arrow `Field`: the type as SQLite declares it
/// (`INTEGER`, `VARCHAR(255)`, `NUMERIC(10,2)`…), when it declares one.
pub const METADATA_DECLARED_TYPE: &str = "oxyn.sqlite.declared_type";

/// Metadata key of an Arrow `Field`: the storage classes actually encountered,
/// separated by `|`, when the column mixes several.
///
/// Its presence signals that a column mixes types and that the chosen rendering
/// is a fallback. The interface must be able to show it.
pub const METADATA_STORAGE_CLASSES: &str = "oxyn.sqlite.storage_classes";

/// The largest integer `f64` represents exactly: 2⁵³.
const EXACT_IN_F64: u64 = 1 << 53;

/// The Arrow type chosen for a column.
///
/// Four values only, because SQLite has only five storage classes and `NULL` is
/// not a type among them. Mapping a declared `DATETIME` to `Timestamp` would be a
/// conversion, not a read: SQLite stores text or a number there depending on what
/// the application wrote, and guessing would shift the data
/// ([`DRIVER-CONTRACT` §7](../../../docs/DRIVER-CONTRACT.md)). The declared type
/// stays readable in [`METADATA_DECLARED_TYPE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// Signed 64-bit integer.
    Int64,
    /// Double-precision float.
    Float64,
    /// UTF-8 text.
    Utf8,
    /// Opaque bytes.
    Binary,
}

impl ColumnKind {
    /// The matching Arrow type.
    #[must_use]
    pub fn data_type(self) -> DataType {
        match self {
            Self::Int64 => DataType::Int64,
            Self::Float64 => DataType::Float64,
            Self::Utf8 => DataType::Utf8,
            Self::Binary => DataType::Binary,
        }
    }

    /// Stable name, the one appearing in an error message.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Int64 => "int64",
            Self::Float64 => "float64",
            Self::Utf8 => "utf8",
            Self::Binary => "binary",
        }
    }
}

impl fmt::Display for ColumnKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The SQLite affinity of a declared type.
///
/// Applies SQLite's five affinity determination rules, **in their order**,
/// including their surprising consequences: `POINT` contains `INT`, so it gets
/// INTEGER affinity. Reproducing the rule is the only right choice — it is what
/// the engine will do with the value.
///
/// Returns `None` when nothing is declared: a column without a declared type has
/// no affinity, it has BLOB affinity in SQLite's sense, but we prefer to say
/// "I don't know" and let the values decide.
#[must_use]
pub fn affinity(declared: Option<&str>) -> Option<ColumnKind> {
    declared_affinity(declared).map(|class| match class {
        Affinity::Integer => ColumnKind::Int64,
        Affinity::Real => ColumnKind::Float64,
        Affinity::Blob => ColumnKind::Binary,
        // SQLite stores the value as integer, float or text depending on what is
        // exact. No Arrow type covers all three; text is the only rendering that
        // loses nothing — it is the case of `DECIMAL(10,2)`.
        Affinity::Text | Affinity::Numeric => ColumnKind::Utf8,
    })
}

/// SQLite's affinity classes, as the five determination rules name them.
///
/// Kept distinct from [`ColumnKind`] because NUMERIC maps to `Utf8` for an empty
/// column yet must also be recognised on its own: a column of that affinity
/// stores `10.00` as an INTEGER and `10.50` as a REAL, so what the first batch
/// observed says nothing about the later ones ([`ColumnPlan::resolve`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Affinity {
    Integer,
    Text,
    Blob,
    Real,
    Numeric,
}

/// The affinity class of a declared type, `None` when nothing is declared.
fn declared_affinity(declared: Option<&str>) -> Option<Affinity> {
    let declared = declared?.trim();
    if declared.is_empty() {
        return None;
    }
    let upper = declared.to_ascii_uppercase();
    if upper.contains("INT") {
        return Some(Affinity::Integer);
    }
    if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        return Some(Affinity::Text);
    }
    if upper.contains("BLOB") {
        return Some(Affinity::Blob);
    }
    if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        return Some(Affinity::Real);
    }
    Some(Affinity::Numeric)
}

/// The storage classes encountered in a column.
///
/// `NULL` is not one: an entirely null column observed nothing, and that is a
/// different piece of information from "it mixes types".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Observed(u8);

impl Observed {
    const INTEGER: u8 = 1;
    const REAL: u8 = 1 << 1;
    const TEXT: u8 = 1 << 2;
    const BLOB: u8 = 1 << 3;

    /// Records a value.
    ///
    /// A TEXT that is not valid UTF-8 counts as **bytes**: it cannot join a `Utf8`
    /// column, and finding it out here avoids noticing it in the middle of the
    /// stream.
    pub(crate) fn observe(&mut self, value: ValueRef<'_>) {
        self.0 |= match value {
            ValueRef::Null => 0,
            ValueRef::Integer(_) => Self::INTEGER,
            ValueRef::Real(_) => Self::REAL,
            ValueRef::Text(bytes) if std::str::from_utf8(bytes).is_err() => Self::BLOB,
            ValueRef::Text(_) => Self::TEXT,
            ValueRef::Blob(_) => Self::BLOB,
        };
    }

    /// No non-null value was seen.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Several classes coexist in the column.
    #[must_use]
    pub const fn is_mixed(self) -> bool {
        self.0.count_ones() > 1
    }

    /// The classes seen, separated by `|`, in lattice order.
    #[must_use]
    pub fn names(self) -> String {
        let mut joined = String::new();
        for (bit, class_name) in [
            (Self::INTEGER, "integer"),
            (Self::REAL, "real"),
            (Self::TEXT, "text"),
            (Self::BLOB, "blob"),
        ] {
            if self.0 & bit != 0 {
                if !joined.is_empty() {
                    joined.push('|');
                }
                joined.push_str(class_name);
            }
        }
        joined
    }

    /// The type that holds every class seen without loss.
    ///
    /// See the module table. `None` when nothing was seen.
    #[must_use]
    fn join(self) -> Option<ColumnKind> {
        if self.is_empty() {
            return None;
        }
        if self.0 & Self::BLOB != 0 {
            // Only bytes contain everything: a blob has no textual rendering,
            // whereas an integer, a float and a text all have a byte
            // representation.
            return Some(ColumnKind::Binary);
        }
        if self.0 & Self::TEXT != 0 {
            return Some(ColumnKind::Utf8);
        }
        if self.0 & Self::INTEGER != 0 && self.0 & Self::REAL != 0 {
            // The trap: no `f64` holds every `i64`. A column mixing exact values
            // and floats renders as text, not as `Float64`.
            return Some(ColumnKind::Utf8);
        }
        if self.0 & Self::REAL != 0 {
            return Some(ColumnKind::Float64);
        }
        Some(ColumnKind::Int64)
    }
}

/// What was decided for a column of the result.
#[derive(Debug, Clone)]
pub(crate) struct ColumnPlan {
    /// Name returned by SQLite. **Hostile input**: an arbitrary alias chosen by
    /// the user or a column name from the schema.
    pub name: String,
    /// Declared type, when the column comes from a table and not an expression.
    pub declared: Option<String>,
    /// Chosen Arrow type.
    pub kind: ColumnKind,
    /// Storage classes seen in the first batch.
    pub observed: Observed,
    /// The type comes from the values, not from the declaration.
    pub inferred: bool,
}

impl ColumnPlan {
    /// Decides a column's type from what the first batch saw and what the
    /// schema declares.
    pub(crate) fn resolve(name: String, declared: Option<String>, observed: Observed) -> Self {
        let declared_kind = affinity(declared.as_deref());
        let numeric = declared_affinity(declared.as_deref()) == Some(Affinity::Numeric);
        let kind = observed
            .join()
            // Under NUMERIC affinity, `10.00` is stored as INTEGER and `10.50` as
            // REAL: a first batch of whole amounts must not type the column
            // `Int64`, or the first fraction after it fails the whole stream with
            // a `ColumnConflict`. Rendering both as text is lossless.
            .map(|joined| match joined {
                ColumnKind::Int64 | ColumnKind::Float64 if numeric => ColumnKind::Utf8,
                other => other,
            })
            .or(declared_kind)
            // Nothing seen, nothing declared: text loses the least.
            .unwrap_or(ColumnKind::Utf8);
        Self {
            name,
            declared,
            kind,
            observed,
            // "Inferred" means "the chosen type is not the one the schema
            // implies" — whether the schema is silent, or says something else than
            // what the values show.
            inferred: declared_kind != Some(kind),
        }
    }

    /// The Arrow `Field`, metadata included.
    ///
    /// The field is **always** declared nullable: SQLite returns `NULL` for any
    /// expression, and a `NOT NULL` column seen through an outer join is just as
    /// nullable.
    fn field(&self) -> Field {
        let mut metadata = HashMap::new();
        if let Some(declared) = &self.declared {
            metadata.insert(METADATA_DECLARED_TYPE.to_owned(), declared.clone());
        }
        if self.inferred {
            metadata.insert(METADATA_INFERRED.to_owned(), "true".to_owned());
        }
        if self.observed.is_mixed() {
            metadata.insert(METADATA_STORAGE_CLASSES.to_owned(), self.observed.names());
        }
        Field::new(self.name.as_str(), self.kind.data_type(), true).with_metadata(metadata)
    }
}

/// The Arrow schema of a set of resolved columns.
pub(crate) fn schema_of(plans: &[ColumnPlan]) -> SchemaRef {
    Arc::new(Schema::new(
        plans.iter().map(ColumnPlan::field).collect::<Vec<_>>(),
    ))
}

/// A value set aside while its column's type is resolved.
///
/// Cannot be `rusqlite::types::Value`: that one stores TEXT in a `String`, hence
/// loses — or refuses — a text that is not valid UTF-8. Here the bytes are kept
/// as they are.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProbeValue {
    /// Absence of value.
    Null,
    /// 64-bit integer.
    Integer(i64),
    /// Double-precision float.
    Real(f64),
    /// Text, **as bytes**: nothing guarantees it is valid UTF-8.
    Text(Vec<u8>),
    /// Opaque bytes.
    Blob(Vec<u8>),
}

impl ProbeValue {
    /// Copies a value borrowed from the engine.
    pub(crate) fn capture(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(i) => Self::Integer(i),
            ValueRef::Real(x) => Self::Real(x),
            ValueRef::Text(bytes) => Self::Text(bytes.to_vec()),
            ValueRef::Blob(bytes) => Self::Blob(bytes.to_vec()),
        }
    }

    /// Borrowed view, as the engine would have returned it.
    pub(crate) fn borrow(&self) -> ValueRef<'_> {
        match self {
            Self::Null => ValueRef::Null,
            Self::Integer(i) => ValueRef::Integer(*i),
            Self::Real(x) => ValueRef::Real(*x),
            Self::Text(bytes) => ValueRef::Text(bytes),
            Self::Blob(bytes) => ValueRef::Blob(bytes),
        }
    }
}

/// The data bytes of a value.
///
/// **This** measure bounds a batch, not the number of rows
/// ([`BatchLimits`](crate::BatchLimits)).
#[must_use]
pub(crate) fn value_bytes(value: ValueRef<'_>) -> usize {
    match value {
        ValueRef::Null => 0,
        ValueRef::Integer(_) | ValueRef::Real(_) => size_of::<i64>(),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => bytes.len(),
    }
}

/// The builder of an Arrow column, reused from one batch to the next.
///
/// Carries a shared rendering buffer: converting an integer to text would
/// otherwise allocate a `String` per value, and the conversion to `RecordBatch`
/// is a **per-value** path ([rust.md](../../../.claude/rules/rust.md)).
#[derive(Debug)]
pub(crate) struct ColumnBuilder {
    inner: Inner,
    scratch: String,
    index: usize,
}

#[derive(Debug)]
enum Inner {
    Int64(Int64Builder),
    Float64(Float64Builder),
    Utf8(StringBuilder),
    Binary(BinaryBuilder),
}

impl ColumnBuilder {
    /// A builder for the chosen type, sized for `rows` rows.
    pub(crate) fn new(kind: ColumnKind, index: usize, rows: usize) -> Self {
        let inner = match kind {
            ColumnKind::Int64 => Inner::Int64(Int64Builder::with_capacity(rows)),
            ColumnKind::Float64 => Inner::Float64(Float64Builder::with_capacity(rows)),
            // The second capacity is the byte buffer's: sixteen bytes per row is a
            // low estimate that avoids the first reallocations without reserving
            // blindly.
            ColumnKind::Utf8 => {
                Inner::Utf8(StringBuilder::with_capacity(rows, rows.saturating_mul(16)))
            }
            ColumnKind::Binary => {
                Inner::Binary(BinaryBuilder::with_capacity(rows, rows.saturating_mul(16)))
            }
        };
        Self {
            inner,
            scratch: String::new(),
            index,
        }
    }

    /// The chosen type.
    pub(crate) fn kind(&self) -> ColumnKind {
        match self.inner {
            Inner::Int64(_) => ColumnKind::Int64,
            Inner::Float64(_) => ColumnKind::Float64,
            Inner::Utf8(_) => ColumnKind::Utf8,
            Inner::Binary(_) => ColumnKind::Binary,
        }
    }

    /// Appends a value, or refuses.
    ///
    /// Every accepted conversion is **lossless**. Those that would lose — a float
    /// in an integer column, an integer beyond 2⁵³ in a float column, a blob in a
    /// textual column — return [`SqliteError::ColumnConflict`] rather than a wrong
    /// value.
    ///
    /// # Errors
    /// [`SqliteError::ColumnConflict`] when the value has no lossless rendering in
    /// the column's type.
    pub(crate) fn append(&mut self, value: ValueRef<'_>) -> Result<(), SqliteError> {
        // The error is built in advance: it costs only three words, and producing
        // it from a `match` arm would require borrowing `self` while it already
        // is.
        let conflict = SqliteError::ColumnConflict {
            column: self.index,
            resolved: self.kind().as_str(),
            found: storage_class_name(value),
        };
        // The rendering buffer leaves the builder for the duration of the borrow,
        // then goes back in place: it is reused from one value to the next.
        let mut scratch = std::mem::take(&mut self.scratch);
        let issue = append_into(&mut self.inner, &mut scratch, value, conflict);
        self.scratch = scratch;
        issue
    }

    /// Closes the batch and returns the column. The builder becomes empty again.
    pub(crate) fn finish(&mut self) -> ArrayRef {
        match &mut self.inner {
            Inner::Int64(b) => Arc::new(b.finish()),
            Inner::Float64(b) => Arc::new(b.finish()),
            Inner::Utf8(b) => Arc::new(b.finish()),
            Inner::Binary(b) => Arc::new(b.finish()),
        }
    }
}

/// The body of [`ColumnBuilder::append`], taken out so that the builder and its
/// rendering buffer are two distinct borrows.
fn append_into(
    inner: &mut Inner,
    scratch: &mut String,
    value: ValueRef<'_>,
    conflict: SqliteError,
) -> Result<(), SqliteError> {
    if matches!(value, ValueRef::Null) {
        match inner {
            Inner::Int64(b) => b.append_null(),
            Inner::Float64(b) => b.append_null(),
            Inner::Utf8(b) => b.append_null(),
            Inner::Binary(b) => b.append_null(),
        }
        return Ok(());
    }

    match (inner, value) {
        (Inner::Int64(b), ValueRef::Integer(i)) => b.append_value(i),
        (Inner::Float64(b), ValueRef::Real(x)) => b.append_value(x),
        (Inner::Float64(b), ValueRef::Integer(i)) => {
            if i.unsigned_abs() > EXACT_IN_F64 {
                return Err(conflict);
            }
            // Bounded just above: `f64` represents exactly every integer of
            // absolute value ≤ 2⁵³, so the conversion loses nothing.
            #[allow(clippy::cast_precision_loss)]
            b.append_value(i as f64);
        }
        (Inner::Utf8(b), ValueRef::Text(bytes)) => {
            let Ok(decoded) = std::str::from_utf8(bytes) else {
                return Err(conflict);
            };
            b.append_value(decoded);
        }
        (Inner::Utf8(b), ValueRef::Integer(_) | ValueRef::Real(_)) => {
            render(scratch, value);
            b.append_value(&*scratch);
        }
        (Inner::Binary(b), ValueRef::Text(bytes) | ValueRef::Blob(bytes)) => b.append_value(bytes),
        (Inner::Binary(b), ValueRef::Integer(_) | ValueRef::Real(_)) => {
            render(scratch, value);
            b.append_value(scratch.as_bytes());
        }
        _ => return Err(conflict),
    }
    Ok(())
}

/// Writes the textual rendering of a number into the shared buffer.
///
/// `f64`'s `Display` produces the shortest representation that reads back
/// identically: the conversion is reversible.
fn render(scratch: &mut String, value: ValueRef<'_>) {
    scratch.clear();
    // Writing into a `String` cannot fail.
    let _ = match value {
        ValueRef::Integer(i) => write!(scratch, "{i}"),
        ValueRef::Real(x) => write!(scratch, "{x}"),
        _ => Ok(()),
    };
}

/// The name of a value's storage class, for an error message.
const fn storage_class_name(value: ValueRef<'_>) -> &'static str {
    match value {
        ValueRef::Null => "null",
        ValueRef::Integer(_) => "integer",
        ValueRef::Real(_) => "real",
        ValueRef::Text(_) => "text",
        ValueRef::Blob(_) => "blob",
    }
}

#[cfg(test)]
mod tests;
