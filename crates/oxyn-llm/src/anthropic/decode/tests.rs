use super::*;

#[test]
fn anthropic_stop_reasons_are_translated() {
    for (raw, expected) in [
        ("end_turn", StopReason::EndTurn),
        ("max_tokens", StopReason::MaxTokens),
        ("stop_sequence", StopReason::StopSequence),
        ("tool_use", StopReason::ToolCalls),
        ("pause_turn", StopReason::Paused),
        ("refusal", StopReason::Refusal),
        (
            "model_context_window_exceeded",
            StopReason::ContextWindowExceeded,
        ),
    ] {
        assert_eq!(stop_reason(raw), expected, "{raw}");
    }
    assert_eq!(
        stop_reason("future_reason"),
        StopReason::Other("future_reason".to_owned()),
        "the documentation says this list may grow"
    );
}

/// Plays a sequence of `data` fields and returns all the events produced.
fn play(frames: &[&str]) -> Vec<ChatEvent> {
    let mut decoder = MessageDecoder::new();
    let mut outputs = Vec::new();
    for frame in frames {
        decoder.on_data(frame, &mut outputs);
    }
    if !decoder.finished() {
        decoder.finish(&mut outputs);
    }
    outputs
}

fn texts(events: &[ChatEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

fn reasoning(events: &[ChatEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ReasoningDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn calls(events: &[ChatEvent]) -> Vec<&ToolCall> {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ToolCallComplete(call) => Some(call),
            _ => None,
        })
        .collect()
}

fn blocks(events: &[ChatEvent]) -> Vec<&ReasoningBlock> {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ReasoningComplete { block, .. } => Some(block),
            _ => None,
        })
        .collect()
}

#[test]
fn a_text_stream_is_reassembled_in_order() {
    let events = play(&[
        r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":25,"output_tokens":1}}}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        r#"{"type":"ping"}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"SELECT "}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"1"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":15}}"#,
        r#"{"type":"message_stop"}"#,
    ]);

    assert_eq!(texts(&events), "SELECT 1");
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::EndTurn
        })
    );
}

#[test]
fn a_keep_alive_produces_nothing() {
    let events = play(&[
        r#"{"type":"ping"}"#,
        r#"{"type":"ping"}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    assert_eq!(events.len(), 1, "seul `Done` doit rester : {events:?}");
    assert!(events[0].is_terminal());
}

#[test]
fn done_is_emitted_exactly_once() {
    let mut decoder = MessageDecoder::new();
    let mut outputs = Vec::new();
    decoder.on_data(
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        &mut outputs,
    );
    decoder.on_data(r#"{"type":"message_stop"}"#, &mut outputs);
    // The server closes after `message_stop`: `finish` must add nothing.
    decoder.finish(&mut outputs);
    assert_eq!(outputs.iter().filter(|e| e.is_terminal()).count(), 1);
}

#[test]
fn cache_usage_is_reported_both_ways() {
    let events = play(&[
        r#"{"type":"message_start","message":{"usage":{"input_tokens":50,"output_tokens":1,"cache_creation_input_tokens":148,"cache_read_input_tokens":2000}}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    assert!(
        events.contains(&ChatEvent::Usage {
            prompt_tokens: 50,
            completion_tokens: 1,
            cache_write_tokens: Some(148),
            cache_read_tokens: Some(2000),
            reasoning_tokens: None,
        }),
        "{events:?}"
    );
}

#[test]
fn an_undeclared_usage_is_not_invented() {
    let events = play(&[
        r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    let Some(ChatEvent::Usage {
        cache_write_tokens,
        cache_read_tokens,
        ..
    }) = events.iter().find(|e| matches!(e, ChatEvent::Usage { .. }))
    else {
        panic!("no usage: {events:?}");
    };
    assert_eq!(*cache_write_tokens, None);
    assert_eq!(*cache_read_tokens, None);
}

#[test]
fn a_fragmented_tool_call_is_reassembled() {
    let events = play(&[
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"execute_query","input":{}}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"sql\":"}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"SELECT 1\"}"}}"#,
        r#"{"type":"content_block_stop","index":1}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        r#"{"type":"message_stop"}"#,
    ]);

    let complete = calls(&events);
    assert_eq!(complete.len(), 1, "{events:?}");
    assert_eq!(complete[0].id, "toolu_01");
    assert_eq!(complete[0].name, "execute_query");
    assert_eq!(complete[0].arguments["sql"], "SELECT 1");

    assert!(events.contains(&ChatEvent::ToolCallStarted {
        index: 1,
        id: "toolu_01".to_owned(),
        name: "execute_query".to_owned(),
    }));
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::ToolCalls
        })
    );
}

#[test]
fn two_tool_blocks_do_not_mix() {
    // This protocol closes a block before opening another, but nothing
    // forces it to: the state is kept by index, not by order of arrival.
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"lire"}}"#,
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"b","name":"ecrire"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"t\":1}"}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"u\":2}"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"content_block_stop","index":1}"#,
        r#"{"type":"message_stop"}"#,
    ]);

    let complete = calls(&events);
    assert_eq!(complete.len(), 2, "{events:?}");
    assert_eq!(complete[0].arguments, serde_json::json!({"t": 1}));
    assert_eq!(complete[1].arguments, serde_json::json!({"u": 2}));
}

#[test]
fn a_tool_without_arguments_receives_an_empty_object() {
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"lister","input":{}}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    assert_eq!(calls(&events)[0].arguments, serde_json::json!({}));
}

#[test]
fn invalid_arguments_are_an_invalid_call_and_the_sibling_stands() {
    // The fine-grained stream is not validated by the server: the
    // accumulated string may not be JSON. It costs this call, not the
    // turn: the valid call next to it is still proposed.
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"execute"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\": \"SELECT secret"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"d","name":"lister","input":{}}}"#,
        r#"{"type":"content_block_stop","index":1}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    let complete = calls(&events);
    assert_eq!(complete.len(), 1, "{events:?}");
    assert_eq!(complete[0].id, "d");
    let invalid = events
        .iter()
        .find_map(|e| match e {
            ChatEvent::ToolCallInvalid { id, name, detail } => Some((id, name, detail)),
            _ => None,
        })
        .expect("the invalid call is reported as such");
    assert_eq!((invalid.0.as_str(), invalid.1.as_str()), ("c", "execute"));
    assert!(
        !events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
        "not a stream error: {events:?}"
    );
    assert!(
        !invalid.2.contains("secret"),
        "the model output must not be copied: {}",
        invalid.2
    );
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::ToolCalls
        })
    );
}

#[test]
fn a_reasoning_block_is_reassembled_with_its_signature() {
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"I set 1071 = 2 × 462 + 147"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":", then 462 = 3 × 147 + 21"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCgIYAhIM"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_stop"}"#,
    ]);

    assert_eq!(
        reasoning(&events),
        "I set 1071 = 2 × 462 + 147, then 462 = 3 × 147 + 21"
    );
    let blocks = blocks(&events);
    assert_eq!(blocks.len(), 1, "{events:?}");
    assert_eq!(
        blocks[0],
        &ReasoningBlock::Summarized {
            text: "I set 1071 = 2 × 462 + 147, then 462 = 3 × 147 + 21".to_owned(),
            signature: Some("EqQBCgIYAhIM".to_owned()),
        }
    );
}

#[test]
fn a_masked_reasoning_remains_transportable() {
    // The default of several models: the block arrives without text, but
    // with its signature. It must still be rendered, otherwise the next
    // turn is refused.
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EosnCkYICxIM"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_stop"}"#,
    ]);

    assert_eq!(reasoning(&events), "", "nothing to display");
    let blocks = blocks(&events);
    assert_eq!(blocks.len(), 1, "the block must exist all the same");
    assert_eq!(blocks[0].display_text(), None);
}

#[test]
fn an_encrypted_reasoning_produces_no_text() {
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"EvwBCoYBGAIiQL"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_stop"}"#,
    ]);

    assert_eq!(reasoning(&events), "");
    let blocks = blocks(&events);
    assert_eq!(blocks.len(), 1, "{events:?}");
    assert!(blocks[0].is_redacted());
    assert_eq!(blocks[0].display_text(), None);
}

#[test]
fn a_cancellation_in_the_middle_of_a_reasoning_returns_no_incomplete_block() {
    // A block without its signature would be refused at the next turn:
    // rendering it would make the following request fail, far from here.
    let mut decoder = MessageDecoder::new();
    let mut outputs = Vec::new();
    decoder.on_data(
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
        &mut outputs,
    );
    decoder.on_data(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"I start with"}}"#,
        &mut outputs,
    );
    decoder.cancel(&mut outputs);

    assert!(blocks(&outputs).is_empty(), "{outputs:?}");
    assert_eq!(
        outputs.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Cancelled
        })
    );
}

#[test]
fn a_cancellation_in_the_middle_of_a_tool_call_proposes_nothing() {
    let mut decoder = MessageDecoder::new();
    let mut outputs = Vec::new();
    decoder.on_data(
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"drop_table"}}"#,
        &mut outputs,
    );
    decoder.on_data(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"name\":"}}"#,
        &mut outputs,
    );
    decoder.cancel(&mut outputs);

    assert!(
        calls(&outputs).is_empty(),
        "a truncated call never becomes a proposed action: {outputs:?}"
    );
    assert_eq!(
        outputs.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Cancelled
        })
    );
}

#[test]
fn a_refusal_is_distinct_from_an_end_of_turn() {
    let events = play(&[
        r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Refusal
        })
    );
}

#[test]
fn a_turn_pause_reports_itself_as_incomplete() {
    let events = play(&[
        r#"{"type":"message_delta","delta":{"stop_reason":"pause_turn"}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    let Some(ChatEvent::Done { stop_reason }) = events.last() else {
        panic!("{events:?}");
    };
    assert_eq!(*stop_reason, StopReason::Paused);
    assert!(stop_reason.is_truncated());
}

#[test]
fn an_error_in_the_stream_is_terminal() {
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
        r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
    ]);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("Overloaded"))),
        "{events:?}"
    );
    assert_eq!(
        events.iter().filter(|e| e.is_terminal()).count(),
        1,
        "{events:?}"
    );
}

#[test]
fn a_stream_closed_cleanly_without_message_stop_is_interrupted() {
    // A load balancer that cuts at its duration limit closes the
    // connection cleanly: no transport error, only a stream that stops.
    // Presenting this text as complete would be lying.
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
    ]);
    assert_eq!(texts(&events), "a");
    let Some(ChatEvent::Done { stop_reason }) = events.last() else {
        panic!("the stream must end: {events:?}");
    };
    assert_eq!(*stop_reason, StopReason::Interrupted);
    assert!(stop_reason.is_truncated() && stop_reason.is_ambiguous());
    assert!(
        events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
        "the cut must show: {events:?}"
    );
}

#[test]
fn an_announced_reason_without_message_stop_remains_a_cut() {
    // `message_delta` announces the end of the generation, not that of the
    // message: what followed was not received.
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
    ]);
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Interrupted
        })
    );
}

#[test]
fn a_readable_but_unclosed_tool_call_is_never_proposed() {
    // The trap: the JSON received so far happens to be complete. The model
    // may not have finished — `{"sql":"DELETE FROM t"}` can be the start
    // of `{"sql":"DELETE FROM t", "where": …}`.
    let events = play(&[
        r#"{"type":"message_start","message":{"usage":{"input_tokens":10}}}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"execute","input":{}}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\":\"DELETE FROM t\"}"}}"#,
    ]);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("tool call #0"))),
        "the call vanishing must show: {events:?}"
    );
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Interrupted
        })
    );
}

#[test]
fn a_block_not_closed_before_message_stop_is_thrown_away() {
    // Server inconsistency: the message says it is finished, the block is not.
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"execute","input":{}}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
        "{events:?}"
    );
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::ToolCalls
        })
    );
}

#[test]
fn an_unknown_event_does_not_break_the_stream() {
    // The documentation announces that new types can appear.
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
        r#"{"type":"a_future_event","payload":{"what":"ever"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    assert_eq!(texts(&events), "a");
    assert!(
        !events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
        "an unknown type is not an error: {events:?}"
    );
}

#[test]
fn a_block_of_unknown_type_produces_nothing_but_absorbs_its_deltas() {
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"x\"}"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    assert!(calls(&events).is_empty(), "{events:?}");
    assert!(
        !events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
        "{events:?}"
    );
}

#[test]
fn a_delta_without_an_open_block_is_ignored() {
    let events = play(&[
        r#"{"type":"content_block_delta","index":7,"delta":{"type":"text_delta","text":"fantome"}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    assert_eq!(texts(&events), "", "{events:?}");
}

#[test]
fn an_isolated_unreadable_frame_does_not_kill_the_stream() {
    let events = play(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
        "{this is not json",
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"b"}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    assert_eq!(texts(&events), "b");
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Error(_))));
}

#[test]
fn a_series_of_unreadable_frames_gives_up_the_stream() {
    let mut decoder = MessageDecoder::new();
    let mut outputs = Vec::new();
    for _ in 0..MAX_DECODE_ERRORS {
        decoder.on_data("not json", &mut outputs);
    }
    assert!(decoder.finished(), "{outputs:?}");
    assert!(outputs.last().is_some_and(ChatEvent::is_terminal));
}

/// A real SSE trace, as the documentation publishes it: event names,
/// interleaved `ping`, blocks opened and closed.
const TRACE: &str = concat!(
    "event: message_start\n",
    r#"data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-model","stop_reason":null,"usage":{"input_tokens":472,"output_tokens":2}}}"#,
    "\n\n",
    "event: content_block_start\n",
    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    "\n\n",
    "event: ping\n",
    r#"data: {"type": "ping"}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I check the café"}}"#,
    "\n\n",
    "event: content_block_stop\n",
    r#"data: {"type":"content_block_stop","index":0}"#,
    "\n\n",
    "event: content_block_start\n",
    r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"get_weather","input":{}}}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"ville\":"}}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"Besançon\"}"}}"#,
    "\n\n",
    "event: content_block_stop\n",
    r#"data: {"type":"content_block_stop","index":1}"#,
    "\n\n",
    "event: message_delta\n",
    r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":89}}"#,
    "\n\n",
    "event: message_stop\n",
    r#"data: {"type":"message_stop"}"#,
    "\n\n",
);

/// Replays a trace through the real SSE path, with a given split.
fn replay(chunks: Vec<bytes::Bytes>) -> Vec<ChatEvent> {
    use futures::stream::StreamExt as _;

    let bytes = chunks
        .into_iter()
        .map(Ok::<bytes::Bytes, String>)
        .collect::<Vec<_>>();
    let flux = crate::stream::events_stream(
        Box::pin(futures::stream::iter(bytes)),
        MessageDecoder::new(),
        oxyn_core::CancelToken::new(),
        None,
    );
    futures::executor::block_on(flux.collect())
}

#[test]
fn a_real_trace_decodes_in_one_block() {
    let events = replay(vec![bytes::Bytes::from_static(TRACE.as_bytes())]);

    assert_eq!(texts(&events), "I check the café");
    let complete = calls(&events);
    assert_eq!(complete.len(), 1, "{events:?}");
    assert_eq!(complete[0].id, "toolu_01");
    assert_eq!(complete[0].arguments["ville"], "Besançon");
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::ToolCalls
        })
    );
}

#[test]
fn the_same_trace_split_at_every_byte_gives_the_same_result() {
    // The network does not respect frame boundaries, and the trace
    // contains multi-byte characters ("é", "ç"): the only split that
    // covers every case is the one that respects none.
    let byte_by_byte: Vec<bytes::Bytes> = TRACE
        .as_bytes()
        .iter()
        .map(|byte| bytes::Bytes::copy_from_slice(&[*byte]))
        .collect();

    assert_eq!(
        replay(byte_by_byte),
        replay(vec![bytes::Bytes::from_static(TRACE.as_bytes())]),
        "the network split must change nothing"
    );
}

#[test]
fn a_stream_cut_mid_turn_is_truncated_and_ambiguous() {
    // The case that costs money, and that shows neither at compile time
    // nor in review: the connection drops while the model has started to
    // answer. The server may have finished — and billed — the turn.
    // Replaying would pay twice (I-13).
    let cut: Vec<std::result::Result<bytes::Bytes, String>> = vec![
        Ok(bytes::Bytes::from_static(
            concat!(
                "event: content_block_start\n",
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
                "\n\n",
                "event: content_block_delta\n",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"the customers table "}}"#,
                "\n\n",
            )
            .as_bytes(),
        )),
        // Neither `message_delta` nor `message_stop`: the transport gives out.
        Err("lost the connection while receiving the stream".to_owned()),
    ];

    let events = {
        use futures::stream::StreamExt as _;
        let flux = crate::stream::events_stream(
            Box::pin(futures::stream::iter(cut)),
            MessageDecoder::new(),
            oxyn_core::CancelToken::new(),
            None,
        );
        futures::executor::block_on(flux.collect::<Vec<_>>())
    };

    // What was received stays shown: the user sees where it stopped rather
    // than losing the beginning.
    assert_eq!(texts(&events), "the customers table ");

    let Some(ChatEvent::Done { stop_reason }) = events.last() else {
        panic!("the stream must end: {events:?}");
    };
    assert_eq!(*stop_reason, StopReason::Interrupted);
    assert!(
        stop_reason.is_truncated(),
        "a response cut in the middle that claims to be complete is a false response"
    );
    assert!(
        stop_reason.is_ambiguous(),
        "the server may have produced the rest: replaying would pay twice (I-13)"
    );
    assert_eq!(
        events.iter().filter(|e| e.is_terminal()).count(),
        1,
        "{events:?}"
    );
}

#[test]
fn a_tool_call_cut_by_the_transport_is_never_proposed() {
    // The cut falls in the middle of the arguments. Proposing them would
    // amount to submitting an action whose reach no one knows.
    let cut: Vec<std::result::Result<bytes::Bytes, String>> = vec![
        Ok(bytes::Bytes::from_static(
            concat!(
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"execute_query"}}"#,
                "\n\n",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\": \"DELETE FROM audit"}}"#,
                "\n\n",
            )
            .as_bytes(),
        )),
        Err("lost the connection while receiving the stream".to_owned()),
    ];

    let events = {
        use futures::stream::StreamExt as _;
        let flux = crate::stream::events_stream(
            Box::pin(futures::stream::iter(cut)),
            MessageDecoder::new(),
            oxyn_core::CancelToken::new(),
            None,
        );
        futures::executor::block_on(flux.collect::<Vec<_>>())
    };

    assert!(
        calls(&events).is_empty(),
        "truncated arguments are not arguments: {events:?}"
    );
    // And the partial SQL must not come out in an error message: what a
    // model writes can copy what it was given.
    for event in &events {
        if let ChatEvent::Error(message) = event {
            assert!(!message.contains("DELETE FROM"), "{message}");
        }
    }
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Interrupted
        })
    );
}

#[test]
fn nothing_is_decoded_after_the_end() {
    let mut decoder = MessageDecoder::new();
    let mut outputs = Vec::new();
    decoder.on_data(r#"{"type":"message_stop"}"#, &mut outputs);
    let after = outputs.len();
    decoder.on_data(
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
        &mut outputs,
    );
    assert_eq!(outputs.len(), after, "{outputs:?}");
}
