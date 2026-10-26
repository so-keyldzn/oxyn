use super::*;

#[test]
fn a_message_debug_does_not_show_its_content() {
    // The targeted failure: `tracing::debug!("{msg:?}")` writes a row of
    // the customer database into a log file (I-03).
    let msg = ChatMessage::user("client 4711, IBAN FR76 3000 6000 0112 3456 7890 189");
    let rendered = format!("{msg:?}");
    assert!(!rendered.contains("FR76"), "{rendered}");
    assert!(!rendered.contains("4711"), "{rendered}");
    assert!(rendered.contains("masked"), "{rendered}");
    assert!(
        rendered.contains("User"),
        "the role remains useful for diagnosis"
    );
}

#[test]
fn a_request_debug_does_not_show_the_messages() {
    let req = ChatRequest::new(
        "gpt-4o-mini",
        vec![
            ChatMessage::system("you are a SQL assistant"),
            ChatMessage::user("SELECT * FROM patients WHERE hiv_status = true"),
        ],
    )
    .with_tools(vec![ToolSpec::new(
        "execute",
        "run a query",
        serde_json::json!({"type": "object"}),
    )]);

    let rendered = format!("{req:?}");
    assert!(!rendered.contains("hiv_status"), "{rendered}");
    assert!(!rendered.contains("patients"), "{rendered}");
    assert!(
        rendered.contains("gpt-4o-mini"),
        "the model remains visible"
    );
    assert!(rendered.contains("execute"), "tool names remain visible");
}

#[test]
fn a_tool_message_carries_the_call_identifier() {
    let msg = ChatMessage::tool_result("call_42", "3 rows");
    assert_eq!(msg.role, Role::Tool);
    assert_eq!(msg.tool_call_id.as_deref(), Some("call_42"));
}

#[test]
fn an_unknown_capability_neither_promises_nor_hides() {
    let unknown = Support::default();
    assert_eq!(unknown, Support::Unknown);
    assert!(!unknown.is_yes());
    assert!(!unknown.is_no());
    assert!(Support::known(true).is_yes());
    assert!(Support::known(false).is_no());
}

#[test]
fn a_model_record_asserts_nothing_by_default() {
    let info = ModelInfo::new("llama3.2");
    assert_eq!(info.display_name, "llama3.2");
    assert_eq!(info.context_window, None);
    assert_eq!(info.supports_tools, Support::Unknown);
    assert_eq!(info.cost, None);
}

#[test]
fn only_done_ends_a_stream() {
    assert!(
        ChatEvent::Done {
            stop_reason: StopReason::EndTurn
        }
        .is_terminal()
    );
    assert!(!ChatEvent::TextDelta("a".to_owned()).is_terminal());
    assert!(
        !ChatEvent::Error("boum".to_owned()).is_terminal(),
        "an error can precede other events"
    );
}

#[test]
fn a_serialized_request_reads_back() {
    let req = ChatRequest::new("m", vec![ChatMessage::user("hello")]).with_max_tokens(64);
    let json = serde_json::to_string(&req).expect("serialization");
    let reread_request: ChatRequest = serde_json::from_str(&json).expect("deserialization");
    assert_eq!(reread_request, req);
}

#[test]
fn a_new_request_asks_for_neither_effort_nor_budget() {
    // The default must remain "what the provider usually does": imposing
    // an effort would bill a reasoning nobody asked for.
    let req = ChatRequest::new("m", vec![ChatMessage::user("a")]);
    assert_eq!(req.reasoning_effort, None);
    assert_eq!(req.reasoning_budget_tokens, None);
    assert!(!req.wants_reasoning());
    assert!(!req.cache_tools);
}

#[test]
fn reasoning_settings_survive_a_round_trip() {
    let req = ChatRequest::new("m", vec![ChatMessage::user("a")])
        .with_reasoning_effort(ReasoningEffort::XHigh)
        .with_reasoning_budget_tokens(8192)
        .with_cached_tools();
    assert!(req.wants_reasoning());

    let json = serde_json::to_value(&req).expect("serialization");
    assert_eq!(json["reasoning_effort"], "xhigh");
    assert_eq!(json["reasoning_budget_tokens"], 8192);
    assert_eq!(json["cache_tools"], true);

    let reread_request: ChatRequest = serde_json::from_value(json).expect("deserialization");
    assert_eq!(reread_request, req);
}

#[test]
fn a_missing_setting_does_not_go_on_the_wire() {
    // Several endpoints reject the whole request on an unknown field: a
    // `null` is not an omission.
    let req = ChatRequest::new("m", vec![ChatMessage::user("a")]);
    let json = serde_json::to_value(&req).expect("serialization");
    assert!(json.get("reasoning_effort").is_none(), "{json}");
    assert!(json.get("reasoning_budget_tokens").is_none(), "{json}");
    assert!(json.get("cache_tools").is_none(), "{json}");
}

#[test]
fn a_reasoning_block_attaches_to_the_assistant_turn() {
    let blocks = vec![
        ReasoningBlock::summarized("I count", Some("sig".to_owned())),
        ReasoningBlock::redacted("chiffre"),
    ];
    let msg = ChatMessage::assistant("42").with_reasoning(blocks.clone());
    assert_eq!(msg.reasoning, blocks);

    // The turn serializes and reads back identically: that is the
    // condition for the next turn to be accepted.
    let json = serde_json::to_string(&msg).expect("serialization");
    let reread_message: ChatMessage = serde_json::from_str(&json).expect("deserialization");
    assert_eq!(reread_message.reasoning, blocks);
}

#[test]
fn a_message_debug_does_not_show_the_reasoning() {
    // A reasoning block repeats what was given to the model — at the
    // `Sampled` tier, database rows (I-03).
    let msg = ChatMessage::assistant("ok").with_reasoning(vec![ReasoningBlock::summarized(
        "the patients table has an hiv_status column",
        None,
    )]);
    let rendered = format!("{msg:?}");
    assert!(!rendered.contains("hiv_status"), "{rendered}");
    assert!(rendered.contains("reasoning"), "{rendered}");
}

#[test]
fn a_message_marked_stable_stays_so_after_serialization() {
    let msg = ChatMessage::system("schema context").cached();
    assert!(msg.cache_breakpoint);
    let json = serde_json::to_value(&msg).expect("serialization");
    assert_eq!(json["cache_breakpoint"], true);

    let ordinary = ChatMessage::user("and the duplicates?");
    let json = serde_json::to_value(&ordinary).expect("serialization");
    assert!(
        json.get("cache_breakpoint").is_none(),
        "a false flag does not go out: {json}"
    );
}

#[test]
fn a_model_record_asserts_nothing_about_reasoning_by_default() {
    let info = ModelInfo::new("llama3.2");
    assert_eq!(info.supports_reasoning, Support::Unknown);
    assert!(info.reasoning_efforts.is_empty());

    let declared = ModelInfo::new("m")
        .with_reasoning_support(Support::Yes)
        .with_reasoning_efforts(vec![
            ReasoningEffort::High,
            ReasoningEffort::Low,
            ReasoningEffort::High,
        ]);
    assert_eq!(
        declared.reasoning_efforts,
        vec![ReasoningEffort::Low, ReasoningEffort::High],
        "the list is sorted and deduplicated: it is shown as is"
    );
}

#[test]
fn an_undeclared_usage_is_distinct_from_zero() {
    let event = ChatEvent::Usage {
        prompt_tokens: 10,
        completion_tokens: 2,
        cache_write_tokens: None,
        cache_read_tokens: Some(0),
        reasoning_tokens: None,
    };
    let ChatEvent::Usage {
        cache_write_tokens,
        cache_read_tokens,
        ..
    } = event
    else {
        panic!("unexpected variant");
    };
    assert_eq!(cache_write_tokens, None, "the provider said nothing");
    assert_eq!(cache_read_tokens, Some(0), "the provider said zero");
}
