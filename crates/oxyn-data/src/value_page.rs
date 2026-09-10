//! Bounded text pages of a single Arrow value, without building its entire string.

use crate::{DataError, Result};
use arrow::record_batch::RecordBatch;
use arrow::util::display::{ArrayFormatter, FormatOptions};
use oxyn_core::CancelToken;
use std::fmt::{self, Write};

/// Maximum UTF-8 bytes returned for one inspected-value page.
pub const VALUE_PAGE_BYTES: usize = 16 * 1024;

/// One rendered slice. Debug deliberately excludes the value's text.
#[derive(Clone, PartialEq, Eq)]
pub struct ValuePage {
    /// Raw formatted text; a string containing `NULL` remains a string.
    pub text: String,
    /// Whether the Arrow value itself is absent.
    pub is_null: bool,
    /// UTF-8 byte position of the first rendered character in this page.
    pub offset: usize,
    /// Next byte position, on a character boundary, if text remains.
    pub next_offset: Option<usize>,
    /// Total bytes in the formatted value, not the underlying binary array.
    pub total_bytes: usize,
}

impl fmt::Debug for ValuePage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValuePage")
            .field("bytes", &self.text.len())
            .field("is_null", &self.is_null)
            .field("offset", &self.offset)
            .field("next_offset", &self.next_offset)
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

struct PageWriter<'a> {
    text: String,
    requested: usize,
    total: usize,
    start: Option<usize>,
    end: usize,
    sealed: bool,
    cancel: &'a CancelToken,
}

impl Write for PageWriter<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.cancel.is_cancelled() {
            return Err(fmt::Error);
        }
        let base = self.total;
        self.total = self.total.saturating_add(value.len());
        if self.total <= self.requested || self.text.len() == VALUE_PAGE_BYTES || self.sealed {
            return Ok(());
        }
        let mut start = self.requested.saturating_sub(base).min(value.len());
        while start < value.len() && !value.is_char_boundary(start) {
            start = start.saturating_add(1);
        }
        let mut end = start
            .saturating_add(VALUE_PAGE_BYTES.saturating_sub(self.text.len()))
            .min(value.len());
        while end > start && !value.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        if end < value.len() {
            self.sealed = true;
        }
        if let Some(slice) = value.get(start..end).filter(|slice| !slice.is_empty()) {
            self.start.get_or_insert(base.saturating_add(start));
            self.text.push_str(slice);
            self.end = base.saturating_add(end);
        }
        Ok(())
    }
}

/// Formats only one retained text page. Iterates formatting off the UI thread.
/// Returns None for an invalid row or column; cancellation and Arrow errors propagate.
pub fn inspect_value(
    batch: &RecordBatch,
    row: usize,
    column: usize,
    offset: usize,
    cancel: &CancelToken,
) -> Result<Option<ValuePage>> {
    if cancel.is_cancelled() {
        return Err(DataError::Cancelled);
    }
    let Some(array) = batch
        .columns()
        .get(column)
        .filter(|array| row < array.len())
    else {
        return Ok(None);
    };
    if array.is_null(row) {
        return Ok(Some(ValuePage {
            text: String::new(),
            is_null: true,
            offset: 0,
            next_offset: None,
            total_bytes: 0,
        }));
    }
    let options = FormatOptions::default();
    let formatter = ArrayFormatter::try_new(array.as_ref(), &options)?;
    let mut writer = PageWriter {
        text: String::with_capacity(VALUE_PAGE_BYTES),
        requested: offset,
        total: 0,
        start: None,
        end: offset,
        sealed: false,
        cancel,
    };
    let formatted = formatter.value(row).write(&mut writer);
    if cancel.is_cancelled() {
        return Err(DataError::Cancelled);
    }
    formatted?;
    Ok(Some(ValuePage {
        offset: writer.start.unwrap_or(offset.min(writer.total)),
        next_offset: (writer.end < writer.total).then_some(writer.end),
        text: writer.text,
        is_null: false,
        total_bytes: writer.total,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::StringArray;
    use arrow::datatypes::{DataType, Field, Schema};
    use std::sync::Arc;

    fn batch(values: Vec<Option<&str>>) -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("value", DataType::Utf8, true)])),
            vec![Arc::new(StringArray::from(values))],
        )
        .expect("batch")
    }

    #[test]
    fn long_unicode_value_round_trips_through_bounded_pages() {
        let text = "A🦊é\n日本語".repeat(9000);
        let batch = batch(vec![Some(&text)]);
        let mut offset = 0;
        let mut reconstructed = String::new();
        loop {
            let page = inspect_value(&batch, 0, 0, offset, &CancelToken::new())
                .expect("format")
                .expect("cell");
            assert!(page.text.len() <= VALUE_PAGE_BYTES);
            assert_eq!(page.offset, offset);
            assert_eq!(page.total_bytes, text.len());
            reconstructed.push_str(&page.text);
            let Some(next) = page.next_offset else {
                break;
            };
            assert!(next > offset);
            assert!(text.is_char_boundary(next));
            offset = next;
        }
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn fragmented_formatter_never_skips_a_character_that_does_not_fit() {
        let prefix = "a".repeat(VALUE_PAGE_BYTES - 1);
        let cancel = CancelToken::new();
        let mut writer = PageWriter {
            text: String::new(),
            requested: 0,
            total: 0,
            start: None,
            end: 0,
            sealed: false,
            cancel: &cancel,
        };
        writer.write_str(&prefix).expect("prefix");
        writer.write_str("🦊").expect("unicode boundary");
        writer.write_str("z").expect("tail");
        assert_eq!(writer.text, prefix);
        assert_eq!(writer.end, VALUE_PAGE_BYTES - 1);
        assert_eq!(writer.total, VALUE_PAGE_BYTES + 4);
    }

    #[test]
    fn absent_empty_and_literal_null_remain_distinct_and_debug_hides_text() {
        let batch = batch(vec![
            None,
            Some(""),
            Some("NULL"),
            Some("sensitive cell content"),
        ]);
        let cancel = CancelToken::new();
        let null = inspect_value(&batch, 0, 0, 0, &cancel)
            .expect("format")
            .expect("cell");
        let empty = inspect_value(&batch, 1, 0, 0, &cancel)
            .expect("format")
            .expect("cell");
        let literal = inspect_value(&batch, 2, 0, 0, &cancel)
            .expect("format")
            .expect("cell");
        assert!(null.is_null);
        assert!(!empty.is_null);
        assert_eq!(empty.total_bytes, 0);
        assert!(!literal.is_null);
        assert_eq!(literal.text, "NULL");
        let sensitive = inspect_value(&batch, 3, 0, 0, &cancel)
            .expect("format")
            .expect("cell");
        assert!(!format!("{sensitive:?}").contains("sensitive cell content"));
        assert!(
            inspect_value(&batch, usize::MAX, 0, 0, &cancel)
                .expect("safe bounds")
                .is_none()
        );
        assert!(
            inspect_value(&batch, 0, usize::MAX, 0, &cancel)
                .expect("safe bounds")
                .is_none()
        );
        cancel.cancel();
        assert!(matches!(
            inspect_value(&batch, 0, 0, 0, &cancel),
            Err(DataError::Cancelled)
        ));
    }
}
