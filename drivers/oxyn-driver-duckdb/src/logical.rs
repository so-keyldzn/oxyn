//! DuckDB's logical types, read once per result column.
//!
//! DuckDB exports several of its types to Arrow under a carrier that loses
//! what they are: `HUGEINT`, `UHUGEINT` and `DECIMAL(38,0)` all leave as
//! `Decimal128(38, 0)` — and a `UHUGEINT` above `i128::MAX` arrives as a
//! negative number —, `TIMESTAMP WITH TIME ZONE` leaves with the session's
//! zone name, `BIT` leaves as DuckDB's internal bytes, and `TIME WITH TIME
//! ZONE` leaves without its offset (`ArrowTimeTzConverter`,
//! `src/include/duckdb/common/arrow/appender/scalar_data.hpp`, v1.5.6). The
//! logical type DuckDB reports for the column is what tells them apart, at
//! every nesting level: [`DuckType`] is that type, owned, so that conversion
//! never holds a handle into the engine.
//!
//! # What is called on `duckdb-rs`, and why it cannot panic here
//!
//! [`LogicalTypeHandle::child`] and [`LogicalTypeHandle::child_name`] panic on
//! an index out of range or on a type without children. [`describe`] calls
//! them only for an identifier it matched to a type with children, and only
//! with an index below [`LogicalTypeHandle::num_children`], which DuckDB
//! computes for that same identifier. `try_id` maps an identifier this
//! version of `duckdb-rs` does not know to an error instead of a panic.
//! [`LogicalTypeHandle::get_alias`] is never called: it unwraps the alias as
//! UTF-8 (`core/logical_type.rs`, 1.10506.1).

use duckdb::core::{LogicalTypeHandle, LogicalTypeId};

/// Nesting deeper than this is refused: a hostile file can declare a type
/// nested arbitrarily deep, and the description recurses.
pub(crate) const MAX_DEPTH: usize = 64;

/// A DuckDB logical type, as far as conversion to Arrow needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuckType {
    /// `BOOLEAN`.
    Boolean,
    /// Signed and unsigned integers up to 64 bits, floats, `DATE`, `TIME`,
    /// `TIME_NS`, `INTERVAL`, `UUID`, `BLOB`, `GEOMETRY`, `"NULL"`: their
    /// Arrow carrier is faithful. The name is DuckDB's.
    Plain(&'static str),
    /// `VARCHAR`, with its alias when it has one (`JSON`).
    Varchar(Option<String>),
    /// `HUGEINT`: a signed 128-bit integer.
    HugeInt,
    /// `UHUGEINT`: an unsigned 128-bit integer.
    UHugeInt,
    /// `DECIMAL(width, scale)`.
    Decimal {
        /// Total digits.
        width: u8,
        /// Digits after the point.
        scale: u8,
    },
    /// `TIMESTAMP`, `TIMESTAMP_S`, `TIMESTAMP_MS`, `TIMESTAMP_NS`: no zone.
    Timestamp(&'static str),
    /// `TIMESTAMP WITH TIME ZONE`: an instant, carried in UTC.
    TimestampTz,
    /// `TIME WITH TIME ZONE`: a time of day and an offset.
    TimeTz,
    /// `BIT`: a string of bits.
    Bit,
    /// `BIGNUM`: an integer of arbitrary size.
    Bignum,
    /// `ENUM`.
    Enum,
    /// `T[]`.
    List(Box<DuckType>),
    /// `T[n]`.
    Array(Box<DuckType>),
    /// `MAP(K, V)`.
    Map(Box<DuckType>, Box<DuckType>),
    /// `STRUCT(…)`.
    Struct(Vec<(String, DuckType)>),
    /// `UNION(…)`.
    Union(Vec<(String, DuckType)>),
    /// A type this driver does not know, with DuckDB's numeric identifier.
    Unknown(u32),
}

/// Describes a logical type DuckDB reported for a result column.
///
/// Returns [`DuckType::Unknown`] for an identifier `duckdb-rs` does not know,
/// and for nesting beyond [`MAX_DEPTH`]: the conversion then refuses the
/// column explicitly rather than guessing.
#[must_use]
pub fn describe(handle: &LogicalTypeHandle) -> DuckType {
    describe_at(handle, 0)
}

fn describe_at(handle: &LogicalTypeHandle, depth: usize) -> DuckType {
    let id = match handle.try_id() {
        Ok(id) => id,
        Err(raw) => return DuckType::Unknown(raw),
    };
    if depth > MAX_DEPTH {
        return DuckType::Unknown(handle.raw_id());
    }
    let child = |index: usize| describe_at(&handle.child(index), depth + 1);
    let members = || {
        (0..handle.num_children())
            .map(|index| {
                (
                    handle.child_name(index),
                    describe_at(&handle.child(index), depth + 1),
                )
            })
            .collect::<Vec<_>>()
    };
    match id {
        LogicalTypeId::Boolean => DuckType::Boolean,
        LogicalTypeId::Tinyint => DuckType::Plain("TINYINT"),
        LogicalTypeId::Smallint => DuckType::Plain("SMALLINT"),
        LogicalTypeId::Integer => DuckType::Plain("INTEGER"),
        LogicalTypeId::Bigint => DuckType::Plain("BIGINT"),
        LogicalTypeId::UTinyint => DuckType::Plain("UTINYINT"),
        LogicalTypeId::USmallint => DuckType::Plain("USMALLINT"),
        LogicalTypeId::UInteger => DuckType::Plain("UINTEGER"),
        LogicalTypeId::UBigint => DuckType::Plain("UBIGINT"),
        LogicalTypeId::Float => DuckType::Plain("FLOAT"),
        LogicalTypeId::Double => DuckType::Plain("DOUBLE"),
        LogicalTypeId::Date => DuckType::Plain("DATE"),
        LogicalTypeId::Time => DuckType::Plain("TIME"),
        LogicalTypeId::TimeNs => DuckType::Plain("TIME_NS"),
        LogicalTypeId::Interval => DuckType::Plain("INTERVAL"),
        LogicalTypeId::Uuid => DuckType::Plain("UUID"),
        LogicalTypeId::Blob => DuckType::Plain("BLOB"),
        LogicalTypeId::Geometry => DuckType::Plain("GEOMETRY"),
        LogicalTypeId::SqlNull => DuckType::Plain("\"NULL\""),
        // Not `get_alias`: it unwraps the alias as UTF-8, and a type name in a
        // hostile file need not be. The JSON alias is read from the Arrow
        // field instead ([`crate::convert`]).
        LogicalTypeId::Varchar => DuckType::Varchar(None),
        LogicalTypeId::Hugeint => DuckType::HugeInt,
        LogicalTypeId::UHugeint => DuckType::UHugeInt,
        LogicalTypeId::Decimal => DuckType::Decimal {
            width: handle.decimal_width(),
            scale: handle.decimal_scale(),
        },
        LogicalTypeId::Timestamp => DuckType::Timestamp("TIMESTAMP"),
        LogicalTypeId::TimestampS => DuckType::Timestamp("TIMESTAMP_S"),
        LogicalTypeId::TimestampMs => DuckType::Timestamp("TIMESTAMP_MS"),
        LogicalTypeId::TimestampNs => DuckType::Timestamp("TIMESTAMP_NS"),
        LogicalTypeId::TimestampTZ => DuckType::TimestampTz,
        LogicalTypeId::TimeTZ => DuckType::TimeTz,
        LogicalTypeId::Bit => DuckType::Bit,
        LogicalTypeId::Bignum => DuckType::Bignum,
        LogicalTypeId::Enum => DuckType::Enum,
        // `num_children` is 1 for a list and an array, 2 for a map: the
        // indexes below are those DuckDB declares for these identifiers.
        LogicalTypeId::List => DuckType::List(Box::new(child(0))),
        LogicalTypeId::Array => DuckType::Array(Box::new(child(0))),
        LogicalTypeId::Map => DuckType::Map(Box::new(child(0)), Box::new(child(1))),
        LogicalTypeId::Struct => DuckType::Struct(members()),
        LogicalTypeId::Union => DuckType::Union(members()),
        _ => DuckType::Unknown(handle.raw_id()),
    }
}

impl DuckType {
    /// The type as DuckDB writes it, for the field metadata and the catalog.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Boolean => "BOOLEAN".to_owned(),
            Self::Plain(name) | Self::Timestamp(name) => (*name).to_owned(),
            Self::Varchar(alias) => alias.clone().unwrap_or_else(|| "VARCHAR".to_owned()),
            Self::HugeInt => "HUGEINT".to_owned(),
            Self::UHugeInt => "UHUGEINT".to_owned(),
            Self::Decimal { width, scale } => format!("DECIMAL({width},{scale})"),
            Self::TimestampTz => "TIMESTAMP WITH TIME ZONE".to_owned(),
            Self::TimeTz => "TIME WITH TIME ZONE".to_owned(),
            Self::Bit => "BIT".to_owned(),
            Self::Bignum => "BIGNUM".to_owned(),
            Self::Enum => "ENUM".to_owned(),
            Self::List(child) => format!("{}[]", child.name()),
            Self::Array(child) => format!("{}[n]", child.name()),
            Self::Map(key, value) => format!("MAP({}, {})", key.name(), value.name()),
            Self::Struct(members) => format!("STRUCT({})", member_list(members)),
            Self::Union(members) => format!("UNION({})", member_list(members)),
            Self::Unknown(raw) => format!("DuckDB type #{raw}"),
        }
    }

    /// The children a nested type pairs with the children of its Arrow
    /// carrier, in Arrow's order.
    #[must_use]
    pub(crate) fn arrow_children(&self) -> Vec<Self> {
        match self {
            Self::List(child) | Self::Array(child) => vec![(**child).clone()],
            // A map travels as a list of `{key, value}` structs.
            Self::Map(key, value) => vec![Self::Struct(vec![
                ("key".to_owned(), (**key).clone()),
                ("value".to_owned(), (**value).clone()),
            ])],
            Self::Struct(members) | Self::Union(members) => {
                members.iter().map(|(_, child)| child.clone()).collect()
            }
            // The dictionary of an enum holds its labels.
            Self::Enum => vec![Self::Varchar(None)],
            _ => Vec::new(),
        }
    }

    /// The name of this type, or of one nested in it, that has no faithful
    /// Arrow form.
    #[must_use]
    pub(crate) fn unfaithful(&self) -> Option<String> {
        match self {
            Self::TimeTz | Self::Unknown(_) => Some(self.name()),
            _ => self.arrow_children().iter().find_map(Self::unfaithful),
        }
    }
}

/// `name TYPE, …`, a name quoted when it is not a plain identifier.
fn member_list(members: &[(String, DuckType)]) -> String {
    members
        .iter()
        .map(|(name, child)| {
            let plain = !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit());
            if plain {
                format!("{name} {}", child.name())
            } else {
                format!("\"{}\" {}", name.replace('"', "\"\""), child.name())
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_like_duckdb_writes_them() {
        let nested = DuckType::Struct(vec![
            ("id".to_owned(), DuckType::HugeInt),
            (
                "tags".to_owned(),
                DuckType::List(Box::new(DuckType::Varchar(None))),
            ),
            (
                "odd \"name\"".to_owned(),
                DuckType::Map(
                    Box::new(DuckType::Plain("INTEGER")),
                    Box::new(DuckType::Bit),
                ),
            ),
        ]);
        assert_eq!(
            nested.name(),
            "STRUCT(id HUGEINT, tags VARCHAR[], \"odd \"\"name\"\"\" MAP(INTEGER, BIT))"
        );
        assert_eq!(
            DuckType::Decimal {
                width: 38,
                scale: 10
            }
            .name(),
            "DECIMAL(38,10)"
        );
        assert_eq!(DuckType::Varchar(Some("JSON".to_owned())).name(), "JSON");
    }

    #[test]
    fn an_unfaithful_type_is_found_at_any_depth() {
        let deep = DuckType::List(Box::new(DuckType::Struct(vec![(
            "t".to_owned(),
            DuckType::TimeTz,
        )])));
        assert_eq!(deep.unfaithful().as_deref(), Some("TIME WITH TIME ZONE"));
        assert!(
            DuckType::List(Box::new(DuckType::HugeInt))
                .unfaithful()
                .is_none()
        );
        assert!(DuckType::Unknown(250).unfaithful().is_some());
    }

    #[test]
    fn a_map_pairs_with_a_list_of_key_value_structs() {
        let map = DuckType::Map(
            Box::new(DuckType::Plain("INTEGER")),
            Box::new(DuckType::UHugeInt),
        );
        let children = map.arrow_children();
        assert_eq!(children.len(), 1);
        assert!(matches!(&children[0], DuckType::Struct(members) if members.len() == 2));
    }
}
