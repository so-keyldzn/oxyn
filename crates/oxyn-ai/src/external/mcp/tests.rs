use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::executor::block_on;
use oxyn_core::{
    AgentId, AgentSessionId, CancelToken, Command, ConnectionId, QueryLanguage, SessionId,
};

use super::*;
use crate::runtime::{CommandSink, DispatchOutcome};

/// A bus that records what reached it, and answers what the test wants.
#[derive(Debug)]
struct Bus {
    answer: DispatchOutcome,
    seen: Mutex<Vec<(Actor, Command)>>,
}

impl Bus {
    fn answering(answer: DispatchOutcome) -> Self {
        Self {
            answer,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn completed() -> Self {
        Self::answering(DispatchOutcome::Completed {
            summary: "1 rows, 1 batches".to_owned(),
        })
    }

    fn commands(&self) -> Vec<(Actor, Command)> {
        self.seen.lock().expect("no panic held the lock").clone()
    }
}

#[async_trait]
impl CommandSink for Bus {
    async fn dispatch(
        &self,
        actor: Actor,
        command: Command,
        _cancel: &CancelToken,
    ) -> DispatchOutcome {
        self.seen
            .lock()
            .expect("no panic held the lock")
            .push((actor, command));
        self.answer.clone()
    }
}

fn actor() -> Actor {
    Actor::agent(AgentId::new(), AgentSessionId::new())
}

fn service(actor: Actor) -> ToolService {
    ToolService::new(
        ToolRegistry::builtin(),
        vec![
            crate::tools::EXECUTE_QUERY.to_owned(),
            crate::tools::REFRESH_CATALOG.to_owned(),
        ],
        ToolScope::new(ConnectionId::new(), SessionId::new(), QueryLanguage::SQL),
        crate::external::mcp::TierCell::holding(PrivacyTier::Metadata),
        actor,
    )
}

/// A question open on `bus`, for the length of one message.
fn ask(service: &ToolService, bus: &Arc<Bus>, message: &str) -> Option<String> {
    let turns = ToolTurns::new(8, Arc::new(|| false));
    let _turn = turns.open(
        Arc::clone(bus) as Arc<dyn CommandSink>,
        Arc::new(()),
        CancelToken::new(),
    );
    block_on(service.respond(message, &turns))
}

fn answered(service: &ToolService, bus: &Arc<Bus>, message: &str) -> Value {
    let reply = ask(service, bus, message).expect("a request is answered");
    serde_json::from_str(&reply).expect("the reply is JSON")
}

#[test]
fn the_agent_is_offered_exactly_the_registry() {
    // An extra tool written for external agents would be a path the internal
    // assistant never takes, and the one nobody reviews (ADR-0030).
    let bus = Arc::new(Bus::completed());
    let service = service(actor());
    let reply = answered(
        &service,
        &bus,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    );

    let tools = reply["result"]["tools"]
        .as_array()
        .expect("a list of tools");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, vec!["execute_query", "refresh_catalog"]);

    // The schema is the one the internal assistant sees, and it has a single
    // field: there is nowhere for an agent to name another connection.
    let execute = &tools[0]["inputSchema"];
    let properties = execute["properties"]
        .as_object()
        .expect("the schema has properties");
    assert_eq!(properties.keys().collect::<Vec<_>>(), vec!["statement"]);
}

#[test]
fn a_tool_call_becomes_a_command_carrying_actor_agent() {
    // I-07: an agent's output never runs directly. It becomes a `Command` and
    // crosses the gate, exactly as the internal loop's does.
    let bus = Arc::new(Bus::completed());
    let mine = actor();
    let service = service(mine);
    let reply = answered(
        &service,
        &bus,
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"execute_query",
                      "arguments":{"statement":"SELECT count(*) FROM clients"}}}"#,
    );

    let commands = bus.commands();
    assert_eq!(commands.len(), 1);
    let (seen, command) = &commands[0];
    assert_eq!(*seen, mine, "the actor is the host's, not the agent's");
    assert!(seen.is_agent());
    assert_eq!(command.name(), "Execute");

    assert_eq!(reply["result"]["isError"], json!(false));
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .expect("a text block");
    assert!(text.contains("1 rows"), "{text}");
}

#[test]
fn the_scope_is_the_hosts_even_when_the_agent_sends_another() {
    // The agent cannot widen what it reaches: the schema has no field for a
    // connection, and `deny_unknown_fields` refuses the one it invents.
    let bus = Arc::new(Bus::completed());
    let service = service(actor());
    let reply = answered(
        &service,
        &bus,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"execute_query",
                      "arguments":{"statement":"SELECT 1",
                                   "connection":"the-production-one",
                                   "read_only":false}}}"#,
    );

    assert!(
        bus.commands().is_empty(),
        "nothing should reach the bus from a widened call"
    );
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .expect("a text block");
    assert!(text.contains("rejected"), "{text}");
}

#[test]
fn a_denied_write_is_told_to_the_agent_as_a_refusal() {
    // I-02: for an agent, a write on production is a refusal, not a
    // confirmation. The agent must read it as final, not retry it.
    let bus = Arc::new(Bus::answering(DispatchOutcome::Denied {
        reason: "an agent is requesting a write operation on \"billing\"".to_owned(),
    }));
    let service = service(actor());
    let reply = answered(
        &service,
        &bus,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"execute_query",
                      "arguments":{"statement":"DELETE FROM invoices"}}}"#,
    );

    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .expect("a text block");
    assert!(text.contains("denied"), "{text}");
    assert!(text.contains("billing"), "the connection is named: {text}");
    assert!(
        text.contains("Do not retry"),
        "a refusal must not read as a hint to rephrase: {text}"
    );
}

#[test]
fn an_unknown_tool_is_answered_not_run() {
    let bus = Arc::new(Bus::completed());
    let service = service(actor());
    let reply = answered(
        &service,
        &bus,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"name":"drop_everything","arguments":{}}}"#,
    );

    assert!(bus.commands().is_empty());
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .expect("a text block");
    assert!(text.contains("rejected"), "{text}");
}

#[test]
fn hostile_input_is_answered_and_never_panics() {
    // Every byte here comes from another process (I-09).
    let bus = Arc::new(Bus::completed());
    let service = service(actor());

    let broken = answered(&service, &bus, "{not json at all");
    assert_eq!(broken["error"]["code"], json!(code::PARSE_ERROR));

    let headless = answered(&service, &bus, r#"{"jsonrpc":"2.0","id":5}"#);
    assert_eq!(headless["error"]["code"], json!(code::INVALID_REQUEST));

    let unknown = answered(
        &service,
        &bus,
        r#"{"jsonrpc":"2.0","id":6,"method":"resources/read"}"#,
    );
    assert_eq!(unknown["error"]["code"], json!(code::METHOD_NOT_FOUND));

    let nameless = answered(
        &service,
        &bus,
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{}}"#,
    );
    assert_eq!(nameless["error"]["code"], json!(code::INVALID_PARAMS));

    assert!(
        bus.commands().is_empty(),
        "nothing ran from malformed input"
    );
}

#[test]
fn a_notification_is_not_answered() {
    // Answering a notification is itself a protocol error, and a client that
    // does it desynchronises the exchange.
    let bus = Arc::new(Bus::completed());
    let service = service(actor());
    assert!(
        ask(
            &service,
            &bus,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .is_none()
    );
    assert!(
        ask(
            &service,
            &bus,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}"#
        )
        .is_none()
    );
}

#[test]
fn the_revision_is_negotiated_not_imposed() {
    let bus = Arc::new(Bus::completed());
    let service = service(actor());

    // What the agent asks for, when we know it: a newer agent keeps its own.
    let known = answered(
        &service,
        &bus,
        r#"{"jsonrpc":"2.0","id":9,"method":"initialize",
            "params":{"protocolVersion":"2025-06-18"}}"#,
    );
    assert_eq!(known["result"]["protocolVersion"], json!("2025-06-18"));

    // A revision we do not know: ours, rather than an echo we cannot honour.
    let unknown = answered(
        &service,
        &bus,
        r#"{"jsonrpc":"2.0","id":10,"method":"initialize",
            "params":{"protocolVersion":"1999-01-01"}}"#,
    );
    assert_eq!(
        unknown["result"]["protocolVersion"],
        json!(SUPPORTED_VERSIONS[0])
    );

    // Tools, and only tools: announcing more would invite calls we refuse.
    let capabilities = unknown["result"]["capabilities"]
        .as_object()
        .expect("capabilities");
    assert_eq!(capabilities.keys().collect::<Vec<_>>(), vec!["tools"]);
}

#[test]
fn a_cancelled_conversation_runs_nothing_more() {
    let bus = Arc::new(Bus::completed());
    let service = service(actor());
    let cancel = CancelToken::new();
    cancel.cancel();

    let turns = ToolTurns::new(8, Arc::new(|| false));
    let _turn = turns.open(
        Arc::clone(&bus) as Arc<dyn CommandSink>,
        Arc::new(()),
        cancel,
    );
    let reply = block_on(service.respond(
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call",
            "params":{"name":"execute_query","arguments":{"statement":"SELECT 1"}}}"#,
        &turns,
    ))
    .expect("a request is answered");
    assert!(reply.contains("\"id\":11"), "{reply}");
}

#[test]
fn the_tools_handed_to_external_agents_are_frozen_here() {
    // The desktop hands an external agent `sql_agent().allowed_tools`. A tool
    // added there for the internal assistant would reach every external agent
    // without anyone reviewing the bridge: this test fails first, on purpose.
    // Changing it means a security review of ADR-0030, not an updated list.
    assert_eq!(
        crate::sql_agent().allowed_tools,
        vec![crate::tools::EXECUTE_QUERY.to_owned()]
    );
}

#[test]
fn the_tier_is_read_at_every_call_not_at_launch() {
    // I-04, word for word: the tier is attached to the connection, not to the
    // agent's session. A user who moves a client's database from `sampled` back
    // to `metadata` between two questions must stop sending the server's words
    // — which quote row values — to the agent still running.
    let bus = Arc::new(Bus::answering(DispatchOutcome::Failed {
        class: oxyn_core::ErrorClass::Permanent,
        message: "Key (email)=(dupont@example.com) already exists".to_owned(),
    }));
    let tier = crate::external::mcp::TierCell::holding(PrivacyTier::Sampled);
    let service = ToolService::new(
        ToolRegistry::builtin(),
        vec![crate::tools::EXECUTE_QUERY.to_owned()],
        ToolScope::new(ConnectionId::new(), SessionId::new(), QueryLanguage::SQL),
        Arc::clone(&tier) as Arc<dyn crate::external::mcp::TierSource>,
        actor(),
    );
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"execute_query","arguments":{"statement":"INSERT INTO clients VALUES (1)"}}}"#;
    let text = |reply: Value| {
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };

    let sampled = text(answered(&service, &bus, call));
    assert!(sampled.contains("dupont@example.com"), "{sampled}");

    tier.set(Some(PrivacyTier::Metadata));
    let metadata = text(answered(&service, &bus, call));
    assert!(!metadata.contains("dupont@example.com"), "{metadata}");

    // Local-only now: an external agent may not read it at all.
    tier.set(Some(PrivacyTier::Local));
    let local = answered(&service, &bus, call);
    assert_eq!(local["result"]["isError"], json!(true));
    assert!(text(local).contains("local-only"));

    // Deleted: nothing to act on.
    tier.set(None);
    assert!(text(answered(&service, &bus, call)).contains("no longer in the workspace"));

    assert_eq!(bus.commands().len(), 2, "only the two permitted calls ran");
}
