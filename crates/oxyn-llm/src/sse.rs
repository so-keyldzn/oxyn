//! Decoding a `text/event-stream` stream.
//!
//! The three provider families stream in SSE; only the content of the frames
//! differs. This module therefore knows **no** provider: it returns frames, and
//! it is the caller that interprets them.
//!
//! # What decoding must hold
//!
//! An HTTP stream arrives in chunks that have nothing to do with lines: a frame
//! can be cut in its middle, a multi-byte character too. The decoder therefore
//! accumulates up to the end of line before interpreting anything — that is
//! what avoids a `from_utf8_lossy` that would replace an `é` cut in two with a
//! replacement character.
//!
//! # The bound
//!
//! The buffer is **bounded**. A faulty, or hostile, server that emitted bytes
//! without ever sending an end of line would otherwise make memory swell
//! without limit. Exceeding the bound is a decoding error, not a panic.

use bytes::BytesMut;

/// Bound of the accumulation buffer, in bytes.
///
/// A completion stream frame weighs a few hundred bytes; eight mebibytes
/// leave a considerable margin while keeping memory bounded.
pub(crate) const DEFAULT_BUFFER_LIMIT: usize = 8 * 1024 * 1024;

/// A complete SSE frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseFrame {
    /// Content of the `event` field, when the server sends one. OpenAI-compatible
    /// protocols do not send any; Anthropic does.
    pub(crate) event: Option<String>,
    /// Concatenated `data` fields, separated by line breaks, as the
    /// specification requires.
    pub(crate) data: String,
}

/// The buffer exceeded its bound without a frame ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("SSE frame exceeded {limit} bytes without a line break")]
pub(crate) struct SseOverflow {
    /// Exceeded bound, in bytes.
    pub(crate) limit: usize,
}

/// SSE stream accumulator.
///
/// Usage: [`push`](Self::push) at each received chunk, then
/// [`next_frame`](Self::next_frame) in a loop until `None`. When the stream
/// closes, [`finish`](Self::finish) then a last loop: some servers close
/// without sending the final empty line.
#[derive(Debug)]
pub(crate) struct SseDecoder {
    buffer: BytesMut,
    event: Option<String>,
    data: String,
    has_data: bool,
    limit: usize,
}

impl SseDecoder {
    /// Decoder with the default bound.
    pub(crate) fn new() -> Self {
        Self::with_limit(DEFAULT_BUFFER_LIMIT)
    }

    /// Decoder with a chosen bound. Reserved for tests.
    pub(crate) fn with_limit(limit: usize) -> Self {
        Self {
            buffer: BytesMut::new(),
            event: None,
            data: String::new(),
            has_data: false,
            limit,
        }
    }

    /// Adds a chunk received from the network.
    ///
    /// # Errors
    /// [`SseOverflow`] if the accumulation — buffer of incomplete lines plus
    /// data already gathered — exceeds the bound.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<(), SseOverflow> {
        self.buffer.extend_from_slice(chunk);
        let accumule = self.buffer.len().saturating_add(self.data.len());
        if accumule > self.limit {
            return Err(SseOverflow { limit: self.limit });
        }
        Ok(())
    }

    /// Signals the closing of the stream.
    ///
    /// Injects the end of frame the server may not have sent, so that a last
    /// `next_frame` returns what was in progress. Does not go through the bound
    /// check: two bytes put nothing at risk.
    pub(crate) fn finish(&mut self) {
        self.buffer.extend_from_slice(b"\n\n");
    }

    /// Extracts a complete line, without its `\n` nor a possible `\r`.
    fn take_line(&mut self) -> Option<Vec<u8>> {
        let position = self.buffer.iter().position(|octet| *octet == b'\n')?;
        let mut ligne = self.buffer.split_to(position + 1);
        // Retire le `\n` terminal.
        let _ = ligne.split_off(position);
        let mut ligne = ligne.to_vec();
        if ligne.last() == Some(&b'\r') {
            ligne.pop();
        }
        Some(ligne)
    }

    /// Returns the current frame and resets the accumulation.
    fn take_frame(&mut self) -> SseFrame {
        self.has_data = false;
        SseFrame {
            event: self.event.take(),
            data: std::mem::take(&mut self.data),
        }
    }

    /// Returns the next complete frame, or `None` if more bytes are needed.
    pub(crate) fn next_frame(&mut self) -> Option<SseFrame> {
        while let Some(ligne) = self.take_line() {
            // Empty line: end of frame. A frame without a `data` field is a
            // heartbeat, not an event.
            if ligne.is_empty() {
                if self.has_data || self.event.is_some() {
                    return Some(self.take_frame());
                }
                continue;
            }
            // Line starting with `:`: a comment, often a connection
            // keep-alive.
            if ligne.first() == Some(&b':') {
                continue;
            }

            // At this point the line is complete: UTF-8 can no longer be cut by
            // a chunk boundary, and `from_utf8_lossy` only replaces what is
            // really invalid.
            let texte = String::from_utf8_lossy(&ligne);
            let texte: &str = &texte;
            let (champ, valeur) = match texte.find(':') {
                Some(i) => {
                    let (champ, reste) = texte.split_at(i);
                    // A single space after the colon belongs to the syntax,
                    // not to the value; the following ones are part of it.
                    let brut = reste.get(1..).unwrap_or_default();
                    (champ, brut.strip_prefix(' ').unwrap_or(brut))
                }
                None => (texte, ""),
            };

            match champ {
                "data" => {
                    if self.has_data {
                        self.data.push('\n');
                    }
                    self.data.push_str(valeur);
                    self.has_data = true;
                }
                "event" => self.event = Some(valeur.to_owned()),
                // `id` and `retry` only serve connection resumption, which this
                // crate does not do: a generation is not resumed in the middle,
                // it is restarted.
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pushes a chunk and gathers all available frames.
    fn trames(decodeur: &mut SseDecoder, morceau: &[u8]) -> Vec<SseFrame> {
        decodeur.push(morceau).expect("no overflow expected");
        let mut sorties = Vec::new();
        while let Some(trame) = decodeur.next_frame() {
            sorties.push(trame);
        }
        sorties
    }

    #[test]
    fn a_simple_frame_is_read() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: bonjour\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, "bonjour");
        assert_eq!(sorties[0].event, None);
    }

    #[test]
    fn a_frame_cut_by_the_network_is_reassembled() {
        // The nominal case: HTTP chunks have nothing to do with lines.
        let mut d = SseDecoder::new();
        assert!(trames(&mut d, b"data: {\"cho").is_empty());
        assert!(trames(&mut d, b"ices\":[]}").is_empty());
        let sorties = trames(&mut d, b"\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, r#"{"choices":[]}"#);
    }

    #[test]
    fn a_cut_multibyte_character_is_not_corrupted() {
        // "é" is 0xC3 0xA9. Cut between the two, an immediate decoding would
        // produce a replacement character.
        let mut d = SseDecoder::new();
        assert!(trames(&mut d, b"data: caf\xc3").is_empty());
        let sorties = trames(&mut d, b"\xa9\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, "café");
    }

    #[test]
    fn windows_line_endings_are_accepted() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: a\r\n\r\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, "a");
    }

    #[test]
    fn keep_alive_comments_are_ignored() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b": ping\n\ndata: utile\n\n");
        assert_eq!(sorties.len(), 1, "{sorties:?}");
        assert_eq!(sorties[0].data, "utile");
    }

    #[test]
    fn several_data_fields_concatenate_with_a_line_break() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: une\ndata: deux\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, "une\ndeux");
    }

    #[test]
    fn the_event_field_is_kept() {
        // Anthropic names its frames; decoding must return the name.
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"event: content_block_delta\ndata: {}\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].event.as_deref(), Some("content_block_delta"));
        assert_eq!(sorties[0].data, "{}");
    }

    #[test]
    fn the_end_sentinel_is_data_like_any_other() {
        // It is up to the caller to recognize `[DONE]`, not the SSE decoder.
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: [DONE]\n\n");
        assert_eq!(sorties[0].data, "[DONE]");
    }

    #[test]
    fn several_frames_in_a_single_chunk() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: a\n\ndata: b\n\ndata: c\n\n");
        let contenus: Vec<&str> = sorties.iter().map(|t| t.data.as_str()).collect();
        assert_eq!(contenus, ["a", "b", "c"]);
    }

    #[test]
    fn a_field_without_space_after_the_colon_is_read_too() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data:{\"a\":1}\n\n");
        assert_eq!(sorties[0].data, r#"{"a":1}"#);
    }

    #[test]
    fn a_single_space_is_removed_the_following_ones_are_not() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data:   trois espaces\n\n");
        assert_eq!(sorties[0].data, "  trois espaces");
    }

    #[test]
    fn a_close_without_final_empty_line_returns_the_last_frame() {
        // Plusieurs serveurs locaux ferment ainsi.
        let mut d = SseDecoder::new();
        assert!(trames(&mut d, b"data: dernier\n").is_empty());
        d.finish();
        let trame = d.next_frame().expect("the current frame must be returned");
        assert_eq!(trame.data, "dernier");
        assert!(d.next_frame().is_none());
    }

    #[test]
    fn a_close_on_a_clean_stream_makes_no_frame() {
        let mut d = SseDecoder::new();
        let _ = trames(&mut d, b"data: a\n\n");
        d.finish();
        assert_eq!(d.next_frame(), None);
    }

    #[test]
    fn a_stream_without_line_ending_is_bounded() {
        // Without a bound, a faulty server makes memory swell without limit.
        let mut d = SseDecoder::with_limit(64);
        let erreur = d.push(&[b'x'; 128]).expect_err("overflow expected");
        assert_eq!(erreur.limit, 64);
    }

    #[test]
    fn the_bound_also_counts_the_data_already_gathered() {
        let mut d = SseDecoder::with_limit(64);
        d.push(b"data: ").expect("under the bound");
        d.push(&[b'y'; 40]).expect("under the bound");
        d.push(b"\n").expect("under the bound");
        // The 40 bytes moved from the buffer to `data`.
        assert!(d.next_frame().is_none());
        let erreur = d.push(&[b'z'; 40]).expect_err("overflow expected");
        assert_eq!(erreur.limit, 64);
    }
}
