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
    let declared = declared?.trim();
    if declared.is_empty() {
        return None;
    }
    let upper = declared.to_ascii_uppercase();
    if upper.contains("INT") {
        return Some(ColumnKind::Int64);
    }
    if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        return Some(ColumnKind::Utf8);
    }
    if upper.contains("BLOB") {
        return Some(ColumnKind::Binary);
    }
    if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        return Some(ColumnKind::Float64);
    }
    // NUMERIC affinity: SQLite stores the value as integer, float or text
    // depending on what is exact. No Arrow type covers all three; text is the only
    // rendering that loses nothing — it is the case of `DECIMAL(10,2)`.
    Some(ColumnKind::Utf8)
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
        let mut sortie = String::new();
        for (bit, nom) in [
            (Self::INTEGER, "integer"),
            (Self::REAL, "real"),
            (Self::TEXT, "text"),
            (Self::BLOB, "blob"),
        ] {
            if self.0 & bit != 0 {
                if !sortie.is_empty() {
                    sortie.push('|');
                }
                sortie.push_str(nom);
            }
        }
        sortie
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
        let kind = observed
            .join()
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
            let Ok(texte) = std::str::from_utf8(bytes) else {
                return Err(conflict);
            };
            b.append_value(texte);
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
mod tests {
    use arrow::array::{Array, BinaryArray, Float64Array, Int64Array, StringArray};

    use super::*;

    fn observed(values: &[ValueRef<'_>]) -> Observed {
        let mut vues = Observed::default();
        for value in values {
            vues.observe(*value);
        }
        vues
    }

    fn resolve(values: &[ValueRef<'_>], declared: Option<&str>) -> ColumnPlan {
        ColumnPlan::resolve(
            "c".to_owned(),
            declared.map(str::to_owned),
            observed(values),
        )
    }

    #[test]
    fn sqlite_affinity_rules_are_reproduced_as_they_are() {
        assert_eq!(affinity(Some("INTEGER")), Some(ColumnKind::Int64));
        assert_eq!(affinity(Some("BIGINT")), Some(ColumnKind::Int64));
        assert_eq!(affinity(Some("VARCHAR(255)")), Some(ColumnKind::Utf8));
        assert_eq!(affinity(Some("TEXT")), Some(ColumnKind::Utf8));
        assert_eq!(affinity(Some("BLOB")), Some(ColumnKind::Binary));
        assert_eq!(
            affinity(Some("DOUBLE PRECISION")),
            Some(ColumnKind::Float64)
        );
        assert_eq!(affinity(Some("REAL")), Some(ColumnKind::Float64));
        // NUMERIC affinity: neither integer nor float covers it.
        assert_eq!(affinity(Some("DECIMAL(10,2)")), Some(ColumnKind::Utf8));
        // The surprising consequence of rule 1, and it is correct: SQLite does
        // give INTEGER affinity to `POINT`.
        assert_eq!(affinity(Some("POINT")), Some(ColumnKind::Int64));
        // Nothing declared: no affinity, the values will decide.
        assert_eq!(affinity(None), None);
        assert_eq!(affinity(Some("   ")), None);
    }

    #[test]
    fn a_homogeneous_column_keeps_its_exact_type() {
        assert_eq!(
            resolve(&[ValueRef::Integer(1), ValueRef::Integer(2)], None).kind,
            ColumnKind::Int64
        );
        assert_eq!(
            resolve(&[ValueRef::Real(1.5)], None).kind,
            ColumnKind::Float64
        );
        assert_eq!(
            resolve(&[ValueRef::Text(b"a")], None).kind,
            ColumnKind::Utf8
        );
        assert_eq!(
            resolve(&[ValueRef::Blob(b"\x00\xff")], None).kind,
            ColumnKind::Binary
        );
    }

    #[test]
    fn a_column_mixing_integers_and_floats_falls_back_to_text() {
        // The trap: no `f64` holds every `i64`. Converting
        // 9_007_199_254_740_993 to a float changes it into 9_007_199_254_740_992.
        let plan = resolve(&[ValueRef::Integer(1), ValueRef::Real(1.5)], None);
        assert_eq!(plan.kind, ColumnKind::Utf8);
        assert!(plan.observed.is_mixed());
        assert_eq!(plan.observed.names(), "integer|real");
    }

    #[test]
    fn a_blob_imposes_bytes_on_the_whole_column() {
        for autre in [
            ValueRef::Integer(1),
            ValueRef::Real(1.0),
            ValueRef::Text(b"a"),
        ] {
            let plan = resolve(&[autre, ValueRef::Blob(b"\x00")], None);
            assert_eq!(
                plan.kind,
                ColumnKind::Binary,
                "only bytes contain everything"
            );
        }
    }

    #[test]
    fn a_text_that_is_not_utf8_counts_as_bytes() {
        // Found here rather than in the middle of the stream: that is the whole
        // point of the probe.
        let plan = resolve(&[ValueRef::Text(b"\xff\xfe")], None);
        assert_eq!(plan.kind, ColumnKind::Binary);
    }

    #[test]
    fn an_entirely_null_column_falls_back_on_the_declaration() {
        let plan = resolve(&[ValueRef::Null, ValueRef::Null], Some("INTEGER"));
        assert_eq!(plan.kind, ColumnKind::Int64);
        assert!(
            !plan.inferred,
            "the type comes from the declaration: nothing is inferred"
        );
        assert!(plan.observed.is_empty());
    }

    #[test]
    fn without_value_or_declaration_text_is_the_fallback() {
        let plan = resolve(&[], None);
        assert_eq!(plan.kind, ColumnKind::Utf8);
        assert!(plan.inferred, "nothing declared it: it is an inference");
    }

    #[test]
    fn a_column_whose_type_does_not_follow_its_declaration_declares_itself_inferred() {
        // `CREATE TABLE t(n INTEGER)` then `INSERT INTO t VALUES('abc')`: SQLite
        // accepts it, and the interface must be able to say that the displayed type
        // is not the schema's.
        let plan = resolve(&[ValueRef::Text(b"abc")], Some("INTEGER"));
        assert_eq!(plan.kind, ColumnKind::Utf8);
        assert!(plan.inferred);

        let champ = plan.field();
        assert_eq!(
            champ.metadata().get(METADATA_INFERRED).map(String::as_str),
            Some("true")
        );
        assert_eq!(
            champ
                .metadata()
                .get(METADATA_DECLARED_TYPE)
                .map(String::as_str),
            Some("INTEGER")
        );
    }

    #[test]
    fn a_mixed_column_declares_it_in_its_metadata() {
        let plan = resolve(&[ValueRef::Integer(1), ValueRef::Text(b"a")], None);
        let champ = plan.field();
        assert_eq!(
            champ
                .metadata()
                .get(METADATA_STORAGE_CLASSES)
                .map(String::as_str),
            Some("integer|text"),
            "the interface must be able to show that the rendering is a fallback"
        );
    }

    #[test]
    fn every_field_is_nullable() {
        // SQLite returns NULL for any expression, and a NOT NULL column seen
        // through an outer join is just as nullable.
        assert!(
            resolve(&[ValueRef::Integer(1)], Some("INTEGER"))
                .field()
                .is_nullable()
        );
    }

    #[test]
    fn accepted_conversions_are_lossless() {
        let mut b = ColumnBuilder::new(ColumnKind::Utf8, 0, 4);
        b.append(ValueRef::Integer(-42)).expect("integer as text");
        b.append(ValueRef::Real(0.1)).expect("float as text");
        b.append(ValueRef::Text(b"caf\xc3\xa9")).expect("text");
        b.append(ValueRef::Null).expect("null");

        let colonne = b.finish();
        let colonne = colonne
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Utf8 column");
        assert_eq!(colonne.value(0), "-42");
        assert_eq!(
            colonne.value(1),
            "0.1",
            "f64's Display renders the shortest form that reads back identically"
        );
        assert_eq!(colonne.value(2), "café");
        assert!(colonne.is_null(3));
    }

    #[test]
    fn an_integer_beyond_two_to_the_53_does_not_go_through_a_float() {
        // The silent corruption this refusal avoids: 2⁵³+1 would become 2⁵³.
        let mut b = ColumnBuilder::new(ColumnKind::Float64, 2, 1);
        b.append(ValueRef::Integer(1 << 53)).expect("2^53 is exact");

        let err = b
            .append(ValueRef::Integer((1_i64 << 53) + 1))
            .expect_err("refus attendu");
        let SqliteError::ColumnConflict {
            column, resolved, ..
        } = err
        else {
            panic!("mauvaise variante : {err:?}");
        };
        assert_eq!((column, resolved), (2, "float64"));
    }

    #[test]
    fn a_blob_does_not_slip_into_a_textual_column() {
        let mut b = ColumnBuilder::new(ColumnKind::Utf8, 1, 1);
        let err = b
            .append(ValueRef::Blob(b"\x00\xff"))
            .expect_err("refus attendu");
        assert!(
            matches!(
                err,
                SqliteError::ColumnConflict {
                    found: "blob",
                    resolved: "utf8",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn an_invalid_text_does_not_slip_into_a_textual_column() {
        let mut b = ColumnBuilder::new(ColumnKind::Utf8, 0, 1);
        let err = b
            .append(ValueRef::Text(b"\xff"))
            .expect_err("refus attendu");
        assert!(matches!(err, SqliteError::ColumnConflict { .. }), "{err:?}");
    }

    #[test]
    fn a_text_does_not_slip_into_an_integer_column() {
        let mut b = ColumnBuilder::new(ColumnKind::Int64, 0, 1);
        assert!(b.append(ValueRef::Text(b"12")).is_err());
        assert!(b.append(ValueRef::Real(1.0)).is_err());
        assert!(b.append(ValueRef::Blob(b"x")).is_err());
        b.append(ValueRef::Integer(7)).expect("an integer passes");
        let colonne = b.finish();
        let colonne = colonne
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("Int64 column");
        assert_eq!(colonne.value(0), 7);
    }

    #[test]
    fn a_bytes_column_holds_everything_without_losing_content() {
        let mut b = ColumnBuilder::new(ColumnKind::Binary, 0, 4);
        b.append(ValueRef::Blob(b"\x00\xff")).expect("bytes");
        b.append(ValueRef::Text(b"abc")).expect("text as bytes");
        b.append(ValueRef::Integer(12)).expect("integer as bytes");
        b.append(ValueRef::Null).expect("null");

        let colonne = b.finish();
        let colonne = colonne
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("Binary column");
        assert_eq!(colonne.value(0), b"\x00\xff");
        assert_eq!(colonne.value(1), b"abc");
        assert_eq!(colonne.value(2), b"12");
        assert!(colonne.is_null(3));
    }

    #[test]
    fn a_float_stays_a_float() {
        let mut b = ColumnBuilder::new(ColumnKind::Float64, 0, 2);
        b.append(ValueRef::Real(1.5)).expect("float");
        b.append(ValueRef::Integer(3)).expect("exact integer");
        let colonne = b.finish();
        let colonne = colonne
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("Float64 column");
        assert!((colonne.value(0) - 1.5).abs() < f64::EPSILON);
        assert!((colonne.value(1) - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_batch_is_measured_in_data_bytes() {
        assert_eq!(value_bytes(ValueRef::Null), 0);
        assert_eq!(value_bytes(ValueRef::Integer(1)), 8);
        assert_eq!(value_bytes(ValueRef::Blob(&[0; 1_048_576])), 1_048_576);
    }

    #[test]
    fn a_value_set_aside_reads_back_identically() {
        for valeur in [
            ValueRef::Null,
            ValueRef::Integer(-1),
            ValueRef::Real(2.5),
            ValueRef::Text(b"\xff non-utf8"),
            ValueRef::Blob(b"\x00"),
        ] {
            let capturee = ProbeValue::capture(valeur);
            assert_eq!(capturee.borrow(), valeur);
        }
    }
}
