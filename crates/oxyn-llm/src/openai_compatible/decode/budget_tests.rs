//! Each budget of a generation, exceeded by a sequence of frames none of
//! which, alone, comes close to the bound of an SSE frame.

use serde_json::json;

use super::*;
use crate::budget::{
    MAX_GENERATION_BYTES, MAX_TOOL_ARGUMENTS_BYTES, MAX_TOOL_CALLS, MAX_TOOL_NAME_BYTES,
};

/// A `delta` fragment frame, serialized as a server would.
fn frame(delta: &serde_json::Value) -> String {
    json!({ "choices": [{ "delta": delta }] }).to_string()
}

/// Plays the frames one by one and returns everything that came out.
fn play(frames: impl IntoIterator<Item = String>) -> (Vec<ChatEvent>, bool) {
    let mut decoder = ChunkDecoder::new();
    let mut outputs = Vec::new();
    for frame in frames {
        decoder.on_data(&frame, &mut outputs);
        if decoder.is_done() {
            break;
        }
    }
    (outputs, decoder.is_done())
}

/// What every overrun must hold: an error that names the limit, a cut and
/// not an end, and no tool call proposed.
fn stopped(outputs: &[ChatEvent], finished: bool, limit: usize) {
    assert!(finished, "the decoder stops at the first overrun");
    let error = outputs
        .iter()
        .find_map(|e| match e {
            ChatEvent::Error(m) if m.contains("stopped reading") => Some(m.as_str()),
            _ => None,
        })
        .expect("the overrun is reported");
    assert!(error.contains(&limit.to_string()), "{error}");
    assert!(
        !outputs
            .iter()
            .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
        "no partial tool call is proposed"
    );
    assert_eq!(
        outputs.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Interrupted
        })
    );
}

#[test]
fn text_past_the_generation_budget_stops_the_stream() {
    let chunk = "x".repeat(1024 * 1024);
    let frames = (0..9).map(|_| frame(&json!({ "content": chunk })));
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_GENERATION_BYTES);
    let emitted: usize = outputs
        .iter()
        .map(|e| match e {
            ChatEvent::TextDelta(t) => t.len(),
            _ => 0,
        })
        .sum();
    assert!(emitted <= MAX_GENERATION_BYTES, "{emitted}");
}

#[test]
fn refusals_count_in_the_generation_budget() {
    let chunk = "r".repeat(1024 * 1024);
    let frames = (0..9).map(|_| frame(&json!({ "refusal": chunk })));
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_GENERATION_BYTES);
}

#[test]
fn arguments_past_their_budget_stop_the_call_even_with_an_announced_end() {
    let fragment = "a".repeat(300 * 1024);
    let mut frames = vec![frame(&json!({ "tool_calls": [
        { "index": 0, "id": "call_0", "function": { "name": "execute_query" } }
    ] }))];
    frames.extend((0..4).map(|_| {
        frame(&json!({ "tool_calls": [
            { "index": 0, "function": { "arguments": fragment } }
        ] }))
    }));
    // The announced end arrives too late: the call is already thrown away.
    frames.push(json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }).to_string());
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_TOOL_ARGUMENTS_BYTES);
}

#[test]
fn a_name_fragmented_past_its_budget_stops_the_stream_although_nothing_was_emitted() {
    // After the first fragment, the name grows without emitting anything.
    let fragment = "n".repeat(100);
    let frames = (0..4).map(|_| {
        frame(&json!({ "tool_calls": [
            { "index": 0, "function": { "name": fragment } }
        ] }))
    });
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_TOOL_NAME_BYTES);
}

#[test]
fn too_many_tool_calls_stop_the_stream() {
    let frames = (0..=MAX_TOOL_CALLS).map(|index| {
        frame(&json!({ "tool_calls": [ { "index": index, "id": format!("c{index}") } ] }))
    });
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_TOOL_CALLS);
}
