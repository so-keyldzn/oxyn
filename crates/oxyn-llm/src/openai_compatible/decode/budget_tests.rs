//! Each budget of a generation, exceeded by a sequence of frames none of
//! which, alone, comes close to the bound of an SSE frame.

use serde_json::json;

use super::*;
use crate::budget::{
    MAX_GENERATION_BYTES, MAX_TOOL_ARGUMENTS_BYTES, MAX_TOOL_CALLS, MAX_TOOL_NAME_BYTES,
};

/// A `delta` fragment frame, serialized as a server would.
fn trame(delta: &serde_json::Value) -> String {
    json!({ "choices": [{ "delta": delta }] }).to_string()
}

/// Plays the frames one by one and returns everything that came out.
fn jouer(trames: impl IntoIterator<Item = String>) -> (Vec<ChatEvent>, bool) {
    let mut decodeur = ChunkDecoder::new();
    let mut sorties = Vec::new();
    for trame in trames {
        decodeur.on_data(&trame, &mut sorties);
        if decodeur.is_done() {
            break;
        }
    }
    (sorties, decodeur.is_done())
}

/// What every overrun must hold: an error that names the limit, a cut and
/// not an end, and no tool call proposed.
fn arrete(sorties: &[ChatEvent], termine: bool, limite: usize) {
    assert!(termine, "the decoder stops at the first overrun");
    let erreur = sorties
        .iter()
        .find_map(|e| match e {
            ChatEvent::Error(m) if m.contains("stopped reading") => Some(m.as_str()),
            _ => None,
        })
        .expect("the overrun is reported");
    assert!(erreur.contains(&limite.to_string()), "{erreur}");
    assert!(
        !sorties
            .iter()
            .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
        "no partial tool call is proposed"
    );
    assert_eq!(
        sorties.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Interrupted
        })
    );
}

#[test]
fn text_past_the_generation_budget_stops_the_stream() {
    let morceau = "x".repeat(1024 * 1024);
    let trames = (0..9).map(|_| trame(&json!({ "content": morceau })));
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_GENERATION_BYTES);
    let emis: usize = sorties
        .iter()
        .map(|e| match e {
            ChatEvent::TextDelta(t) => t.len(),
            _ => 0,
        })
        .sum();
    assert!(emis <= MAX_GENERATION_BYTES, "{emis}");
}

#[test]
fn refusals_count_in_the_generation_budget() {
    let morceau = "r".repeat(1024 * 1024);
    let trames = (0..9).map(|_| trame(&json!({ "refusal": morceau })));
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_GENERATION_BYTES);
}

#[test]
fn arguments_past_their_budget_stop_the_call_even_with_an_announced_end() {
    let fragment = "a".repeat(300 * 1024);
    let mut trames = vec![trame(&json!({ "tool_calls": [
        { "index": 0, "id": "call_0", "function": { "name": "execute_query" } }
    ] }))];
    trames.extend((0..4).map(|_| {
        trame(&json!({ "tool_calls": [
            { "index": 0, "function": { "arguments": fragment } }
        ] }))
    }));
    // The announced end arrives too late: the call is already thrown away.
    trames.push(json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }).to_string());
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_TOOL_ARGUMENTS_BYTES);
}

#[test]
fn a_name_fragmented_past_its_budget_stops_the_stream_although_nothing_was_emitted() {
    // After the first fragment, the name grows without emitting anything.
    let fragment = "n".repeat(100);
    let trames = (0..4).map(|_| {
        trame(&json!({ "tool_calls": [
            { "index": 0, "function": { "name": fragment } }
        ] }))
    });
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_TOOL_NAME_BYTES);
}

#[test]
fn too_many_tool_calls_stop_the_stream() {
    let trames = (0..=MAX_TOOL_CALLS).map(|index| {
        trame(&json!({ "tool_calls": [ { "index": index, "id": format!("c{index}") } ] }))
    });
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_TOOL_CALLS);
}
