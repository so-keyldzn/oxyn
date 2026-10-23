//! The body of a `RowBinaryWithNamesAndTypes` response, as it arrives.
//!
//! # An exception after the first byte
//!
//! Once the server has started sending rows, the status is already 200. An
//! exception raised later is written **into the body**, after the last rows:
//! `\r\n__exception__\r\n<tag>\r\n<message>\n<length> <tag>\r\n__exception__\r\n`,
//! where `<tag>` is the random text the response announced in
//! `X-ClickHouse-Exception-Tag` (observed on ClickHouse 26.3, 2026-10-22,
//! RESEARCH-NOTES). `RowBinary` carries no framing of its own, so without this
//! marker the message would be decoded as rows.
//!
//! [`Body`] therefore looks for the marker in everything received, and never
//! hands the decoder the last bytes of the buffer before more arrive — the
//! start of a marker cut by a chunk boundary would otherwise be read as a
//! value. With a random tag of 16 letters, a marker that is in fact data is
//! not a case to design for.
//!
//! A server that sends no tag gets no detection: its exception is decoded as
//! garbage and ends, at worst, as a decoding error or a truncated row — never
//! as a clean end.

use crate::error::{MAX_MESSAGE_BYTES, ServerError};
use crate::measure::{DecodeError, MAX_ROW_BYTES, value_error, varint};
use crate::schema::Column;
use crate::types;

/// What opens the exception block, before the tag.
const OPENING: &[u8] = b"\r\n__exception__\r\n";

/// The most columns a result header may announce: ClickHouse's own limits are
/// far below, and a hostile count must not size an allocation.
const MAX_COLUMNS: u64 = 100_000;

/// The response body, split between rows and a possible exception.
#[derive(Debug, Default)]
pub struct Body {
    pending: Vec<u8>,
    start: usize,
    marker: Option<Vec<u8>>,
    /// Where the marker search resumes: everything before it was searched.
    scanned: usize,
    /// Where the rows end, once the marker was found.
    rows_end: Option<usize>,
    ended: bool,
}

impl Body {
    /// A body whose exception, if any, carries `tag`.
    #[must_use]
    pub fn new(tag: Option<&str>) -> Self {
        let marker = tag.filter(|tag| !tag.is_empty()).map(|tag| {
            let mut marker = OPENING.to_vec();
            marker.extend_from_slice(tag.as_bytes());
            marker.extend_from_slice(b"\r\n");
            marker
        });
        Self {
            marker,
            ..Self::default()
        }
    }

    /// Appends a chunk of the body.
    pub fn push(&mut self, chunk: &[u8]) {
        if self.rows_end.is_some() {
            // Past the marker: only the message, bounded.
            let room = (MAX_MESSAGE_BYTES * 2).saturating_sub(self.pending.len());
            self.pending
                .extend_from_slice(chunk.get(..room.min(chunk.len())).unwrap_or(&[]));
            return;
        }
        self.pending.extend_from_slice(chunk);
        if let Some(marker) = &self.marker {
            let from = self.scanned.saturating_sub(marker.len());
            if let Some(found) = find(self.pending.get(from..).unwrap_or(&[]), marker) {
                self.rows_end = Some(from + found);
            }
            self.scanned = self.pending.len();
        }
    }

    /// The body ended.
    pub fn end(&mut self) {
        self.ended = true;
    }

    /// Whether the body ended with nothing left to decode.
    #[must_use]
    pub fn is_drained(&self) -> bool {
        self.ended && self.rows_end.is_none() && self.start >= self.pending.len()
    }

    /// Bytes ready for the decoder.
    ///
    /// Before the end, the last marker's length is held back: they may be
    /// the beginning of an exception.
    #[must_use]
    pub fn ready(&self) -> &[u8] {
        let end = match (self.rows_end, &self.marker) {
            (Some(end), _) => end,
            _ if self.ended => self.pending.len(),
            (None, Some(marker)) => self.pending.len().saturating_sub(marker.len()),
            (None, None) => self.pending.len(),
        };
        self.pending.get(self.start..end).unwrap_or(&[])
    }

    /// Marks `n` bytes of [`Self::ready`] as decoded.
    pub fn consume(&mut self, n: usize) {
        self.start = self.start.saturating_add(n).min(self.pending.len());
        // Compacted once the decoded prefix is large, so that a long stream
        // does not keep everything it ever received.
        if self.start >= (1 << 20) && self.start * 2 >= self.pending.len() {
            self.pending.drain(..self.start);
            self.scanned = self.scanned.saturating_sub(self.start);
            self.rows_end = self.rows_end.map(|end| end.saturating_sub(self.start));
            self.start = 0;
        }
    }

    /// Bytes received and not decoded, the marker's and the message's
    /// included.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.pending.len().saturating_sub(self.start)
    }

    /// The exception the body carries, once it ended, and only if every row
    /// before it was decoded.
    #[must_use]
    pub fn exception(&self) -> Option<ServerError> {
        let rows_end = self.rows_end?;
        let marker = self.marker.as_ref()?;
        let text = self.pending.get(rows_end + marker.len()..)?;
        // `<message>\n<length> <tag>\r\n__exception__\r\n`: the message is
        // what precedes the last line.
        let text = text.strip_suffix(b"\r\n__exception__\r\n").unwrap_or(text);
        let message = match text.iter().rposition(|byte| *byte == b'\n') {
            Some(newline) => text.get(..newline).unwrap_or(text),
            None => text,
        };
        Some(ServerError::from_text(None, message, None))
    }

    /// Whether an exception block was found.
    ///
    /// After one, the server leaves the chunked body unterminated on purpose
    /// (its `WriteBufferFromHTTPServerResponse`, 26.8): the transport error
    /// that follows is the end of the body, not a lost connection.
    #[must_use]
    pub const fn has_exception(&self) -> bool {
        self.rows_end.is_some()
    }

    /// Whether the rows have all been seen: the body ended, or an exception
    /// closed them.
    #[must_use]
    pub const fn rows_closed(&self) -> bool {
        self.ended || self.rows_end.is_some()
    }
}

/// The first position of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    let first = *needle.first()?;
    let mut from = 0;
    while let Some(offset) = haystack.get(from..)?.iter().position(|byte| *byte == first) {
        let at = from + offset;
        if haystack.get(at..at + needle.len())? == needle {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// The names and types the result opens with, and where they end; `None`
/// while the buffer holds only part of them.
///
/// # Errors
/// [`DecodeError`] for a header no server sends: a count beyond
/// [`MAX_COLUMNS`], a name or a type that is not UTF-8 or not a type.
pub fn header(buf: &[u8]) -> Result<Option<(Vec<Column>, usize)>, DecodeError> {
    let limit = buf.len().min(MAX_ROW_BYTES);
    let Some((count, mut at)) = varint(buf, 0, limit, "header")? else {
        return incomplete(buf);
    };
    if count > MAX_COLUMNS {
        return Err(value_error("header", "more columns than a result can have"));
    }
    let mut texts = Vec::new();
    for _ in 0..count.saturating_mul(2) {
        let Some((length, start)) = varint(buf, at, limit, "header")? else {
            return incomplete(buf);
        };
        let end = usize::try_from(length)
            .ok()
            .and_then(|length| start.checked_add(length))
            .ok_or(DecodeError::RowTooLarge)?;
        let Some(bytes) = buf.get(start..end).filter(|_| end <= limit) else {
            return incomplete(buf);
        };
        let text = std::str::from_utf8(bytes)
            .map_err(|_| value_error("header", "a column name or type that is not UTF-8"))?;
        texts.push(text.to_owned());
        at = end;
    }
    let half = texts.len() / 2;
    let types = texts.split_off(half);
    let mut columns = Vec::with_capacity(half);
    for (name, raw_type) in texts.into_iter().zip(types) {
        let ty = types::parse(&raw_type)
            .map_err(|_| value_error("header", "a column type the driver cannot read"))?;
        columns.push(Column { name, raw_type, ty });
    }
    Ok(Some((columns, at)))
}

fn incomplete(buf: &[u8]) -> Result<Option<(Vec<Column>, usize)>, DecodeError> {
    if buf.len() >= MAX_ROW_BYTES {
        Err(DecodeError::RowTooLarge)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChType;

    const TAG: &str = "udzlehrafnlmimdg";

    fn exception_block(message: &str) -> Vec<u8> {
        let mut block = b"\r\n__exception__\r\n".to_vec();
        block.extend_from_slice(TAG.as_bytes());
        block.extend_from_slice(b"\r\n");
        block.extend_from_slice(message.as_bytes());
        block.extend_from_slice(
            format!("\n{} {TAG}\r\n__exception__\r\n", message.len()).as_bytes(),
        );
        block
    }

    #[test]
    fn an_exception_after_rows_is_split_from_them_whatever_the_chunks() {
        let rows = [7_u8; 40];
        let mut stream = rows.to_vec();
        stream.extend(exception_block("Code: 395. DB::Exception: boom"));
        for cut in 1..stream.len() {
            let mut body = Body::new(Some(TAG));
            let (first, second) = stream.split_at(cut);
            body.push(first);
            // Before the end, nothing that could start the marker is ready.
            assert!(body.ready().len() <= rows.len(), "cut {cut}");
            body.push(second);
            body.end();
            assert_eq!(body.ready(), rows, "cut {cut}");
            let error = body.exception().expect("the exception is found");
            assert_eq!(error.code, Some(395), "cut {cut}");
            assert_eq!(error.message, "Code: 395. DB::Exception: boom");
        }
    }

    #[test]
    fn a_body_without_exception_is_all_rows_at_its_end() {
        let mut body = Body::new(Some(TAG));
        body.push(&[1, 2, 3]);
        assert!(body.ready().is_empty(), "held back until more comes");
        body.end();
        assert_eq!(body.ready(), [1, 2, 3]);
        assert!(body.exception().is_none());
        body.consume(3);
        assert!(body.is_drained());
    }

    #[test]
    fn without_a_tag_nothing_is_held_back() {
        let mut body = Body::new(None);
        body.push(b"\r\n__exception__\r\nabc");
        assert_eq!(body.ready().len(), 20);
        assert!(body.exception().is_none());
    }

    #[test]
    fn the_header_announces_names_and_types() {
        let buf = [
            &[2_u8][..],
            &[1, b'a'],
            &[3, b'b', b';', b'"'],
            &[5],
            b"UInt8",
            &[16],
            b"Nullable(String)",
            &[9, 9, 9],
        ]
        .concat();
        let (columns, end) = header(&buf).expect("valid").expect("whole");
        assert_eq!(end, buf.len() - 3);
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].name, "a");
        assert_eq!(columns[1].name, "b;\"", "a hostile name is data");
        assert_eq!(columns[1].ty, ChType::Nullable(Box::new(ChType::String)));
        for cut in 0..end {
            assert_eq!(header(buf.get(..cut).expect("in range")), Ok(None), "{cut}");
        }
    }

    #[test]
    fn a_hostile_header_is_refused() {
        assert!(
            header(&[0xff, 0xff, 0xff, 0x7f]).is_err(),
            "too many columns"
        );
        assert!(
            header(&[1, 1, 0xff, 1, b'x']).is_err(),
            "a name that is not UTF-8"
        );
        assert!(
            header(&[1, 1, b'a', 3, b'(', b'(', b'(']).is_err(),
            "not a type"
        );
    }
}
