//! Each budget of a generation, exceeded by a sequence of frames none of
//! which, alone, comes close to the bound of an SSE frame.

use serde_json::json;

use super::*;
use crate::budget::{
    MAX_CONTENT_BLOCKS, MAX_GENERATION_BYTES, MAX_TOOL_ARGUMENTS_BYTES, MAX_TOOL_CALLS,
    MAX_TOOL_NAME_BYTES,
};

fn open_block(index: usize, block: &serde_json::Value) -> String {
    json!({ "type": "content_block_start", "index": index, "content_block": block }).to_string()
}

fn fragment(index: usize, delta: &serde_json::Value) -> String {
    json!({ "type": "content_block_delta", "index": index, "delta": delta }).to_string()
}

fn play(frames: impl IntoIterator<Item = String>) -> (Vec<ChatEvent>, bool) {
    let mut decoder = MessageDecoder::new();
    let mut outputs = Vec::new();
    for frame in frames {
        decoder.on_data(&frame, &mut outputs);
        if decoder.finished() {
            break;
        }
    }
    (outputs, decoder.finished())
}

/// An error that names the limit, a cut and not an end, no tool call
/// proposed.
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
    let mut frames = vec![open_block(0, &json!({ "type": "text", "text": "" }))];
    frames.extend((0..9).map(|_| fragment(0, &json!({ "type": "text_delta", "text": chunk }))));
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_GENERATION_BYTES);
}

#[test]
fn reasoning_past_the_generation_budget_stops_the_stream() {
    let chunk = "t".repeat(1024 * 1024);
    let mut frames = vec![open_block(
        0,
        &json!({ "type": "thinking", "thinking": "" }),
    )];
    frames.extend(
        (0..9).map(|_| fragment(0, &json!({ "type": "thinking_delta", "thinking": chunk }))),
    );
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_GENERATION_BYTES);
}

#[test]
fn arguments_past_their_budget_stop_the_call_before_its_close() {
    let chunk = "a".repeat(300 * 1024);
    let mut frames = vec![open_block(
        0,
        &json!({ "type": "tool_use", "id": "toolu_1", "name": "execute_query", "input": {} }),
    )];
    frames.extend((0..4).map(|_| {
        fragment(
            0,
            &json!({ "type": "input_json_delta", "partial_json": chunk }),
        )
    }));
    // The closing arrives too late: the call is already thrown away.
    frames.push(json!({ "type": "content_block_stop", "index": 0 }).to_string());
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_TOOL_ARGUMENTS_BYTES);
}

#[test]
fn a_tool_name_past_its_budget_is_refused_at_the_opening() {
    let long_name = "n".repeat(MAX_TOOL_NAME_BYTES + 1);
    let (outputs, finished) = play([open_block(
        0,
        &json!({ "type": "tool_use", "id": "toolu_1", "name": long_name, "input": {} }),
    )]);
    stopped(&outputs, finished, MAX_TOOL_NAME_BYTES);
}

#[test]
fn too_many_tool_calls_stop_the_stream() {
    let frames = (0..=MAX_TOOL_CALLS).map(|index| {
        open_block(
            index,
            &json!({ "type": "tool_use", "id": format!("t{index}"), "name": "execute_query", "input": {} }),
        )
    });
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_TOOL_CALLS);
}

#[test]
fn blocks_opened_and_never_closed_stop_the_stream() {
    let frames = (0..=MAX_CONTENT_BLOCKS)
        .map(|index| open_block(index, &json!({ "type": "text", "text": "" })));
    let (outputs, finished) = play(frames);
    stopped(&outputs, finished, MAX_CONTENT_BLOCKS);
}
