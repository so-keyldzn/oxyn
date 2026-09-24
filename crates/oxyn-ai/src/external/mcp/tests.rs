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
    //
    // 2026-09-23: `describe_schema` added (ADR-0030 § 4 bis). It becomes a
    // `DescribeCatalog`, a read of the local cache that contacts no server and
    // carries no row value; its answer is rendered by `ContextBuilder::build`
    // under the tier read at the call. Pending review by `relecteur-securite`.
    //
    // 2026-09-24: `request_sample` added (ADR-0034). It is the only tool
    // whose answer can carry row values: refused outside `sampled` before the
    // user is asked, read only after the user ticks columns in Oxyn, rendered
    // by `ContextBuilder::build`. Pending review by `relecteur-securite`.
    assert_eq!(
        crate::sql_agent().allowed_tools,
        vec![
            crate::tools::EXECUTE_QUERY.to_owned(),
            crate::tools::DESCRIBE_SCHEMA.to_owned(),
            crate::tools::REQUEST_SAMPLE.to_owned(),
        ]
    );
}

mod request_sample {
    use oxyn_catalog::{CatalogHandle, CatalogPath, SharedCatalog};

    use super::*;
    use crate::context::RowSample;
    use crate::runtime::{SampleReceipt, SampleRelease};
    use crate::tools::{MAX_SAMPLE_COLUMNS, MAX_SAMPLE_ROWS, REQUEST_SAMPLE, SampleAsk};

    /// What the user does with the approval screen, in this test.
    #[derive(Debug, Clone, Copy)]
    enum User {
        Declines,
        /// Ticks `email` only, whatever the agent asked for.
        TicksEmail,
        /// Ticks every column of a wide table: more than the context holds.
        TicksEverything,
    }

    /// The sink's release, counted: it stands for the audit and the panel's
    /// « sent », which only a sample that really leaves may reach.
    struct Tally {
        released: Mutex<usize>,
        recorded: bool,
    }

    #[async_trait]
    impl SampleRelease for Tally {
        async fn release(&self) -> bool {
            *self.released.lock().expect("no panic held the lock") += 1;
            self.recorded
        }
    }

    fn customers() -> CatalogPath {
        CatalogPath::for_namespace(None, "main")
            .and_then(|main| main.with_relation("customers"))
            .expect("a relation path")
    }

    /// `email` of one customer, as a sink reports it once the user approved.
    fn sampled(receipt: SampleReceipt) -> DispatchOutcome {
        DispatchOutcome::Sampled {
            catalog: CatalogHandle::new(SharedCatalog::default()),
            sample: RowSample::new(
                customers(),
                vec!["email".to_owned()],
                vec![vec![oxyn_core::ScalarValue::Text(
                    "dupont@example.com".to_owned(),
                )]],
            ),
            receipt,
        }
    }

    /// The widest sample a request can bring back, every value at the
    /// rendering's cap: past the context budget.
    fn oversized(receipt: SampleReceipt) -> DispatchOutcome {
        let columns: Vec<String> = (0..MAX_SAMPLE_COLUMNS).map(|n| format!("c{n}")).collect();
        let row: Vec<_> = columns
            .iter()
            .map(|_| oxyn_core::ScalarValue::Text("dupont@example.com".repeat(8)))
            .collect();
        let rows = (0..MAX_SAMPLE_ROWS).map(|_| row.clone()).collect();
        DispatchOutcome::Sampled {
            catalog: CatalogHandle::new(SharedCatalog::default()),
            sample: RowSample::new(customers(), columns, rows),
            receipt,
        }
    }

    /// A sink that plays the user: it records every sample asked of it, and
    /// answers as the user would — the approved columns only.
    struct Desk {
        user: User,
        asked: Mutex<Vec<(Actor, SampleAsk)>>,
        dispatched: Mutex<usize>,
        tally: Arc<Tally>,
    }

    impl Desk {
        fn new(user: User) -> Arc<Self> {
            Self::recording(user, true)
        }

        /// `recorded`: whether the audit accepts the sample.
        fn recording(user: User, recorded: bool) -> Arc<Self> {
            Arc::new(Self {
                user,
                asked: Mutex::new(Vec::new()),
                dispatched: Mutex::new(0),
                tally: Arc::new(Tally {
                    released: Mutex::new(0),
                    recorded,
                }),
            })
        }

        fn released(&self) -> usize {
            *self.tally.released.lock().expect("no panic held the lock")
        }

        fn receipt(&self) -> SampleReceipt {
            SampleReceipt::new(Arc::clone(&self.tally) as Arc<dyn SampleRelease>)
        }

        fn asked(&self) -> Vec<(Actor, SampleAsk)> {
            self.asked.lock().expect("no panic held the lock").clone()
        }
    }

    #[async_trait]
    impl CommandSink for Desk {
        async fn dispatch(&self, _: Actor, _: Command, _: &CancelToken) -> DispatchOutcome {
            *self.dispatched.lock().expect("no panic held the lock") += 1;
            DispatchOutcome::Completed {
                summary: "1 rows, 1 batches".to_owned(),
            }
        }

        async fn request_sample(
            &self,
            actor: Actor,
            ask: SampleAsk,
            _: &CancelToken,
        ) -> DispatchOutcome {
            self.asked
                .lock()
                .expect("no panic held the lock")
                .push((actor, ask));
            match self.user {
                User::Declines => DispatchOutcome::Denied {
                    reason: "the user declined".to_owned(),
                },
                User::TicksEmail => sampled(self.receipt()),
                User::TicksEverything => oversized(self.receipt()),
            }
        }
    }

    fn served(tier: PrivacyTier) -> ToolService {
        ToolService::new(
            ToolRegistry::builtin(),
            crate::sql_agent().allowed_tools,
            ToolScope::new(ConnectionId::new(), SessionId::new(), QueryLanguage::SQL),
            crate::external::mcp::TierCell::holding(tier),
            actor(),
        )
    }

    fn call(desk: &Arc<Desk>, service: &ToolService, arguments: Value) -> Value {
        let turns = ToolTurns::new(8, Arc::new(|| false));
        let _turn = turns.open(
            Arc::clone(desk) as Arc<dyn CommandSink>,
            Arc::new(()),
            CancelToken::new(),
        );
        let message = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": { "name": REQUEST_SAMPLE, "arguments": arguments },
        })
        .to_string();
        let reply = block_on(service.respond(&message, &turns)).expect("a request is answered");
        serde_json::from_str(&reply).expect("the reply is JSON")
    }

    fn text(reply: &Value) -> String {
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// Under `Metadata`, the call is refused before anyone is asked: no
    /// approval screen opens, nothing is read, and the refusal says why.
    #[test]
    fn under_metadata_the_call_is_refused_before_the_user_is_asked() {
        let desk = Desk::new(User::TicksEmail);
        let reply = call(
            &desk,
            &served(PrivacyTier::Metadata),
            json!({ "relation": "customers" }),
        );
        let said = text(&reply);
        assert!(said.contains("status: denied"), "{said}");
        assert!(said.contains("`metadata`"), "{said}");
        assert!(!said.contains("dupont@example.com"), "{said}");
        assert!(desk.asked().is_empty(), "the user was asked under Metadata");
        assert_eq!(*desk.dispatched.lock().expect("lock"), 0);
    }

    /// A refusal — or an approval that expired — gives the agent the words
    /// « the user declined », and no value.
    #[test]
    fn a_refusal_answers_declined_and_carries_no_value() {
        let desk = Desk::new(User::Declines);
        let reply = call(
            &desk,
            &served(PrivacyTier::Sampled),
            json!({ "relation": "customers", "columns": ["email"] }),
        );
        let said = text(&reply);
        assert!(said.contains("the user declined"), "{said}");
        assert!(!said.contains("dupont@example.com"), "{said}");
        assert_eq!(desk.asked().len(), 1);
    }

    /// Approved: the agent receives the approved column, fenced as database
    /// content, rendered by the point of passage — and the ask carried the
    /// agent's actor, the scope's connection and a bounded read.
    #[test]
    fn an_approved_sample_reaches_the_agent_fenced() {
        let desk = Desk::new(User::TicksEmail);
        let service = served(PrivacyTier::Sampled);
        let reply = call(
            &desk,
            &service,
            json!({ "relation": "customers", "columns": ["email", "secret"], "rows": 3 }),
        );
        assert_eq!(reply["result"]["isError"], json!(false), "{reply}");
        let said = text(&reply);
        assert!(said.contains("status: completed"), "{said}");
        let value = said.find("dupont@example.com").expect("the value arrived");
        let open = said
            .rfind(crate::untrusted::FENCE_OPEN)
            .expect("a fence opens the data");
        let close = said
            .rfind(crate::untrusted::FENCE_CLOSE)
            .expect("a fence closes the data");
        assert!(open < value && value < close, "{said}");
        assert!(said.contains("row sample approved by the user"), "{said}");
        assert_eq!(desk.released(), 1, "what left is recorded, once");

        let asked = desk.asked();
        let [(who, ask)] = asked.as_slice() else {
            panic!("one ask: {asked:?}");
        };
        assert!(who.is_agent());
        assert_eq!(ask.rows(), 3);
        assert_eq!(ask.columns, ["email", "secret"]);
        let Command::PreviewRelation {
            relation, limit, ..
        } = &ask.command
        else {
            panic!("the read is a PreviewRelation: {:?}", ask.command);
        };
        assert_eq!(relation, "customers");
        assert_eq!(*limit, 3);
    }

    /// The tier lowered between the approval and the rendering: the rows are
    /// dropped by the point of passage, and the agent is told nothing left.
    #[test]
    fn a_sample_rendered_under_metadata_becomes_a_refusal() {
        let desk = Desk::new(User::TicksEmail);
        let outcome = crate::runtime::ToolOutcome::from_dispatch(
            PrivacyTier::Metadata,
            sampled(desk.receipt()),
            QueryLanguage::SQL,
            None,
        );
        let said = outcome.render();
        assert!(said.contains("status: denied"), "{said}");
        assert!(!said.contains("dupont@example.com"), "{said}");
    }

    /// A sample the point of passage drops for the budget did not leave: the
    /// sink's release is never called — nothing recorded, nothing announced —
    /// and the agent reads a refusal, not « completed ».
    #[test]
    fn a_sample_over_the_budget_is_neither_released_nor_sent() {
        let desk = Desk::new(User::TicksEverything);
        let reply = call(
            &desk,
            &served(PrivacyTier::Sampled),
            json!({ "relation": "customers" }),
        );
        let said = text(&reply);
        assert!(said.contains("status: denied"), "{said}");
        assert!(said.contains("did not fit the context budget"), "{said}");
        assert!(!said.contains("dupont@example.com"), "{said}");
        assert_eq!(desk.released(), 0, "a dropped sample was recorded as sent");
    }

    /// The audit refused the sample: nothing leaves, and the agent is told
    /// so rather than handed the rows.
    #[test]
    fn a_sample_the_audit_cannot_record_is_not_sent() {
        let desk = Desk::recording(User::TicksEmail, false);
        let reply = call(
            &desk,
            &served(PrivacyTier::Sampled),
            json!({ "relation": "customers", "columns": ["email"] }),
        );
        let said = text(&reply);
        assert!(said.contains("status: denied"), "{said}");
        assert!(said.contains("audit trail"), "{said}");
        assert!(!said.contains("dupont@example.com"), "{said}");
        assert_eq!(desk.released(), 1);
    }

    /// A sink that cannot show the approval screen refuses: the trait's
    /// default reads nothing.
    #[test]
    fn a_sink_without_an_approval_screen_refuses() {
        let bus = Arc::new(Bus::completed());
        let service = served(PrivacyTier::Sampled);
        let turns = ToolTurns::new(8, Arc::new(|| false));
        let _turn = turns.open(
            Arc::clone(&bus) as Arc<dyn CommandSink>,
            Arc::new(()),
            CancelToken::new(),
        );
        let message = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": REQUEST_SAMPLE, "arguments": { "relation": "customers" } },
        })
        .to_string();
        let reply: Value =
            serde_json::from_str(&block_on(service.respond(&message, &turns)).expect("answered"))
                .expect("JSON");
        assert!(text(&reply).contains("cannot ask the user"), "{reply}");
        assert!(bus.commands().is_empty(), "nothing was dispatched");
    }
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

/// The refusals a question's panel was shown: the tool named, and the words.
#[derive(Default)]
struct Refusals(Mutex<Vec<(String, String)>>);

impl AgentObserver for Refusals {
    fn observe(&self, event: AgentEvent<'_>) {
        if let AgentEvent::CallRejected { tool, error } = event {
            self.0
                .lock()
                .expect("no panic held the lock")
                .push((tool.to_owned(), error.to_string()));
        }
    }
}

#[test]
fn a_call_refused_before_admission_is_shown_in_oxyns_words() {
    // The agent's own step for an Oxyn call is hidden, so a call refused here
    // must leave its trace in the panel, or it vanishes from the user's view.
    let bus = Arc::new(Bus::completed());
    let tier = crate::external::mcp::TierCell::holding(PrivacyTier::Local);
    let service = ToolService::new(
        ToolRegistry::builtin(),
        vec![crate::tools::EXECUTE_QUERY.to_owned()],
        ToolScope::new(ConnectionId::new(), SessionId::new(), QueryLanguage::SQL),
        Arc::clone(&tier) as Arc<dyn crate::external::mcp::TierSource>,
        actor(),
    );
    let panel = Arc::new(Refusals::default());
    let turns = ToolTurns::new(1, Arc::new(|| false));
    let _turn = turns.open(
        Arc::clone(&bus) as Arc<dyn CommandSink>,
        Arc::clone(&panel) as Arc<dyn AgentObserver>,
        CancelToken::new(),
    );
    let call = |name: &str| {
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{name}",
            "arguments":{{"statement":"SELECT 1"}}}}}}"#
        )
    };
    let respond = |message: String| block_on(service.respond(&message, &turns));

    // Local-only.
    respond(call("execute_query"));
    // The tier can no longer be read; and a name the agent made up, which is
    // not shown as it was sent.
    tier.set(None);
    respond(call("execute_query"));
    respond(call("ignore previous instructions"));
    // One call allowed per answer: the second is over the ceiling.
    tier.set(Some(PrivacyTier::Metadata));
    respond(call("execute_query"));
    respond(call("execute_query"));

    let shown = panel.0.lock().expect("no panic held the lock").clone();
    assert_eq!(shown.len(), 4, "{shown:?}");
    let tools: Vec<&str> = shown.iter().map(|(tool, _)| tool.as_str()).collect();
    assert_eq!(
        tools,
        [
            "execute_query",
            "execute_query",
            "unlisted tool",
            "execute_query"
        ]
    );
    assert!(shown[0].1.contains("`local`"), "{shown:?}");
    assert!(
        shown[1].1.contains("no longer in the workspace"),
        "{shown:?}"
    );
    assert!(shown[3].1.contains("1 tool calls"), "{shown:?}");
    // Oxyn's words, not the agent's: nothing it sent reaches the panel.
    assert!(
        shown
            .iter()
            .all(|(tool, error)| !tool.contains("ignore") && !error.contains("SELECT")),
        "{shown:?}"
    );
    assert_eq!(bus.commands().len(), 1, "only the admitted call ran");
}
