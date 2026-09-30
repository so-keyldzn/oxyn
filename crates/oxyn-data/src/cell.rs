//! Rendering a cell, called once per visible cell and per frame.
//!
//! # What governs this file
//!
//! **`NULL` is not a string.** [`CellValue::Null`] is a variant, never the
//! text `"NULL"`: a text column really containing the string `NULL` must be
//! distinguishable from an absent value. The interface decides on italics and
//! grey ([UX-SPEC](../../../docs/UX-SPEC.md)).
//!
//! **A rendering failure does not disguise itself as `NULL`.** A type that
//! cannot be displayed returns [`CellValue::Unrenderable`], not an empty
//! cell: the latter is a silent lie about real data.
//!
//! **Text is lent, not copied.** A `Utf8` column — by far the most frequent
//! case — returns a [`Cow::Borrowed`] over the Arrow buffer. No allocation
//! between the `RecordBatch` and the screen.
//!
//! # What is delegated to Arrow, and why
//!
//! Dates, times, timestamps (time zone included), lists, structs,
//! dictionaries and floats go through [`arrow::util::display::ArrayFormatter`].
//! Two reasons, neither of which is laziness: converting a timestamp with a
//! time zone is calendar work that `chrono` does correctly and that is not in
//! this crate's dependency contract; and above all **what the grid displays
//! must be exactly what the CSV export writes**, and the export goes through
//! these same formatters. Two implementations would diverge within a few
//! months, and the user would only find out by comparing an exported file to
//! their screen.
//!
//! Durations are the exception, and the export follows it: Arrow writes them
//! in ISO 8601, and both paths go through the crate's `duration` module
//! instead.

use std::borrow::Cow;
use std::fmt::Write as _;

use arrow::array::{Array, AsArray};
use arrow::datatypes::{
    DataType, Decimal128Type, Decimal256Type, Int8Type, Int16Type, Int32Type, Int64Type, Schema,
    UInt8Type, UInt16Type, UInt32Type, UInt64Type,
};
use arrow::record_batch::RecordBatch;
use arrow::util::display::{ArrayFormatter, FormatOptions as ArrowFormatOptions};
use serde::{Deserialize, Serialize};

use crate::duration;

/// Length beyond which a cell is cut for display.
///
/// A grid cell is a few tens of characters; 512 leaves room for a tooltip
/// without ever rendering a 4 MB JSON document.
pub const DEFAULT_MAX_LEN: usize = 512;

/// What a cell gives to display.
///
/// Borrows from the `RecordBatch` when possible, hence the lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CellValue<'a> {
    /// Absence of value. **Not** the string `"NULL"`: the interface draws it
    /// its own way, and a text column can literally contain `NULL`.
    Null,
    /// The whole value.
    Text(Cow<'a, str>),
    /// The value, cut for display.
    Truncated {
        /// The beginning of the value, cut on a character boundary.
        text: Cow<'a, str>,
        /// Size of the full value, **in bytes**.
        ///
        /// In bytes and not in characters because it is an O(1) piece of
        /// information: counting the characters of a 10 MB document once per
        /// cell and per frame would cost the whole display budget.
        full_bytes: usize,
    },
    /// The column's type could not be rendered.
    ///
    /// The interface must show it as such — a badge, a question mark — never
    /// as an empty cell.
    Unrenderable {
        /// What failed, in one line, without a database value.
        reason: Cow<'static, str>,
    },
}

impl CellValue<'_> {
    /// Is the cell null?
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Is the displayed value cut?
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        matches!(self, Self::Truncated { .. })
    }

    /// The text to draw, if there is one.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(txt) | Self::Truncated { text: txt, .. } => Some(txt),
            Self::Null | Self::Unrenderable { .. } => None,
        }
    }

    /// The text to draw, with the configured replacement for `NULL`.
    #[must_use]
    pub fn display_with<'b>(&'b self, opts: &'b FormatOptions) -> &'b str {
        match self {
            Self::Text(txt) | Self::Truncated { text: txt, .. } => txt,
            Self::Null => &opts.null_text,
            Self::Unrenderable { reason } => reason,
        }
    }

    /// Detaches the value from the `RecordBatch` it comes from.
    ///
    /// Allocates if the value was borrowed: to be kept for cases where the cell
    /// outlives the batch — clipboard, agent context — never for rendering.
    #[must_use]
    pub fn into_owned(self) -> CellValue<'static> {
        match self {
            Self::Null => CellValue::Null,
            Self::Text(txt) => CellValue::Text(Cow::Owned(txt.into_owned())),
            Self::Truncated { text, full_bytes } => CellValue::Truncated {
                text: Cow::Owned(text.into_owned()),
                full_bytes,
            },
            Self::Unrenderable { reason } => CellValue::Unrenderable { reason },
        }
    }
}

/// How to render a binary column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BinaryDisplay {
    /// Lowercase hexadecimal, no separator: `48656c6c6f`.
    #[default]
    Hex,
    /// Standard Base64 with padding: `SGVsbG8=`.
    Base64,
    /// Only the size: `<5 B>`, `<1.2 MiB>`.
    ///
    /// It is the right default for a column of photos: displaying 4 MB of
    /// hexadecimal informs nobody and costs a frame.
    Size,
}

/// How to group the digits of a number.
///
/// # Why this setting exists
///
/// `4823917` and `102` aligned in a column cannot be compared at a glance:
/// one has to count the digits. It is the only work the grid can spare
/// someone reading a column of integers.
///
/// # Why it defaults to [`None`](Self::None)
///
/// The export builds its own options ([`crate::export::ExportOptions`]) and
/// does not go through here, but the module-level rule — *what the grid
/// displays must be exactly what the export writes* — holds as a safeguard:
/// grouping active by default would make the screen diverge from the file
/// without anyone asking for it. It is a reading comfort, so it is an explicit
/// choice of the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum NumberGrouping {
    /// The digits in a row: `4823917`. It is what the server writes.
    #[default]
    None,
    /// In groups of three, separated by a no-break space: `4 823 917`.
    ///
    /// A no-break space ([`GROUP_SEPARATOR`]) and not a comma or a period:
    /// those two are **decimal** separators in half the world, and `1,234`
    /// read by someone whose convention it is means a thousandth of what is
    /// displayed. A space cannot be mistaken for anything.
    Thousands,
}

/// What separates two groups of three digits: U+00A0, no-break space.
///
/// No-break so that the number is not cut at the end of a cell.
pub const GROUP_SEPARATOR: char = '\u{a0}';

/// Rendering settings of a cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FormatOptions {
    /// Characters beyond which the value is cut. `0` = no cut.
    pub max_len: usize,
    /// What the interface writes in place of an absent value, when it chooses
    /// to write something.
    pub null_text: Cow<'static, str>,
    /// Timestamp formatting pattern, in the `chrono` sense.
    ///
    /// `None` = RFC 3339, which is also what the export writes: keeping the
    /// default is what makes the screen and the file comparable.
    pub timestamp_format: Option<Cow<'static, str>>,
    /// Rendering of binary columns.
    pub binary_display: BinaryDisplay,
    /// Digit grouping of integers and decimals.
    ///
    /// `#[serde(default)]`: a document written before this field existed stays
    /// readable ([I-11](../../../CLAUDE.md#i-11)). Without this annotation,
    /// adding a display setting would make a workspace saved the day before
    /// unreadable.
    #[serde(default)]
    pub number_grouping: NumberGrouping,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            max_len: DEFAULT_MAX_LEN,
            null_text: Cow::Borrowed("NULL"),
            timestamp_format: None,
            binary_display: BinaryDisplay::default(),
            number_grouping: NumberGrouping::default(),
        }
    }
}

impl FormatOptions {
    /// Changes the cut length.
    #[must_use]
    pub fn with_max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len;
        self
    }

    /// Changes the replacement text of absent values.
    #[must_use]
    pub fn with_null_text(mut self, txt: impl Into<Cow<'static, str>>) -> Self {
        self.null_text = txt.into();
        self
    }

    /// Changes the timestamp pattern.
    #[must_use]
    pub fn with_timestamp_format(mut self, pattern: impl Into<Option<Cow<'static, str>>>) -> Self {
        self.timestamp_format = pattern.into();
        self
    }

    /// Changes the rendering of binary columns.
    #[must_use]
    pub fn with_binary_display(mut self, mode: BinaryDisplay) -> Self {
        self.binary_display = mode;
        self
    }

    /// Changes the digit grouping.
    #[must_use]
    pub fn with_number_grouping(mut self, mode: NumberGrouping) -> Self {
        self.number_grouping = mode;
        self
    }

    /// Translates these settings for Arrow's formatters.
    ///
    /// `with_display_error(false)`: a formatting error must come up to become
    /// [`CellValue::Unrenderable`], rather than be written in the cell where
    /// the user expects a value.
    ///
    /// `null_text` only concerns **nested** absent values here — those of a
    /// list or a struct. A null cell never reaches this path: it returns
    /// [`CellValue::Null`].
    fn arrow(&self) -> ArrowFormatOptions<'_> {
        let pattern = self.timestamp_format.as_deref();
        ArrowFormatOptions::new()
            .with_display_error(false)
            .with_null(&self.null_text)
            .with_timestamp_format(pattern)
            .with_timestamp_tz_format(pattern)
    }
}

/// Renders the cell `(row, col)` of `batch`.
///
/// Never panics: an out-of-bounds index, an unexpected type or a formatting
/// failure return [`CellValue::Unrenderable`]
/// ([I-09](../../../CLAUDE.md#i-09)). An absent value returns
/// [`CellValue::Null`], never text.
///
/// Does not allocate for `Utf8`, `LargeUtf8` and `Utf8View` columns, which
/// are most of what a grid displays.
#[must_use]
pub fn format_cell<'a>(
    batch: &'a RecordBatch,
    row: usize,
    col: usize,
    opts: &FormatOptions,
) -> CellValue<'a> {
    let Some(column) = batch.columns().get(col) else {
        return unrenderable("column index out of range");
    };
    if row >= column.len() {
        return unrenderable("row index out of range");
    }
    if column.is_null(row) {
        return CellValue::Null;
    }
    format_value(&**column, row, opts)
}

/// Renders a non-null value of any Arrow array.
///
/// Separate from [`format_cell`] so that the export and the tests can format a
/// column without building a `RecordBatch`.
///
/// # Preconditions
///
/// `row < array.len()` and the value is not null; both are checked by
/// [`format_cell`]. Called directly, it returns `Unrenderable` rather than
/// panicking.
#[must_use]
pub fn format_value<'a>(array: &'a dyn Array, row: usize, opts: &FormatOptions) -> CellValue<'a> {
    if row >= array.len() {
        return unrenderable("row index out of range");
    }
    if array.is_null(row) {
        return CellValue::Null;
    }

    match array.data_type() {
        DataType::Null => CellValue::Null,

        DataType::Boolean => match array.as_boolean_opt() {
            Some(values) => CellValue::Text(Cow::Borrowed(if values.value(row) {
                "true"
            } else {
                "false"
            })),
            None => delegate(array, row, opts),
        },

        // Integers are formatted by hand: Rust's `Display` and Arrow's
        // formatter produce the same string, and this avoids building a
        // formatter per cell.
        DataType::Int8 => integer::<Int8Type>(array, row, opts),
        DataType::Int16 => integer::<Int16Type>(array, row, opts),
        DataType::Int32 => integer::<Int32Type>(array, row, opts),
        DataType::Int64 => integer::<Int64Type>(array, row, opts),
        DataType::UInt8 => integer::<UInt8Type>(array, row, opts),
        DataType::UInt16 => integer::<UInt16Type>(array, row, opts),
        DataType::UInt32 => integer::<UInt32Type>(array, row, opts),
        DataType::UInt64 => integer::<UInt64Type>(array, row, opts),

        DataType::Utf8 => match array.as_string_opt::<i32>() {
            Some(values) => finish(Cow::Borrowed(values.value(row)), opts.max_len),
            None => delegate(array, row, opts),
        },
        DataType::LargeUtf8 => match array.as_string_opt::<i64>() {
            Some(values) => finish(Cow::Borrowed(values.value(row)), opts.max_len),
            None => delegate(array, row, opts),
        },
        DataType::Utf8View => match array.as_string_view_opt() {
            Some(values) => finish(Cow::Borrowed(values.value(row)), opts.max_len),
            None => delegate(array, row, opts),
        },

        DataType::Binary => match array.as_binary_opt::<i32>() {
            Some(values) => binary(values.value(row), opts),
            None => delegate(array, row, opts),
        },
        DataType::LargeBinary => match array.as_binary_opt::<i64>() {
            Some(values) => binary(values.value(row), opts),
            None => delegate(array, row, opts),
        },
        DataType::BinaryView => match array.as_binary_view_opt() {
            Some(values) => binary(values.value(row), opts),
            None => delegate(array, row, opts),
        },
        DataType::FixedSizeBinary(_) => match array
            .as_any()
            .downcast_ref::<arrow::array::FixedSizeBinaryArray>()
        {
            Some(values) => binary(values.value(row), opts),
            None => delegate(array, row, opts),
        },

        // `value_as_string` places the decimal point according to the column's
        // scale. Formatting the underlying integer would display 12345 for 123.45.
        DataType::Decimal128(_, _) => match array.as_primitive_opt::<Decimal128Type>() {
            Some(values) => finish(
                Cow::Owned(group(values.value_as_string(row), opts.number_grouping)),
                opts.max_len,
            ),
            None => delegate(array, row, opts),
        },
        // MySQL sends `DECIMAL` up to 65 digits: beyond 38 it arrives here, and
        // is rendered from the exact integer and scale, like `Decimal128` —
        // never through a float, which keeps 17 significant digits.
        DataType::Decimal256(_, _) => match array.as_primitive_opt::<Decimal256Type>() {
            Some(values) => finish(
                Cow::Owned(group(values.value_as_string(row), opts.number_grouping)),
                opts.max_len,
            ),
            None => delegate(array, row, opts),
        },

        // Not delegated: Arrow writes ISO 8601 (`-PT3023999S`), and a MySQL
        // `TIME` is read as `-838:59:59`. See `crate::duration`.
        DataType::Duration(_) => match duration::value_at(array, row) {
            Some((value, unit)) => {
                let mut txt = String::new();
                if duration::write(&mut txt, value, unit).is_err() {
                    return unrenderable("duration formatting failed");
                }
                finish(Cow::Owned(txt), opts.max_len)
            }
            None => delegate(array, row, opts),
        },

        // Dates, times, timestamps, intervals, lists, structs,
        // maps, dictionaries, floats: see the note at the top of the module.
        _ => delegate(array, row, opts),
    }
}

/// Renders an integer of any width.
fn integer<'a, T>(array: &'a dyn Array, row: usize, opts: &FormatOptions) -> CellValue<'a>
where
    T: arrow::datatypes::ArrowPrimitiveType,
    T::Native: std::fmt::Display,
{
    match array.as_primitive_opt::<T>() {
        Some(values) => {
            let mut txt = String::new();
            if write!(txt, "{}", values.value(row)).is_err() {
                return unrenderable("integer formatting failed");
            }
            finish(Cow::Owned(group(txt, opts.number_grouping)), opts.max_len)
        }
        None => delegate(array, row, opts),
    }
}

/// Inserts thousands separators into an already formatted number.
///
/// Returns the string **as is** when grouping is off: it is the default, so
/// by far the most frequent case, and it must cost nothing. This function is
/// called once per visible numeric cell and per frame — the budget is 8 ms
/// for the whole frame
/// ([PERFORMANCE](../../../docs/PERFORMANCE.md#interaction-budgets)).
///
/// Groups only the integer part, and leaves the sign, the decimal part and a
/// possible exponent intact: `-1234.5678` becomes `-1 234.5678`, never
/// `-1 234.567 8`. Grouping after the decimal point is a typographic mistake
/// that makes the decimals unreadable.
fn group(txt: String, mode: NumberGrouping) -> String {
    if matches!(mode, NumberGrouping::None) {
        return txt;
    }
    // The integer part stops at the first character that is not a digit,
    // skipping a leading sign. A text without digits — which should not
    // reach this function — comes out unchanged rather than mangled.
    //
    // Everything is sliced with `get`, never indexed: the bounds are derived
    // from the text, and that text comes from formatting a **server** value. A
    // slicing proven right today becomes a panic at the first type whose
    // Arrow formatter returns something other than what is assumed here
    // ([I-09](../../../CLAUDE.md#i-09)).
    let start = usize::from(txt.starts_with(['-', '+']));
    let Some(rest) = txt.get(start..) else {
        return txt;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits <= 3 {
        return txt;
    }
    let end = start.saturating_add(digits);
    let (Some(sign), Some(int_value), Some(suite)) =
        (txt.get(..start), txt.get(start..end), txt.get(end..))
    else {
        return txt;
    };

    let mut output = String::with_capacity(txt.len() + (digits / 3) * GROUP_SEPARATOR.len_utf8());
    output.push_str(sign);
    for (rank, digit) in int_value.chars().enumerate() {
        // A separator falls where a multiple of three digits remains to be
        // written, never at the head of the number.
        if rank > 0 && (digits - rank).is_multiple_of(3) {
            output.push(GROUP_SEPARATOR);
        }
        output.push(digit);
    }
    output.push_str(suite);
    output
}

/// Hands over to Arrow's formatters.
fn delegate<'a>(array: &'a dyn Array, row: usize, opts: &FormatOptions) -> CellValue<'a> {
    let options = opts.arrow();
    let Ok(formatter) = ArrayFormatter::try_new(array, &options) else {
        return unrenderable("unsupported column type");
    };
    match formatter.value(row).try_to_string() {
        Ok(txt) => finish(Cow::Owned(txt), opts.max_len),
        Err(_) => unrenderable("value could not be formatted"),
    }
}

/// Renders a binary value according to [`BinaryDisplay`].
///
/// Encodes only the bytes that will be shown: converting 4 MB of BLOB to
/// hexadecimal to display 512 characters of it would cost the whole frame.
fn binary<'a>(bytes: &[u8], opts: &FormatOptions) -> CellValue<'a> {
    if matches!(opts.binary_display, BinaryDisplay::Size) {
        return CellValue::Text(Cow::Owned(human_size(bytes.len())));
    }

    // A little more than the limit, so that `finish` **observes** the overflow
    // instead of assuming it.
    let useful = match opts.binary_display {
        BinaryDisplay::Base64 if opts.max_len > 0 => {
            // Three source bytes give four characters; staying on a multiple
            // of three avoids emitting `=` padding in the middle of a value
            // that continues.
            bytes.len().min((opts.max_len / 4 + 1) * 3)
        }
        BinaryDisplay::Hex if opts.max_len > 0 => bytes.len().min(opts.max_len / 2 + 1),
        _ => bytes.len(),
    };

    let Some(start) = bytes.get(..useful) else {
        return unrenderable("binary slice out of range");
    };

    let mut txt = String::with_capacity(useful.saturating_mul(2));
    match opts.binary_display {
        BinaryDisplay::Base64 => push_base64(&mut txt, start),
        _ => {
            for byte in start {
                push_hex(&mut txt, *byte);
            }
        }
    }

    // `full_bytes` counts the bytes of the value, not the characters of its
    // encoding: it is the BLOB size the user wants to know.
    let complete = bytes.len();
    match finish(Cow::Owned(txt), opts.max_len) {
        CellValue::Truncated { text, .. } => CellValue::Truncated {
            text,
            full_bytes: complete,
        },
        CellValue::Text(text) if useful < complete => CellValue::Truncated {
            text,
            full_bytes: complete,
        },
        other => other,
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn push_hex(output: &mut String, byte: u8) {
    let high = usize::from(byte >> 4);
    let low = usize::from(byte & 0x0f);
    if let (Some(a), Some(b)) = (HEX.get(high), HEX.get(low)) {
        output.push(char::from(*a));
        output.push(char::from(*b));
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard Base64 (RFC 4648), with padding.
///
/// Written by hand: the only alternative would be a direct dependency for
/// twenty lines, which the dependency policy discourages
/// ([SECURITY](../../../docs/SECURITY.md#dependencies)).
fn push_base64(output: &mut String, bytes: &[u8]) {
    for chunk in bytes.chunks(3) {
        let a = chunk.first().copied().unwrap_or(0);
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        let block = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);

        // 1 source byte → 2 characters then "=="; 2 bytes → 3 then "=".
        let significant = chunk.len().saturating_add(1).min(4);
        for rank in 0..4_usize {
            if rank < significant {
                let index = usize::try_from((block >> (18 - rank * 6)) & 0x3f).unwrap_or(0);
                if let Some(character) = BASE64.get(index) {
                    output.push(char::from(*character));
                }
            } else {
                output.push('=');
            }
        }
    }
}

/// Readable size: `<5 B>`, `<1.2 KiB>`, `<3.3 MiB>`.
///
/// Computed in integers: `usize as f64` loses precision beyond 2^53 and the
/// repository rule forbids silent `as` conversions
/// ([rust.md](../../../.claude/rules/rust.md)).
fn human_size(bytes: usize) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes;
    let mut rest = 0_usize;
    let mut rank = 0_usize;
    while value >= 1024 && rank + 1 < UNITS.len() {
        rest = value % 1024;
        value /= 1024;
        rank += 1;
    }
    let unit = UNITS.get(rank).copied().unwrap_or("B");
    if rank == 0 {
        format!("<{value} {unit}>")
    } else {
        let tenths = rest.saturating_mul(10) / 1024;
        format!("<{value}.{tenths} {unit}>")
    }
}

/// Cuts the value if it exceeds `max_len` characters.
fn finish(txt: Cow<'_, str>, max_len: usize) -> CellValue<'_> {
    // O(1) fast path: fewer bytes than allowed characters, so a fortiori fewer
    // characters.
    if max_len == 0 || txt.len() <= max_len {
        return CellValue::Text(txt);
    }
    let Some((cut, _)) = txt.char_indices().nth(max_len) else {
        return CellValue::Text(txt);
    };
    let full_bytes = txt.len();
    match txt {
        Cow::Borrowed(value) => match value.get(..cut) {
            Some(start) => CellValue::Truncated {
                text: Cow::Borrowed(start),
                full_bytes,
            },
            None => unrenderable("truncation landed inside a character"),
        },
        Cow::Owned(mut value) => {
            value.truncate(cut);
            CellValue::Truncated {
                text: Cow::Owned(value),
                full_bytes,
            }
        }
    }
}

const fn unrenderable<'a>(why: &'static str) -> CellValue<'a> {
    CellValue::Unrenderable {
        reason: Cow::Borrowed(why),
    }
}

/// The time zone in which a result displays its zone-aware instants.
///
/// Derived from the schema rather than chosen: Oxyn converts nothing. A driver
/// declares the zone on the Arrow field — the PostgreSQL driver maps
/// `timestamptz` to `Timestamp(_, Some("UTC"))` — and the cell is rendered in
/// that zone, offset included.
///
/// **Naive timestamps are deliberately ignored.** `timestamp without time zone`
/// arrives as `Timestamp(_, None)` and carries no zone at all; it renders
/// without a `Z` and without an offset. Counting it here would let the interface
/// announce a zone the server never sent, which is the one thing a timestamp
/// label must not do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TimestampDisplay {
    /// No zone-aware column in this result: there is nothing to announce.
    #[default]
    Absent,
    /// Every zone-aware column declares this same zone.
    Uniform(String),
    /// Zone-aware columns disagree. Naming one of them would describe the other
    /// columns wrongly, so the interface must say that they differ instead.
    Mixed,
}

/// The zone in which this schema's instants will be displayed.
///
/// Scans fields once; a result has tens of columns, not thousands, and this is
/// called when a result completes rather than per frame.
/// Maximum length kept for a time zone name coming from the server.
///
/// See the body of [`timestamp_display`]: this name is a hostile input, and it
/// shares its line with the notice that says which execution the displayed
/// rows belong to.
const ZONE_MAX_CHARS: usize = 64;

#[must_use]
pub fn timestamp_display(schema: &Schema) -> TimestampDisplay {
    let mut seen: Option<&str> = None;
    for field in schema.fields() {
        // Only the top level: a zone buried in a struct or a list is not what
        // the footer is describing, and claiming it would overstate.
        let DataType::Timestamp(_, Some(zone)) = field.data_type() else {
            continue;
        };
        match seen {
            None => seen = Some(zone.as_ref()),
            Some(already) if already == zone.as_ref() => {}
            Some(_) => return TimestampDisplay::Mixed,
        }
    }
    // The time zone name comes from the schema, hence from the server: it is a
    // hostile input, including when it is "only" displayed. The footer sentence
    // ends with "Results belong to this execution", the only notice that
    // guarantees the rows come from the current execution; a very long name,
    // or one carrying a direction mark, would push it out of the frame or
    // scramble its reading. A real time zone fits well within the bound —
    // `America/Argentina/ComodRivadavia` is 32 characters.
    seen.map_or(TimestampDisplay::Absent, |zone| {
        let clean: String = zone
            .chars()
            .filter(|character| !character.is_control())
            .take(ZONE_MAX_CHARS)
            .collect();
        if clean.is_empty() {
            // Nothing nameable: stay silent rather than display an empty string
            // after "Display timezone".
            TimestampDisplay::Absent
        } else {
            TimestampDisplay::Uniform(clean)
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{
        BinaryArray, BooleanArray, Date32Array, Decimal128Array, Decimal256Array,
        DurationMicrosecondArray, Float64Array, Int32Array, Int64Array, ListArray, StringArray,
        StructArray, Time64MicrosecondArray, TimestampMillisecondArray, UInt8Array,
    };
    use arrow::datatypes::{Field, Fields, Schema, TimeUnit, i256};

    use super::*;

    fn render(array: Arc<dyn Array>, opts: &FormatOptions) -> CellValue<'static> {
        // `into_owned` detaches the value from the array, which dies at the end
        // of the call: it is the price of a test helper, not of the rendering path.
        format_value(array.as_ref(), 0, opts).into_owned()
    }

    fn as_text(array: Arc<dyn Array>) -> String {
        let opts = FormatOptions::default();
        render(array, &opts)
            .text()
            .map(str::to_owned)
            .unwrap_or_else(|| "<not rendered>".to_owned())
    }

    #[test]
    fn grouping_off_touches_nothing() {
        // It is the default, hence the hot path: what the server sent comes out
        // as is, whatever the number of digits.
        for value in ["0", "-7", "1234567", "-1234567.89", "abc"] {
            assert_eq!(
                group(value.to_owned(), NumberGrouping::None),
                value,
                "{value}"
            );
        }
    }

    #[test]
    fn thousands_grouping_respects_sign_and_decimals() {
        let case = [
            ("0", "0"),
            ("999", "999"),
            ("1000", "1\u{a0}000"),
            ("-1000", "-1\u{a0}000"),
            ("+1000", "+1\u{a0}000"),
            ("4823917", "4\u{a0}823\u{a0}917"),
            // The decimal part is never grouped: "1 234.567 8" is unreadable,
            // and it is not what typography asks for.
            ("1234.5678", "1\u{a0}234.5678"),
            ("-1234567.89", "-1\u{a0}234\u{a0}567.89"),
            // Nothing to group: the text comes out intact rather than mangled.
            ("", ""),
            ("-", "-"),
            ("NaN", "NaN"),
            ("inf", "inf"),
            // Arrow's formatter returns something other than a number for many
            // types. No slicing may fall in the middle of a character.
            ("—12345", "—12345"),
            ("1 234 €", "1 234 €"),
            ("日本語", "日本語"),
            ("+日本", "+日本"),
        ];
        for (entry, expected) in case {
            assert_eq!(
                group(entry.to_owned(), NumberGrouping::Thousands),
                expected,
                "{entry}"
            );
        }
    }

    #[test]
    fn integers_and_decimals_follow_the_setting() {
        let opts = FormatOptions::default().with_number_grouping(NumberGrouping::Thousands);
        let integers = Arc::new(Int64Array::from(vec![Some(-4_823_917)])) as Arc<dyn Array>;
        assert_eq!(
            render(integers, &opts),
            CellValue::Text(Cow::Owned("-4\u{a0}823\u{a0}917".to_owned()))
        );

        // `value_as_string` places the decimal point; grouping must stop
        // before it.
        let decimals = Decimal128Array::from(vec![Some(123_456_789_i128)])
            .with_precision_and_scale(12, 2)
            .expect("valid precision and scale for a test literal");
        assert_eq!(
            render(Arc::new(decimals), &opts),
            CellValue::Text(Cow::Owned("1\u{a0}234\u{a0}567.89".to_owned()))
        );
    }

    #[test]
    fn a_setting_written_before_grouping_stays_readable() {
        // I-11: a workspace saved before the field was added must not become
        // unreadable. That is what `#[serde(default)]` guarantees, and it is
        // the kind of guarantee that is lost at the first field added without
        // thinking about it.
        let old =
            r#"{"max_len":512,"null_text":"NULL","timestamp_format":null,"binary_display":"Hex"}"#;
        let options: FormatOptions =
            serde_json::from_str(old).expect("an earlier document stays readable");
        assert_eq!(options.number_grouping, NumberGrouping::None);
    }

    #[test]
    fn booleans_are_not_allocated() {
        let array = BooleanArray::from(vec![Some(true), Some(false)]);
        let opts = FormatOptions::default();
        assert_eq!(
            format_value(&array, 0, &opts),
            CellValue::Text(Cow::Borrowed("true"))
        );
        assert_eq!(
            format_value(&array, 1, &opts),
            CellValue::Text(Cow::Borrowed("false"))
        );
    }

    #[test]
    fn integers_of_every_width_render() {
        assert_eq!(as_text(Arc::new(Int32Array::from(vec![-42]))), "-42");
        assert_eq!(
            as_text(Arc::new(Int64Array::from(vec![i64::MIN]))),
            i64::MIN.to_string()
        );
        assert_eq!(as_text(Arc::new(UInt8Array::from(vec![255_u8]))), "255");
    }

    #[test]
    fn text_is_borrowed_not_copied() {
        let array = StringArray::from(vec![Some("hello")]);
        let opts = FormatOptions::default();
        match format_value(&array, 0, &opts) {
            CellValue::Text(Cow::Borrowed(value)) => assert_eq!(value, "hello"),
            other => panic!("expected a borrow, got {other:?}"),
        }
    }

    #[test]
    fn an_absent_value_is_null_not_text() {
        let array = StringArray::from(vec![None::<&str>]);
        let opts = FormatOptions::default();
        assert_eq!(format_value(&array, 0, &opts), CellValue::Null);
    }

    /// The trap: a text column that really contains `NULL`.
    #[test]
    fn the_null_string_is_not_an_absent_value() {
        let array = StringArray::from(vec![Some("NULL")]);
        let opts = FormatOptions::default();
        let value = format_value(&array, 0, &opts);
        assert!(!value.is_null());
        assert_eq!(value.text(), Some("NULL"));
    }

    #[test]
    fn a_too_long_value_is_cut_on_a_character_boundary() {
        // 10 characters, 20 bytes: the naive cut at byte 5 would break UTF-8.
        let array = StringArray::from(vec![Some("éàéàéàéàéà")]);
        let opts = FormatOptions::default().with_max_len(5);
        match format_value(&array, 0, &opts) {
            CellValue::Truncated { text, full_bytes } => {
                assert_eq!(text, "éàéàé");
                assert_eq!(full_bytes, 20);
            }
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn a_disabled_cut_leaves_the_whole_value() {
        let long = "x".repeat(10_000);
        let array = StringArray::from(vec![Some(long.as_str())]);
        let opts = FormatOptions::default().with_max_len(0);
        let value = format_value(&array, 0, &opts);
        assert!(!value.is_truncated());
        assert_eq!(value.text().map(str::len), Some(10_000));
    }

    #[test]
    fn binary_renders_as_hexadecimal() {
        let array = BinaryArray::from(vec![Some(b"Hello".as_slice())]);
        let opts = FormatOptions::default();
        assert_eq!(format_value(&array, 0, &opts).text(), Some("48656c6c6f"));
    }

    #[test]
    fn binary_renders_as_base64() {
        let opts = FormatOptions::default().with_binary_display(BinaryDisplay::Base64);
        // The three remainder lengths, which are the three padding cases.
        for (entry, expected) in [
            (b"Hello".as_slice(), "SGVsbG8="),
            (b"Hell".as_slice(), "SGVsbA=="),
            (b"Hel".as_slice(), "SGVs"),
        ] {
            let array = BinaryArray::from(vec![Some(entry)]);
            assert_eq!(
                format_value(&array, 0, &opts).text(),
                Some(expected),
                "input of {} bytes",
                entry.len()
            );
        }
    }

    #[test]
    fn a_large_binary_renders_as_a_size() {
        let large = vec![0_u8; 3_500_000];
        let array = BinaryArray::from(vec![Some(large.as_slice())]);
        let opts = FormatOptions::default().with_binary_display(BinaryDisplay::Size);
        assert_eq!(format_value(&array, 0, &opts).text(), Some("<3.3 MiB>"));
    }

    /// The point that costs a frame if missed: 4 MB of BLOB are not converted
    /// to display 32 characters of it.
    #[test]
    fn a_large_hexadecimal_binary_encodes_only_what_is_shown() {
        let large = vec![0xab_u8; 1_000_000];
        let array = BinaryArray::from(vec![Some(large.as_slice())]);
        let opts = FormatOptions::default().with_max_len(32);
        match format_value(&array, 0, &opts) {
            CellValue::Truncated { text, full_bytes } => {
                assert!(text.len() <= 40, "{} characters produced", text.len());
                assert_eq!(full_bytes, 1_000_000);
            }
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn a_decimal_carries_its_decimal_point() {
        let array = Decimal128Array::from(vec![Some(12_345_i128)])
            .with_precision_and_scale(10, 2)
            .expect("valid precision and scale for 12345");
        assert_eq!(as_text(Arc::new(array)), "123.45");
    }

    /// A MySQL `DECIMAL(65, 30)`: 65 exact digits, which a float would round
    /// after the seventeenth.
    #[test]
    fn a_wide_decimal_keeps_every_digit() {
        let digits = "12345678901234567890123456789012345123456789012345678901234567890";
        let magnitude = i256::from_string(digits).expect("65 digits fit in 256 bits");
        let array = Decimal256Array::from(vec![Some(magnitude), Some(magnitude.wrapping_neg())])
            .with_precision_and_scale(65, 30)
            .expect("valid precision and scale for a MySQL DECIMAL(65, 30)");
        let opts = FormatOptions::default();
        assert_eq!(
            format_value(&array, 0, &opts).text(),
            Some("12345678901234567890123456789012345.123456789012345678901234567890")
        );
        assert_eq!(
            format_value(&array, 1, &opts).text(),
            Some("-12345678901234567890123456789012345.123456789012345678901234567890")
        );

        let whole = Decimal256Array::from(vec![Some(magnitude)])
            .with_precision_and_scale(65, 0)
            .expect("valid precision and scale for a MySQL DECIMAL(65, 0)");
        assert_eq!(as_text(Arc::new(whole)), digits);

        // Grouping stops at the decimal point, as for `Decimal128`.
        let grouped = FormatOptions::default().with_number_grouping(NumberGrouping::Thousands);
        let small = Decimal256Array::from(vec![Some(i256::from_i128(-123_456_789))])
            .with_precision_and_scale(40, 2)
            .expect("valid precision and scale for a test literal");
        assert_eq!(
            format_value(&small, 0, &grouped).text(),
            Some("-1\u{a0}234\u{a0}567.89")
        );
    }

    /// A MySQL `TIME` renders as MySQL prints it, not as ISO 8601.
    #[test]
    fn a_mysql_time_renders_as_hours_minutes_seconds() {
        let bound = 3_020_399_000_000_i64;
        let array = DurationMicrosecondArray::from(vec![
            Some(-bound),
            Some(bound),
            Some(0),
            Some(43_384_500_000),
            Some(i64::MIN),
            None,
        ]);
        let opts = FormatOptions::default();
        for (row, expected) in [
            (0, "-838:59:59.000000"),
            (1, "838:59:59.000000"),
            (2, "00:00:00.000000"),
            (3, "12:03:04.500000"),
            (4, "-2562047788:00:54.775808"),
        ] {
            assert_eq!(format_value(&array, row, &opts).text(), Some(expected));
        }
        assert!(format_value(&array, 5, &opts).is_null());
    }

    #[test]
    fn floats_follow_the_arrow_convention() {
        // `ryu`, like the export: 1.0 and not 1.
        assert_eq!(as_text(Arc::new(Float64Array::from(vec![1.0_f64]))), "1.0");
        assert_eq!(as_text(Arc::new(Float64Array::from(vec![0.5_f64]))), "0.5");
    }

    #[test]
    fn dates_and_times_render() {
        // 2021-01-01 = 18,628 days after the epoch.
        assert_eq!(
            as_text(Arc::new(Date32Array::from(vec![18_628]))),
            "2021-01-01"
        );
        let hour = Time64MicrosecondArray::from(vec![3_661_000_000_i64]);
        let rendered = as_text(Arc::new(hour));
        assert!(rendered.starts_with("01:01:01"), "{rendered}");
    }

    /// The time zone is not decorative: the same value displayed without an
    /// offset gives an hour off by as much, and nobody notices.
    #[test]
    fn a_timestamp_with_a_time_zone_carries_its_offset() {
        let array =
            TimestampMillisecondArray::from(vec![1_609_459_200_000_i64]).with_timezone("+02:00");
        let rendered = as_text(Arc::new(array));
        assert!(rendered.starts_with("2021-01-01T02:00:00"), "{rendered}");
        assert!(rendered.contains("+02:00"), "{rendered}");
    }

    #[test]
    fn postgres_utc_microseconds_render_without_losing_precision_or_nulls() {
        let array =
            arrow::array::TimestampMicrosecondArray::from(vec![Some(1_609_459_200_123_456), None])
                .with_timezone("UTC");
        let opts = FormatOptions::default();
        assert_eq!(
            format_value(&array, 0, &opts).text(),
            Some("2021-01-01T00:00:00.123456Z")
        );
        assert!(format_value(&array, 1, &opts).is_null());
    }

    #[test]
    fn a_custom_timestamp_pattern_is_respected() {
        let array = TimestampMillisecondArray::from(vec![1_609_459_200_000_i64]);
        let opts = FormatOptions::default().with_timestamp_format(Some(Cow::Borrowed("%Y/%m/%d")));
        assert_eq!(format_value(&array, 0, &opts).text(), Some("2021/01/01"));
    }

    #[test]
    fn a_list_renders() {
        let array = ListArray::from_iter_primitive::<arrow::datatypes::Int32Type, _, _>(vec![
            Some(vec![Some(1), Some(2), None]),
        ]);
        // The exact form belongs to Arrow; what is checked here is that the
        // nested column is rendered rather than declared unrecoverable.
        let rendered = as_text(Arc::new(array));
        assert!(rendered.starts_with('['), "{rendered}");
        assert!(
            rendered.contains('1') && rendered.contains('2'),
            "{rendered}"
        );
        assert!(rendered.ends_with(']'), "{rendered}");
    }

    #[test]
    fn a_struct_renders() {
        let fields = Fields::from(vec![
            Field::new("a", DataType::Int32, false),
            Field::new("b", DataType::Utf8, false),
        ]);
        let array = StructArray::new(
            fields,
            vec![
                Arc::new(Int32Array::from(vec![7])),
                Arc::new(StringArray::from(vec!["sept"])),
            ],
            None,
        );
        let rendered = as_text(Arc::new(array));
        assert!(
            rendered.contains('7') && rendered.contains("sept"),
            "{rendered}"
        );
    }

    #[test]
    fn an_entirely_null_column_renders_null() {
        let array = arrow::array::NullArray::new(3);
        let opts = FormatOptions::default();
        assert_eq!(format_value(&array, 1, &opts), CellValue::Null);
    }

    /// An out-of-bounds index is a caller bug, never a panic
    /// ([I-09](../../../CLAUDE.md#i-09)).
    #[test]
    fn an_out_of_bounds_index_does_not_panic() {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![1, 2]))])
            .expect("batch built for the test");
        let opts = FormatOptions::default();

        assert!(matches!(
            format_cell(&batch, 99, 0, &opts),
            CellValue::Unrenderable { .. }
        ));
        assert!(matches!(
            format_cell(&batch, 0, 99, &opts),
            CellValue::Unrenderable { .. }
        ));
    }

    #[test]
    fn format_cell_goes_through_the_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("at", DataType::Timestamp(TimeUnit::Millisecond, None), true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec![Some("one"), None])),
                Arc::new(TimestampMillisecondArray::from(vec![Some(0), None])),
            ],
        )
        .expect("batch built for the test");
        let opts = FormatOptions::default();

        assert_eq!(format_cell(&batch, 0, 0, &opts).text(), Some("1"));
        assert_eq!(format_cell(&batch, 0, 1, &opts).text(), Some("one"));
        assert!(format_cell(&batch, 1, 1, &opts).is_null());
        assert!(format_cell(&batch, 1, 2, &opts).is_null());
        assert_eq!(
            format_cell(&batch, 0, 2, &opts).text(),
            Some("1970-01-01T00:00:00")
        );
    }

    /// A schema with one timestamp column, with or without a time zone.
    fn timestamp_schema(zones: &[Option<&str>]) -> Schema {
        Schema::new(
            zones
                .iter()
                .enumerate()
                .map(|(index, zone)| {
                    Field::new(
                        format!("t{index}"),
                        DataType::Timestamp(TimeUnit::Microsecond, zone.map(Into::into)),
                        true,
                    )
                })
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn the_display_time_zone_is_derived_from_the_schema() {
        // No timestamp column: nothing to announce.
        assert_eq!(
            timestamp_display(&Schema::new(vec![Field::new("id", DataType::Int32, false)])),
            TimestampDisplay::Absent
        );
        // The common PostgreSQL case: `timestamptz` arrives in UTC.
        assert_eq!(
            timestamp_display(&timestamp_schema(&[Some("UTC"), Some("UTC")])),
            TimestampDisplay::Uniform("UTC".to_owned())
        );
        // Two different zones: naming one would wrongly describe the other.
        assert_eq!(
            timestamp_display(&timestamp_schema(&[Some("UTC"), Some("+02:00")])),
            TimestampDisplay::Mixed
        );
    }

    #[test]
    fn a_timestamp_without_time_zone_does_not_announce_one() {
        // `timestamp without time zone` carries no time zone. It is the flaw
        // this test keeps closed: announcing "UTC" here would invent
        // information the server did not send, and the value itself renders
        // without `Z` or offset — the two must stay consistent.
        assert_eq!(
            timestamp_display(&timestamp_schema(&[None, None])),
            TimestampDisplay::Absent
        );
        // Mixed with a column that does carry one: only that one counts.
        assert_eq!(
            timestamp_display(&timestamp_schema(&[None, Some("UTC")])),
            TimestampDisplay::Uniform("UTC".to_owned())
        );
    }

    #[test]
    fn the_null_replacement_text_is_configurable() {
        let opts = FormatOptions::default().with_null_text("(empty)");
        assert_eq!(CellValue::Null.display_with(&opts), "(empty)");
    }
}
