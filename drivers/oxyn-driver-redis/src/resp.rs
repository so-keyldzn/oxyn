//! The wire: RESP2 and RESP3 as the server writes them, read without trusting
//! a single length.
//!
//! # Why the driver reads the protocol itself
//!
//! A Redis client library parses a whole reply into a value tree before
//! handing it over ([ADR-0061](../../../docs/adr/0061-redis-driver-speaks-resp3-itself.md)).
//! `HGETALL` on a hash of ten million fields would sit whole in memory before
//! the first row reaches the grid ([I-06](../../../CLAUDE.md#i-06)). This
//! reader hands out one *head* at a time — a type byte and its length — so the
//! cursor can turn an aggregate into batches element by element, and keeps a
//! bounded prefix of anything larger than it is allowed to hold.
//!
//! # What is never trusted
//!
//! Every length comes from the server ([I-09](../../../CLAUDE.md#i-09)): none
//! sizes an allocation beyond `PREALLOCATED` (1,024) elements, a line longer than
//! [`MAX_LINE`] is refused rather than buffered, a nesting deeper than
//! [`MAX_DEPTH`] is skipped rather than recursed into, and a reply cut by the
//! end of the stream is [`WireError::Truncated`], never a short value.

use std::io;

use futures::future::BoxFuture;
use tokio::io::{AsyncBufRead, AsyncBufReadExt as _};

/// The longest line accepted: a simple string, a simple error, a length.
///
/// Redis writes short lines there — the long texts travel as bulk strings —
/// and a server that sends a megabyte without a line feed is broken or
/// hostile: buffering it until the end would let it choose our memory.
pub const MAX_LINE: usize = 1 << 20;

/// The deepest nesting kept as a tree. Deeper aggregates are read and
/// discarded, and the value says so ([`Value::Elided`]).
pub const MAX_DEPTH: usize = 32;

/// The most elements reserved ahead for an aggregate, whatever its announced
/// length: `*9223372036854775807` must cost nothing until elements arrive.
const PREALLOCATED: usize = 1024;

/// What one element of a kept tree costs against a budget, besides its bytes:
/// ten million empty strings are not free.
const ELEMENT_COST: usize = std::mem::size_of::<Value>();

/// Why the reader stopped.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WireError {
    /// The stream ended inside a reply: the server or the network closed the
    /// connection after part of it.
    #[error("the connection closed in the middle of a reply")]
    Truncated,
    /// The transport failed.
    #[error("the connection failed while reading a reply: {0}")]
    Io(#[source] io::Error),
    /// The bytes are not RESP. The connection cannot be resynchronized.
    #[error("the server sent something that is not RESP: {0}")]
    Malformed(&'static str),
}

impl From<io::Error> for WireError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Io(error)
        }
    }
}

/// The kind of an aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aggregate {
    /// `*`: an ordered list.
    Array,
    /// `~`: an unordered collection of distinct elements.
    Set,
    /// `%`: key and value pairs. Its length counts pairs.
    Map,
    /// `>`: out-of-band data the server sends on its own.
    Push,
}

impl Aggregate {
    /// The number of RESP elements an aggregate of `len` entries holds.
    ///
    /// # Errors
    /// [`WireError::Malformed`] when a map's element count overflows.
    pub fn elements(self, len: u64) -> Result<u64, WireError> {
        match self {
            Self::Map => len
                .checked_mul(2)
                .ok_or(WireError::Malformed("a map length overflows")),
            Self::Array | Self::Set | Self::Push => Ok(len),
        }
    }

    /// The name shown in a result's `type` column.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Array => "array",
            Self::Set => "set",
            Self::Map => "map",
            Self::Push => "push",
        }
    }
}

/// The first line of a reply element: its type, and what the line carries.
#[derive(Debug, Clone, PartialEq)]
pub enum Head {
    /// `+`: a short text without line breaks.
    Simple(Vec<u8>),
    /// `-`: an error, whole on its line.
    Error(Vec<u8>),
    /// `!`: an error whose text follows, of this many bytes.
    BulkError(u64),
    /// `:`: a signed 64-bit integer.
    Integer(i64),
    /// `,`: a double, `inf`, `-inf` and `nan` included.
    Double(f64),
    /// `#`: a boolean.
    Boolean(bool),
    /// `(`: an integer beyond 64 bits, as its decimal digits.
    BigNumber(String),
    /// `_`, `$-1` or `*-1`: no value.
    Null,
    /// `$`: a binary-safe string of this many bytes.
    Bulk(u64),
    /// `=`: a string of this many bytes, three of them its format and one a
    /// colon.
    Verbatim(u64),
    /// An aggregate of this many entries.
    Aggregate(Aggregate, u64),
}

/// A string kept up to a bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clipped {
    /// The bytes kept, the first ones of the value.
    pub bytes: Vec<u8>,
    /// The length the server announced. Larger than `bytes.len()` when the
    /// value was cut.
    pub full_len: u64,
}

impl Clipped {
    /// Was part of the value discarded?
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        u64::try_from(self.bytes.len()).map_or(true, |kept| kept < self.full_len)
    }
}

/// One reply element, read whole within a byte budget.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// No value.
    Null,
    /// A simple string.
    Simple(Vec<u8>),
    /// A bulk string, possibly cut.
    Bulk(Clipped),
    /// A verbatim string and its three-byte format.
    Verbatim {
        /// The format, `txt` or `mkd` today.
        format: Vec<u8>,
        /// The text after the colon.
        text: Clipped,
    },
    /// An integer.
    Integer(i64),
    /// A double.
    Double(f64),
    /// A boolean.
    Boolean(bool),
    /// An integer beyond 64 bits, as its digits.
    BigNumber(String),
    /// An error **inside** an aggregate — `EXEC` and scripts return them. A
    /// reply that *is* an error never becomes a value: it is the execution's
    /// error.
    Error(Vec<u8>),
    /// An aggregate other than a map.
    List(Aggregate, Vec<Value>),
    /// A map, pair by pair.
    Map(Vec<(Value, Value)>),
    /// An aggregate that was read and discarded: deeper than [`MAX_DEPTH`], or
    /// past the budget. Its length is what the server announced.
    Elided(Aggregate, u64),
}

impl Value {
    /// Was anything of this value discarded?
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        match self {
            Self::Bulk(text) | Self::Verbatim { text, .. } => text.is_truncated(),
            Self::List(_, items) => items.iter().any(Self::is_truncated),
            Self::Map(pairs) => pairs
                .iter()
                .any(|(key, value)| key.is_truncated() || value.is_truncated()),
            Self::Elided(..) => true,
            Self::Null
            | Self::Simple(_)
            | Self::Integer(_)
            | Self::Double(_)
            | Self::Boolean(_)
            | Self::BigNumber(_)
            | Self::Error(_) => false,
        }
    }
}

/// Encodes a command as the server expects it: an array of bulk strings.
///
/// Binary-safe: an argument is sent as its bytes, whatever they hold. That is
/// what keeps a key named `a b` or `x\r\n*1` one argument
/// ([I-10](../../../CLAUDE.md#i-10)).
#[must_use]
pub fn encode<A: AsRef<[u8]>>(argv: &[A]) -> Vec<u8> {
    let payload: usize = argv.iter().map(|arg| arg.as_ref().len() + 24).sum();
    let mut out = Vec::with_capacity(payload + 16);
    out.push(b'*');
    out.extend_from_slice(argv.len().to_string().as_bytes());
    out.extend_from_slice(b"\r\n");
    for arg in argv {
        let arg = arg.as_ref();
        out.push(b'$');
        out.extend_from_slice(arg.len().to_string().as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(arg);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// A RESP reader over a buffered stream.
#[derive(Debug)]
pub struct Reader<R> {
    inner: R,
}

impl<R: AsyncBufRead + Unpin + Send> Reader<R> {
    /// Wraps a buffered stream.
    pub const fn new(inner: R) -> Self {
        Self { inner }
    }

    /// The stream, to write a command on it.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// The next head, attributes skipped.
    ///
    /// A RESP3 attribute describes the element after it and is not part of
    /// the reply; Redis sends none today. It is read and dropped.
    ///
    /// # Errors
    /// [`WireError`] on a transport failure, a cut stream, or bytes that are
    /// not RESP.
    pub async fn head(&mut self) -> Result<Head, WireError> {
        loop {
            match self.raw_head().await? {
                RawHead::Attribute(len) => {
                    self.skip(Aggregate::Map.elements(len)?).await?;
                }
                RawHead::Head(head) => return Ok(head),
            }
        }
    }

    /// Reads a bulk payload of `len` bytes and its terminator, keeping the
    /// first `keep` of them.
    ///
    /// # Errors
    /// As [`Reader::head`].
    pub async fn bulk(&mut self, len: u64, keep: usize) -> Result<Clipped, WireError> {
        let kept = usize::try_from(len).map_or(keep, |len| len.min(keep));
        let mut bytes = Vec::with_capacity(kept.min(1 << 16));
        let mut remaining = len;
        while remaining > 0 {
            let available = self.inner.fill_buf().await?;
            if available.is_empty() {
                return Err(WireError::Truncated);
            }
            let take =
                usize::try_from(remaining).map_or(available.len(), |r| r.min(available.len()));
            let wanted = kept.saturating_sub(bytes.len()).min(take);
            if let Some(slice) = available.get(..wanted) {
                bytes.extend_from_slice(slice);
            }
            self.inner.consume(take);
            remaining = remaining.saturating_sub(u64::try_from(take).unwrap_or(u64::MAX));
        }
        self.terminator().await?;
        Ok(Clipped {
            bytes,
            full_len: len,
        })
    }

    /// Reads one whole element after its `head`, keeping at most `budget`
    /// bytes of it; what exceeds is read and discarded, and marked.
    ///
    /// # Errors
    /// As [`Reader::head`].
    pub async fn rest_of(&mut self, head: Head, budget: &mut usize) -> Result<Value, WireError> {
        self.element(head, 0, budget).await
    }

    /// Reads one whole element, head included.
    ///
    /// # Errors
    /// As [`Reader::head`].
    pub async fn value(&mut self, budget: &mut usize) -> Result<Value, WireError> {
        let head = self.head().await?;
        self.element(head, 0, budget).await
    }

    /// Reads and discards `elements` whole elements, nested ones included.
    ///
    /// Iterative: a hostile nesting a million levels deep costs a counter, not
    /// a stack.
    ///
    /// # Errors
    /// As [`Reader::head`].
    pub async fn skip(&mut self, elements: u64) -> Result<(), WireError> {
        let mut pending = elements;
        while pending > 0 {
            pending -= 1;
            match self.raw_head().await? {
                RawHead::Attribute(len) => {
                    // The attribute's pairs, then the element it describes.
                    pending = pending
                        .checked_add(Aggregate::Map.elements(len)?)
                        .and_then(|p| p.checked_add(1))
                        .ok_or(WireError::Malformed("an attribute length overflows"))?;
                }
                RawHead::Head(Head::Bulk(len) | Head::Verbatim(len) | Head::BulkError(len)) => {
                    self.bulk(len, 0).await?;
                }
                RawHead::Head(Head::Aggregate(kind, len)) => {
                    pending = pending
                        .checked_add(kind.elements(len)?)
                        .ok_or(WireError::Malformed("an aggregate length overflows"))?;
                }
                RawHead::Head(_) => {}
            }
        }
        Ok(())
    }

    fn element<'a>(
        &'a mut self,
        head: Head,
        depth: usize,
        budget: &'a mut usize,
    ) -> BoxFuture<'a, Result<Value, WireError>> {
        Box::pin(async move {
            // The element's own cost is charged after its bytes: a budget of
            // three bytes keeps three bytes of a string, not none.
            let value = match head {
                Head::Simple(text) => {
                    *budget = budget.saturating_sub(text.len());
                    Value::Simple(text)
                }
                Head::Error(text) => {
                    *budget = budget.saturating_sub(text.len());
                    Value::Error(text)
                }
                Head::BulkError(len) => {
                    let text = self.bulk(len, (*budget).min(MAX_LINE)).await?;
                    *budget = budget.saturating_sub(text.bytes.len());
                    Value::Error(text.bytes)
                }
                Head::Integer(n) => Value::Integer(n),
                Head::Double(x) => Value::Double(x),
                Head::Boolean(b) => Value::Boolean(b),
                Head::BigNumber(digits) => {
                    *budget = budget.saturating_sub(digits.len());
                    Value::BigNumber(digits)
                }
                Head::Null => Value::Null,
                Head::Bulk(len) => {
                    let text = self.bulk(len, *budget).await?;
                    *budget = budget.saturating_sub(text.bytes.len());
                    Value::Bulk(text)
                }
                Head::Verbatim(len) => {
                    let whole = self.bulk(len, budget.saturating_add(4)).await?;
                    *budget = budget.saturating_sub(whole.bytes.len());
                    split_verbatim(whole)?
                }
                Head::Aggregate(kind, len) => {
                    if depth >= MAX_DEPTH || *budget == 0 {
                        self.skip(kind.elements(len)?).await?;
                        Value::Elided(kind, len)
                    } else {
                        self.aggregate(kind, len, depth, budget).await?
                    }
                }
            };
            *budget = budget.saturating_sub(ELEMENT_COST);
            Ok(value)
        })
    }

    async fn aggregate(
        &mut self,
        kind: Aggregate,
        len: u64,
        depth: usize,
        budget: &mut usize,
    ) -> Result<Value, WireError> {
        let reserve = usize::try_from(len).map_or(PREALLOCATED, |n| n.min(PREALLOCATED));
        let mut remaining = len;
        if kind == Aggregate::Map {
            let mut pairs = Vec::with_capacity(reserve);
            while remaining > 0 {
                remaining -= 1;
                if *budget == 0 {
                    // Out of budget: the rest is consumed, and the pairs kept
                    // are followed by a marker of what was not.
                    self.skip(Aggregate::Map.elements(remaining.saturating_add(1))?)
                        .await?;
                    pairs.push((
                        Value::Elided(kind, remaining.saturating_add(1)),
                        Value::Null,
                    ));
                    return Ok(Value::Map(pairs));
                }
                let key_head = self.head().await?;
                let key = self.element(key_head, depth + 1, budget).await?;
                let value_head = self.head().await?;
                let value = self.element(value_head, depth + 1, budget).await?;
                pairs.push((key, value));
            }
            return Ok(Value::Map(pairs));
        }
        let mut items = Vec::with_capacity(reserve);
        while remaining > 0 {
            remaining -= 1;
            if *budget == 0 {
                self.skip(remaining.saturating_add(1)).await?;
                items.push(Value::Elided(kind, remaining.saturating_add(1)));
                return Ok(Value::List(kind, items));
            }
            let head = self.head().await?;
            items.push(self.element(head, depth + 1, budget).await?);
        }
        Ok(Value::List(kind, items))
    }

    async fn raw_head(&mut self) -> Result<RawHead, WireError> {
        let line = self.line().await?;
        let (&marker, rest) = line.split_first().ok_or(WireError::Malformed(
            "an empty line where a type byte belongs",
        ))?;
        let head = match marker {
            b'+' => Head::Simple(rest.to_vec()),
            b'-' => Head::Error(rest.to_vec()),
            b':' => Head::Integer(parse_integer(rest)?),
            b',' => Head::Double(parse_double(rest)?),
            b'#' => match rest {
                b"t" => Head::Boolean(true),
                b"f" => Head::Boolean(false),
                _ => return Err(WireError::Malformed("a boolean that is neither t nor f")),
            },
            b'(' => Head::BigNumber(parse_big_number(rest)?),
            b'_' if rest.is_empty() => Head::Null,
            b'$' => length(rest)?.map_or(Head::Null, Head::Bulk),
            b'!' => Head::BulkError(length(rest)?.ok_or(WireError::Malformed("a null error"))?),
            b'=' => {
                Head::Verbatim(length(rest)?.ok_or(WireError::Malformed("a null verbatim string"))?)
            }
            b'*' => length(rest)?.map_or(Head::Null, |n| Head::Aggregate(Aggregate::Array, n)),
            b'~' => Head::Aggregate(Aggregate::Set, required_length(rest)?),
            b'%' => Head::Aggregate(Aggregate::Map, required_length(rest)?),
            b'>' => Head::Aggregate(Aggregate::Push, required_length(rest)?),
            b'|' => return Ok(RawHead::Attribute(required_length(rest)?)),
            _ => return Err(WireError::Malformed("a byte that starts no RESP type")),
        };
        Ok(RawHead::Head(head))
    }

    /// A line up to its CRLF, without it, bounded by [`MAX_LINE`].
    async fn line(&mut self) -> Result<Vec<u8>, WireError> {
        let mut line = Vec::new();
        loop {
            let available = self.inner.fill_buf().await?;
            if available.is_empty() {
                return Err(WireError::Truncated);
            }
            match available.iter().position(|&b| b == b'\n') {
                Some(end) => {
                    if let Some(part) = available.get(..end) {
                        line.extend_from_slice(part);
                    }
                    self.inner.consume(end.saturating_add(1));
                    if line.len() > MAX_LINE.saturating_add(1) {
                        return Err(WireError::Malformed(
                            "a line longer than the driver accepts",
                        ));
                    }
                    if line.pop() != Some(b'\r') {
                        return Err(WireError::Malformed(
                            "a line feed without a carriage return",
                        ));
                    }
                    return Ok(line);
                }
                None => {
                    line.extend_from_slice(available);
                    let consumed = available.len();
                    self.inner.consume(consumed);
                }
            }
            if line.len() > MAX_LINE {
                return Err(WireError::Malformed(
                    "a line longer than the driver accepts",
                ));
            }
        }
    }

    async fn terminator(&mut self) -> Result<(), WireError> {
        let mut seen = [0_u8; 2];
        let mut filled = 0;
        while filled < 2 {
            let available = self.inner.fill_buf().await?;
            let Some(&byte) = available.first() else {
                return Err(WireError::Truncated);
            };
            if let Some(slot) = seen.get_mut(filled) {
                *slot = byte;
            }
            self.inner.consume(1);
            filled += 1;
        }
        if seen == *b"\r\n" {
            Ok(())
        } else {
            Err(WireError::Malformed("a bulk string not followed by CRLF"))
        }
    }
}

enum RawHead {
    Head(Head),
    Attribute(u64),
}

fn split_verbatim(whole: Clipped) -> Result<Value, WireError> {
    let Clipped { bytes, full_len } = whole;
    let (format, rest) = match (bytes.get(..3), bytes.get(3), bytes.get(4..)) {
        (Some(format), Some(b':'), Some(rest)) => (format.to_vec(), rest.to_vec()),
        _ => return Err(WireError::Malformed("a verbatim string without its format")),
    };
    Ok(Value::Verbatim {
        format,
        text: Clipped {
            bytes: rest,
            full_len: full_len.saturating_sub(4),
        },
    })
}

/// A length: `-1` is the RESP2 null, anything else negative is malformed, and
/// `?` — a streamed RESP3 type Redis never emits — is refused.
fn length(text: &[u8]) -> Result<Option<u64>, WireError> {
    if text == b"?" {
        return Err(WireError::Malformed(
            "a streamed RESP3 type, which this driver does not read",
        ));
    }
    if text == b"-1" {
        return Ok(None);
    }
    let digits = std::str::from_utf8(text)
        .map_err(|_| WireError::Malformed("a length that is not digits"))?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WireError::Malformed("a length that is not digits"));
    }
    digits
        .parse::<u64>()
        .map(Some)
        .map_err(|_| WireError::Malformed("a length beyond 64 bits"))
}

fn required_length(text: &[u8]) -> Result<u64, WireError> {
    length(text)?.ok_or(WireError::Malformed(
        "a null where an aggregate length belongs",
    ))
}

fn parse_integer(text: &[u8]) -> Result<i64, WireError> {
    let digits = text.strip_prefix(b"+").unwrap_or(text);
    std::str::from_utf8(digits)
        .ok()
        .filter(|d| {
            !d.is_empty()
                && d.bytes()
                    .enumerate()
                    .all(|(i, b)| b.is_ascii_digit() || (i == 0 && b == b'-'))
        })
        .and_then(|d| d.parse::<i64>().ok())
        .ok_or(WireError::Malformed(
            "an integer outside the signed 64-bit range",
        ))
}

fn parse_double(text: &[u8]) -> Result<f64, WireError> {
    let accepted = |b: u8| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E');
    let literal = std::str::from_utf8(text)
        .map_err(|_| WireError::Malformed("a double that is not ASCII"))?;
    match literal {
        "inf" | "+inf" => return Ok(f64::INFINITY),
        "-inf" => return Ok(f64::NEG_INFINITY),
        "nan" | "-nan" => return Ok(f64::NAN),
        _ => {}
    }
    if literal.is_empty() || !literal.bytes().all(accepted) {
        return Err(WireError::Malformed("a double that is not a number"));
    }
    literal
        .parse::<f64>()
        .map_err(|_| WireError::Malformed("a double that is not a number"))
}

fn parse_big_number(text: &[u8]) -> Result<String, WireError> {
    let digits = text
        .strip_prefix(b"+")
        .or_else(|| text.strip_prefix(b"-"))
        .unwrap_or(text);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(WireError::Malformed("a big number that is not digits"));
    }
    String::from_utf8(text.to_vec())
        .map_err(|_| WireError::Malformed("a big number that is not digits"))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn reader(bytes: &[u8]) -> Reader<&[u8]> {
        Reader::new(bytes)
    }

    async fn one(bytes: &[u8]) -> Result<Value, WireError> {
        let mut budget = 1 << 20;
        reader(bytes).value(&mut budget).await
    }

    #[test]
    fn a_command_is_an_array_of_binary_safe_bulk_strings() {
        let argv: [&[u8]; 3] = [b"SET", b"a b\r\n*1", b"\x00\xff"];
        assert_eq!(
            encode(&argv),
            b"*3\r\n$3\r\nSET\r\n$7\r\na b\r\n*1\r\n$2\r\n\x00\xff\r\n".to_vec()
        );
    }

    #[tokio::test]
    async fn every_resp3_scalar_is_read_with_its_type() {
        assert_eq!(
            one(b"+OK\r\n").await.expect("valid RESP"),
            Value::Simple(b"OK".to_vec())
        );
        assert_eq!(
            one(b":-42\r\n").await.expect("valid RESP"),
            Value::Integer(-42)
        );
        assert_eq!(
            one(b":+7\r\n").await.expect("valid RESP"),
            Value::Integer(7)
        );
        assert_eq!(
            one(b"#t\r\n").await.expect("valid RESP"),
            Value::Boolean(true)
        );
        assert_eq!(one(b"_\r\n").await.expect("valid RESP"), Value::Null);
        assert_eq!(one(b"$-1\r\n").await.expect("valid RESP"), Value::Null);
        assert_eq!(one(b"*-1\r\n").await.expect("valid RESP"), Value::Null);
        assert_eq!(
            one(b",1.5e3\r\n").await.expect("valid RESP"),
            Value::Double(1500.0)
        );
        assert_eq!(
            one(b",-inf\r\n").await.expect("valid RESP"),
            Value::Double(f64::NEG_INFINITY)
        );
        assert!(
            matches!(one(b",nan\r\n").await.expect("valid RESP"), Value::Double(x) if x.is_nan())
        );
        assert_eq!(
            one(b"(3492890328409238509324850943850943825024385\r\n")
                .await
                .expect("valid RESP"),
            Value::BigNumber("3492890328409238509324850943850943825024385".to_owned())
        );
        assert_eq!(
            one(b"=15\r\ntxt:Some string\r\n")
                .await
                .expect("valid RESP"),
            Value::Verbatim {
                format: b"txt".to_vec(),
                text: Clipped {
                    bytes: b"Some string".to_vec(),
                    full_len: 11
                }
            }
        );
        assert_eq!(
            one(b"!21\r\nSYNTAX invalid syntax\r\n")
                .await
                .expect("valid RESP"),
            Value::Error(b"SYNTAX invalid syntax".to_vec())
        );
    }

    #[tokio::test]
    async fn a_bulk_string_keeps_its_bytes_whatever_they_are() {
        let value = one(b"$6\r\n\x00\r\n\xff\xfe\n\r\n")
            .await
            .expect("valid RESP");
        assert_eq!(
            value,
            Value::Bulk(Clipped {
                bytes: b"\x00\r\n\xff\xfe\n".to_vec(),
                full_len: 6
            })
        );
    }

    #[tokio::test]
    async fn aggregates_nest_and_maps_keep_their_pairs() {
        let value = one(b"%2\r\n+first\r\n:1\r\n+second\r\n*2\r\n:2\r\n~1\r\n$1\r\nx\r\n")
            .await
            .expect("valid RESP");
        let Value::Map(pairs) = value else {
            panic!("a map")
        };
        assert_eq!(pairs.len(), 2);
        assert_eq!(
            pairs[0],
            (Value::Simple(b"first".to_vec()), Value::Integer(1))
        );
        assert!(matches!(&pairs[1].1, Value::List(Aggregate::Array, items) if items.len() == 2));
    }

    #[tokio::test]
    async fn an_attribute_is_skipped_and_the_reply_after_it_read() {
        let bytes = b"|1\r\n+key-popularity\r\n%1\r\n$1\r\na\r\n,0.19\r\n*1\r\n:2039123\r\n";
        assert_eq!(
            one(bytes).await.expect("valid RESP"),
            Value::List(Aggregate::Array, vec![Value::Integer(2_039_123)])
        );
    }

    #[tokio::test]
    async fn a_value_past_its_budget_is_cut_and_says_so() {
        let mut budget = 3;
        let value = reader(b"$10\r\n0123456789\r\n:1\r\n")
            .value(&mut budget)
            .await
            .expect("valid RESP");
        assert!(value.is_truncated());
        assert_eq!(
            value,
            Value::Bulk(Clipped {
                bytes: b"012".to_vec(),
                full_len: 10
            })
        );
    }

    #[tokio::test]
    async fn the_stream_stays_in_step_after_a_cut_or_an_elision() {
        // What follows the cut value must read as the next reply: a reader
        // out of step would hand the next command the previous one's bytes.
        let bytes = b"*3\r\n$5\r\nhello\r\n*2\r\n:1\r\n:2\r\n$3\r\nend\r\n+NEXT\r\n";
        let mut r = reader(bytes);
        let mut budget = ELEMENT_COST * 2 + 2;
        let value = r.value(&mut budget).await.expect("valid RESP");
        assert!(value.is_truncated(), "{value:?}");
        let mut more = 1 << 10;
        assert_eq!(
            r.value(&mut more).await.expect("valid RESP"),
            Value::Simple(b"NEXT".to_vec())
        );
    }

    #[tokio::test]
    async fn a_nesting_deeper_than_kept_is_skipped_not_recursed() {
        let mut bytes = Vec::new();
        for _ in 0..10_000 {
            bytes.extend_from_slice(b"*1\r\n");
        }
        bytes.extend_from_slice(b":1\r\n+NEXT\r\n");
        let mut r = reader(&bytes);
        let mut budget = 1 << 20;
        let value = r.value(&mut budget).await.expect("valid RESP");
        assert!(value.is_truncated());
        assert_eq!(
            r.value(&mut budget).await.expect("valid RESP"),
            Value::Simple(b"NEXT".to_vec())
        );
    }

    #[tokio::test]
    async fn an_announced_length_allocates_nothing_by_itself() {
        // Nine quintillion elements announced, two sent: the reader fails on
        // the cut stream, it does not try to reserve them.
        let error = one(b"*9223372036854775807\r\n:1\r\n:2\r\n")
            .await
            .expect_err("refused");
        assert!(matches!(error, WireError::Truncated), "{error:?}");
    }

    #[rstest]
    #[case::cut_in_a_bulk(b"$10\r\nabc".as_slice())]
    #[case::cut_before_crlf(b"$3\r\nabc".as_slice())]
    #[case::cut_in_an_array(b"*3\r\n:1\r\n".as_slice())]
    #[case::cut_in_a_line(b"+OK".as_slice())]
    #[case::nothing(b"".as_slice())]
    #[tokio::test]
    async fn a_reply_cut_by_the_end_of_the_stream_is_truncated(#[case] bytes: &[u8]) {
        assert!(matches!(one(bytes).await, Err(WireError::Truncated)));
    }

    #[rstest]
    #[case::unknown_type(b"?what\r\n".as_slice())]
    #[case::bare_line_feed(b"+OK\n".as_slice())]
    #[case::integer_overflow(b":9223372036854775808\r\n".as_slice())]
    #[case::integer_garbage(b":12a\r\n".as_slice())]
    #[case::negative_length(b"$-2\r\n".as_slice())]
    #[case::streamed_string(b"$?\r\n;5\r\nhello\r\n;0\r\n".as_slice())]
    #[case::bad_boolean(b"#x\r\n".as_slice())]
    #[case::bad_double(b",1.2.3x\r\n".as_slice())]
    #[case::bad_big_number(b"(12x\r\n".as_slice())]
    #[case::missing_terminator(b"$3\r\nabcXY".as_slice())]
    #[case::verbatim_without_format(b"=2\r\nab\r\n".as_slice())]
    #[case::null_set(b"~-1\r\n".as_slice())]
    #[tokio::test]
    async fn garbage_is_malformed_never_a_panic(#[case] bytes: &[u8]) {
        assert!(
            matches!(one(bytes).await, Err(WireError::Malformed(_))),
            "{bytes:?}"
        );
    }

    #[tokio::test]
    async fn a_line_without_end_is_refused_before_it_fills_memory() {
        let mut bytes = vec![b'+'];
        bytes.extend(std::iter::repeat_n(b'a', MAX_LINE + 10));
        bytes.extend_from_slice(b"\r\n");
        let mut budget = 1 << 20;
        let error = Reader::new(tokio::io::BufReader::with_capacity(4096, bytes.as_slice()))
            .value(&mut budget)
            .await
            .expect_err("refused");
        assert!(matches!(error, WireError::Malformed(_)), "{error:?}");
    }
}
