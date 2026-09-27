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

    /// Le transport a rompu en cours de flux.
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
    let etat = StreamState {
        bytes,
        sse: SseDecoder::new(),
        decoder,
        pending: VecDeque::new(),
        cancel,
        key,
        finished: false,
    };

    futures::stream::unfold(etat, |mut etat| async move {
        loop {
            if let Some(evenement) = etat.pending.pop_front() {
                return Some((evenement, etat));
            }
            if etat.finished {
                return None;
            }

            let mut sorties = Vec::new();

            // Check before the wait: an already cancelled token must not make
            // one more chunk be read.
            if etat.cancel.is_cancelled() {
                etat.decoder.cancel(&mut sorties);
                etat.finished = true;
                let cle = etat.key.as_ref();
                etat.pending
                    .extend(sorties.into_iter().map(|evenement| redact(evenement, cle)));
                continue;
            }

            let pas = {
                let attente = pin!(etat.cancel.cancelled());
                let suivant = pin!(etat.bytes.next());
                match select(attente, suivant).await {
                    Either::Left(((), _)) => Step::Cancelled,
                    Either::Right((morceau, _)) => Step::Chunk(morceau),
                }
            };

            match pas {
                Step::Cancelled => {
                    // The read future is dropped here. No resumption is
                    // attempted: an SSE decoder that lost bytes is out of
                    // sync, and a generation is restarted, it is not
                    // resumed.
                    etat.decoder.cancel(&mut sorties);
                    etat.finished = true;
                }
                Step::Chunk(None) => {
                    etat.sse.finish();
                    drain(&mut etat, &mut sorties);
                    etat.decoder.finish(&mut sorties);
                    etat.finished = true;
                }
                Step::Chunk(Some(Err(detail))) => {
                    etat.decoder.transport_error(detail, &mut sorties);
                    etat.finished = true;
                }
                Step::Chunk(Some(Ok(morceau))) => {
                    // The accumulation is taken out of the `match`: a borrow
                    // taken in the judged expression would stay alive during
                    // the arms, which borrow the state again.
                    let accumulation = etat.sse.push(&morceau);
                    match accumulation {
                        Ok(()) => {
                            drain(&mut etat, &mut sorties);
                            if etat.decoder.is_done() {
                                etat.finished = true;
                            }
                        }
                        Err(depassement) => {
                            etat.decoder
                                .transport_error(depassement.to_string(), &mut sorties);
                            etat.finished = true;
                        }
                    }
                }
            }

            let cle = etat.key.as_ref();
            etat.pending
                .extend(sorties.into_iter().map(|evenement| redact(evenement, cle)));
        }
    })
    .boxed()
}

/// Passes an error message through the guard of HTTP error bodies.
///
/// Only [`ChatEvent::Error`] becomes a displayed message: the model's text is
/// content, not a diagnostic, and it does not have to be rewritten.
fn redact(evenement: ChatEvent, cle: Option<&ApiKey>) -> ChatEvent {
    match evenement {
        ChatEvent::Error(message) => ChatEvent::Error(sanitize(&message, cle)),
        autre => autre,
    }
}

/// Drains the SSE decoder into the protocol decoder.
fn drain<D: EventDecoder>(etat: &mut StreamState<D>, sorties: &mut Vec<ChatEvent>) {
    loop {
        // `let … else` rather than `while let`: the borrow of the SSE decoder
        // ends at the end of the statement, before the protocol decoder is
        // borrowed in turn.
        let Some(trame) = etat.sse.next_frame() else {
            return;
        };
        etat.decoder.on_frame(&trame, sorties);
        if etat.decoder.is_done() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StopReason;

    /// Test decoder: each `data:` frame becomes a text fragment.
    ///
    /// It knows no real protocol — it is the driver that is tested here, and
    /// the real decoders are tested in their own module.
    #[derive(Default)]
    struct Echo {
        done: bool,
        stop: Option<StopReason>,
    }

    impl Echo {
        fn emit_done(&mut self, out: &mut Vec<ChatEvent>) {
            if self.done {
                return;
            }
            self.done = true;
            out.push(ChatEvent::Done {
                stop_reason: self.stop.take().unwrap_or(StopReason::Unspecified),
            });
        }
    }

    impl EventDecoder for Echo {
        fn on_frame(&mut self, frame: &SseFrame, out: &mut Vec<ChatEvent>) {
            if self.done {
                return;
            }
            if frame.data == "fin" {
                self.stop = Some(StopReason::EndTurn);
                self.emit_done(out);
                return;
            }
            out.push(ChatEvent::TextDelta(frame.data.clone()));
        }

        fn is_done(&self) -> bool {
            self.done
        }

        fn finish(&mut self, out: &mut Vec<ChatEvent>) {
            self.emit_done(out);
        }

        fn cancel(&mut self, out: &mut Vec<ChatEvent>) {
            self.stop = Some(StopReason::Cancelled);
            self.emit_done(out);
        }

        fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>) {
            if self.done {
                return;
            }
            out.push(ChatEvent::Error(detail));
            self.stop = Some(StopReason::Interrupted);
            self.emit_done(out);
        }
    }

    fn morceaux(parts: &[&'static str]) -> ByteStream {
        let items: Vec<std::result::Result<Bytes, String>> = parts
            .iter()
            .map(|p| Ok(Bytes::from_static(p.as_bytes())))
            .collect();
        Box::pin(futures::stream::iter(items))
    }

    fn collecter(flux: BoxStream<'static, ChatEvent>) -> Vec<ChatEvent> {
        futures::executor::block_on(flux.collect())
    }

    fn jouer(parts: &[&'static str], jeton: CancelToken) -> Vec<ChatEvent> {
        collecter(events_stream(morceaux(parts), Echo::default(), jeton, None))
    }

    #[test]
    fn a_complete_stream_becomes_events() {
        let evenements = jouer(
            &["data: a\n\n", "data: b\n\n", "data: fin\n\n"],
            CancelToken::new(),
        );
        assert_eq!(
            evenements,
            vec![
                ChatEvent::TextDelta("a".to_owned()),
                ChatEvent::TextDelta("b".to_owned()),
                ChatEvent::Done {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    /// The driver guarantees the presence of `Done`; the reason is the
    /// decoder's business, tested in each protocol module.
    #[test]
    fn a_stream_closed_without_end_marker_still_ends() {
        let evenements = jouer(&["data: a\n\n"], CancelToken::new());
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Unspecified
            })
        );
    }

    #[test]
    fn an_empty_stream_still_produces_an_end() {
        let evenements = jouer(&[], CancelToken::new());
        assert_eq!(evenements.len(), 1, "{evenements:?}");
        assert!(evenements[0].is_terminal());
    }

    #[test]
    fn done_is_emitted_only_once_even_with_frames_after() {
        let evenements = jouer(&["data: fin\n\n", "data: fantome\n\n"], CancelToken::new());
        assert_eq!(evenements.len(), 1, "{evenements:?}");
        assert!(evenements[0].is_terminal());
    }

    #[test]
    fn an_already_cancelled_token_reads_no_chunk() {
        let jeton = CancelToken::new();
        jeton.cancel();
        assert_eq!(
            jouer(&["data: a\n\n"], jeton),
            vec![ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            }]
        );
    }

    #[test]
    fn a_cancellation_mid_stream_stops_the_reading() {
        let jeton = CancelToken::new();
        let declencheur = jeton.clone();
        let octets = futures::stream::iter(vec!["data: a\n\n", "data: b\n\n"]).map(
            move |morceau| -> std::result::Result<Bytes, String> {
                declencheur.cancel();
                Ok(Bytes::from_static(morceau.as_bytes()))
            },
        );

        let evenements = collecter(events_stream(
            Box::pin(octets),
            Echo::default(),
            jeton,
            None,
        ));
        assert_eq!(
            evenements,
            vec![
                ChatEvent::TextDelta("a".to_owned()),
                ChatEvent::Done {
                    stop_reason: StopReason::Cancelled
                },
            ],
            "the second chunk must never be read"
        );
    }

    #[test]
    fn a_transport_break_ends_cleanly() {
        let octets = futures::stream::iter(vec![
            Ok(Bytes::from_static(b"data: a\n\n")),
            Err("lost the connection while receiving the stream".to_owned()),
        ]);
        let evenements = collecter(events_stream(
            Box::pin(octets),
            Echo::default(),
            CancelToken::new(),
            None,
        ));
        assert_eq!(
            evenements.first(),
            Some(&ChatEvent::TextDelta("a".to_owned()))
        );
        assert!(
            evenements.last().is_some_and(ChatEvent::is_terminal),
            "{evenements:?}"
        );
    }

    #[test]
    fn a_stream_without_line_ending_is_bounded_and_ends() {
        // A faulty server that never sends a line ending must not make memory
        // swell without limit: the decoder bounds it, and the stream ends on
        // an error followed by `Done`.
        let deluge: Vec<std::result::Result<Bytes, String>> = vec![Ok(Bytes::from(vec![
            b'x';
            crate::sse::DEFAULT_BUFFER_LIMIT
                + 1
        ]))];
        let evenements = collecter(events_stream(
            Box::pin(futures::stream::iter(deluge)),
            Echo::default(),
            CancelToken::new(),
            None,
        ));
        assert!(
            evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "{evenements:?}"
        );
        assert!(evenements.last().is_some_and(ChatEvent::is_terminal));
    }
}
