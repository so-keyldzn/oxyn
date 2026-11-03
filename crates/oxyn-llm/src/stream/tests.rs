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

fn chunks(parts: &[&'static str]) -> ByteStream {
    let items: Vec<std::result::Result<Bytes, String>> = parts
        .iter()
        .map(|p| Ok(Bytes::from_static(p.as_bytes())))
        .collect();
    Box::pin(futures::stream::iter(items))
}

fn collect(flux: BoxStream<'static, ChatEvent>) -> Vec<ChatEvent> {
    futures::executor::block_on(flux.collect())
}

fn play(parts: &[&'static str], token: CancelToken) -> Vec<ChatEvent> {
    collect(events_stream(chunks(parts), Echo::default(), token, None))
}

#[test]
fn a_complete_stream_becomes_events() {
    let events = play(
        &["data: a\n\n", "data: b\n\n", "data: fin\n\n"],
        CancelToken::new(),
    );
    assert_eq!(
        events,
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
    let events = play(&["data: a\n\n"], CancelToken::new());
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Unspecified
        })
    );
}

#[test]
fn an_empty_stream_still_produces_an_end() {
    let events = play(&[], CancelToken::new());
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].is_terminal());
}

#[test]
fn done_is_emitted_only_once_even_with_frames_after() {
    let events = play(&["data: fin\n\n", "data: fantome\n\n"], CancelToken::new());
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].is_terminal());
}

#[test]
fn an_already_cancelled_token_reads_no_chunk() {
    let token = CancelToken::new();
    token.cancel();
    assert_eq!(
        play(&["data: a\n\n"], token),
        vec![ChatEvent::Done {
            stop_reason: StopReason::Cancelled
        }]
    );
}

#[test]
fn a_cancellation_mid_stream_stops_the_reading() {
    let token = CancelToken::new();
    let trigger = token.clone();
    let bytes = futures::stream::iter(vec!["data: a\n\n", "data: b\n\n"]).map(
        move |chunk| -> std::result::Result<Bytes, String> {
            trigger.cancel();
            Ok(Bytes::from_static(chunk.as_bytes()))
        },
    );

    let events = collect(events_stream(Box::pin(bytes), Echo::default(), token, None));
    assert_eq!(
        events,
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
    let bytes = futures::stream::iter(vec![
        Ok(Bytes::from_static(b"data: a\n\n")),
        Err("lost the connection while receiving the stream".to_owned()),
    ]);
    let events = collect(events_stream(
        Box::pin(bytes),
        Echo::default(),
        CancelToken::new(),
        None,
    ));
    assert_eq!(events.first(), Some(&ChatEvent::TextDelta("a".to_owned())));
    assert!(
        events.last().is_some_and(ChatEvent::is_terminal),
        "{events:?}"
    );
}

#[test]
fn a_stream_without_line_ending_is_bounded_and_ends() {
    // A faulty server that never sends a line ending must not make memory
    // swell without limit: the decoder bounds it, and the stream ends on
    // an error followed by `Done`.
    let flood: Vec<std::result::Result<Bytes, String>> = vec![Ok(Bytes::from(vec![
        b'x';
        crate::sse::DEFAULT_BUFFER_LIMIT
            + 1
    ]))];
    let events = collect(events_stream(
        Box::pin(futures::stream::iter(flood)),
        Echo::default(),
        CancelToken::new(),
        None,
    ));
    assert!(
        events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
        "{events:?}"
    );
    assert!(events.last().is_some_and(ChatEvent::is_terminal));
}
