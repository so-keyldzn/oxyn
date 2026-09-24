//! Which reported calls are Oxyn's, with the shapes the two adapters send and
//! the ones that only look like them.

use agent_client_protocol::schema::v1::{
    ToolCall, ToolCallId, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use serde_json::json;

use super::*;

fn served() -> OxynCalls {
    OxynCalls::new(vec![
        "describe_schema".to_owned(),
        "execute_query".to_owned(),
    ])
}

fn meta(value: Value) -> Meta {
    match value {
        Value::Object(map) => map,
        other => panic!("a test meta is an object, not {other}"),
    }
}

/// As `claude-agent-acp` 0.78.0 reports a tool it does not know: its name as
/// title, kind `other`, the name again in `_meta.claudeCode.toolName`.
fn claude(id: &str, name: &str) -> SessionUpdate {
    SessionUpdate::ToolCall(
        ToolCall::new(ToolCallId::new(id), name)
            .kind(ToolKind::Other)
            .meta(meta(json!({ "claudeCode": { "toolName": name } }))),
    )
}

/// As `codex-acp` 1.12.0 reports an MCP call when it starts.
fn codex(id: &str, server: &str, tool: &str) -> SessionUpdate {
    SessionUpdate::ToolCall(
        ToolCall::new(ToolCallId::new(id), format!("mcp.{server}.{tool}"))
            .kind(ToolKind::Execute)
            .status(ToolCallStatus::InProgress)
            .raw_input(json!({ "server": server, "tool": tool, "arguments": {} }))
            .meta(meta(json!({ "is_mcp_tool_call": true }))),
    )
}

/// As `codex-acp` 1.12.0 reports its end: no `_meta` at all.
fn codex_end(id: &str, server: &str, tool: &str) -> SessionUpdate {
    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        ToolCallId::new(id),
        ToolCallUpdateFields::new()
            .status(ToolCallStatus::Completed)
            .raw_input(json!({ "server": server, "tool": tool, "arguments": {} })),
    ))
}

fn status(id: &str, status: ToolCallStatus) -> SessionUpdate {
    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        ToolCallId::new(id),
        ToolCallUpdateFields::new().status(status),
    ))
}

#[test]
fn claudes_call_to_an_oxyn_tool_and_its_updates_are_hidden() {
    let mut calls = served();
    assert!(calls.hides(&claude("c1", "mcp__oxyn__describe_schema")));
    assert!(calls.hides(&status("c1", ToolCallStatus::InProgress)));
    assert!(calls.hides(&status("c1", ToolCallStatus::Completed)));
}

#[test]
fn codexs_call_to_an_oxyn_tool_and_its_unmarked_end_are_hidden() {
    let mut calls = served();
    assert!(calls.hides(&codex("x1", "oxyn", "execute_query")));
    assert!(calls.hides(&codex_end("x1", "oxyn", "execute_query")));
}

#[test]
fn a_finished_call_is_forgotten() {
    let mut calls = served();
    assert!(calls.hides(&claude("c1", "mcp__oxyn__describe_schema")));
    assert!(calls.hides(&status("c1", ToolCallStatus::Failed)));
    assert!(calls.open.is_empty());
    // An update nobody announced is the agent's: relayed.
    assert!(!calls.hides(&status("c1", ToolCallStatus::Completed)));
}

#[test]
fn another_servers_tool_named_like_oxyns_is_not_hidden() {
    let mut calls = served();
    for name in [
        "mcp__oxyn_evil__describe_schema",
        "mcp__oxyn_evil__x",
        "mcp__oxyn__describe_schema__x",
        "mcp__Oxyn__describe_schema",
        "mcp__oxyn__",
        "mcp__oxyn",
        " mcp__oxyn__describe_schema",
    ] {
        assert!(!calls.hides(&claude("c", name)), "{name}");
    }
    for (server, tool) in [
        ("oxyn_evil", "describe_schema"),
        ("oxyn.evil", "describe_schema"),
        ("Oxyn", "describe_schema"),
        ("", "describe_schema"),
    ] {
        assert!(!calls.hides(&codex("x", server, tool)), "{server}/{tool}");
    }
}

#[test]
fn a_tool_oxyn_does_not_serve_is_not_hidden() {
    let mut calls = served();
    assert!(!calls.hides(&claude("c", "mcp__oxyn__drop_everything")));
    assert!(!calls.hides(&codex("x", "oxyn", "request_sample")));

    // No tools served: nothing is Oxyn's, whatever its name.
    let mut none = OxynCalls::new(Vec::new());
    assert!(!none.hides(&claude("c", "mcp__oxyn__describe_schema")));
    assert!(!none.hides(&codex("x", "oxyn", "describe_schema")));
}

#[test]
fn a_title_spelled_like_an_oxyn_tool_is_not_a_signal() {
    let mut calls = served();
    // A shell command spelled like the tool: Claude titles `Bash` with it.
    let bash = SessionUpdate::ToolCall(
        ToolCall::new(ToolCallId::new("b"), "mcp__oxyn__describe_schema")
            .kind(ToolKind::Execute)
            .meta(meta(json!({ "claudeCode": { "toolName": "Bash" } }))),
    );
    assert!(!calls.hides(&bash));
    // No `_meta`: nothing says what the call is.
    let bare = SessionUpdate::ToolCall(
        ToolCall::new(ToolCallId::new("t"), "mcp__oxyn__describe_schema").kind(ToolKind::Other),
    );
    assert!(!calls.hides(&bare));
    let codex_title = SessionUpdate::ToolCall(
        ToolCall::new(ToolCallId::new("t"), "mcp.oxyn.describe_schema").kind(ToolKind::Execute),
    );
    assert!(!calls.hides(&codex_title));
}

#[test]
fn arguments_that_imitate_codexs_shape_are_not_a_signal() {
    let mut calls = served();
    // Another tool whose arguments the model wrote as `{server, tool}`,
    // without the adapter's mark — or with a mark that is not `true`.
    for mark in [json!({}), json!({ "is_mcp_tool_call": "true" })] {
        let lookalike = SessionUpdate::ToolCall(
            ToolCall::new(ToolCallId::new("t"), "mcp.oxyn.describe_schema")
                .kind(ToolKind::Execute)
                .raw_input(json!({ "server": "oxyn", "tool": "describe_schema" }))
                .meta(meta(mark.clone())),
        );
        assert!(!calls.hides(&lookalike), "{mark}");
    }
    // The mark with fields of the wrong type.
    let odd = SessionUpdate::ToolCall(
        ToolCall::new(ToolCallId::new("t"), "x")
            .raw_input(json!({ "server": ["oxyn"], "tool": 7 }))
            .meta(meta(json!({ "is_mcp_tool_call": true }))),
    );
    assert!(!calls.hides(&odd));
}

#[test]
fn what_is_not_a_tool_call_is_never_hidden() {
    use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, TextContent};

    let mut calls = served();
    let text = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
        TextContent::new("mcp__oxyn__describe_schema"),
    )));
    assert!(!calls.hides(&text));
}
