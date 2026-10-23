//! ClickHouse's type names, read into a model the decoder can walk.
//!
//! `RowBinaryWithNamesAndTypes` announces every column's type as **text**
//! (`Nullable(DateTime64(3, 'Europe/Paris'))`), then sends values with no
//! length or tag of their own: how many bytes a value takes is known only from
//! its type. A type this module does not understand therefore cannot be
//! skipped, let alone returned as raw bytes — the next value would be read
//! from the middle of this one. Such a type is [`ChType::Unsupported`], and the
//! result that carries it is refused before its first row, naming the column
//! and the type (ADR-0060).
//!
//! The text comes from the server: it is parsed without recursion limits being
//! assumed, without indexing, and without `unwrap` ([I-09](../../../CLAUDE.md#i-09)).

/// How deep a type may nest before it is refused.
///
/// A server announcing `Array(Array(…))` ten thousand levels deep would
/// otherwise overflow the parser's stack. ClickHouse's own limit on nested
/// types is far below this, so a real type never meets it.
const MAX_DEPTH: usize = 64;

/// How many types, nested ones included, a result header may announce in
/// all. Each becomes an Arrow builder: a header of 64 MiB spelling
/// `Tuple(UInt8, UInt8, …)` would otherwise build ten million of them
/// before the first row.
pub const MAX_NODES: usize = 100_000;

/// Types whose arguments are not read: the driver decodes `JSON` as text and
/// none of the others, so their arguments decide nothing here.
const OPAQUE_ARGUMENTS: [&str; 5] = ["JSON", "Object", "Dynamic", "Variant", "AggregateFunction"];

/// A ClickHouse type, as far as decoding it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChType {
    /// `Bool`, one byte, 0 or 1.
    Bool,
    /// `Int8` … `Int256`, signed, little-endian, width in bits.
    Int(u16),
    /// `UInt8` … `UInt256`, unsigned, little-endian, width in bits.
    UInt(u16),
    /// `Float32`.
    Float32,
    /// `Float64`.
    Float64,
    /// `BFloat16`: the upper half of a `Float32`.
    BFloat16,
    /// `Decimal(P, S)` and its `Decimal32/64/128/256(S)` spellings.
    Decimal {
        /// Digits, 1 to 76.
        precision: u8,
        /// Digits after the point, at most `precision`.
        scale: u8,
    },
    /// `String`: bytes, length-prefixed.
    String,
    /// `FixedString(N)`: exactly `N` bytes.
    FixedString(usize),
    /// `UUID`: sixteen bytes, two little-endian halves.
    Uuid,
    /// `Date`: days since 1970-01-01, unsigned 16 bits.
    Date,
    /// `Date32`: days since 1970-01-01, signed 32 bits.
    Date32,
    /// `DateTime[(tz)]`: seconds since the epoch, unsigned 32 bits.
    DateTime(Option<String>),
    /// `DateTime64(P[, tz])`: ticks of 10^-P seconds, signed 64 bits.
    DateTime64 {
        /// Digits of the fraction, 0 to 9.
        precision: u8,
        /// The zone the column declares, if it declares one.
        zone: Option<String>,
    },
    /// `Time`: signed seconds, 32 bits — a duration, not a time of day.
    Time,
    /// `Time64(P)`: signed ticks of 10^-P seconds, 64 bits.
    Time64(u8),
    /// `Enum8(…)` and `Enum16(…)`: an integer of 8 or 16 bits and its labels.
    Enum {
        /// 8 or 16.
        bits: u8,
        /// Every label and its value, as the type declares them.
        labels: Vec<(String, i16)>,
    },
    /// `IPv4`: an unsigned 32-bit little-endian number.
    Ipv4,
    /// `IPv6`: sixteen bytes in network order.
    Ipv6,
    /// `Interval…`: a signed 64-bit count of the unit the name carries.
    Interval,
    /// `JSON`, sent as text under `output_format_binary_write_json_as_string`.
    Json,
    /// `Nothing`: the type of `NULL` and `[]`, which holds no value.
    Nothing,
    /// `Nullable(T)`: one marker byte, then the value when the marker is 0.
    Nullable(Box<ChType>),
    /// `Array(T)`, and `Nested(…)` which is an array of tuples on the wire.
    Array(Box<ChType>),
    /// `Tuple(…)`: the elements one after the other, names optional.
    Tuple(Vec<(Option<String>, ChType)>),
    /// `Map(K, V)`: a count, then the pairs.
    Map(Box<ChType>, Box<ChType>),
    /// A type this driver cannot decode; the text is the reason.
    Unsupported(String),
}

impl ChType {
    /// The first type, depth first, that this driver cannot decode.
    #[must_use]
    pub fn unsupported(&self) -> Option<&str> {
        match self {
            Self::Unsupported(reason) => Some(reason),
            Self::Nullable(inner) | Self::Array(inner) => inner.unsupported(),
            Self::Map(key, value) => key.unsupported().or_else(|| value.unsupported()),
            Self::Tuple(elements) => elements.iter().find_map(|(_, ty)| ty.unsupported()),
            _ => None,
        }
    }

    /// The types this one is made of, itself included.
    #[must_use]
    pub fn nodes(&self) -> usize {
        1_usize.saturating_add(match self {
            Self::Nullable(inner) | Self::Array(inner) => inner.nodes(),
            Self::Map(key, value) => key.nodes().saturating_add(value.nodes()),
            Self::Tuple(elements) => elements
                .iter()
                .fold(0, |sum, (_, ty)| sum.saturating_add(ty.nodes())),
            _ => 0,
        })
    }

    /// Whether a `JSON` sits anywhere in the type: it is decodable only when
    /// the server sends it as text.
    #[must_use]
    pub fn contains_json(&self) -> bool {
        match self {
            Self::Json => true,
            Self::Nullable(inner) | Self::Array(inner) => inner.contains_json(),
            Self::Map(key, value) => key.contains_json() || value.contains_json(),
            Self::Tuple(elements) => elements.iter().any(|(_, ty)| ty.contains_json()),
            _ => false,
        }
    }

    /// Whether a value of this type may be `NULL`.
    #[must_use]
    pub const fn is_nullable(&self) -> bool {
        matches!(self, Self::Nullable(_) | Self::Nothing)
    }
}

/// Why a type name could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the server announced a type this driver cannot read: {0}")]
pub struct TypeSyntaxError(&'static str);

/// Reads a type name as the server prints it.
///
/// A well-formed name of a type the driver does not decode is not an error: it
/// is [`ChType::Unsupported`], so that the refusal can name the column. Only a
/// text that is not a type name at all fails.
///
/// # Errors
/// [`TypeSyntaxError`] for a text that is not a type name, or nests deeper
/// than `MAX_DEPTH`.
pub fn parse(text: &str) -> Result<ChType, TypeSyntaxError> {
    let mut parser = Parser {
        rest: text.trim(),
        nodes: 0,
    };
    let ty = parser.ty(0)?;
    if parser.rest.trim().is_empty() {
        Ok(ty)
    } else {
        Err(TypeSyntaxError("text after the end of the type"))
    }
}

struct Parser<'a> {
    rest: &'a str,
    nodes: usize,
}

/// One argument of a type: a nested type, a number, a quoted string, or an
/// `'label' = value` enum entry. A named tuple element is a type with a name.
enum Arg {
    Type(Option<String>, ChType),
    Number(i64),
    Text(String),
    Entry(String, i64),
}

impl Parser<'_> {
    fn skip_blanks(&mut self) {
        self.rest = self.rest.trim_start();
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_blanks();
        match self.rest.strip_prefix(c) {
            Some(rest) => {
                self.rest = rest;
                true
            }
            None => false,
        }
    }

    fn word(&mut self) -> Option<String> {
        self.skip_blanks();
        if let Some(rest) = self.rest.strip_prefix('`') {
            self.rest = rest;
            return self.quoted('`');
        }
        let end = self
            .rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(self.rest.len());
        if end == 0 {
            return None;
        }
        let (word, rest) = self.rest.split_at_checked(end)?;
        self.rest = rest;
        Some(word.to_owned())
    }

    /// The body of a quoted text whose opening quote was consumed, with
    /// ClickHouse's backslash escapes and doubled quotes.
    fn quoted(&mut self, quote: char) -> Option<String> {
        let mut out = String::new();
        let mut chars = self.rest.char_indices();
        while let Some((at, c)) = chars.next() {
            if c == '\\' {
                let (_, escaped) = chars.next()?;
                out.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '0' => '\0',
                    other => other,
                });
            } else if c == quote {
                let after = self.rest.get(at + c.len_utf8()..)?;
                if let Some(doubled) = after.strip_prefix(quote) {
                    out.push(quote);
                    self.rest = doubled;
                    chars = self.rest.char_indices();
                    continue;
                }
                self.rest = after;
                return Some(out);
            } else {
                out.push(c);
            }
        }
        None
    }

    fn number(&mut self) -> Option<i64> {
        self.skip_blanks();
        let negative = self.rest.starts_with('-');
        let digits_from = usize::from(negative);
        let body = self.rest.get(digits_from..)?;
        let end = body
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(body.len());
        if end == 0 {
            return None;
        }
        let text = self.rest.get(..digits_from + end)?;
        let value = text.parse().ok()?;
        self.rest = self.rest.get(digits_from + end..)?;
        Some(value)
    }

    fn ty(&mut self, depth: usize) -> Result<ChType, TypeSyntaxError> {
        if depth > MAX_DEPTH {
            return Err(TypeSyntaxError("a type nested beyond 64 levels"));
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(TypeSyntaxError("a type of more than 100,000 parts"));
        }
        let name = self
            .word()
            .ok_or(TypeSyntaxError("a type name was expected"))?;
        if OPAQUE_ARGUMENTS.contains(&name.as_str()) {
            // Their arguments are not types the decoder walks — `JSON` takes
            // `max_dynamic_paths = 8` and path hints, `AggregateFunction` a
            // function — so they are skipped, balanced, and not interpreted.
            if self.eat('(') {
                self.skip_balanced()?;
            }
            return Ok(build(&name, Vec::new()));
        }
        let args = if self.eat('(') {
            self.args(depth)?
        } else {
            Vec::new()
        };
        Ok(build(&name, args))
    }

    /// Consumes up to the `)` that closes an already opened `(`.
    fn skip_balanced(&mut self) -> Result<(), TypeSyntaxError> {
        let mut open = 1_usize;
        loop {
            let mut chars = self.rest.chars();
            let c = chars
                .next()
                .ok_or(TypeSyntaxError("an unterminated argument list"))?;
            self.rest = chars.as_str();
            match c {
                '(' => open += 1,
                ')' => {
                    open -= 1;
                    if open == 0 {
                        return Ok(());
                    }
                }
                '\'' | '`' => {
                    self.quoted(c)
                        .ok_or(TypeSyntaxError("an unterminated string"))?;
                }
                _ => {}
            }
        }
    }

    fn args(&mut self, depth: usize) -> Result<Vec<Arg>, TypeSyntaxError> {
        let mut args = Vec::new();
        if self.eat(')') {
            return Ok(args);
        }
        loop {
            args.push(self.arg(depth)?);
            if self.eat(')') {
                return Ok(args);
            }
            if !self.eat(',') {
                return Err(TypeSyntaxError("`,` or `)` was expected"));
            }
        }
    }

    fn arg(&mut self, depth: usize) -> Result<Arg, TypeSyntaxError> {
        self.skip_blanks();
        if let Some(rest) = self.rest.strip_prefix('\'') {
            self.rest = rest;
            let text = self
                .quoted('\'')
                .ok_or(TypeSyntaxError("an unterminated string"))?;
            if self.eat('=') {
                let value = self
                    .number()
                    .ok_or(TypeSyntaxError("an enum value was expected"))?;
                return Ok(Arg::Entry(text, value));
            }
            return Ok(Arg::Text(text));
        }
        if self
            .rest
            .starts_with(|c: char| c.is_ascii_digit() || c == '-')
        {
            return self
                .number()
                .map(Arg::Number)
                .ok_or(TypeSyntaxError("a number out of range"));
        }
        // `name Type` inside a tuple or a nested: a word followed by another
        // type. Two words in a row cannot be anything else.
        let checkpoint = self.rest;
        let first = self
            .word()
            .ok_or(TypeSyntaxError("an argument was expected"))?;
        self.skip_blanks();
        if self
            .rest
            .starts_with(|c: char| c.is_ascii_alphabetic() || c == '`')
        {
            return Ok(Arg::Type(Some(first), self.ty(depth + 1)?));
        }
        self.rest = checkpoint;
        Ok(Arg::Type(None, self.ty(depth + 1)?))
    }
}

/// A named type, once its arguments are read.
fn build(name: &str, args: Vec<Arg>) -> ChType {
    let simple = |ty: ChType| {
        if args.is_empty() {
            ty
        } else {
            unsupported(name)
        }
    };
    match name {
        "Bool" | "Boolean" => simple(ChType::Bool),
        "Int8" => simple(ChType::Int(8)),
        "Int16" => simple(ChType::Int(16)),
        "Int32" => simple(ChType::Int(32)),
        "Int64" => simple(ChType::Int(64)),
        "Int128" => simple(ChType::Int(128)),
        "Int256" => simple(ChType::Int(256)),
        "UInt8" => simple(ChType::UInt(8)),
        "UInt16" => simple(ChType::UInt(16)),
        "UInt32" => simple(ChType::UInt(32)),
        "UInt64" => simple(ChType::UInt(64)),
        "UInt128" => simple(ChType::UInt(128)),
        "UInt256" => simple(ChType::UInt(256)),
        "Float32" => simple(ChType::Float32),
        "Float64" => simple(ChType::Float64),
        "BFloat16" => simple(ChType::BFloat16),
        "String" => simple(ChType::String),
        "UUID" => simple(ChType::Uuid),
        "Date" => simple(ChType::Date),
        "Date32" => simple(ChType::Date32),
        "Time" => simple(ChType::Time),
        "IPv4" => simple(ChType::Ipv4),
        "IPv6" => simple(ChType::Ipv6),
        "Nothing" => simple(ChType::Nothing),
        "JSON" => ChType::Json,
        "Point" => simple(point()),
        "Ring" | "LineString" => simple(array(point())),
        "Polygon" | "MultiLineString" => simple(array(array(point()))),
        "MultiPolygon" => simple(array(array(array(point())))),
        "Decimal" => decimal(name, &args),
        "Decimal32" => decimal_of_width(name, 9, &args),
        "Decimal64" => decimal_of_width(name, 18, &args),
        "Decimal128" => decimal_of_width(name, 38, &args),
        "Decimal256" => decimal_of_width(name, 76, &args),
        "FixedString" => match args.as_slice() {
            [Arg::Number(n)] => usize::try_from(*n)
                .ok()
                .filter(|n| *n > 0)
                .map_or_else(|| unsupported(name), ChType::FixedString),
            _ => unsupported(name),
        },
        "DateTime" => match args.as_slice() {
            [] => ChType::DateTime(None),
            [Arg::Text(zone)] => ChType::DateTime(Some(zone.clone())),
            _ => unsupported(name),
        },
        "DateTime64" => match args.as_slice() {
            [Arg::Number(p)] => datetime64(name, *p, None),
            [Arg::Number(p), Arg::Text(zone)] => datetime64(name, *p, Some(zone.clone())),
            _ => unsupported(name),
        },
        "Time64" => match args.as_slice() {
            [Arg::Number(p)] => u8::try_from(*p)
                .ok()
                .filter(|p| *p <= 9)
                .map_or_else(|| unsupported(name), ChType::Time64),
            _ => unsupported(name),
        },
        "Enum8" | "Enum16" => enumeration(name, args),
        "Nullable" => one_type(args).map_or_else(|| unsupported(name), nullable),
        "LowCardinality" => one_type(args).unwrap_or_else(|| unsupported(name)),
        "Array" => one_type(args).map_or_else(|| unsupported(name), array),
        "SimpleAggregateFunction" => match args.into_iter().last() {
            Some(Arg::Type(None, ty)) => ty,
            _ => unsupported(name),
        },
        "Tuple" => tuple(name, args),
        "Nested" => match tuple(name, args) {
            ChType::Tuple(elements) => array(ChType::Tuple(elements)),
            other => other,
        },
        "Map" => match <[Arg; 2]>::try_from(args) {
            Ok([Arg::Type(None, key), Arg::Type(None, value)]) if !key.is_nullable() => {
                ChType::Map(Box::new(key), Box::new(value))
            }
            _ => unsupported(name),
        },
        interval if interval.starts_with("Interval") && args.is_empty() => ChType::Interval,
        _ => unsupported(name),
    }
}

fn unsupported(name: &str) -> ChType {
    ChType::Unsupported(name.to_owned())
}

fn point() -> ChType {
    ChType::Tuple(vec![(None, ChType::Float64), (None, ChType::Float64)])
}

fn array(inner: ChType) -> ChType {
    ChType::Array(Box::new(inner))
}

/// `Nullable(T)`, where a `Nullable(Nothing)` is the type of a bare `NULL`.
fn nullable(inner: ChType) -> ChType {
    match inner {
        ChType::Nothing => ChType::Nothing,
        other => ChType::Nullable(Box::new(other)),
    }
}

fn one_type(args: Vec<Arg>) -> Option<ChType> {
    match <[Arg; 1]>::try_from(args) {
        Ok([Arg::Type(None, ty)]) => Some(ty),
        _ => None,
    }
}

fn decimal(name: &str, args: &[Arg]) -> ChType {
    match args {
        [Arg::Number(p), Arg::Number(s)] => decimal_checked(name, *p, *s),
        [Arg::Number(p)] => decimal_checked(name, *p, 0),
        _ => unsupported(name),
    }
}

fn decimal_of_width(name: &str, precision: i64, args: &[Arg]) -> ChType {
    match args {
        [Arg::Number(s)] => decimal_checked(name, precision, *s),
        _ => unsupported(name),
    }
}

fn decimal_checked(name: &str, precision: i64, scale: i64) -> ChType {
    match (u8::try_from(precision), u8::try_from(scale)) {
        (Ok(precision), Ok(scale)) if (1..=76).contains(&precision) && scale <= precision => {
            ChType::Decimal { precision, scale }
        }
        _ => unsupported(name),
    }
}

fn datetime64(name: &str, precision: i64, zone: Option<String>) -> ChType {
    match u8::try_from(precision) {
        Ok(precision) if precision <= 9 => ChType::DateTime64 { precision, zone },
        _ => unsupported(name),
    }
}

fn enumeration(name: &str, args: Vec<Arg>) -> ChType {
    let bits = if name == "Enum8" { 8 } else { 16 };
    let mut labels = Vec::with_capacity(args.len());
    for arg in args {
        let Arg::Entry(label, value) = arg else {
            return unsupported(name);
        };
        let fits = if bits == 8 {
            i8::try_from(value).is_ok()
        } else {
            i16::try_from(value).is_ok()
        };
        match i16::try_from(value) {
            Ok(value) if fits => labels.push((label, value)),
            _ => return unsupported(name),
        }
    }
    ChType::Enum { bits, labels }
}

fn tuple(name: &str, args: Vec<Arg>) -> ChType {
    let mut elements = Vec::with_capacity(args.len());
    for arg in args {
        let Arg::Type(label, ty) = arg else {
            return unsupported(name);
        };
        elements.push((label, ty));
    }
    if elements.is_empty() {
        return unsupported(name);
    }
    ChType::Tuple(elements)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("UInt64", ChType::UInt(64))]
    #[case("Int256", ChType::Int(256))]
    #[case("Decimal(76, 30)", ChType::Decimal { precision: 76, scale: 30 })]
    #[case("Decimal64(4)", ChType::Decimal { precision: 18, scale: 4 })]
    #[case("FixedString(16)", ChType::FixedString(16))]
    #[case("DateTime", ChType::DateTime(None))]
    #[case("DateTime('Asia/Tokyo')", ChType::DateTime(Some("Asia/Tokyo".into())))]
    #[case(
        "DateTime64(9, 'Europe/Paris')",
        ChType::DateTime64 { precision: 9, zone: Some("Europe/Paris".into()) }
    )]
    #[case(
        "LowCardinality(Nullable(String))",
        ChType::Nullable(Box::new(ChType::String))
    )]
    #[case("Nullable(Nothing)", ChType::Nothing)]
    #[case("SimpleAggregateFunction(sum, UInt64)", ChType::UInt(64))]
    #[case("IntervalDay", ChType::Interval)]
    fn type_names_read_as_the_server_prints_them(#[case] text: &str, #[case] expected: ChType) {
        assert_eq!(parse(text), Ok(expected), "{text}");
    }

    #[test]
    fn composite_types_keep_their_structure_and_names() {
        assert_eq!(
            parse("Map(String, Array(Tuple(a UInt8, `b c` Nullable(String))))"),
            Ok(ChType::Map(
                Box::new(ChType::String),
                Box::new(array(ChType::Tuple(vec![
                    (Some("a".into()), ChType::UInt(8)),
                    (
                        Some("b c".into()),
                        ChType::Nullable(Box::new(ChType::String))
                    ),
                ])))
            ))
        );
        assert_eq!(
            parse("Tuple(UInt8, String)"),
            Ok(ChType::Tuple(vec![
                (None, ChType::UInt(8)),
                (None, ChType::String)
            ]))
        );
        assert_eq!(
            parse("Nested(id UInt32)"),
            Ok(array(ChType::Tuple(vec![(
                Some("id".into()),
                ChType::UInt(32)
            )])))
        );
        assert_eq!(parse("MultiPolygon"), Ok(array(array(array(point())))));
    }

    #[test]
    fn enum_labels_survive_quotes_escapes_and_negative_values() {
        assert_eq!(
            parse(r"Enum8('a' = 1, 'it\'s' = -2, 'x''y' = 3)"),
            Ok(ChType::Enum {
                bits: 8,
                labels: vec![("a".into(), 1), ("it's".into(), -2), ("x'y".into(), 3),],
            })
        );
        assert!(matches!(
            parse("Enum8('a' = 300)"),
            Ok(ChType::Unsupported(_))
        ));
    }

    #[rstest]
    #[case("Variant(String, UInt8)")]
    #[case("Dynamic")]
    #[case("AggregateFunction(uniq, UInt64)")]
    #[case("Object('json')")]
    #[case("SomeTypeOfTheFuture")]
    #[case("Decimal(77, 0)")]
    #[case("DateTime64(10)")]
    #[case("FixedString(0)")]
    #[case("Map(Nullable(String), UInt8)")]
    fn a_type_the_driver_cannot_decode_is_named_not_guessed(#[case] text: &str) {
        let parsed = parse(text).expect("a well-formed name");
        assert!(parsed.unsupported().is_some(), "{text}: {parsed:?}");
    }

    #[test]
    fn an_unsupported_type_is_found_however_deep_it_sits() {
        let parsed = parse("Array(Map(String, Tuple(UInt8, Dynamic)))").expect("well formed");
        assert_eq!(parsed.unsupported(), Some("Dynamic"));
    }

    #[rstest]
    #[case("")]
    #[case("Array(")]
    #[case("Array(UInt8")]
    #[case("Enum8('a = 1)")]
    #[case("UInt8 UInt8")]
    #[case("Tuple(,)")]
    #[case("FixedString(99999999999999999999999)")]
    fn hostile_type_texts_fail_without_panicking(#[case] text: &str) {
        let outcome = parse(text);
        assert!(
            outcome.is_err() || outcome.as_ref().is_ok_and(|ty| ty.unsupported().is_some()),
            "{text}: {outcome:?}"
        );
    }

    #[test]
    fn a_type_nested_without_end_is_refused_not_a_stack_overflow() {
        let deep = format!("{}UInt8{}", "Array(".repeat(10_000), ")".repeat(10_000));
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn a_type_too_wide_to_build_is_refused_and_parts_are_counted() {
        let wide = format!("Tuple({})", vec!["UInt8"; MAX_NODES].join(", "));
        assert!(parse(&wide).is_err());
        let parsed = parse("Map(String, Array(Nullable(UInt8)))").expect("well formed");
        assert_eq!(parsed.nodes(), 5);
    }
}
