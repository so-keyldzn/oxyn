use oxyn_core::OxynError;

use super::*;
use crate::types::{ToolCall, ToolSpec};

fn provider() -> GeminiProvider {
    GeminiProvider::new("test-key").expect("construction")
}

#[test]
fn an_unimplemented_exchange_refuses_instead_of_panicking() {
    // I-09: a panic kills the application. A provider that is configured
    // but whose protocol is not written must give a message.
    let f = provider();
    let request = ChatRequest::new("gemini-2.0-flash", vec![ChatMessage::user("hello")]);
    let flux = futures::executor::block_on(f.stream(request, &CancelToken::new()))
        .err()
        .expect("not implemented yet");
    let rendered = flux.to_string();
    assert!(rendered.contains("gemini"), "{rendered}");
    assert!(
        rendered.contains("Oxyn does not implement"),
        "the message must point at Oxyn, not the provider: {rendered}"
    );
}

#[test]
fn an_invalid_request_reports_itself_before_the_refusal() {
    // The order matters: otherwise the refusal would hide a defect of the
    // request, and the validation would stop being tested until phase 2.
    let f = provider();
    let request = ChatRequest::new("  ", vec![ChatMessage::user("hello")]);
    let err = futures::executor::block_on(f.stream(request, &CancelToken::new()))
        .err()
        .expect("empty model");
    assert!(matches!(&err, OxynError::Config(_)), "{err}");
}

#[test]
fn the_model_is_in_the_path_with_streaming_requested() {
    let url = provider()
        .stream_url("gemini-2.0-flash")
        .expect("valid URL");
    assert_eq!(
        url.as_str(),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
    );
}

#[test]
fn a_model_name_that_would_rewrite_the_request_is_refused() {
    // I-10: a received identifier is not concatenated without checking.
    let f = provider();
    for malformed in ["", "../other", "model?key=stolen", "model#x", "mo del"] {
        assert!(
            f.stream_url(malformed).is_err(),
            "`{malformed}` should have been refused"
        );
    }
}

#[test]
fn the_system_instruction_is_a_separate_field() {
    let request = ChatRequest::new(
        "gemini-2.0-flash",
        vec![
            ChatMessage::system("you are a SQL assistant"),
            ChatMessage::user("hello"),
        ],
    );
    let body = provider().wire_request(&request).expect("valid request");
    assert_eq!(
        body["systemInstruction"]["parts"][0]["text"],
        "you are a SQL assistant"
    );
    assert_eq!(body["contents"].as_array().map(Vec::len), Some(1));
}

#[test]
fn the_assistant_role_is_called_model() {
    let request = ChatRequest::new(
        "m",
        vec![ChatMessage::user("a"), ChatMessage::assistant("b")],
    );
    let body = provider().wire_request(&request).expect("valid request");
    assert_eq!(body["contents"][0]["role"], "user");
    assert_eq!(body["contents"][1]["role"], "model");
}

#[test]
fn a_tool_response_finds_the_function_name_again() {
    // The point that rules out the adapter: this protocol designates by name.
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::user("count"),
            ChatMessage::assistant("").with_tool_calls(vec![ToolCall::new(
                "call_1",
                "execute_query",
                json!({"sql": "SELECT 1"}),
            )]),
            ChatMessage::tool_result("call_1", "42"),
        ],
    );
    let body = provider().wire_request(&request).expect("valid request");
    let response = &body["contents"][2]["parts"][0]["functionResponse"];
    assert_eq!(response["name"], "execute_query");
    assert_eq!(response["response"]["result"], "42");
}

#[test]
fn two_tool_responses_do_not_mix() {
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::user("do both"),
            ChatMessage::assistant("").with_tool_calls(vec![
                ToolCall::new("c1", "lire", json!({})),
                ToolCall::new("c2", "count_rows", json!({})),
            ]),
            ChatMessage::tool_result("c2", "two"),
            ChatMessage::tool_result("c1", "one"),
        ],
    );
    let body = provider().wire_request(&request).expect("valid request");
    let parts = body["contents"][2]["parts"]
        .as_array()
        .expect("both responses are merged into one turn");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["functionResponse"]["name"], "count_rows");
    assert_eq!(parts[1]["functionResponse"]["name"], "lire");
}

#[test]
fn a_tool_call_becomes_a_function_call() {
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::user("a"),
            ChatMessage::assistant("I am looking").with_tool_calls(vec![ToolCall::new(
                "c1",
                "lister",
                json!({"schema": "public"}),
            )]),
        ],
    );
    let body = provider().wire_request(&request).expect("valid request");
    let parts = body["contents"][1]["parts"].as_array().expect("parts");
    assert_eq!(parts[0]["text"], "I am looking");
    assert_eq!(parts[1]["functionCall"]["name"], "lister");
    assert_eq!(parts[1]["functionCall"]["args"]["schema"], "public");
}

#[test]
fn tools_are_grouped_into_declarations() {
    let request = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![
        ToolSpec::new("lister", "liste", json!({"type": "object"})),
        ToolSpec::new("count_rows", "count", json!({"type": "object"})),
    ]);
    let body = provider().wire_request(&request).expect("valid request");
    let declarations = body["tools"][0]["functionDeclarations"]
        .as_array()
        .expect("a single group of declarations");
    assert_eq!(declarations.len(), 2);
    assert_eq!(declarations[0]["name"], "lister");
}

#[test]
fn the_token_ceiling_carries_the_protocol_name() {
    let request = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_max_tokens(256);
    let body = provider().wire_request(&request).expect("valid request");
    assert_eq!(body["generationConfig"]["maxOutputTokens"], json!(256));
    assert!(body["generationConfig"].get("max_tokens").is_none());
}

#[test]
fn a_request_without_a_useful_turn_is_refused() {
    let request = ChatRequest::new("m", vec![ChatMessage::system("seule")]);
    assert!(provider().wire_request(&request).is_err());
}

#[test]
fn debug_does_not_show_the_key() {
    let rendered = format!("{:?}", GeminiProvider::new("THISMUSTNOTLEAK").expect("ok"));
    assert!(!rendered.contains("THISMUSTNOTLEAK"), "{rendered}");
}

#[test]
fn a_key_with_a_line_break_is_refused_without_being_displayed() {
    // A key pasted from a terminal often carries a `\n`.
    let f = GeminiProvider::new("key-with\nbreak").expect("construction");
    let err = f
        .prepared_request("gemini-2.0-flash")
        .expect_err("invalid header");
    assert!(!err.to_string().contains("key-with"), "{err}");
}

#[test]
fn a_prepared_request_does_not_put_the_key_in_the_url() {
    // A key as a query parameter ends up in the access logs.
    let f = provider();
    let url = f.stream_url("gemini-2.0-flash").expect("valid URL");
    assert!(!url.as_str().contains("test-key"), "{url}");
    assert!(f.prepared_request("gemini-2.0-flash").is_ok());
}
