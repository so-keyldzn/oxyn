use super::*;

#[test]
fn openai_stop_reasons_are_translated() {
    assert_eq!(stop_reason("stop"), StopReason::EndTurn);
    assert_eq!(stop_reason("length"), StopReason::MaxTokens);
    assert_eq!(stop_reason("tool_calls"), StopReason::ToolCalls);
    assert_eq!(stop_reason("function_call"), StopReason::ToolCalls);
    assert_eq!(stop_reason("content_filter"), StopReason::ContentFilter);
    assert_eq!(
        stop_reason("guardrail_intervened"),
        StopReason::Other("guardrail_intervened".to_owned()),
        "an unknown reason is kept, it is not folded into EndTurn"
    );
}

/// Plays a sequence of `data` fields and returns all the events produced.
fn play(frames: &[&str]) -> Vec<ChatEvent> {
    let mut decoder = ChunkDecoder::new();
    let mut outputs = Vec::new();
    for frame in frames {
        decoder.on_data(frame, &mut outputs);
    }
    if !decoder.is_done() {
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

fn calls(events: &[ChatEvent]) -> Vec<&crate::types::ToolCall> {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ToolCallComplete(call) => Some(call),
            _ => None,
        })
        .collect()
}

#[test]
fn a_text_stream_is_reassembled_in_order() {
    let events = play(&[
        r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
        r#"{"choices":[{"delta":{"content":"SELECT "}}]}"#,
        r#"{"choices":[{"delta":{"content":"1"}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        DONE_SENTINEL,
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
fn done_is_emitted_exactly_once() {
    let mut decoder = ChunkDecoder::new();
    let mut outputs = Vec::new();
    decoder.on_data(
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        &mut outputs,
    );
    decoder.on_data(DONE_SENTINEL, &mut outputs);
    // The server closes after the sentinel: `finish` must add nothing.
    decoder.finish(&mut outputs);
    let ends = outputs.iter().filter(|e| e.is_terminal()).count();
    assert_eq!(ends, 1, "{outputs:?}");
}

#[test]
fn usage_arriving_after_the_end_of_generation_is_kept() {
    // The trap: emitting `Done` on `finish_reason` would lose this frame.
    let events = play(&[
        r#"{"choices":[{"delta":{"content":"ok"}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":120,"completion_tokens":8}}"#,
        DONE_SENTINEL,
    ]);
    assert!(
        events.contains(&ChatEvent::Usage {
            prompt_tokens: 120,
            completion_tokens: 8,
            cache_write_tokens: None,
            cache_read_tokens: None,
            reasoning_tokens: None,
        }),
        "{events:?}"
    );
    assert!(
        events.last().is_some_and(ChatEvent::is_terminal),
        "Done must remain the last event"
    );
}

#[test]
fn a_fragmented_tool_call_is_reassembled() {
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"execute_query","arguments":""}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"sql\":"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"SELECT 1\"}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        DONE_SENTINEL,
    ]);

    let complete = calls(&events);
    assert_eq!(complete.len(), 1, "{events:?}");
    assert_eq!(complete[0].id, "call_abc");
    assert_eq!(complete[0].name, "execute_query");
    assert_eq!(complete[0].arguments["sql"], "SELECT 1");

    assert!(events.contains(&ChatEvent::ToolCallStarted {
        index: 0,
        id: "call_abc".to_owned(),
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
fn two_interleaved_calls_do_not_mix() {
    // The defect this module exists to avoid: concatenating in order of
    // arrival would produce a single incoherent JSON.
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"lire"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"ecrire"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"t\":"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"u\":"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"2}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);

    let complete = calls(&events);
    assert_eq!(complete.len(), 2, "{events:?}");
    assert_eq!(complete[0].name, "lire");
    assert_eq!(complete[0].arguments, serde_json::json!({"t": 1}));
    assert_eq!(complete[1].name, "ecrire");
    assert_eq!(complete[1].arguments, serde_json::json!({"u": 2}));
}

#[test]
fn calls_without_index_are_told_apart_by_their_identifier() {
    // A gateway that omits `index`: defaulting it to 0 glued the names
    // into `lireecrire` and the arguments into `{"t":1}{"u":2}`.
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"id":"a","function":{"name":"lire","arguments":"{\"t\":"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"1}"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"id":"b","function":{"name":"ecrire","arguments":"{\"u\":"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"2}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);

    let complete = calls(&events);
    assert_eq!(complete.len(), 2, "{events:?}");
    assert_eq!(
        (complete[0].id.as_str(), complete[0].name.as_str()),
        ("a", "lire")
    );
    assert_eq!(complete[0].arguments, serde_json::json!({"t": 1}));
    assert_eq!(
        (complete[1].id.as_str(), complete[1].name.as_str()),
        ("b", "ecrire")
    );
    assert_eq!(complete[1].arguments, serde_json::json!({"u": 2}));
    assert!(events.contains(&ChatEvent::ToolCallStarted {
        index: 1,
        id: "b".to_owned(),
        name: "ecrire".to_owned(),
    }));
}

#[test]
fn a_reused_index_with_another_identifier_is_another_call() {
    // A gateway that numbers every call 0: the second `id` used to be
    // ignored, and its call merged into the first.
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"lire","arguments":"{}"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"b","function":{"name":"ecrire","arguments":"{\"u\":"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"2}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        DONE_SENTINEL,
    ]);

    let complete = calls(&events);
    assert_eq!(complete.len(), 2, "{events:?}");
    assert_eq!(complete[0].id, "a");
    assert_eq!(complete[0].name, "lire");
    assert_eq!(complete[0].arguments, serde_json::json!({}));
    assert_eq!(complete[1].id, "b");
    assert_eq!(complete[1].name, "ecrire");
    assert_eq!(complete[1].arguments, serde_json::json!({"u": 2}));
}

#[test]
fn an_identifier_repeated_on_every_fragment_stays_one_call() {
    // Some servers repeat the `id` on each fragment: it is the same call.
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"lire","arguments":"{\"t\":"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"arguments":"1}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);
    let complete = calls(&events);
    assert_eq!(complete.len(), 1, "{events:?}");
    assert_eq!(complete[0].arguments, serde_json::json!({"t": 1}));
}

#[test]
fn a_call_without_identifier_receives_one() {
    // Ollama and llama.cpp omit `id`.
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"ping","arguments":"{}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);
    let complete = calls(&events);
    assert_eq!(complete.len(), 1);
    assert!(
        !complete[0].id.is_empty(),
        "without an identifier, the tool result cannot be attached"
    );
}

#[test]
fn a_tool_without_arguments_receives_an_empty_object() {
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"lister"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);
    assert_eq!(calls(&events)[0].arguments, serde_json::json!({}));
}

#[test]
fn invalid_arguments_are_an_invalid_call_and_the_sibling_stands() {
    // A model sometimes produces broken JSON. Emitting a call with null
    // arguments would be a silent lie; failing the turn would throw away
    // the valid call next to it.
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"execute","arguments":"{\"sql\": "}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"d","function":{"name":"lister","arguments":"{}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        DONE_SENTINEL,
    ]);
    let complete = calls(&events);
    assert_eq!(complete.len(), 1, "{events:?}");
    assert_eq!(complete[0].id, "d");
    assert!(
        events.iter().any(|e| matches!(
            e,
            ChatEvent::ToolCallInvalid { id, name, detail }
                if id == "c" && name == "execute" && detail.contains("truncated JSON")
        )),
        "{events:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
        "not a stream error: {events:?}"
    );
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::ToolCalls
        })
    );
}

#[test]
fn a_stream_closed_after_finish_reason_without_sentinel_is_complete() {
    // The generation was announced finished: the absence of `[DONE]` only
    // costs the usage frame.
    let events = play(&[
        r#"{"choices":[{"delta":{"content":"a"}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
    ]);
    assert_eq!(texts(&events), "a");
    assert!(
        !events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
        "{events:?}"
    );
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::EndTurn
        })
    );
}

#[test]
fn a_stream_closed_cleanly_without_any_announcement_is_interrupted() {
    // Neither `finish_reason` nor `[DONE]`: a proxy closed in the middle of
    // the generation. Nothing failed on the transport side, and yet the
    // response is cut.
    let events = play(&[r#"{"choices":[{"delta":{"content":"a"}}]}"#]);
    let Some(ChatEvent::Done { stop_reason }) = events.last() else {
        panic!("the stream must end: {events:?}");
    };
    assert_eq!(*stop_reason, StopReason::Interrupted);
    assert!(stop_reason.is_ambiguous());
    assert!(
        events.iter().any(|e| matches!(e, ChatEvent::Error(_))),
        "the cut must show: {events:?}"
    );
}

#[test]
fn a_readable_tool_call_without_end_announcement_is_never_proposed() {
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"execute","arguments":""}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"sql\":\"DELETE FROM t\"}"}}]}}]}"#,
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
        "{events:?}"
    );
    assert_eq!(
        events.last(),
        Some(&ChatEvent::Done {
            stop_reason: StopReason::Interrupted
        })
    );
}

#[test]
fn an_error_in_the_stream_throws_away_the_calls_in_progress() {
    let events = play(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"execute","arguments":"{}"}}]}}]}"#,
        r#"{"error":{"message":"overloaded"}}"#,
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
            stop_reason: StopReason::ProviderError
        })
    );
}

#[test]
fn an_isolated_unreadable_frame_does_not_kill_the_stream() {
    let events = play(&[
        r#"{"choices":[{"delta":{"content":"a"}}]}"#,
        "{this is not json",
        r#"{"choices":[{"delta":{"content":"b"}}]}"#,
        DONE_SENTINEL,
    ]);
    assert_eq!(texts(&events), "ab");
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Error(_))));
}

#[test]
fn a_series_of_unreadable_frames_gives_up_the_stream() {
    let mut decoder = ChunkDecoder::new();
    let mut outputs = Vec::new();
    for _ in 0..MAX_DECODE_ERRORS {
        decoder.on_data("not json", &mut outputs);
    }
    assert!(decoder.is_done(), "{outputs:?}");
    assert!(outputs.last().is_some_and(ChatEvent::is_terminal));
}

#[test]
fn an_error_carried_in_the_stream_is_terminal() {
    let events = play(&[
        r#"{"choices":[{"delta":{"content":"a"}}]}"#,
        r#"{"error":{"message":"upstream refused","code":502}}"#,
    ]);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("upstream refused"))),
        "{events:?}"
    );
    assert_eq!(
        events.iter().filter(|e| e.is_terminal()).count(),
        1,
        "{events:?}"
    );
}

#[test]
fn a_cancellation_invents_no_half_described_call() {
    let mut decoder = ChunkDecoder::new();
    let mut outputs = Vec::new();
    decoder.on_data(
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"drop_table","arguments":"{\"name\":"}}]}}]}"#,
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
fn a_transport_break_ends_the_stream() {
    let mut decoder = ChunkDecoder::new();
    let mut outputs = Vec::new();
    decoder.on_data(r#"{"choices":[{"delta":{"content":"a"}}]}"#, &mut outputs);
    decoder.transport_error("connection reset".to_owned(), &mut outputs);
    assert!(decoder.is_done());
    assert!(outputs.last().is_some_and(ChatEvent::is_terminal));
}

#[test]
fn nothing_is_decoded_after_the_end() {
    let mut decoder = ChunkDecoder::new();
    let mut outputs = Vec::new();
    decoder.on_data(DONE_SENTINEL, &mut outputs);
    let after = outputs.len();
    decoder.on_data(
        r#"{"choices":[{"delta":{"content":"fantome"}}]}"#,
        &mut outputs,
    );
    assert_eq!(outputs.len(), after, "{outputs:?}");
}
