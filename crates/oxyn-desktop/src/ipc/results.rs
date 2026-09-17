//! What crosses the IPC boundary for a result the backend holds: a window of
//! rows, where a search matched, one value page, the export formats.
//!
//! None of these carries more than a bounded slice of the result
//! ([I-06](../../../../CLAUDE.md#i-06)): a window is at most
//! `MAX_PAGE_ROWS` rows (`backend/results.rs`), a value page 16 KiB,
//! a list of matches one window.

use std::fmt;

use oxyn_core::ExportFormat;
use oxyn_data::FindOutcome;
use oxyn_data::value_page::ValuePage;
use serde::Serialize;

use crate::ipc::ResultPage;

/// A window of rows, or the reason there is none.
///
/// « Expired » is data, not a message to parse: a result evicted by retention
/// is never recreated by running its query again (ADR-0017), and the front
/// draws that state instead of an error.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ResultWindow {
    Page(ResultPage),
    Expired,
}

/// Where a search found the needle, around the row the user stands on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindAnswer {
    /// Rows that matched, counted once each.
    pub total: usize,
    /// The row to reveal: the next (or previous) match, wrapping around.
    pub row: Option<usize>,
    /// One-based rank of `row` among the matches.
    pub ordinal: Option<usize>,
    /// Batches not searched because they spilled to disk. Said on screen:
    /// « no match » would otherwise read as « not in this result ».
    pub skipped_batches: usize,
    /// The search stopped counting at its limit; `total` is a floor.
    pub capped: bool,
}

impl FindAnswer {
    /// The answer for a move from `from` in one direction.
    ///
    /// Forward finds the first match at or after `from`; backward the last one
    /// strictly before it. Both wrap, as every editor's search does.
    #[must_use]
    pub fn of(outcome: &FindOutcome, from: usize, forward: bool) -> Self {
        let row = if forward {
            outcome.next_from(from)
        } else {
            outcome.previous_from(from)
        };
        let ordinal = row.and_then(|row| {
            outcome
                .rows
                .binary_search(&row)
                .ok()
                .map(|index| index.saturating_add(1))
        });
        Self {
            total: outcome.rows.len(),
            row,
            ordinal,
            skipped_batches: outcome.skipped_batches,
            capped: outcome.capped,
        }
    }
}

/// The matching rows inside `[offset, offset + limit)`, for the grid to mark.
#[must_use]
pub fn matches_in_window(outcome: &FindOutcome, offset: usize, limit: usize) -> Vec<usize> {
    let end = offset.saturating_add(limit);
    // `rows` is sorted: two binary searches bound the window without walking
    // fifty thousand matches for a page of two hundred rows.
    let start = outcome.rows.partition_point(|row| *row < offset);
    let stop = outcome.rows.partition_point(|row| *row < end);
    outcome
        .rows
        .get(start..stop)
        .map(<[usize]>::to_vec)
        .unwrap_or_default()
}

/// One page of a single value, as the value inspector shows it.
///
/// **No `Debug` derive**: the text is a cell of the user's data. The manual
/// implementation renders its length only, as `ValuePage` does.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValuePageView {
    pub column: String,
    pub data_type: String,
    /// The Arrow value is absent. Distinct from an empty text and from the
    /// text `NULL`.
    pub is_null: bool,
    pub text: String,
    /// Byte position of this page in the formatted value.
    pub offset: usize,
    /// Where the next page starts, when text remains.
    pub next_offset: Option<usize>,
    /// Bytes of formatted text, not the stored size of the value.
    pub total_bytes: usize,
}

impl fmt::Debug for ValuePageView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValuePageView")
            .field("column", &self.column)
            .field("bytes", &self.text.len())
            .field("is_null", &self.is_null)
            .field("offset", &self.offset)
            .field("next_offset", &self.next_offset)
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

impl ValuePageView {
    #[must_use]
    pub fn of(page: ValuePage, column: String, data_type: String) -> Self {
        Self {
            column,
            data_type,
            is_null: page.is_null,
            text: page.text,
            offset: page.offset,
            next_offset: page.next_offset,
            total_bytes: page.total_bytes,
        }
    }
}

/// Every export format the domain names, and whether it can be written today.
///
/// A format the product cannot write yet is shown **unavailable**, not absent
/// ([UX-SPEC](../../../../docs/UX-SPEC.md#ce-qui-est-exporté-est-ce-qui-est-affiché)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFormatChoice {
    /// The value `export_result` accepts.
    pub format: String,
    pub label: String,
    pub extension: String,
    pub supported: bool,
}

/// The formats, in the order the GPUI menu lists them.
pub const EXPORT_FORMATS: [(ExportFormat, &str, &str); 8] = [
    (ExportFormat::Csv, "csv", "CSV"),
    (ExportFormat::Tsv, "tsv", "TSV"),
    (ExportFormat::Json, "json", "JSON"),
    (ExportFormat::JsonLines, "jsonl", "JSON lines"),
    (ExportFormat::Parquet, "parquet", "Parquet"),
    (ExportFormat::ArrowIpc, "arrow", "Arrow IPC"),
    (ExportFormat::Sql, "sql", "SQL inserts"),
    (ExportFormat::Markdown, "markdown", "Markdown"),
];

/// The formats, each with whether `oxyn-data` writes it.
#[must_use]
pub fn export_formats() -> Vec<ExportFormatChoice> {
    EXPORT_FORMATS
        .iter()
        .map(|(format, name, label)| ExportFormatChoice {
            format: (*name).to_owned(),
            label: (*label).to_owned(),
            extension: format.extension().to_owned(),
            supported: oxyn_data::is_supported(*format),
        })
        .collect()
}

/// Longest file name offered in the save dialog, in characters.
const MAX_SUGGESTED_NAME: usize = 120;

/// The file name the save dialog proposes, from what the webview suggested.
///
/// Only a **name**: separators, control characters and leading dots are
/// dropped, so that a suggestion built from a hostile object name cannot point
/// the dialog elsewhere. The user still chooses the folder and the final name
/// in the native dialog; the webview never sends a path (SECURITY, « Surface
/// d'entrée »).
#[must_use]
pub fn suggested_file_name(suggestion: &str, extension: &str) -> String {
    let cleaned: String = suggestion
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .take(MAX_SUGGESTED_NAME)
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    let stem = if cleaned.is_empty() {
        "result"
    } else {
        cleaned
    };
    let suffix = format!(".{extension}");
    if stem.to_lowercase().ends_with(&suffix) {
        stem.to_owned()
    } else {
        format!("{stem}{suffix}")
    }
}

/// The domain format for a name the front sent, if it names one.
#[must_use]
pub fn export_format(name: &str) -> Option<ExportFormat> {
    EXPORT_FORMATS
        .iter()
        .find(|(_, known, _)| *known == name)
        .map(|(format, _, _)| *format)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(rows: Vec<usize>) -> FindOutcome {
        let mut outcome = FindOutcome::default();
        outcome.rows = rows;
        outcome
    }

    #[test]
    fn a_search_moves_forward_and_back_and_wraps() {
        let found = outcome(vec![3, 10, 42]);
        let next = FindAnswer::of(&found, 11, true);
        assert_eq!((next.row, next.ordinal), (Some(42), Some(3)));
        let wrapped = FindAnswer::of(&found, 43, true);
        assert_eq!((wrapped.row, wrapped.ordinal), (Some(3), Some(1)));
        let back = FindAnswer::of(&found, 10, false);
        assert_eq!((back.row, back.ordinal), (Some(3), Some(1)));
        let none = FindAnswer::of(&FindOutcome::default(), 0, true);
        assert_eq!((none.total, none.row), (0, None));
    }

    #[test]
    fn a_window_lists_only_its_own_matches() {
        let found = outcome(vec![1, 199, 200, 399, 400, 50_000]);
        assert_eq!(matches_in_window(&found, 200, 200), vec![200, 399]);
        assert_eq!(
            matches_in_window(&found, usize::MAX, 200),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn unwritable_formats_are_listed_unavailable_not_hidden() {
        let formats = export_formats();
        assert_eq!(formats.len(), EXPORT_FORMATS.len());
        let parquet = formats
            .iter()
            .find(|choice| choice.format == "parquet")
            .expect("listed");
        assert_eq!(
            parquet.supported,
            oxyn_data::is_supported(ExportFormat::Parquet)
        );
        assert!(
            formats
                .iter()
                .all(|choice| export_format(&choice.format).is_some())
        );
        assert!(export_format("xlsx").is_none());
    }

    #[test]
    fn a_suggested_name_is_a_name_never_a_path() {
        assert_eq!(suggested_file_name("invoices", "csv"), "invoices.csv");
        assert_eq!(suggested_file_name("report.CSV", "csv"), "report.CSV");
        assert_eq!(
            suggested_file_name("../../etc/passwd", "csv"),
            "_.._etc_passwd.csv"
        );
        assert_eq!(
            suggested_file_name(r#"users"; DROP TABLE audit; --"#, "json"),
            "users_; DROP TABLE audit; --.json"
        );
        assert_eq!(suggested_file_name("  ...  ", "tsv"), "result.tsv");
        assert_eq!(suggested_file_name("a\u{0}b", "csv"), "a_b.csv");
        assert!(
            suggested_file_name(&"x".repeat(10_000), "csv")
                .chars()
                .count()
                <= 124
        );
    }

    #[test]
    fn a_value_page_debug_never_shows_the_value() {
        let view = ValuePageView {
            column: "notes".into(),
            data_type: "Utf8".into(),
            is_null: false,
            text: "confidential remark".into(),
            offset: 0,
            next_offset: None,
            total_bytes: 19,
        };
        assert!(!format!("{view:?}").contains("confidential"));
    }
}
