//! Chaque budget d'une génération, dépassé par une suite de trames dont
//! aucune, seule, n'approche de la borne d'une trame SSE.

use serde_json::json;

use super::*;
use crate::budget::{
    MAX_CONTENT_BLOCKS, MAX_GENERATION_BYTES, MAX_TOOL_ARGUMENTS_BYTES, MAX_TOOL_CALLS,
    MAX_TOOL_NAME_BYTES,
};

fn ouvre(index: usize, bloc: &serde_json::Value) -> String {
    json!({ "type": "content_block_start", "index": index, "content_block": bloc }).to_string()
}

fn fragment(index: usize, delta: &serde_json::Value) -> String {
    json!({ "type": "content_block_delta", "index": index, "delta": delta }).to_string()
}

fn jouer(trames: impl IntoIterator<Item = String>) -> (Vec<ChatEvent>, bool) {
    let mut decodeur = MessageDecoder::new();
    let mut sorties = Vec::new();
    for trame in trames {
        decodeur.on_data(&trame, &mut sorties);
        if decodeur.finished() {
            break;
        }
    }
    (sorties, decodeur.finished())
}

/// Une erreur qui nomme la limite, une coupure et non une fin, aucun appel
/// d'outil proposé.
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
    let mut trames = vec![ouvre(0, &json!({ "type": "text", "text": "" }))];
    trames.extend((0..9).map(|_| fragment(0, &json!({ "type": "text_delta", "text": morceau }))));
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_GENERATION_BYTES);
}

#[test]
fn reasoning_past_the_generation_budget_stops_the_stream() {
    let morceau = "t".repeat(1024 * 1024);
    let mut trames = vec![ouvre(0, &json!({ "type": "thinking", "thinking": "" }))];
    trames.extend(
        (0..9).map(|_| fragment(0, &json!({ "type": "thinking_delta", "thinking": morceau }))),
    );
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_GENERATION_BYTES);
}

#[test]
fn arguments_past_their_budget_stop_the_call_before_its_close() {
    let morceau = "a".repeat(300 * 1024);
    let mut trames = vec![ouvre(
        0,
        &json!({ "type": "tool_use", "id": "toolu_1", "name": "execute_query", "input": {} }),
    )];
    trames.extend((0..4).map(|_| {
        fragment(
            0,
            &json!({ "type": "input_json_delta", "partial_json": morceau }),
        )
    }));
    // La fermeture arrive trop tard : l'appel est déjà jeté.
    trames.push(json!({ "type": "content_block_stop", "index": 0 }).to_string());
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_TOOL_ARGUMENTS_BYTES);
}

#[test]
fn a_tool_name_past_its_budget_is_refused_at_the_opening() {
    let nom = "n".repeat(MAX_TOOL_NAME_BYTES + 1);
    let (sorties, termine) = jouer([ouvre(
        0,
        &json!({ "type": "tool_use", "id": "toolu_1", "name": nom, "input": {} }),
    )]);
    arrete(&sorties, termine, MAX_TOOL_NAME_BYTES);
}

#[test]
fn too_many_tool_calls_stop_the_stream() {
    let trames = (0..=MAX_TOOL_CALLS).map(|index| {
        ouvre(
            index,
            &json!({ "type": "tool_use", "id": format!("t{index}"), "name": "execute_query", "input": {} }),
        )
    });
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_TOOL_CALLS);
}

#[test]
fn blocks_opened_and_never_closed_stop_the_stream() {
    let trames =
        (0..=MAX_CONTENT_BLOCKS).map(|index| ouvre(index, &json!({ "type": "text", "text": "" })));
    let (sorties, termine) = jouer(trames);
    arrete(&sorties, termine, MAX_CONTENT_BLOCKS);
}
