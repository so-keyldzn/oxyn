use oxyn_core::OxynError;
use serde_json::json;

use super::*;
use crate::reasoning::{ReasoningBlock, ReasoningEffort};
use crate::types::{ChatMessage, ToolCall, ToolSpec};

fn provider() -> AnthropicProvider {
    AnthropicProvider::new("sk-ant-test").expect("construction")
}

fn body(request: &ChatRequest) -> Value {
    provider().wire_request(request).expect("valid request")
}

// ── System instruction, blocks, alternation ────────────────────────────

#[test]
fn the_system_instruction_leaves_the_messages() {
    // The point that distinguishes this protocol: `system` is a field, not
    // a message.
    let request = ChatRequest::new(
        "claude-model",
        vec![
            ChatMessage::system("you are a SQL assistant"),
            ChatMessage::user("hello"),
        ],
    );
    let body = body(&request);

    assert_eq!(body["system"][0]["type"], "text");
    assert_eq!(body["system"][0]["text"], "you are a SQL assistant");
    assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["messages"][0]["role"], "user");
}

#[test]
fn several_system_instructions_are_concatenated_not_lost() {
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::system("rule 1"),
            ChatMessage::system("rule 2"),
            ChatMessage::user("go"),
        ],
    );
    assert_eq!(body(&request)["system"][0]["text"], "rule 1\n\nrule 2");
}

#[test]
fn the_content_is_a_list_of_blocks() {
    let request = ChatRequest::new("m", vec![ChatMessage::user("hello")]);
    let body = body(&request);
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
}

#[test]
fn a_tool_call_becomes_a_tool_use_block() {
    let call = ToolCall::new("call_1", "execute", json!({"sql": "SELECT 1"}));
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::user("count the rows"),
            ChatMessage::assistant("").with_tool_calls(vec![call]),
        ],
    );
    let block = body(&request)["messages"][1]["content"][0].clone();
    assert_eq!(block["type"], "tool_use");
    assert_eq!(block["id"], "call_1");
    assert_eq!(block["name"], "execute");
    assert_eq!(
        block["input"],
        json!({"sql": "SELECT 1"}),
        "the input is an object, not a string: this protocol differs from OpenAI"
    );
}

#[test]
fn a_tool_result_becomes_a_user_block() {
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::user("count"),
            ChatMessage::assistant("").with_tool_calls(vec![ToolCall::new(
                "call_1",
                "execute",
                json!({}),
            )]),
            ChatMessage::tool_result("call_1", "42"),
        ],
    );
    let last = body(&request)["messages"][2].clone();
    assert_eq!(last["role"], "user", "this protocol has no `tool` role");
    assert_eq!(last["content"][0]["type"], "tool_result");
    assert_eq!(last["content"][0]["tool_use_id"], "call_1");
    assert_eq!(last["content"][0]["content"], "42");
}

#[test]
fn two_consecutive_messages_of_the_same_role_are_merged() {
    // The API requires alternation; two successive tool results are the
    // common case when the model asked for several.
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::user("a"),
            ChatMessage::tool_result("call_1", "r1"),
            ChatMessage::tool_result("call_2", "r2"),
        ],
    );
    let body = body(&request);
    let messages = body["messages"].as_array().expect("array");
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0]["content"].as_array().map(Vec::len), Some(3));
}

#[test]
fn tools_use_input_schema() {
    let request =
        ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![ToolSpec::new(
            "lister",
            "list the tables",
            json!({"type": "object"}),
        )]);
    let body = body(&request);
    assert_eq!(body["tools"][0]["name"], "lister");
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    assert!(
        body["tools"][0].get("parameters").is_none(),
        "`parameters` is OpenAI's name, not this one's"
    );
}

#[test]
fn the_token_ceiling_is_always_present() {
    // `max_tokens` is mandatory in this protocol.
    let without_limit = ChatRequest::new("m", vec![ChatMessage::user("a")]);
    assert_eq!(
        body(&without_limit)["max_tokens"],
        json!(DEFAULT_MAX_TOKENS)
    );

    let with_limit = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_max_tokens(128);
    assert_eq!(body(&with_limit)["max_tokens"], json!(128));
}

#[test]
fn a_request_without_model_or_message_is_refused() {
    let f = provider();
    assert!(
        f.wire_request(&ChatRequest::new("", vec![ChatMessage::user("a")]))
            .is_err()
    );
    assert!(
        f.wire_request(&ChatRequest::new("m", vec![ChatMessage::system("seule")]))
            .is_err(),
        "a system instruction alone does not make a conversation"
    );
}

// ── Reasoning ──────────────────────────────────────────────────────────

#[test]
fn effort_goes_through_output_config_and_leaves_the_thinking_mode_alone() {
    // Sending a thinking mode that was not requested would make the request
    // fail on models that do not know it.
    let request = ChatRequest::new("m", vec![ChatMessage::user("a")])
        .with_reasoning_effort(ReasoningEffort::XHigh);
    let body = body(&request);
    assert_eq!(body["output_config"]["effort"], "xhigh");
    assert!(body.get("thinking").is_none(), "{body}");
}

#[test]
fn a_thinking_budget_requires_the_explicit_mode() {
    let request =
        ChatRequest::new("m", vec![ChatMessage::user("a")]).with_reasoning_budget_tokens(8192);
    let body = body(&request);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 8192);
    assert_eq!(
        body["thinking"]["display"], "summarized",
        "without it the reasoning comes back empty"
    );
}

#[test]
fn both_settings_coexist() {
    let request = ChatRequest::new("m", vec![ChatMessage::user("a")])
        .with_reasoning_effort(ReasoningEffort::Low)
        .with_reasoning_budget_tokens(1024);
    let body = body(&request);
    assert_eq!(body["output_config"]["effort"], "low");
    assert_eq!(body["thinking"]["budget_tokens"], 1024);
}

#[test]
fn a_request_without_reasoning_request_carries_no_trace_of_it() {
    let body = body(&ChatRequest::new("m", vec![ChatMessage::user("a")]));
    assert!(body.get("thinking").is_none(), "{body}");
    assert!(body.get("output_config").is_none(), "{body}");
}

#[test]
fn reasoning_blocks_go_back_first_and_intact() {
    // The API checks their signature: reordering them, editing them or
    // losing one makes the request refused.
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::user("count"),
            ChatMessage::assistant("here it is")
                .with_reasoning(vec![
                    ReasoningBlock::summarized("I am thinking", Some("SIG".to_owned())),
                    ReasoningBlock::redacted("CHIFFRE"),
                ])
                .with_tool_calls(vec![ToolCall::new("c1", "execute", json!({}))]),
        ],
    );
    let content = body(&request)["messages"][1]["content"].clone();
    let blocks = content.as_array().expect("array");

    assert_eq!(blocks[0]["type"], "thinking");
    assert_eq!(blocks[0]["thinking"], "I am thinking");
    assert_eq!(blocks[0]["signature"], "SIG");
    assert_eq!(blocks[1]["type"], "redacted_thinking");
    assert_eq!(blocks[1]["data"], "CHIFFRE");
    assert_eq!(blocks[2]["type"], "text", "reasoning comes first");
    assert_eq!(blocks[3]["type"], "tool_use");
}

#[test]
fn a_reasoning_block_without_signature_does_not_carry_the_field() {
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::user("a"),
            ChatMessage::assistant("b").with_reasoning(vec![ReasoningBlock::summarized("t", None)]),
        ],
    );
    let block = body(&request)["messages"][1]["content"][0].clone();
    assert!(
        block.get("signature").is_none(),
        "an empty field is not an absence: {block}"
    );
}

// ── Cache de prompt ────────────────────────────────────────────────────

#[test]
fn a_marked_message_carries_the_cache_marker() {
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::system("the full schema").cached(),
            ChatMessage::user("and the duplicates?"),
        ],
    );
    let body = body(&request);
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    assert!(
        body["messages"][0]["content"][0]
            .get("cache_control")
            .is_none(),
        "the question changes at every turn: marking it would be useless"
    );
}

#[test]
fn the_marker_is_set_on_the_last_block_of_the_message() {
    // A marker closes a prefix; setting it at the head would cache nothing.
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::assistant("text")
                .with_tool_calls(vec![ToolCall::new("c1", "execute", json!({}))])
                .cached(),
            ChatMessage::user("suite"),
        ],
    );
    let content = body(&request)["messages"][0]["content"].clone();
    let blocks = content.as_array().expect("array");
    assert!(blocks[0].get("cache_control").is_none(), "{blocks:?}");
    assert_eq!(
        blocks[blocks.len() - 1]["cache_control"]["type"],
        "ephemeral"
    );
}

#[test]
fn marked_tools_carry_the_marker_on_the_last_one() {
    let request = ChatRequest::new("m", vec![ChatMessage::user("a")])
        .with_tools(vec![
            ToolSpec::new("one", "d1", json!({})),
            ToolSpec::new("two", "d2", json!({})),
        ])
        .with_cached_tools();
    let body = body(&request);
    assert!(body["tools"][0].get("cache_control").is_none());
    assert_eq!(body["tools"][1]["cache_control"]["type"], "ephemeral");
}

#[test]
fn the_number_of_markers_is_bounded() {
    // Beyond the bound, the API refuses the whole request: Oxyn stops
    // before rather than let a `400` reach the user.
    let mut messages = vec![ChatMessage::system("contexte").cached()];
    for number in 0..10 {
        messages.push(ChatMessage::user(format!("question {number}")).cached());
        messages.push(ChatMessage::assistant(format!("answer {number}")).cached());
    }
    let request = ChatRequest::new("m", messages)
        .with_tools(vec![ToolSpec::new("t", "d", json!({}))])
        .with_cached_tools();

    let markers = count_markers(&body(&request));
    assert!(
        markers <= wire::MAX_CACHE_BREAKPOINTS,
        "{markers} markers set"
    );
}

/// Counts the `cache_control`s present anywhere in the body.
fn count_markers(value: &Value) -> usize {
    match value {
        Value::Object(object) => {
            let here = usize::from(object.contains_key("cache_control"));
            here + object.values().map(count_markers).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(count_markers).sum(),
        _ => 0,
    }
}

// ── Token counting ─────────────────────────────────────────────────────

#[test]
fn the_counting_body_carries_neither_stream_nor_ceiling() {
    // The counting endpoint refuses `stream`.
    let request = ChatRequest::new("m", vec![ChatMessage::user("a")]);
    let body = provider()
        .build_body(&request, false)
        .expect("valid request");
    assert!(body.get("stream").is_none(), "{body}");
    assert!(body.get("max_tokens").is_none(), "{body}");
    assert_eq!(body["model"], "m");
}

// ── Privacy and headers ────────────────────────────────────────────────

#[test]
fn debug_does_not_show_the_key() {
    let rendered = format!("{:?}", AnthropicProvider::new("sk-ant-THIS").expect("ok"));
    assert!(!rendered.contains("THIS"), "{rendered}");
    assert!(rendered.contains("<present>"), "{rendered}");
}

#[test]
fn a_key_with_a_line_break_is_refused_without_being_displayed() {
    // A key pasted from a terminal often carries a `\n`.
    let f = AnthropicProvider::new("sk-ant-with\nbreak").expect("construction");
    let err = f.prepared_request().expect_err("invalid header");
    assert!(!err.to_string().contains("sk-ant-with"), "{err}");
}

#[test]
fn a_blank_key_is_refused_locally() {
    // A `401` would suggest an account problem whereas the configuration is
    // simply incomplete.
    let f = AnthropicProvider::new("   ").expect("construction");
    let err = f.prepared_request().expect_err("blank key");
    assert!(matches!(err, OxynError::Authentication(_)), "{err}");
}

#[test]
fn a_prepared_request_builds_with_a_valid_key() {
    assert!(provider().prepared_request().is_ok());
}

#[test]
fn urls_are_assembled() {
    let f = provider();
    assert_eq!(
        f.messages_url().expect("URL").as_str(),
        "https://api.anthropic.com/v1/messages"
    );
    assert_eq!(
        f.join(COUNT_TOKENS_PATH).expect("URL").as_str(),
        "https://api.anthropic.com/v1/messages/count_tokens"
    );
    assert_eq!(
        f.join(MODELS_PATH).expect("URL").as_str(),
        "https://api.anthropic.com/v1/models"
    );
}

#[test]
fn a_base_url_without_final_slash_keeps_its_path() {
    // The `Url::join` trap: without a final `/`, the last segment is
    // replaced — a corporate gateway often serves under a prefix.
    let f = AnthropicProvider::with_base_url("sk-ant-test", "https://passerelle.example/api")
        .expect("construction");
    assert_eq!(
        f.messages_url().expect("URL").as_str(),
        "https://passerelle.example/api/v1/messages"
    );
}

#[test]
fn the_headers_carry_the_version() {
    let f = provider().with_version("2099-01-01");
    let headers = f.headers();
    assert!(
        headers.iter().any(
            |(header_name, value)| *header_name == "anthropic-version" && value == "2099-01-01"
        ),
        "{headers:?}"
    );
    assert!(
        headers
            .iter()
            .all(|(header_name, _)| *header_name != API_KEY_HEADER),
        "the key does not go through there: {headers:?}"
    );
}

#[test]
fn an_unreadable_base_url_is_a_configuration_error() {
    let err =
        AnthropicProvider::with_base_url("sk-ant-test", "not a url").expect_err("invalid URL");
    assert!(matches!(err, OxynError::Config(_)), "{err}");
}

// ── Cancellation before sending───────────────────────────────────────────

#[test]
fn an_already_cancelled_token_short_circuits_the_call() {
    let f = provider();
    let token = CancelToken::new();
    token.cancel();
    let request = ChatRequest::new("claude-model", vec![ChatMessage::user("hello")]);
    let issue = futures::executor::block_on(f.stream(request, &token));
    let err = match issue {
        Ok(_) => panic!("cancelled beforehand"),
        Err(err) => err,
    };
    assert!(err.is_cancelled(), "{err}");
}

// ── Failure statuses ───────────────────────────────────────────────────

#[test]
fn failure_statuses_carry_their_class() {
    // The class decides a retry, and it is the caller that decides — never
    // this crate (I-13). The values come from the error table checked on
    // 2026-09-16.
    let key = ApiKey::new("sk-ant-test");
    let cases = [
        (
            401,
            r#"{"type":"error","error":{"type":"authentication_error"}}"#,
            false,
        ),
        (
            413,
            r#"{"type":"error","error":{"type":"request_too_large"}}"#,
            false,
        ),
        (
            429,
            r#"{"type":"error","error":{"type":"rate_limit_error"}}"#,
            true,
        ),
        (
            500,
            r#"{"type":"error","error":{"type":"api_error"}}"#,
            true,
        ),
        (
            529,
            r#"{"type":"error","error":{"type":"overloaded_error"}}"#,
            true,
        ),
    ];
    for (statut, body, retryable) in cases {
        let err = LlmError::from_response(ProviderId::anthropic(), statut, body, Some(&key));
        assert_eq!(err.is_retryable(), retryable, "HTTP {statut} : {err}");
    }
}

/// Only the documented overflow is recognized: the generic type of every
/// `400`, the phrase elsewhere in a message, another status or another
/// shape stay ordinary refusals — the side that drops nothing.
#[test]
fn only_the_documented_overflow_is_a_context_too_long() {
    let overflow = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 215000 tokens > 200000 maximum"},"request_id":"req_1"}"#;
    assert!(wire::is_context_overflow(400, overflow));

    for (status, body) in [
        (413, overflow),
        (
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"This model does not support assistant message prefill."}}"#,
        ),
        (
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages.0: the prompt is too long"}}"#,
        ),
        (
            400,
            r#"{"type":"error","error":{"type":"api_error","message":"prompt is too long"}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"invalid_request_error","message":"prompt is too long"}}"#,
        ),
        (
            402,
            r#"{"type":"error","error":{"type":"billing_error","message":"prompt is too long"}}"#,
        ),
        (400, "prompt is too long"),
        (400, ""),
    ] {
        assert!(!wire::is_context_overflow(status, body), "{status} {body}");
    }
}

#[test]
fn a_context_too_long_reaches_the_domain_typed_and_final() {
    let err = LlmError::ContextTooLong {
        provider: ProviderId::anthropic(),
        message: "prompt is too long".to_owned(),
    };
    assert!(!err.is_retryable());
    let projected: OxynError = err.into();
    assert!(
        matches!(projected, OxynError::ContextTooLong(_)),
        "{projected}"
    );
    assert!(!projected.is_retryable());
}

#[test]
fn an_authentication_refusal_never_copies_the_key() {
    // This provider sometimes copies the received key into its message.
    let key = ApiKey::new("sk-ant-api03-TRES-SECRET");
    let err = LlmError::from_response(
        ProviderId::anthropic(),
        401,
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key: sk-ant-api03-TRES-SECRET"}}"#,
        Some(&key),
    );
    let rendered = err.to_string();
    assert!(!rendered.contains("TRES-SECRET"), "{rendered}");
    assert!(rendered.contains(crate::secret::REDACTED), "{rendered}");

    let projected: OxynError = err.into();
    assert!(matches!(projected, OxynError::Authentication(_)));
    assert!(!projected.to_string().contains("TRES-SECRET"));
}

// ── Exact shape of the request ─────────────────────────────────────────

#[test]
fn a_complete_request_has_the_expected_shape() {
    // A deliberately readable snapshot rather than a file on the side: what
    // matters is that a change of shape shows **in review**, and a JSON
    // value comparison says exactly what moved.
    let request = ChatRequest::new(
        "claude-model",
        vec![
            ChatMessage::system("you are a SQL assistant").cached(),
            ChatMessage::user("count the customers"),
        ],
    )
    .with_max_tokens(256)
    .with_tools(vec![ToolSpec::new(
        "execute_query",
        "run a query",
        json!({"type": "object", "properties": {"sql": {"type": "string"}}}),
    )])
    .with_cached_tools()
    .with_reasoning_effort(ReasoningEffort::Medium);

    assert_eq!(
        body(&request),
        json!({
            "model": "claude-model",
            "tools": [{
                "name": "execute_query",
                "description": "run a query",
                "input_schema": {
                    "type": "object",
                    "properties": {"sql": {"type": "string"}}
                },
                "cache_control": {"type": "ephemeral"}
            }],
            "system": [{
                "type": "text",
                "text": "you are a SQL assistant",
                "cache_control": {"type": "ephemeral"}
            }],
            "messages": [{
                "role": "user",
                "content": [{"type": "text", "text": "count the customers"}]
            }],
            "stream": true,
            "max_tokens": 256,
            "output_config": {"effort": "medium"}
        })
    );
}

#[test]
fn a_local_connection_cannot_reach_this_provider() {
    // ADR-0006: `Local` promises that nothing leaves the machine. This
    // provider is remote by construction — there is no Anthropic
    // deployment on the loopback —, so the combination is refused. The
    // test resolves no name: it relies on the fact that the tier, itself,
    // does not depend on resolution.
    assert!(
        !oxyn_core::PrivacyTier::Local.allows_remote_provider(),
        "a tier that let a remote provider through would promise nothing anymore"
    );
    for niveau in [
        oxyn_core::PrivacyTier::Metadata,
        oxyn_core::PrivacyTier::Sampled,
    ] {
        assert!(niveau.allows_remote_provider(), "{niveau:?}");
    }

    // And the default endpoint is not a literal loopback address: nothing
    // here can pass itself off as local.
    let f = provider();
    let point = f.endpoint().expect("this provider has a URL");
    assert_ne!(
        crate::reach::literal_reach(point),
        Some(crate::reach::Reach::Local),
        "{point}"
    );
}

#[test]
fn an_invalid_request_reports_itself_before_any_network_call() {
    let f = provider();
    let request = ChatRequest::new("  ", vec![ChatMessage::user("hello")]);
    let issue = futures::executor::block_on(f.stream(request, &CancelToken::new()));
    let err = match issue {
        Ok(_) => panic!("empty model"),
        Err(err) => err,
    };
    assert!(matches!(&err, OxynError::Config(_)), "{err}");
}
