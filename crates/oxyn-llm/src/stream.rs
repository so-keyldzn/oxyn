//! From the HTTP byte stream to the event stream, for **every** protocol.
//!
//! This module speaks neither of provider, nor of authentication, nor of frame
//! format: it assembles the SSE decoder ([`crate::sse`]) and a protocol decoder
//! ([`EventDecoder`]) into a cancellable [`Stream`].
//!
//! # Why a single driver for two protocols
//!
//! The guarantees below are exactly what gets missed on the second writing: a
//! `Done` emitted twice, a `Done` never emitted on a break, a cancellation that
//! interrupts nothing. Holding them in a single place is what makes the
//! invariant reviewable; two drivers side by side would diverge within one
//! added variant.
//!
//! # What the stream guarantees
//!
//! * **`Done` is emitted exactly once, last.** Clean close, abrupt close,
//!   transport break, cancellation, buffer overrun: the five exits go through
//!   the same emission.
//! * **Cancellation interrupts the reading.** The token is checked before
//!   each wait *and* concurrently with it: a provider that stops answering
//!   does not leave the user in front of a button with no effect.
//! * **No error message goes out without scrubbing.** A provider can copy the
//!   received key into an error streamed **after** a `200` status — a gateway
//!   that quotes it in its diagnostic, for example. That message then becomes
//!   a displayed error: it goes here through the same guard as an HTTP error
//!   body (I-03), because this is the only place through which the events of
//!   every decoder pass.
//! * **Nothing is resumed after an interruption.** An SSE decoder that lost
//!   bytes is out of sync; a generation is restarted, it is not resumed.

use std::collections::VecDeque;
use std::pin::{Pin, pin};

use bytes::Bytes;
use futures::future::{Either, select};
use futures::stream::{BoxStream, Stream, StreamExt};
use oxyn_core::CancelToken;

use crate::error::sanitize;
use crate::secret::ApiKey;
use crate::sse::{SseDecoder, SseFrame};
use crate::types::ChatEvent;

/// Describes a stream break **without** copying the raw message.
///
/// The message of a transport error can contain the URL, hence the
/// credentials it may carry. We classify rather than copy.
pub(crate) fn describe_stream_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "timed out while receiving the stream".to_owned()
    } else if err.is_body() || err.is_decode() {
        "the provider interrupted the stream".to_owned()
    } else {
        "lost the connection while receiving the stream".to_owned()
    }
}

/// Byte stream already classified, as the decoder consumes it.
pub(crate) type ByteStream = Pin<Box<dyn Stream<Item = std::result::Result<Bytes, String>> + Send>>;

/// What a protocol decoder must be able to do to be driven here.
///
/// The termination methods are distinct because the decisions are.
///
/// **The driver does not know what an announced end is**: it is a protocol
/// frame (`message_stop`, `finish_reason`, `[DONE]`), and only the decoder
/// recognizes it. The driver only reports what it observes — the server
/// closed, the transport broke, the caller cancelled. A decoder that saw its
/// end announcement declares itself finished through [`is_done`](Self::is_done),
/// and the driver no longer calls it.
pub(crate) trait EventDecoder: Send {
    /// Consumes a complete SSE frame.
    fn on_frame(&mut self, frame: &SseFrame, out: &mut Vec<ChatEvent>);

    /// Has the decoder already emitted its end? Nothing more must be given to it.
    fn is_done(&self) -> bool;

    /// The server closed the stream **without** the decoder having declared
    /// itself finished.
    ///
    /// A clean close is not an announced end: a proxy that cuts at its
    /// duration limit closes cleanly. Unless the protocol announced the end of
    /// the generation, the decoder answers
    /// [`StopReason::Interrupted`](crate::types::StopReason::Interrupted) and
    /// throws away what it did not see close.
    fn finish(&mut self, out: &mut Vec<ChatEvent>);

    /// The caller cancelled.
    fn cancel(&mut self, out: &mut Vec<ChatEvent>);

    /// The transport broke mid-stream.
    fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>);
}

/// State carried from one decoding step to the next.
struct StreamState<D> {
    bytes: ByteStream,
    sse: SseDecoder,
    decoder: D,
    pending: VecDeque<ChatEvent>,
    cancel: CancelToken,
    /// The key sent to this provider, only to erase it from errors.
    key: Option<ApiKey>,
    finished: bool,
}

/// Outcome of a wait: a chunk, an end, or a cancellation.
enum Step {
    Cancelled,
    Chunk(Option<std::result::Result<Bytes, String>>),
}

/// Turns an SSE byte stream into a stream of domain events.
///
/// The returned stream is `'static` and `Send`: it can be handed to a task. It
/// emits exactly one [`ChatEvent::Done`], last, including on cancellation,
/// transport error or abrupt close.
///
/// `key` is the key presented to the provider: every [`ChatEvent::Error`] is
/// scrubbed of it, and truncated like an HTTP error body, before going out.
pub(crate) fn events_stream<D: EventDecoder + 'static>(
    bytes: ByteStream,
    decoder: D,
    cancel: CancelToken,
    key: Option<ApiKey>,
) -> BoxStream<'static, ChatEvent> {
    let state = StreamState {
        bytes,
        sse: SseDecoder::new(),
        decoder,
        pending: VecDeque::new(),
        cancel,
        key,
        finished: false,
    };

    futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.pending.pop_front() {
                return Some((event, state));
            }
            if state.finished {
                return None;
            }

            let mut outputs = Vec::new();

            // Check before the wait: an already cancelled token must not make
            // one more chunk be read.
            if state.cancel.is_cancelled() {
                state.decoder.cancel(&mut outputs);
                state.finished = true;
                let scrub_key = state.key.as_ref();
                state
                    .pending
                    .extend(outputs.into_iter().map(|event| redact(event, scrub_key)));
                continue;
            }

            let step = {
                let pending = pin!(state.cancel.cancelled());
                let next = pin!(state.bytes.next());
                match select(pending, next).await {
                    Either::Left(((), _)) => Step::Cancelled,
                    Either::Right((chunk, _)) => Step::Chunk(chunk),
                }
            };

            match step {
                Step::Cancelled => {
                    // The read future is dropped here. No resumption is
                    // attempted: an SSE decoder that lost bytes is out of
                    // sync, and a generation is restarted, it is not
                    // resumed.
                    state.decoder.cancel(&mut outputs);
                    state.finished = true;
                }
                Step::Chunk(None) => {
                    state.sse.finish();
                    drain(&mut state, &mut outputs);
                    state.decoder.finish(&mut outputs);
                    state.finished = true;
                }
                Step::Chunk(Some(Err(detail))) => {
                    state.decoder.transport_error(detail, &mut outputs);
                    state.finished = true;
                }
                Step::Chunk(Some(Ok(chunk))) => {
                    // The accumulation is taken out of the `match`: a borrow
                    // taken in the judged expression would stay alive during
                    // the arms, which borrow the state again.
                    let accumulation = state.sse.push(&chunk);
                    match accumulation {
                        Ok(()) => {
                            drain(&mut state, &mut outputs);
                            if state.decoder.is_done() {
                                state.finished = true;
                            }
                        }
                        Err(overflow) => {
                            state
                                .decoder
                                .transport_error(overflow.to_string(), &mut outputs);
                            state.finished = true;
                        }
                    }
                }
            }

            let scrub_key = state.key.as_ref();
            state
                .pending
                .extend(outputs.into_iter().map(|event| redact(event, scrub_key)));
        }
    })
    .boxed()
}

/// Passes an error message through the guard of HTTP error bodies.
///
/// Only [`ChatEvent::Error`] becomes a displayed message: the model's text is
/// content, not a diagnostic, and it does not have to be rewritten.
fn redact(event: ChatEvent, key: Option<&ApiKey>) -> ChatEvent {
    match event {
        ChatEvent::Error(message) => ChatEvent::Error(sanitize(&message, key)),
        other => other,
    }
}

/// Drains the SSE decoder into the protocol decoder.
fn drain<D: EventDecoder>(state: &mut StreamState<D>, outputs: &mut Vec<ChatEvent>) {
    loop {
        // `let … else` rather than `while let`: the borrow of the SSE decoder
        // ends at the end of the statement, before the protocol decoder is
        // borrowed in turn.
        let Some(frame) = state.sse.next_frame() else {
            return;
        };
        state.decoder.on_frame(&frame, outputs);
        if state.decoder.is_done() {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
