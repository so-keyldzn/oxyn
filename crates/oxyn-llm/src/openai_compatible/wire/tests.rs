use super::*;
use crate::types::{ChatMessage, ToolSpec};

fn serialise(request: &ChatCompletionRequest) -> serde_json::Value {
    serde_json::to_value(request).expect("request serialization")
}

#[test]
fn a_minimal_request_carries_only_what_is_needed() {
    let req = ChatRequest::new("llama3.2", vec![ChatMessage::user("hello")]);
    let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));

    assert_eq!(body["model"], "llama3.2");
    assert_eq!(body["stream"], true);
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "hello");
    assert!(
        body.get("tools").is_none(),
        "an empty tools array must not go out: some servers refuse it"
    );
    assert!(body.get("temperature").is_none());
    assert!(body.get("max_tokens").is_none());
    assert!(
        body.get("stream_options").is_none(),
        "local servers do not know stream_options"
    );
}

#[test]
fn the_usage_request_is_explicit() {
    let req = ChatRequest::new("gpt-4o-mini", vec![ChatMessage::user("a")]);
    let body = serialise(&ChatCompletionRequest::from_request(&req, true, false));
    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[test]
fn tools_go_out_in_function_format() {
    let req = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![ToolSpec::new(
        "execute_query",
        "run a query",
        serde_json::json!({"type": "object", "properties": {}}),
    )]);
    let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "execute_query");
    assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
}

#[test]
fn call_arguments_go_out_as_a_json_string() {
    // The protocol detail that gets missed: `arguments` is a string.
    let call = ToolCall::new("call_1", "execute", serde_json::json!({"sql": "SELECT 1"}));
    let req = ChatRequest::new(
        "m",
        vec![ChatMessage::assistant("").with_tool_calls(vec![call])],
    );
    let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));
    let arguments = &body["messages"][0]["tool_calls"][0]["function"]["arguments"];
    assert!(arguments.is_string(), "{arguments}");
    assert_eq!(arguments.as_str(), Some(r#"{"sql":"SELECT 1"}"#));
}

#[test]
fn an_assistant_turn_without_text_sends_no_empty_content() {
    let call = ToolCall::new("call_1", "execute", serde_json::json!({}));
    let req = ChatRequest::new(
        "m",
        vec![ChatMessage::assistant("").with_tool_calls(vec![call])],
    );
    let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));
    assert!(body["messages"][0].get("content").is_none());
}

#[test]
fn a_tool_message_carries_its_identifier() {
    let req = ChatRequest::new("m", vec![ChatMessage::tool_result("call_1", "42 rows")]);
    let body = serialise(&ChatCompletionRequest::from_request(&req, false, false));
    assert_eq!(body["messages"][0]["role"], "tool");
    assert_eq!(body["messages"][0]["tool_call_id"], "call_1");
    assert_eq!(body["messages"][0]["content"], "42 rows");
}

#[test]
fn a_negative_or_missing_usage_counts_as_zero() {
    let usage: WireUsage =
        serde_json::from_str(r#"{"prompt_tokens": -1}"#).expect("tolerant deserialization");
    assert_eq!(usage.prompt(), 0);
    assert_eq!(usage.completion(), 0);
}

#[test]
fn an_empty_frame_deserializes() {
    // The first fragment of several servers is `{"choices":[{"delta":{}}]}`.
    let chunk: ChatChunk =
        serde_json::from_str(r#"{"choices":[{"delta":{}}]}"#).expect("frame tolerated");
    assert_eq!(chunk.choices.len(), 1);
    assert!(chunk.choices[0].delta.content.is_none());
}

#[test]
fn unknown_fields_do_not_fail_a_frame() {
    let chunk: ChatChunk = serde_json::from_str(
        r#"{"id":"x","object":"chat.completion.chunk","system_fingerprint":"fp","choices":[]}"#,
    )
    .expect("unknown fields are ignored");
    assert!(chunk.choices.is_empty());
}

#[test]
fn a_variable_price_is_not_displayed() {
    let variable = WirePricing {
        prompt: Some("-1".to_owned()),
        completion: Some("-1".to_owned()),
    };
    assert!(variable.into_cost().is_none());

    let unreadable = WirePricing {
        prompt: Some("gratuit".to_owned()),
        completion: Some("0".to_owned()),
    };
    assert!(unreadable.into_cost().is_none());
}

#[test]
fn a_per_token_price_becomes_a_per_million_price() {
    let price = WirePricing {
        prompt: Some("0.0000005".to_owned()),
        completion: Some("0.0000015".to_owned()),
    }
    .into_cost()
    .expect("readable price");
    assert!((price.input_per_million - 0.5).abs() < 1e-9, "{price:?}");
    assert!((price.output_per_million - 1.5).abs() < 1e-9, "{price:?}");
}

#[test]
fn an_unreadable_model_entry_does_not_lose_the_others() {
    let response: ModelsResponse =
        serde_json::from_str(r#"{"data":[{"id":"bon"},{"pas_d_id":true},{"id":"aussi-bon"}]}"#)
            .expect("list tolerated");
    let infos = parse_models(response);
    let ids: Vec<&str> = infos.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, ["bon", "aussi-bon"]);
}

#[test]
fn a_model_without_metadata_asserts_nothing() {
    let response: ModelsResponse =
        serde_json::from_str(r#"{"data":[{"id":"llama3.2"}]}"#).expect("list tolerated");
    let infos = parse_models(response);
    assert_eq!(infos[0].context_window, None);
    assert_eq!(infos[0].supports_tools, Support::Unknown);
    assert_eq!(infos[0].display_name, "llama3.2");
}

#[test]
fn a_model_that_declares_its_parameters_is_believed() {
    let response: ModelsResponse = serde_json::from_str(
        r#"{"data":[
                {"id":"a","supported_parameters":["tools","temperature"],"context_length":128000},
                {"id":"b","supported_parameters":["temperature"]}
            ]}"#,
    )
    .expect("list tolerated");
    let infos = parse_models(response);
    assert_eq!(infos[0].supports_tools, Support::Yes);
    assert_eq!(infos[0].context_window, Some(128_000));
    assert_eq!(infos[1].supports_tools, Support::No);
}

#[test]
fn the_main_provider_window_serves_as_fallback() {
    let response: ModelsResponse =
        serde_json::from_str(r#"{"data":[{"id":"a","top_provider":{"context_length":8192}}]}"#)
            .expect("list tolerated");
    assert_eq!(parse_models(response)[0].context_window, Some(8192));
}

#[test]
fn a_bare_array_is_a_model_list() {
    // Together AI's shape.
    let response: ModelsResponse = serde_json::from_str(
        r#"[{"id":"meta/llama","display_name":"Llama","context_length":131072,"type":"chat"}]"#,
    )
    .expect("bare array read");
    let infos = parse_models(response);
    assert_eq!(infos[0].id, "meta/llama");
    assert_eq!(infos[0].display_name, "Llama");
    assert_eq!(infos[0].context_window, Some(131_072));
}

#[test]
fn an_object_without_data_is_not_an_empty_list() {
    // A native endpoint answering at this address: wrong address, not « no
    // model ».
    for body in [
        r#"{"models":[{"name":"llama3"}]}"#,
        "{}",
        r#"{"data":3}"#,
        "3",
    ] {
        assert!(
            serde_json::from_str::<ModelsResponse>(body).is_err(),
            "`{body}` must be refused"
        );
    }
}

#[test]
fn a_bare_array_is_bounded_like_a_wrapped_one() {
    let entries = vec![serde_json::json!({"id": "m"}); crate::http::MAX_MODELS + 1];
    let body = serde_json::to_string(&entries).expect("serialized");
    let err = serde_json::from_str::<ModelsResponse>(&body).expect_err("too many entries");
    assert!(err.to_string().contains("too many entries"), "{err}");
}

#[test]
fn every_name_of_the_context_window_is_read() {
    let response: ModelsResponse = serde_json::from_str(
        r#"{"data":[
                {"id":"groq","context_window":131072},
                {"id":"mistral","max_context_length":32768},
                {"id":"deepseek","name":"DeepSeek","context_window":65536}
            ]}"#,
    )
    .expect("list tolerated");
    let infos = parse_models(response);
    assert_eq!(infos[0].context_window, Some(131_072));
    assert_eq!(infos[1].context_window, Some(32_768));
    assert_eq!(infos[2].context_window, Some(65_536));
    assert_eq!(infos[2].display_name, "DeepSeek");
}

#[test]
fn an_out_of_range_field_loses_the_field_not_the_model() {
    let response: ModelsResponse = serde_json::from_str(
        r#"{"data":[
                {"id":"a","context_length":-1,"name":42},
                {"id":"b","context_window":99999999999},
                {"id":"c","context_length":1.5,"display_name":null}
            ]}"#,
    )
    .expect("list tolerated");
    let infos = parse_models(response);
    let ids: Vec<&str> = infos.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c"]);
    assert!(infos.iter().all(|info| info.context_window.is_none()));
    assert_eq!(infos[0].display_name, "a");
}

#[test]
fn kinds_no_conversation_can_use_are_left_out() {
    let response: ModelsResponse = serde_json::from_str(
        r#"[{"id":"chat","type":"chat"},{"id":"embed","type":"embedding"},
                {"id":"img","type":"image"},{"id":"code","type":"code"},{"id":"plain"}]"#,
    )
    .expect("list tolerated");
    let ids: Vec<String> = parse_models(response).into_iter().map(|f| f.id).collect();
    assert_eq!(ids, ["chat", "code", "plain"]);
}

#[test]
fn missing_arguments_count_as_an_empty_object() {
    let ChatEvent::ToolCallComplete(call) =
        tool_call_event("c1".to_owned(), "ping".to_owned(), "  ")
    else {
        panic!("a tool without parameters is legitimate");
    };
    assert_eq!(call.arguments, serde_json::json!({}));
}

#[test]
fn truncated_arguments_keep_the_call_without_copying_them() {
    let event = tool_call_event(
        "c1".to_owned(),
        "execute".to_owned(),
        r#"{"sql": "SELECT secret FROM"#,
    );
    let ChatEvent::ToolCallInvalid { id, name, detail } = &event else {
        panic!("truncated JSON is an invalid call: {event:?}");
    };
    assert_eq!((id.as_str(), name.as_str()), ("c1", "execute"));
    assert!(detail.contains("truncated JSON"), "{detail}");
    assert!(
        !format!("{event:?}").contains("secret"),
        "the model output must not be copied: {event:?}"
    );
}

#[test]
fn an_error_in_the_stream_describes_itself() {
    let err = WireError {
        message: Some("rate limited".to_owned()),
        code: Some(serde_json::json!(429)),
    };
    assert_eq!(err.describe(), "rate limited (code 429)");
    assert_eq!(WireError::default().describe(), "no details given");
}
