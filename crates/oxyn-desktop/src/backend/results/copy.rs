//! Rows of a held result, composed as text for the clipboard.
//!
//! Pure over a buffer: nothing here dispatches a command, contacts a server or
//! runs what it writes. The SQL formats quote identifiers through
//! `oxyn_catalog` and escape values through
//! [`oxyn_catalog::push_string_literal`] ([I-10](../../../../../CLAUDE.md#i-10)).
//! May read a spilled batch from disk: call on the blocking pool only
//! ([I-05](../../../../../CLAUDE.md#i-05)).

use arrow::array::{Array, AsArray, RecordBatch};
use arrow::datatypes::{DataType, Float32Type, Float64Type};
use oxyn_catalog::{QuoteStyle, check_identifier, push_string_literal, quote_identifier};
use oxyn_core::SqlDialect;
use oxyn_data::{CellValue, FormatOptions, ResultBuffer, format_cell};

use crate::ipc::IpcError;
use crate::ipc::results::{CopyRowsFormat, CopySpec};

#[cfg(test)]
mod tests;

/// The most rows one copy holds.
///
/// Written here, in Rust, because the webview is not trusted to bound itself:
/// a script asking for a whole result in one call would hold it twice in
/// memory, as values and as text ([I-06](../../../../../CLAUDE.md#i-06)). The
/// same bound as a page: a selection is made of rows the grid has shown.
pub const MAX_COPY_ROWS: usize = 2_000;

/// The most text one copy composes, in bytes.
///
/// Rows alone do not bound it: 2 000 rows of wide text columns are gigabytes.
/// Past this budget the copy is refused — never cut, since a clipboard that
/// silently holds part of a selection is a loss nobody sees.
pub const MAX_COPY_BYTES: usize = 32 * 1024 * 1024;

/// Where composed SQL goes: the dialect of its literals, and the relation an
/// `INSERT` names, already qualified and quoted.
#[derive(Debug, Clone)]
pub(crate) struct SqlTarget {
    pub dialect: SqlDialect,
    pub relation: Option<String>,
}

/// What a column's Arrow type allows a copy to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Null,
    Boolean,
    Integer,
    Float,
    Decimal,
    Text,
    /// Dates, times and timestamps: written as text, which every dialect casts.
    Temporal,
    /// Anything else — binary, nested, intervals: shown by the grid, never
    /// guessed into a SQL literal.
    Other,
}

impl Kind {
    fn of(data_type: &DataType) -> Self {
        match data_type {
            DataType::Null => Self::Null,
            DataType::Boolean => Self::Boolean,
            DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64 => Self::Integer,
            DataType::Float32 | DataType::Float64 => Self::Float,
            DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => Self::Decimal,
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Self::Text,
            DataType::Date32
            | DataType::Date64
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::Timestamp(_, _) => Self::Temporal,
            _ => Self::Other,
        }
    }
}

struct Column<'a> {
    index: usize,
    name: &'a str,
    data_type: &'a DataType,
    kind: Kind,
}

/// Composes `spec` over the rows `buffer` holds.
///
/// Refuses rather than trims: a count of zero or above [`MAX_COPY_ROWS`], a
/// range that leaves the rows received so far — never read further from the
/// cursor, never run again —, an unknown or repeated column, a SQL format
/// without its target, a column type with no SQL literal, a value a literal
/// cannot carry, a text past [`MAX_COPY_BYTES`]. Every message names the
/// column, never a value.
///
/// Reads spilled batches from disk when they are not cached: blocking pool
/// only.
pub(crate) fn compose(
    buffer: &ResultBuffer,
    spec: &CopySpec,
    sql: Option<&SqlTarget>,
    options: &FormatOptions,
) -> Result<String, IpcError> {
    if spec.count == 0 {
        return Err(IpcError::invalid("Select at least one row to copy"));
    }
    if spec.count > MAX_COPY_ROWS {
        return Err(IpcError::invalid(format!(
            "A copy holds at most {MAX_COPY_ROWS} rows; export the result to keep more"
        )));
    }
    let total = buffer.row_count();
    let end = spec
        .offset
        .checked_add(spec.count)
        .filter(|end| *end <= total)
        .ok_or_else(|| {
            IpcError::invalid(format!(
                "The rows to copy are not all in this result, which holds {total} rows"
            ))
        })?;
    let schema = buffer.schema();
    let columns = columns(schema.fields(), &spec.columns)?;
    let sql = match spec.format {
        CopyRowsFormat::Insert | CopyRowsFormat::InList => Some(sql_target(spec, sql, &columns)?),
        _ => None,
    };

    let mut out = String::new();
    let insert = match (spec.format, sql) {
        (CopyRowsFormat::Insert, Some(target)) => Some(insert_prefix(target, &columns)?),
        _ => None,
    };
    start(&mut out, spec, &columns);

    let mut row = spec.offset;
    while row < end {
        let Some((position, start_in_batch)) = buffer.locate(row) else {
            break;
        };
        let batch = match buffer.cached_batch(position) {
            Some(batch) => batch,
            None => buffer
                .batch(position)
                .map_err(|error| IpcError::invalid(format!("reading the result: {error}")))?
                .ok_or_else(|| IpcError::invalid("A batch of this result is missing"))?,
        };
        let take = batch
            .num_rows()
            .saturating_sub(start_in_batch)
            .min(end.saturating_sub(row));
        if take == 0 {
            break;
        }
        for local in start_in_batch..start_in_batch.saturating_add(take) {
            let written = row.saturating_add(local.saturating_sub(start_in_batch));
            let line = Line {
                batch: &batch,
                local,
                ordinal: written.saturating_sub(spec.offset),
                row: written,
            };
            write_row(
                &mut out,
                spec,
                &columns,
                sql,
                insert.as_deref(),
                &line,
                options,
            )?;
            if out.len() > MAX_COPY_BYTES {
                return Err(IpcError::invalid(format!(
                    "These rows make more than {} MiB of text; export them instead",
                    MAX_COPY_BYTES / (1024 * 1024)
                )));
            }
        }
        row = row.saturating_add(take);
    }
    if row < end {
        // `locate` and the batch disagree with `row_count`: said, not copied
        // short.
        return Err(IpcError::invalid("Part of these rows could not be read"));
    }
    finish(&mut out, spec.format);
    Ok(out)
}

/// The columns asked for, each valid and named once.
fn columns<'a>(
    fields: &'a arrow::datatypes::Fields,
    asked: &[usize],
) -> Result<Vec<Column<'a>>, IpcError> {
    if asked.is_empty() {
        return Err(IpcError::invalid("Select at least one column to copy"));
    }
    let mut seen = vec![false; fields.len()];
    asked
        .iter()
        .map(|&index| {
            let field = fields
                .get(index)
                .ok_or_else(|| IpcError::invalid("A column to copy is not in the result"))?;
            let once = seen
                .get_mut(index)
                .ok_or_else(|| IpcError::invalid("A column to copy is not in the result"))?;
            if std::mem::replace(once, true) {
                return Err(IpcError::invalid("A column to copy is listed twice"));
            }
            Ok(Column {
                index,
                name: field.name(),
                data_type: field.data_type(),
                kind: Kind::of(field.data_type()),
            })
        })
        .collect()
}

/// The target of a SQL format, checked against what the format needs.
fn sql_target<'t>(
    spec: &CopySpec,
    sql: Option<&'t SqlTarget>,
    columns: &[Column<'_>],
) -> Result<&'t SqlTarget, IpcError> {
    let target = sql.ok_or_else(|| {
        IpcError::invalid("Copying as SQL needs the connection, whose dialect writes the values")
    })?;
    if spec.format == CopyRowsFormat::InList && columns.len() != 1 {
        return Err(IpcError::invalid(
            "An IN list is made of one column; select a single column",
        ));
    }
    if spec.format == CopyRowsFormat::Insert && target.relation.is_none() {
        return Err(IpcError::invalid(
            "An INSERT needs the table it writes into; this result has none",
        ));
    }
    // Decided per column, before any row: a column the copy cannot write is
    // refused whole, not after the rows where it happened to be null.
    if let Some(column) = columns.iter().find(|column| column.kind == Kind::Other) {
        return Err(IpcError::invalid(format!(
            "Column {:?} is of type {}, which has no SQL literal; copy it as text instead",
            column.name, column.data_type
        )));
    }
    Ok(target)
}

/// `INSERT INTO <relation> (<columns>) VALUES (`, quoted once for every row.
fn insert_prefix(target: &SqlTarget, columns: &[Column<'_>]) -> Result<String, IpcError> {
    let relation = target
        .relation
        .as_deref()
        .ok_or_else(|| IpcError::invalid("An INSERT needs the table it writes into"))?;
    let style = QuoteStyle::for_dialect(target.dialect);
    let names: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(position, column)| {
            // A result's column name was never validated as a catalog segment
            // was: refused here, before it is quoted into the clipboard.
            check_identifier(column.name).map_err(|error| {
                IpcError::invalid(format!(
                    "The name of copied column {} cannot be written into SQL: {error}",
                    position.saturating_add(1)
                ))
            })?;
            Ok(quote_identifier(column.name, style))
        })
        .collect::<Result<_, IpcError>>()?;
    Ok(format!(
        "INSERT INTO {relation} ({}) VALUES (",
        names.join(", ")
    ))
}

/// One row being written: where it is in its batch, and in the copy.
struct Line<'b> {
    batch: &'b RecordBatch,
    local: usize,
    /// Zero-based among the rows copied: decides the separator.
    ordinal: usize,
    /// Row number in the result, for messages.
    row: usize,
}

fn start(out: &mut String, spec: &CopySpec, columns: &[Column<'_>]) {
    match spec.format {
        CopyRowsFormat::Tsv | CopyRowsFormat::Csv if spec.header => {
            let delimiter = delimiter(spec.format);
            for (position, column) in columns.iter().enumerate() {
                if position > 0 {
                    out.push(delimiter);
                }
                push_delimited(out, column.name, delimiter);
            }
        }
        CopyRowsFormat::Json => out.push('['),
        CopyRowsFormat::Markdown => {
            out.push('|');
            for column in columns {
                out.push(' ');
                push_markdown(out, column.name);
                out.push_str(" |");
            }
            out.push_str("\n|");
            for _ in columns {
                out.push_str(" --- |");
            }
        }
        CopyRowsFormat::InList => out.push('('),
        _ => {}
    }
}

fn finish(out: &mut String, format: CopyRowsFormat) {
    match format {
        CopyRowsFormat::Json => out.push_str("\n]"),
        CopyRowsFormat::InList => out.push(')'),
        _ => {}
    }
}

fn write_row(
    out: &mut String,
    spec: &CopySpec,
    columns: &[Column<'_>],
    sql: Option<&SqlTarget>,
    insert: Option<&str>,
    line: &Line<'_>,
    options: &FormatOptions,
) -> Result<(), IpcError> {
    let first = line.ordinal == 0;
    match (spec.format, sql) {
        (CopyRowsFormat::Tsv | CopyRowsFormat::Csv, _) => {
            if !first || spec.header {
                out.push('\n');
            }
            let delimiter = delimiter(spec.format);
            for (position, column) in columns.iter().enumerate() {
                if position > 0 {
                    out.push(delimiter);
                }
                // NULL is an empty field: the convention of both formats, and
                // of the export.
                if let Some(text) = shown(line, column, options)? {
                    push_delimited(out, &text, delimiter);
                }
            }
        }
        (CopyRowsFormat::Json, _) => {
            out.push_str(if first { "\n  {" } else { ",\n  {" });
            for (position, column) in columns.iter().enumerate() {
                if position > 0 {
                    out.push_str(", ");
                }
                push_json_string(out, column.name);
                out.push_str(": ");
                match shown(line, column, options)? {
                    None => out.push_str("null"),
                    Some(text) if json_native(line, column, &text) => out.push_str(&text),
                    Some(text) => push_json_string(out, &text),
                }
            }
            out.push('}');
        }
        (CopyRowsFormat::Markdown, _) => {
            out.push_str("\n|");
            for column in columns {
                out.push(' ');
                match shown(line, column, options)? {
                    Some(text) => push_markdown(out, &text),
                    // The grid's own word for an absent value: a table is
                    // read, and an empty cell reads as an empty text.
                    None => push_markdown(out, &options.null_text),
                }
                out.push_str(" |");
            }
        }
        (CopyRowsFormat::Insert, Some(target)) => {
            if !first {
                out.push('\n');
            }
            out.push_str(insert.unwrap_or_default());
            for (position, column) in columns.iter().enumerate() {
                if position > 0 {
                    out.push_str(", ");
                }
                push_sql_value(out, line, column, target.dialect, options)?;
            }
            out.push_str(");");
        }
        (CopyRowsFormat::InList, Some(target)) => {
            if !first {
                out.push_str(", ");
            }
            for column in columns {
                push_sql_value(out, line, column, target.dialect, options)?;
            }
        }
        (CopyRowsFormat::Insert | CopyRowsFormat::InList, None) => {
            return Err(IpcError::invalid(
                "Copying as SQL needs the connection, whose dialect writes the values",
            ));
        }
    }
    Ok(())
}

/// The value as the grid shows it, or `None` for an absent one.
fn shown<'b>(
    line: &Line<'b>,
    column: &Column<'_>,
    options: &FormatOptions,
) -> Result<Option<std::borrow::Cow<'b, str>>, IpcError> {
    match format_cell(line.batch, line.local, column.index, options) {
        CellValue::Null => Ok(None),
        CellValue::Text(text) => Ok(Some(text)),
        // The options never cut; a cut value copied would be a lie.
        CellValue::Truncated { .. } => Err(IpcError::invalid(format!(
            "Column {:?}, row {}: the value was cut and cannot be copied whole",
            column.name, line.row
        ))),
        CellValue::Unrenderable { reason } => Err(IpcError::invalid(format!(
            "Column {:?}, row {}: {reason}",
            column.name, line.row
        ))),
        // `CellValue` is `#[non_exhaustive]`: a rendering nobody knows here
        // is refused rather than copied as something it is not.
        _ => Err(IpcError::invalid(format!(
            "Column {:?}, row {}: this value cannot be copied",
            column.name, line.row
        ))),
    }
}

/// Whether a JSON value can be written as the grid's text, unquoted: a
/// boolean, an integer, a finite float. Anything else is a JSON string.
fn json_native(line: &Line<'_>, column: &Column<'_>, text: &str) -> bool {
    match column.kind {
        Kind::Boolean | Kind::Integer => true,
        Kind::Float => finite(line, column) == Some(true) && is_number(text),
        _ => false,
    }
}

/// Whether the float at this position is finite; `None` if it is not a float.
fn finite(line: &Line<'_>, column: &Column<'_>) -> Option<bool> {
    let array = line.batch.columns().get(column.index)?;
    if line.local >= array.len() {
        return None;
    }
    match array.data_type() {
        DataType::Float32 => Some(
            array
                .as_primitive::<Float32Type>()
                .value(line.local)
                .is_finite(),
        ),
        DataType::Float64 => Some(
            array
                .as_primitive::<Float64Type>()
                .value(line.local)
                .is_finite(),
        ),
        _ => None,
    }
}

/// Whether `text` is exactly `-?\d+(\.\d+)?([eE][+-]?\d+)?`.
///
/// Checked on the text the formatter wrote rather than trusted: a number
/// written into SQL unquoted is the one value that is not escaped, and a
/// formatter change — a digit group separator, a locale — must fail here, not
/// reach the statement. A character class would not do: `--` opens a comment,
/// and `1-2` is an expression.
fn is_number(text: &str) -> bool {
    /// Consumes a run of ASCII digits; `false` if there is none.
    fn digits(bytes: &[u8], at: &mut usize) -> bool {
        let start = *at;
        while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
            *at = at.saturating_add(1);
        }
        *at > start
    }
    let bytes = text.as_bytes();
    let mut at = 0usize;
    if bytes.first() == Some(&b'-') {
        at = 1;
    }
    if !digits(bytes, &mut at) {
        return false;
    }
    if bytes.get(at) == Some(&b'.') {
        at = at.saturating_add(1);
        if !digits(bytes, &mut at) {
            return false;
        }
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at = at.saturating_add(1);
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at = at.saturating_add(1);
        }
        if !digits(bytes, &mut at) {
            return false;
        }
    }
    at == bytes.len()
}

/// Appends one value as a SQL literal of `dialect`.
fn push_sql_value(
    out: &mut String,
    line: &Line<'_>,
    column: &Column<'_>,
    dialect: SqlDialect,
    options: &FormatOptions,
) -> Result<(), IpcError> {
    let refused =
        |why: &str| IpcError::invalid(format!("Column {:?}, row {}: {why}", column.name, line.row));
    let Some(text) = shown(line, column, options)? else {
        out.push_str("NULL");
        return Ok(());
    };
    match column.kind {
        Kind::Null => out.push_str("NULL"),
        Kind::Boolean => {
            let value = line
                .batch
                .columns()
                .get(column.index)
                .filter(|array| line.local < array.len())
                .and_then(|array| {
                    array
                        .as_boolean_opt()
                        .map(|values| values.value(line.local))
                })
                .ok_or_else(|| refused("the value is not a boolean"))?;
            // T-SQL has no boolean literal; a `bit` column takes 1 and 0.
            out.push_str(match (dialect, value) {
                (SqlDialect::SqlServer, true) => "1",
                (SqlDialect::SqlServer, false) => "0",
                (_, true) => "TRUE",
                (_, false) => "FALSE",
            });
        }
        Kind::Integer | Kind::Decimal if is_number(&text) => out.push_str(&text),
        Kind::Float if finite(line, column) == Some(true) && is_number(&text) => {
            out.push_str(&text);
        }
        Kind::Float => {
            return Err(refused("NaN and infinity have no portable SQL literal"));
        }
        Kind::Integer | Kind::Decimal => return Err(refused("the number is not plain digits")),
        Kind::Text | Kind::Temporal => {
            push_string_literal(out, &text, dialect).map_err(|error| refused(&error.to_string()))?
        }
        Kind::Other => return Err(refused("this type has no SQL literal")),
    }
    Ok(())
}

fn delimiter(format: CopyRowsFormat) -> char {
    if format == CopyRowsFormat::Csv {
        ','
    } else {
        '\t'
    }
}

/// A field, quoted the RFC 4180 way when it holds the delimiter, a quote or a
/// line break.
///
/// TSV too: a tab or a line break in a value would otherwise shift every
/// following cell, and spreadsheets read the quoted form when pasting.
fn push_delimited(out: &mut String, text: &str, delimiter: char) {
    if !text.contains([delimiter, '"', '\n', '\r']) {
        out.push_str(text);
        return;
    }
    out.push('"');
    for c in text.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
}

fn push_json_string(out: &mut String, text: &str) {
    use std::fmt::Write as _;
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                // Writing into a `String` cannot fail.
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A table cell: `|` escaped, line breaks as `<br>` — a raw line break would
/// end the row, and the table with it.
fn push_markdown(out: &mut String, text: &str) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '|' => out.push_str("\\|"),
            '\r' => {
                chars.next_if_eq(&'\n');
                out.push_str("<br>");
            }
            '\n' => out.push_str("<br>"),
            c => out.push(c),
        }
    }
}
