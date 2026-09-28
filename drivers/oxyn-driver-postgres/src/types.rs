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
//! | `record`, composite types | `Utf8` | the structure is not split into an Arrow `Struct`: text as `record_out` prints it. A field whose type is not built in (an enum, a domain, an extension type) keeps its bytes as `\\x…` |
//! | `inet`, `cidr`, `macaddr`, `macaddr8`, `bit`, `varbit`, geometric types, `pg_lsn`, `tid`, `pg_snapshot`, `txid_snapshot`, `tsvector`, `tsquery`, ranges | `Utf8` | none: the text the server prints, rebuilt from the binary layout (`render` module) |
//! | `timestamptz` inside a range or a record | `Utf8` | printed in UTC (`+00`), whatever the session's `TimeZone` |
//! | `reg*` (`regclass`, `regtype`…), `xid`, `cid` | `UInt32` | the OID, not the name: resolving it needs the catalog, and `::text` in SQL gives it |
//! | `xid8` | `UInt64` | none |
//! | multiranges, `pg_node_tree` and the other internal `Z` types, `aclitem`, `gtsvector` | `Utf8` for **every** column of that result | the statement runs in the simple protocol (ADR-0048): every value is the server's text, `int4` included, marked `oxyn:fallback = text`; a result without rows has no columns; refused with bound parameters |
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
    /// `regproc`
    pub const REGPROC: u32 = 24;
    /// `tid`
    pub const TID: u32 = 27;
    /// `xid`
    pub const XID: u32 = 28;
    /// `cid`
    pub const CID: u32 = 29;
    /// `point`
    pub const POINT: u32 = 600;
    /// `lseg`
    pub const LSEG: u32 = 601;
    /// `path`
    pub const PATH: u32 = 602;
    /// `box`
    pub const BOX: u32 = 603;
    /// `polygon`
    pub const POLYGON: u32 = 604;
    /// `line`
    pub const LINE: u32 = 628;
    /// `circle`
    pub const CIRCLE: u32 = 718;
    /// `bit`
    pub const BIT: u32 = 1560;
    /// `varbit`
    pub const VARBIT: u32 = 1562;
    /// `regprocedure`
    pub const REGPROCEDURE: u32 = 2202;
    /// `regoper`
    pub const REGOPER: u32 = 2203;
    /// `regoperator`
    pub const REGOPERATOR: u32 = 2204;
    /// `regclass`
    pub const REGCLASS: u32 = 2205;
    /// `regtype`
    pub const REGTYPE: u32 = 2206;
    /// `record`: anonymous row type
    pub const RECORD: u32 = 2249;
    /// `txid_snapshot`
    pub const TXID_SNAPSHOT: u32 = 2970;
    /// `pg_lsn`
    pub const PG_LSN: u32 = 3220;
    /// `tsvector`
    pub const TSVECTOR: u32 = 3614;
    /// `tsquery`
    pub const TSQUERY: u32 = 3615;
    /// `regconfig`
    pub const REGCONFIG: u32 = 3734;
    /// `regdictionary`
    pub const REGDICTIONARY: u32 = 3769;
    /// `regnamespace`
    pub const REGNAMESPACE: u32 = 4089;
    /// `regrole`
    pub const REGROLE: u32 = 4096;
    /// `regcollation`
    pub const REGCOLLATION: u32 = 4191;
    /// `pg_snapshot`, PostgreSQL 13+
    pub const PG_SNAPSHOT: u32 = 5038;
    /// `xid8`, PostgreSQL 13+
    pub const XID8: u32 = 5069;
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
    /// `xid8`: unsigned 64-bit integer.
    UInt64,
    /// Types whose binary representation is already UTF-8: `text`,
    /// `varchar`, `bpchar`, `name`, `"char"`, `xml`, `json`…
    Text,
    /// A built-in type with its own binary layout, rendered as the text the
    /// server itself prints.
    Rendered(TextFormat),
    /// A range, rendered as text: its bounds are decoded with the element's
    /// decoding.
    Range(Box<PgDecoding>),
    /// A multirange (PostgreSQL 14+), rendered as text.
    Multirange(Box<PgDecoding>),
    /// A `record` or a composite type, rendered as text. The wire carries the
    /// type of each field, which decides how it is rendered.
    Record,
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

/// The built-in binary layouts rendered as text by [`PgDecoding::Rendered`].
///
/// Each one reproduces the type's `*_out` function, checked against
/// PostgreSQL's sources in
/// [RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md#postgresql-binary-wire-formats--checked-on-2026-09-28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextFormat {
    /// `inet` and `cidr`.
    Inet,
    /// `macaddr` and `macaddr8`.
    MacAddr,
    /// `bit` and `varbit`.
    Bits,
    /// `point`.
    Point,
    /// `line`.
    Line,
    /// `lseg`.
    Lseg,
    /// `box`.
    Box,
    /// `path`.
    Path,
    /// `polygon`.
    Polygon,
    /// `circle`.
    Circle,
    /// `pg_lsn`.
    Lsn,
    /// `tid`.
    Tid,
    /// `pg_snapshot` and `txid_snapshot`.
    Snapshot,
    /// `tsvector`.
    TsVector,
    /// `tsquery`.
    TsQuery,
    /// `void`: no value, rendered as an empty string like the server does.
    Void,
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
            Self::UInt64 => DataType::UInt64,
            Self::Float32 => DataType::Float32,
            Self::Float64 => DataType::Float64,
            Self::Text
            | Self::Jsonb
            | Self::Numeric
            | Self::Uuid
            | Self::TimeTz
            | Self::Rendered(_)
            | Self::Range(_)
            | Self::Multirange(_)
            | Self::Record => DataType::Utf8,
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
    if let Some(raw_type) = ty.oid()
        && let Some(decoding) = decoding_for_oid(raw_type.0)
    {
        return decoding;
    }

    match ty.kind() {
        // A domain is a base type plus a constraint; the constraint does not
        // change the representation on the wire.
        PgTypeKind::Domain(base) => decoding_for(base),
        PgTypeKind::Array(element) => PgDecoding::List(Box::new(decoding_for(element))),
        // The binary representation of an `enum` is its label in UTF-8.
        PgTypeKind::Enum(_) => PgDecoding::Text,
        PgTypeKind::Range(element) => PgDecoding::Range(Box::new(decoding_for(element))),
        // A composite's binary form is the same as an anonymous `record`'s:
        // each field carries its own type OID.
        PgTypeKind::Composite(_) => PgDecoding::Record,
        _ => decoding_for_name(ty.name()),
    }
}

/// Decoding of a built-in type, recognized by its OID.
pub(crate) fn decoding_for_oid(raw_type: u32) -> Option<PgDecoding> {
    let decoding = match raw_type {
        oid::BOOL => PgDecoding::Bool,
        oid::INT2 => PgDecoding::Int16,
        oid::INT4 => PgDecoding::Int32,
        oid::INT8 => PgDecoding::Int64,
        // The `reg*` types are OIDs on the wire. Their name (`pg_class` for a
        // `regclass`) only exists in the catalog, and resolving it would cost
        // a query per value: the number is what the server sent, and
        // `::text` in SQL gives the name.
        oid::OID
        | oid::XID
        | oid::CID
        | oid::REGPROC
        | oid::REGPROCEDURE
        | oid::REGOPER
        | oid::REGOPERATOR
        | oid::REGCLASS
        | oid::REGTYPE
        | oid::REGCONFIG
        | oid::REGDICTIONARY
        | oid::REGNAMESPACE
        | oid::REGROLE
        | oid::REGCOLLATION => PgDecoding::UInt32,
        oid::XID8 => PgDecoding::UInt64,
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
        | oid::UNKNOWN => PgDecoding::Text,
        // Their binary layout is not text: read as UTF-8, `10.0.0.1` came out
        // as control characters and `192.168.0.1` failed the whole query.
        oid::INET | oid::CIDR => PgDecoding::Rendered(TextFormat::Inet),
        oid::MACADDR | oid::MACADDR8 => PgDecoding::Rendered(TextFormat::MacAddr),
        oid::BIT | oid::VARBIT => PgDecoding::Rendered(TextFormat::Bits),
        oid::POINT => PgDecoding::Rendered(TextFormat::Point),
        oid::LINE => PgDecoding::Rendered(TextFormat::Line),
        oid::LSEG => PgDecoding::Rendered(TextFormat::Lseg),
        oid::BOX => PgDecoding::Rendered(TextFormat::Box),
        oid::PATH => PgDecoding::Rendered(TextFormat::Path),
        oid::POLYGON => PgDecoding::Rendered(TextFormat::Polygon),
        oid::CIRCLE => PgDecoding::Rendered(TextFormat::Circle),
        oid::PG_LSN => PgDecoding::Rendered(TextFormat::Lsn),
        oid::TID => PgDecoding::Rendered(TextFormat::Tid),
        oid::PG_SNAPSHOT | oid::TXID_SNAPSHOT => PgDecoding::Rendered(TextFormat::Snapshot),
        oid::TSVECTOR => PgDecoding::Rendered(TextFormat::TsVector),
        oid::TSQUERY => PgDecoding::Rendered(TextFormat::TsQuery),
        oid::RECORD => PgDecoding::Record,
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
        oid::VOID => PgDecoding::Rendered(TextFormat::Void),
        _ => return None,
    };
    Some(decoding)
}

/// The built-in array types: `(array OID, element OID)`, read in `pg_type`'s
/// `typarray` of PostgreSQL 17 on 2026-09-28. Built-in OIDs do not change from
/// one version to the next.
const BUILTIN_ARRAYS: &[(u32, u32)] = &[
    (1000, 16),
    (1001, 17),
    (1002, 18),
    (1003, 19),
    (1016, 20),
    (1005, 21),
    (1007, 23),
    (1008, 24),
    (1009, 25),
    (1028, 26),
    (1010, 27),
    (1011, 28),
    (1012, 29),
    (199, 114),
    (143, 142),
    (1017, 600),
    (1018, 601),
    (1019, 602),
    (1020, 603),
    (1027, 604),
    (629, 628),
    (651, 650),
    (1021, 700),
    (1022, 701),
    (719, 718),
    (775, 774),
    (791, 790),
    (1040, 829),
    (1041, 869),
    (1014, 1042),
    (1015, 1043),
    (1182, 1082),
    (1183, 1083),
    (1115, 1114),
    (1185, 1184),
    (1187, 1186),
    (1270, 1266),
    (1561, 1560),
    (1563, 1562),
    (1231, 1700),
    (2207, 2202),
    (2208, 2203),
    (2209, 2204),
    (2210, 2205),
    (2211, 2206),
    (2287, 2249),
    (2951, 2950),
    (2949, 2970),
    (3221, 3220),
    (3643, 3614),
    (3645, 3615),
    (3735, 3734),
    (3770, 3769),
    (3807, 3802),
    (3905, 3904),
    (3907, 3906),
    (3909, 3908),
    (3911, 3910),
    (3913, 3912),
    (3927, 3926),
    (4090, 4089),
    (4097, 4096),
    (4192, 4191),
    (5039, 5038),
    (271, 5069),
];

/// The built-in range types and their element: `(range OID, element OID)`.
const BUILTIN_RANGES: &[(u32, u32)] = &[
    (3904, oid::INT4),
    (3906, oid::NUMERIC),
    (3908, oid::TIMESTAMP),
    (3910, oid::TIMESTAMPTZ),
    (3912, oid::DATE),
    (3926, oid::INT8),
];

/// Decoding of a built-in type known only by its OID, arrays and ranges
/// included.
///
/// A column's type comes with its kind from `sqlx`, which already says "array
/// of" or "range of". A record field carries only an OID on the wire: without
/// this table, an `int4[]` field would fall back to raw bytes.
pub(crate) fn decoding_for_builtin_oid(raw_type: u32) -> Option<PgDecoding> {
    if let Some(decoding) = decoding_for_oid(raw_type) {
        return Some(decoding);
    }
    if let Some((_, element)) = BUILTIN_ARRAYS.iter().find(|(array, _)| *array == raw_type) {
        return Some(PgDecoding::List(Box::new(decoding_for_builtin_oid(
            *element,
        )?)));
    }
    let (_, element) = BUILTIN_RANGES
        .iter()
        .find(|(range, _)| *range == raw_type)?;
    Some(PgDecoding::Range(Box::new(decoding_for_oid(*element)?)))
}

/// The built-in types without a binary output function (`typsend = 0`), read
/// in `pg_type` of PostgreSQL 17 on 2026-09-28: `aclitem`, `aclitem[]`,
/// `gtsvector`, `gtsvector[]`. The server refuses a Bind that asks for them in
/// binary.
const NO_BINARY_OUTPUT: &[u32] = &[1033, 1034, 3642, 3644];

/// Does a result column need the text format, the server having no binary
/// form for its type? Follows a domain to its base type.
///
/// Called on a prepared statement's columns only: `sqlx` has resolved every
/// type there. On an unresolved one — what the simple protocol yields —
/// `PgTypeInfo::kind` **panics** (ADR-0048).
pub(crate) fn lacks_binary_output(ty: &PgTypeInfo) -> bool {
    if ty
        .oid()
        .is_some_and(|raw_type| NO_BINARY_OUTPUT.contains(&raw_type.0))
    {
        return true;
    }
    match ty.kind() {
        PgTypeKind::Domain(base) => lacks_binary_output(base),
        _ => false,
    }
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
    let mut fields = Vec::with_capacity(columns.len());
    let mut decodings = Vec::with_capacity(columns.len());

    for column in columns {
        let type_info = column.type_info();
        let decoding = decoding_for(type_info);

        let mut metadata = HashMap::with_capacity(2);
        metadata.insert(META_PG_TYPE.to_owned(), type_info.name().to_owned());
        if decoding.is_opaque() {
            // Keep the original type attached to the raw binary payload.
            metadata.insert(META_FALLBACK.to_owned(), "opaque".to_owned());
        }

        // Every column is declared nullable: the server can return `NULL` in a
        // `NOT NULL` column as soon as an outer join comes into play, and a
        // schema that forbade it would make building the batch fail.
        fields.push(Field::new(column.name(), decoding.arrow_type(), true).with_metadata(metadata));
        decodings.push(decoding);
    }

    (Arc::new(Schema::new(fields)), decodings)
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
        let decoding = decoding_for_oid(oid::NUMERIC).expect("numeric is a built-in type");
        assert_eq!(decoding, PgDecoding::Numeric);
        assert_eq!(decoding.arrow_type(), DataType::Utf8);
        assert_ne!(decoding.arrow_type(), DataType::Float64);
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
        let list = PgDecoding::List(Box::new(PgDecoding::Int32));
        assert_eq!(
            list.arrow_type(),
            DataType::List(Arc::new(Field::new("item", DataType::Int32, true)))
        );
    }

    #[test]
    fn the_list_field_type_is_the_one_the_decoder_builds() {
        // The name `item` is not decorative: `RecordBatch::try_new` compares
        // the schema field to that of the built array, and rejects the gap.
        let DataType::List(field) = PgDecoding::List(Box::new(PgDecoding::Text)).arrow_type()
        else {
            panic!("a list must produce a DataType::List");
        };
        assert_eq!(field.name(), "item");
        assert!(field.is_nullable(), "an array element can be NULL");
    }

    #[test]
    fn money_stays_opaque_for_lack_of_knowing_its_unit() {
        // Rendering it as Int64 would invite summing amounts of different
        // currencies without ever showing which.
        assert_eq!(decoding_for_oid(oid::MONEY), Some(PgDecoding::Opaque));
    }
}
