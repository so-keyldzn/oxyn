//! The mapping of PostgreSQL types to Arrow, **and its losses**.
//!
//! The contract requires this table to go both ways and to document what it
//! loses ([DRIVER-CONTRACT §7](../../../docs/DRIVER-CONTRACT.md)). The "read"
//! direction is here; the "write" direction — binding a
//! [`ScalarValue`](oxyn_core::ScalarValue) as a parameter — is in
//! [`crate::session`], because it depends on `sqlx`'s encoder and not on the
//! decoder.
//!
//! # The three rules that govern this module
//!
//! **Unknown binary types retain their bytes.** [`PgDecoding::Opaque`] produces
//! Arrow `Binary`, with the PostgreSQL type name in `oxyn:pg_type` metadata.
//! Valid UTF-8 bytes are not evidence of a textual wire format: an OID can look
//! like a printable character. Known enums are decoded as text explicitly.
//!
//! **`NUMERIC` never becomes a float.** A `NUMERIC` without precision fits in
//! no `f64`; converting it corrupts amounts. It becomes an **exact** decimal
//! string, rebuilt from the server's binary format (internal `numeric`
//! module).
//!
//! **A `timestamp` without a time zone does not get one.** `timestamp` becomes
//! `Timestamp(Microsecond, None)` and `timestamptz` becomes
//! `Timestamp(Microsecond, Some("UTC"))`. No conversion to the machine's time
//! zone happens here, nor anywhere else in the driver.
//!
//! # What the table loses, explicitly
//!
//! | PostgreSQL type | Arrow | Loss |
//! |---|---|---|
//! | `numeric` | `Utf8` | none on the value; the declared precision and scale are not carried by the protocol (see below) |
//! | `uuid` | `Utf8` | none: canonical lowercase rendering with hyphens |
//! | `timetz` | `Utf8` | none: time and offset rendered as is |
//! | `interval` | `Interval(MonthDayNano)` | beyond ±292 years of microsecond component, the conversion to nanoseconds overflows: decoding **fails** instead of truncating |
//! | `money` | `Binary` | raw representation retained; the currency depends on `lc_monetary`, which is not transmitted |
//! | arrays with more than one dimension | — | not representable by an Arrow list: decoding **fails** rather than silently flattening |
//! | `record`, composite types | `Binary` (opaque) | the structure is not split into an Arrow `Struct` |
//!
//! # Why `numeric` is not a `Decimal128`
//!
//! `Decimal128(p, s)` requires a precision and a scale **fixed before the first
//! row**. The PostgreSQL protocol's `RowDescription` does carry an
//! `atttypmod`, but `sqlx` 0.9 does not expose it on [`PgColumn`] — and a
//! `numeric` without a column constraint has none anyway. Choosing a scale by
//! guesswork would round amounts; exact `Utf8` loses nothing.
//!
// TODO(phase 1): switch to `Decimal128(p, s)` for columns whose catalog gives a
// precision and a scale — the path already exists through
// `PostgresCatalog::describe_relation`, what is missing is the link between the
// result column and the table column (`PgColumn::relation_id`).

use std::collections::HashMap;
use std::sync::Arc;

use arrow::datatypes::{DataType, Field, IntervalUnit, Schema, SchemaRef, TimeUnit};
use sqlx::Column as _;
use sqlx::TypeInfo as _;
use sqlx::postgres::{PgColumn, PgTypeInfo, PgTypeKind};

/// The OIDs of PostgreSQL's built-in types.
///
/// Those of the default `pg_type` catalog are **stable** from one version to
/// the next — unlike the OIDs assigned by `CREATE EXTENSION`, which depend on
/// the installation order. That is why recognition goes by OID for built-in
/// types and by **name** for the others.
pub(crate) mod oid {
    /// `bool`
    pub const BOOL: u32 = 16;
    /// `bytea`
    pub const BYTEA: u32 = 17;
    /// `"char"` (one byte, not `char(n)`)
    pub const CHAR: u32 = 18;
    /// `name`
    pub const NAME: u32 = 19;
    /// `int8`
    pub const INT8: u32 = 20;
    /// `int2`
    pub const INT2: u32 = 21;
    /// `int4`
    pub const INT4: u32 = 23;
    /// `text`
    pub const TEXT: u32 = 25;
    /// `oid`
    pub const OID: u32 = 26;
    /// `json`
    pub const JSON: u32 = 114;
    /// `xml`
    pub const XML: u32 = 142;
    /// `float4`
    pub const FLOAT4: u32 = 700;
    /// `float8`
    pub const FLOAT8: u32 = 701;
    /// `unknown`: untyped literal
    pub const UNKNOWN: u32 = 705;
    /// `money`
    pub const MONEY: u32 = 790;
    /// `macaddr`
    pub const MACADDR: u32 = 829;
    /// `inet`
    pub const INET: u32 = 869;
    /// `cidr`
    pub const CIDR: u32 = 650;
    /// `macaddr8`
    pub const MACADDR8: u32 = 774;
    /// `bpchar` (`char(n)`)
    pub const BPCHAR: u32 = 1042;
    /// `varchar`
    pub const VARCHAR: u32 = 1043;
    /// `date`
    pub const DATE: u32 = 1082;
    /// `time`
    pub const TIME: u32 = 1083;
    /// `timestamp`
    pub const TIMESTAMP: u32 = 1114;
    /// `timestamptz`
    pub const TIMESTAMPTZ: u32 = 1184;
    /// `interval`
    pub const INTERVAL: u32 = 1186;
    /// `timetz`
    pub const TIMETZ: u32 = 1266;
    /// `numeric`
    pub const NUMERIC: u32 = 1700;
    /// `uuid`
    pub const UUID: u32 = 2950;
    /// `jsonb`
    pub const JSONB: u32 = 3802;
    /// `void`
    pub const VOID: u32 = 2278;
}

/// Metadata key carrying the name of the original PostgreSQL type.
pub const META_PG_TYPE: &str = "oxyn:pg_type";
/// Metadata key carrying the fallback mode used for an unknown type.
///
/// Contains `opaque` for an unknown wire format preserved as binary.
/// Absent for recognized types.
pub const META_FALLBACK: &str = "oxyn:fallback";

/// How to decode a PostgreSQL value into its Arrow array.
///
/// One variant per **wire format**, not per SQL type: `text`, `varchar`,
/// `name` and `xml` share [`PgDecoding::Text`] because their binary
/// representation is the same sequence of UTF-8 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PgDecoding {
    /// `bool`: one byte.
    Bool,
    /// `int2`.
    Int16,
    /// `int4`.
    Int32,
    /// `int8`.
    Int64,
    /// `oid`: unsigned 32-bit integer.
    UInt32,
    /// `float4`.
    Float32,
    /// `float8`.
    Float64,
    /// Types whose binary representation is already UTF-8: `text`,
    /// `varchar`, `bpchar`, `name`, `"char"`, `xml`, `json`, `inet`…
    Text,
    /// `jsonb`: a version byte (equal to 1) then UTF-8.
    Jsonb,
    /// `numeric`: its own binary format, rendered as exact decimal.
    Numeric,
    /// `uuid`: sixteen bytes, rendered in canonical form.
    Uuid,
    /// `timetz`: microseconds since midnight, then offset in seconds.
    TimeTz,
    /// `bytea` and everything that remains bytes.
    Bytes,
    /// `date`: days since January 1<sup>st</sup>, 2000.
    Date,
    /// `time`: microseconds since midnight.
    Time,
    /// `timestamp`: microseconds since January 1<sup>st</sup>, 2000, **without
    /// time zone**.
    Timestamp,
    /// `timestamptz`: same, in UTC.
    TimestampTz,
    /// `interval`: microseconds, days, months.
    Interval,
    /// One-dimensional array of a decodable type.
    List(Box<PgDecoding>),
    /// Unknown wire format, preserved as raw bytes with type metadata.
    Opaque,
}

impl PgDecoding {
    /// The corresponding Arrow type.
    ///
    /// Stable for the whole lifetime of a stream: that is what
    /// [`Cursor::schema`](oxyn_driver::Cursor::schema) promises.
    #[must_use]
    pub fn arrow_type(&self) -> DataType {
        match self {
            Self::Bool => DataType::Boolean,
            Self::Int16 => DataType::Int16,
            Self::Int32 => DataType::Int32,
            Self::Int64 => DataType::Int64,
            Self::UInt32 => DataType::UInt32,
            Self::Float32 => DataType::Float32,
            Self::Float64 => DataType::Float64,
            Self::Text | Self::Jsonb | Self::Numeric | Self::Uuid | Self::TimeTz => DataType::Utf8,
            Self::Bytes | Self::Opaque => DataType::Binary,
            Self::Date => DataType::Date32,
            Self::Time => DataType::Time64(TimeUnit::Microsecond),
            Self::Timestamp => DataType::Timestamp(TimeUnit::Microsecond, None),
            // The time zone carried by the Arrow type says "these microseconds
            // are counted in UTC", not "display them in UTC": rendering stays an
            // interface decision (DRIVER-CONTRACT §7).
            Self::TimestampTz => DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from("UTC"))),
            Self::Interval => DataType::Interval(IntervalUnit::MonthDayNano),
            Self::List(element) => {
                DataType::List(Arc::new(Field::new("item", element.arrow_type(), true)))
            }
        }
    }

    /// Is the type rendered through a fallback for lack of being recognized?
    #[must_use]
    pub fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque)
    }
}

/// Chooses the decoding of a PostgreSQL type.
///
/// The order of attempts matters: the OID first, because it is stable for
/// built-in types; then the type's *kind*, to follow a domain to its base type
/// and an array to its element; then the name, the only way to recognize an
/// extension type whose OID varies from one installation to another.
#[must_use]
pub fn decoding_for(ty: &PgTypeInfo) -> PgDecoding {
    if let Some(brut) = ty.oid()
        && let Some(decodage) = decoding_for_oid(brut.0)
    {
        return decodage;
    }

    match ty.kind() {
        // A domain is a base type plus a constraint; the constraint does not
        // change the representation on the wire.
        PgTypeKind::Domain(base) => decoding_for(base),
        PgTypeKind::Array(element) => PgDecoding::List(Box::new(decoding_for(element))),
        // The binary representation of an `enum` is its label in UTF-8.
        PgTypeKind::Enum(_) => PgDecoding::Text,
        _ => decoding_for_name(ty.name()),
    }
}

/// Decoding of a built-in type, recognized by its OID.
fn decoding_for_oid(brut: u32) -> Option<PgDecoding> {
    let decodage = match brut {
        oid::BOOL => PgDecoding::Bool,
        oid::INT2 => PgDecoding::Int16,
        oid::INT4 => PgDecoding::Int32,
        oid::INT8 => PgDecoding::Int64,
        oid::OID => PgDecoding::UInt32,
        oid::FLOAT4 => PgDecoding::Float32,
        oid::FLOAT8 => PgDecoding::Float64,
        oid::NUMERIC => PgDecoding::Numeric,
        oid::TEXT
        | oid::VARCHAR
        | oid::BPCHAR
        | oid::NAME
        | oid::CHAR
        | oid::XML
        | oid::JSON
        | oid::UNKNOWN
        | oid::INET
        | oid::CIDR
        | oid::MACADDR
        | oid::MACADDR8 => PgDecoding::Text,
        oid::JSONB => PgDecoding::Jsonb,
        oid::BYTEA => PgDecoding::Bytes,
        oid::UUID => PgDecoding::Uuid,
        oid::DATE => PgDecoding::Date,
        oid::TIME => PgDecoding::Time,
        oid::TIMETZ => PgDecoding::TimeTz,
        oid::TIMESTAMP => PgDecoding::Timestamp,
        oid::TIMESTAMPTZ => PgDecoding::TimestampTz,
        oid::INTERVAL => PgDecoding::Interval,
        // `money` is a 64-bit integer of base units, but the unit depends on
        // `lc_monetary`, which the protocol does not transmit. Rendering it as
        // `Int64` would invite adding up euros and yen.
        oid::MONEY => PgDecoding::Opaque,
        oid::VOID => PgDecoding::Opaque,
        _ => return None,
    };
    Some(decodage)
}

/// Decoding of an extension type, recognized by its name.
///
/// The list is short on purpose: it contains only types whose binary
/// representation is **known and stable**. Everything else falls on
/// [`PgDecoding::Opaque`], which is not a failure but the documented behavior.
fn decoding_for_name(name: &str) -> PgDecoding {
    match name.to_ascii_lowercase().as_str() {
        // `citext` is a case-insensitive `text`: same bytes.
        "citext" => PgDecoding::Text,
        _ => PgDecoding::Opaque,
    }
}

/// Builds the Arrow schema of a result, and the decoding plan that goes with
/// it.
///
/// Both are returned together because they must stay aligned: a decoding that
/// did not match the declared type would make
/// [`RecordBatch::try_new`](arrow::record_batch::RecordBatch::try_new) fail on
/// the first batch, that is, at the worst moment.
///
/// Each field carries the name of the original PostgreSQL type in its metadata
/// ([`META_PG_TYPE`]): that is what lets the interface tell a `jsonb` from a
/// `text` even though both are `Utf8` columns.
#[must_use]
pub fn schema_for(columns: &[PgColumn]) -> (SchemaRef, Vec<PgDecoding>) {
    let mut champs = Vec::with_capacity(columns.len());
    let mut decodages = Vec::with_capacity(columns.len());

    for colonne in columns {
        let type_info = colonne.type_info();
        let decodage = decoding_for(type_info);

        let mut metadonnees = HashMap::with_capacity(2);
        metadonnees.insert(META_PG_TYPE.to_owned(), type_info.name().to_owned());
        if decodage.is_opaque() {
            // Keep the original type attached to the raw binary payload.
            metadonnees.insert(META_FALLBACK.to_owned(), "opaque".to_owned());
        }

        // Every column is declared nullable: the server can return `NULL` in a
        // `NOT NULL` column as soon as an outer join comes into play, and a
        // schema that forbade it would make building the batch fail.
        champs.push(
            Field::new(colonne.name(), decodage.arrow_type(), true).with_metadata(metadonnees),
        );
        decodages.push(decodage);
    }

    (Arc::new(Schema::new(champs)), decodages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_keep_their_width() {
        // An `int8` promoted to `Int32` would truncate an identifier of 5
        // billion to 705,032,704, with no error anywhere.
        assert_eq!(decoding_for_oid(oid::INT2), Some(PgDecoding::Int16));
        assert_eq!(decoding_for_oid(oid::INT4), Some(PgDecoding::Int32));
        assert_eq!(decoding_for_oid(oid::INT8), Some(PgDecoding::Int64));
        assert_eq!(PgDecoding::Int64.arrow_type(), DataType::Int64);
    }

    #[test]
    fn numeric_never_becomes_a_float() {
        // The loss would be silent and would affect amounts.
        let decodage = decoding_for_oid(oid::NUMERIC).expect("numeric is a built-in type");
        assert_eq!(decodage, PgDecoding::Numeric);
        assert_eq!(decodage.arrow_type(), DataType::Utf8);
        assert_ne!(decodage.arrow_type(), DataType::Float64);
    }

    #[test]
    fn a_timestamp_without_time_zone_does_not_get_one() {
        // DRIVER-CONTRACT §7: the corruption would be invisible and permanent.
        assert_eq!(
            PgDecoding::Timestamp.arrow_type(),
            DataType::Timestamp(TimeUnit::Microsecond, None)
        );
        assert_eq!(
            PgDecoding::TimestampTz.arrow_type(),
            DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from("UTC")))
        );
    }

    #[test]
    fn an_unknown_oid_falls_on_a_fallback_not_on_an_error() {
        // 16,000 is beyond the built-in types: it is a user type.
        assert_eq!(decoding_for_oid(16_000), None);
        assert_eq!(decoding_for_name("geometry"), PgDecoding::Opaque);
        assert_eq!(PgDecoding::Opaque.arrow_type(), DataType::Binary);
    }

    #[test]
    fn an_array_becomes_a_list_of_the_same_element() {
        let liste = PgDecoding::List(Box::new(PgDecoding::Int32));
        assert_eq!(
            liste.arrow_type(),
            DataType::List(Arc::new(Field::new("item", DataType::Int32, true)))
        );
    }

    #[test]
    fn the_list_field_type_is_the_one_the_decoder_builds() {
        // The name `item` is not decorative: `RecordBatch::try_new` compares
        // the schema field to that of the built array, and rejects the gap.
        let DataType::List(champ) = PgDecoding::List(Box::new(PgDecoding::Text)).arrow_type()
        else {
            panic!("a list must produce a DataType::List");
        };
        assert_eq!(champ.name(), "item");
        assert!(champ.is_nullable(), "an array element can be NULL");
    }

    #[test]
    fn money_stays_opaque_for_lack_of_knowing_its_unit() {
        // Rendering it as Int64 would invite summing amounts of different
        // currencies without ever showing which.
        assert_eq!(decoding_for_oid(oid::MONEY), Some(PgDecoding::Opaque));
    }
}
