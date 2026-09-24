//! An agent asks for a sample; an approved sample reaches an external agent.
//!
//! On the fixture's real SQLite rows, executor, `PolicyGate`, catalog and
//! audit ([ADR-0034](../../../../../../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
//! Only the model and the agent's process are scripted.

use oxyn_ai::external::mcp::{ToolService, ToolTurns};
use oxyn_ai::external::session::ExternalSession;

use super::*;
use crate::backend::ai::conversation::sampling::Sampling;
use crate::backend::ai::samples::Offer;
use crate::backend::ai::threads::{AgentLink, Waiting, WithdrawOnRelease};

pub(super) const SQLITE: QueryLanguage = QueryLanguage::Sql(SqlDialect::Sqlite);

/// A sink for `node`, able to put a request before the user, as the one
/// `ask_agent` or `converse` builds.
pub(super) fn sink(
    fixture: &Fixture,
    thread: &Arc<Thread>,
    node: u32,
    (identity, conversation): (AgentId, AgentSessionId),
    recipient: Recipient,
    who: &str,
) -> AgentSink {
    AgentSink {
        sink: ExecutorSink::for_agent(
            Arc::clone(&fixture.backend.inner.executor),
            identity,
            conversation,
        ),
        executor: Arc::clone(&fixture.backend.inner.executor),
        decisions: Arc::clone(&fixture.backend.inner.ai.decisions),
        thread: Arc::clone(thread),
        node,
        question: QuestionOpen::new(),
        sampling: Some(Sampling::new(
            Arc::clone(&fixture.backend.inner.ai.asks),
            recipient,
            who.to_owned(),
            who.to_owned(),
        )),
    }
}

/// The MCP bridge an external agent reaches, with its question open on
/// `sink`: the tier is read from the store at every call.
pub(super) struct Bridge {
    service: Arc<ToolService>,
    turns: ToolTurns,
    _turn: oxyn_ai::external::mcp::OpenTurn,
}

pub(super) fn bridge(
    fixture: &Fixture,
    actor: (AgentId, AgentSessionId),
    sink: AgentSink,
) -> Bridge {
    let service = Arc::new(ToolService::new(
        oxyn_ai::ToolRegistry::builtin(),
        oxyn_ai::sql_agent().allowed_tools,
        oxyn_ai::ToolScope::new(fixture.connection, fixture.session, SQLITE),
        Arc::new(fixture.stored()),
        Actor::agent(actor.0, actor.1),
    ));
    let turns = ToolTurns::new(8, Arc::new(|| false));
    let turn = turns.open(Arc::new(sink), Arc::new(()), CancelToken::new());
    Bridge {
        service,
        turns,
        _turn: turn,
    }
}

impl Bridge {
    /// Sends `request_sample` with `arguments`, without waiting for the
    /// answer: the call waits for the user.
    pub(super) fn ask(
        &self,
        fixture: &Fixture,
        arguments: serde_json::Value,
    ) -> tokio::task::JoinHandle<Option<String>> {
        let call = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": oxyn_ai::tools::REQUEST_SAMPLE, "arguments": arguments },
        })
        .to_string();
        let service = Arc::clone(&self.service);
        let turns = self.turns.clone();
        fixture
            .runtime
            .spawn(async move { service.respond(&call, &turns).await })
    }
}

/// What the tool answered, as the agent reads it.
pub(super) fn text_of(
    fixture: &Fixture,
    pending: tokio::task::JoinHandle<Option<String>>,
) -> String {
    let reply: serde_json::Value = serde_json::from_str(
        &fixture
            .runtime
            .block_on(pending)
            .expect("the call ends")
            .expect("a request is answered"),
    )
    .expect("the reply is JSON");
    reply["result"]["content"][0]["text"]
        .as_str()
        .expect("a text block")
        .to_owned()
}

/// The request the approval screen shows, once the backend opened it.
pub(super) fn screen(fixture: &Fixture, received: &Arc<Mutex<Vec<String>>>) -> serde_json::Value {
    for _ in 0..400 {
        let shown = received.lock().iter().find_map(|json| {
            let update: serde_json::Value = serde_json::from_str(json).ok()?;
            (update["event"]["kind"] == "sampleRequested")
                .then(|| update["event"]["request"].clone())
        });
        if let Some(request) = shown {
            return request;
        }
        fixture
            .runtime
            .block_on(tokio::time::sleep(std::time::Duration::from_millis(25)));
    }
    panic!("no screen opened: {}", received.lock().join("\n"));
}

pub(super) fn open_question(
    fixture: &Fixture,
    parent: Option<u32>,
) -> (Arc<Thread>, u32, Arc<Mutex<Vec<String>>>) {
    let thread = fixture
        .backend
        .inner
        .ai
        .thread_for(fixture.connection, None)
        .expect("a conversation");
    let (channel, received) = recording();
    let node = fixture.begin(&thread, parent, channel);
    (thread, node, received)
}

/// Nothing is read and nothing is recorded before the user answers; then only
/// the ticked column reaches the agent, over MCP, fenced, audited, and the
/// exchange is marked as leaving no memory.
#[test]
fn an_external_agent_receives_only_the_columns_the_user_ticked_and_only_after() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let declared = agent("unused");
    let actor = (AgentId::new(), AgentSessionId::new());
    let (thread, node, received) = open_question(&fixture, None);
    let bridge = bridge(
        &fixture,
        actor,
        sink(
            &fixture,
            &thread,
            node,
            actor,
            Recipient::agent(&declared),
            "Codex",
        ),
    );

    let pending = bridge.ask(
        &fixture,
        serde_json::json!({ "relation": "customers", "columns": ["email", "secret"], "rows": 3 }),
    );
    let request = screen(&fixture, &received);
    assert_eq!(fixture.reads(), 0, "nothing is read before the answer");
    assert!(
        fixture.egress().is_empty(),
        "nothing left before the answer"
    );
    assert!(!thread.is_withheld(node));
    assert_eq!(request["requestedBy"], "Codex", "the screen names who asks");
    assert_eq!(request["rows"], 3);
    assert_eq!(request["reach"], "unresolved");
    let offered: Vec<&str> = request["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(
        offered,
        ["email", "secret"],
        "what the agent asked, no more"
    );

    let id = request["id"].as_str().expect("an id").to_owned();
    fixture
        .backend
        .ai_answer_sample(fixture.connection, &id, Some(&["email".to_owned()]))
        .map_err(|error| error.message)
        .expect("answered");
    let said = text_of(&fixture, pending);
    assert!(said.contains("status: completed"), "{said}");
    assert_eq!(said.matches("@example.com").count(), 3, "{said}");
    assert!(!said.contains("SECRET-"), "an unticked column left: {said}");
    let value = said.find("user1@example.com").expect("a value arrived");
    let fence = said
        .rfind(oxyn_ai::untrusted::FENCE_OPEN)
        .expect("fenced as database content");
    assert!(fence < value, "{said}");

    assert!(fixture.reads() > 0, "the audit shows the real read");
    let recorded = fixture.egress();
    let [entry] = recorded.as_slice() else {
        panic!("one send, one entry: {recorded:?}");
    };
    assert_eq!(entry.columns, ["email"]);
    assert_eq!(entry.rows, 3);
    assert_eq!(entry.recipient, declared.id);
    assert_eq!(entry.reach, EgressReach::Unresolved);
    assert_eq!(entry.model, None, "an agent's model is not invented");
    assert!(thread.is_withheld(node), "the exchange leaves no memory");

    let events = received.lock().join("\n");
    assert!(
        events.contains(r#""kind":"sampleAnswered""#) && events.contains(r#""approved":true"#),
        "{events}"
    );
    assert!(
        events.contains(r#""kind":"sampleApproved","rows":3,"columns":1"#),
        "{events}"
    );
    assert!(
        !serde_json::to_string(&thread.view(None))
            .expect("serializable")
            .contains("example.com"),
        "a value is kept in the conversation"
    );
    // Spent: a second answer approves nothing.
    assert!(
        fixture
            .backend
            .ai_answer_sample(fixture.connection, &id, Some(&["email".to_owned()]))
            .is_err()
    );
}

/// A refusal reads nothing, records nothing, and tells the agent « the user
/// declined » — with no value, and nothing withheld.
#[test]
fn a_declined_request_reads_nothing_and_says_so() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let declared = agent("unused");
    let actor = (AgentId::new(), AgentSessionId::new());
    let (thread, node, received) = open_question(&fixture, None);
    let bridge = bridge(
        &fixture,
        actor,
        sink(
            &fixture,
            &thread,
            node,
            actor,
            Recipient::agent(&declared),
            "Codex",
        ),
    );

    let pending = bridge.ask(&fixture, serde_json::json!({ "relation": "customers" }));
    let request = screen(&fixture, &received);
    fixture
        .backend
        .ai_answer_sample(
            fixture.connection,
            request["id"].as_str().expect("an id"),
            None,
        )
        .map_err(|error| error.message)
        .expect("declined");
    let said = text_of(&fixture, pending);
    assert!(said.contains("the user declined"), "{said}");
    assert!(!said.contains("example.com"), "{said}");
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
    assert!(!thread.is_withheld(node));
    assert!(
        received.lock().join("\n").contains(r#""approved":false"#),
        "the screen closes"
    );
}

/// Under `Metadata`: refused before any screen, whether the tier is read by
/// the bridge at the call or by the sink itself — the internal loop's tier is
/// the question's, and the sink reads the store again.
#[test]
fn under_metadata_no_screen_opens_and_nothing_is_read() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    fixture
        .runtime
        .block_on(fixture.backend.update_connection(
            CommandId::new(),
            fixture.connection,
            edited(PrivacyTier::Metadata),
        ))
        .expect("metadata");
    let declared = agent("unused");
    let actor = (AgentId::new(), AgentSessionId::new());
    let (thread, node, received) = open_question(&fixture, None);

    let bridge = bridge(
        &fixture,
        actor,
        sink(
            &fixture,
            &thread,
            node,
            actor,
            Recipient::agent(&declared),
            "Codex",
        ),
    );
    let said = text_of(
        &fixture,
        bridge.ask(&fixture, serde_json::json!({ "relation": "customers" })),
    );
    assert!(
        said.contains("status: denied") && said.contains("`metadata`"),
        "{said}"
    );

    // The sink alone, as the internal loop reaches it with a stale tier.
    let ask = oxyn_ai::ToolRegistry::builtin()
        .request(
            &oxyn_llm::ToolCall::new(
                "call-1",
                oxyn_ai::tools::REQUEST_SAMPLE,
                serde_json::json!({ "relation": "customers" }),
            ),
            &oxyn_ai::sql_agent().allowed_tools,
            &oxyn_ai::ToolScope::new(fixture.connection, fixture.session, SQLITE),
        )
        .expect("a valid call");
    let oxyn_ai::tools::ToolRequest::Sample(ask) = ask else {
        panic!("a sample request");
    };
    let outcome = fixture.runtime.block_on(
        sink(
            &fixture,
            &thread,
            node,
            actor,
            Recipient::agent(&declared),
            "Codex",
        )
        .request_sample(Actor::agent(actor.0, actor.1), ask, &CancelToken::new()),
    );
    assert!(
        matches!(&outcome, DispatchOutcome::Denied { reason } if reason.contains("`sampled`")),
        "{outcome:?}"
    );
    assert!(
        !received.lock().join("\n").contains("sampleRequested"),
        "no screen opened"
    );
    assert_eq!(fixture.reads(), 0);
}

/// I-10: a relation and a column named to break out of their quotes are
/// read as names. The driver quotes the relation, the column is never in the
/// statement, and `customers` survives the click.
#[test]
fn a_hostile_name_is_quoted_by_the_driver_never_concatenated() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let relation = r#"evil"; DROP TABLE customers; --"#;
    let column = r#"c"; DROP TABLE customers; --"#;
    let quoted = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    for statement in [
        format!(
            "CREATE TABLE {} ({} TEXT)",
            quoted(relation),
            quoted(column)
        ),
        format!("INSERT INTO {} VALUES ('HOSTILE-VALUE')", quoted(relation)),
    ] {
        fixture
            .runtime
            .block_on(fixture.backend.execute(
                CommandId::new(),
                fixture.connection,
                fixture.session,
                statement,
            ))
            .expect("SQLite accepts these names");
    }
    // Listed and described, as the explorer does.
    let tree = fixture
        .backend
        .catalog_tree(fixture.connection)
        .expect("tree");
    let main = find(&tree, "main").expect("the namespace").address;
    fixture
        .runtime
        .block_on(fixture.backend.refresh_catalog(
            CommandId::new(),
            fixture.connection,
            fixture.session,
            Some(main),
        ))
        .expect("the namespace");
    let tree = fixture
        .backend
        .catalog_tree(fixture.connection)
        .expect("tree");
    let hostile = find(&tree, relation).expect("listed").address;
    fixture
        .runtime
        .block_on(fixture.backend.refresh_catalog(
            CommandId::new(),
            fixture.connection,
            fixture.session,
            Some(hostile),
        ))
        .expect("described");

    let declared = agent("unused");
    let actor = (AgentId::new(), AgentSessionId::new());
    let (thread, node, received) = open_question(&fixture, None);
    let bridge = bridge(
        &fixture,
        actor,
        sink(
            &fixture,
            &thread,
            node,
            actor,
            Recipient::agent(&declared),
            "Codex",
        ),
    );

    // A name the catalog does not know is refused before any screen.
    let unknown = text_of(
        &fixture,
        bridge.ask(
            &fixture,
            serde_json::json!({ "relation": relation, "columns": ["c; DROP TABLE t"] }),
        ),
    );
    assert!(unknown.contains("is not a column"), "{unknown}");

    let pending = bridge.ask(
        &fixture,
        serde_json::json!({ "relation": relation, "columns": [column] }),
    );
    let request = screen(&fixture, &received);
    fixture
        .backend
        .ai_answer_sample(
            fixture.connection,
            request["id"].as_str().expect("an id"),
            Some(&[column.to_owned()]),
        )
        .map_err(|error| error.message)
        .expect("answered");
    let said = text_of(&fixture, pending);
    assert!(said.contains("HOSTILE-VALUE"), "{said}");

    // The read went to the bus as a command carrying the name as a field,
    // under the agent: Oxyn composed no statement text for it — the driver
    // did, quoting the relation, and the column never enters it.
    let previews: Vec<_> = fixture
        .backend
        .inner
        .executor
        .store()
        .journal()
        .recent(256)
        .expect("journal")
        .into_iter()
        .filter(|entry| entry.record.command_kind == "PreviewRelation")
        .collect();
    assert!(!previews.is_empty(), "the read is journaled");
    assert!(
        previews
            .iter()
            .all(|entry| entry.record.statement.is_none() && entry.record.actor_id.is_some()),
        "a field, not a text, and the agent's"
    );
    fixture
        .runtime
        .block_on(fixture.backend.execute(
            CommandId::new(),
            fixture.connection,
            fixture.session,
            "SELECT count(*) FROM customers".to_owned(),
        ))
        .expect("customers survived the click");
}

/// The internal assistant asks too, through the same sink: approved, the
/// rows reach the model's conversation; the next question starts without
/// that memory, and says why.
#[test]
fn after_a_sample_the_assistant_answers_the_next_question_without_memory() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let config = fixture.config();
    let provider = ScriptedProvider::new(vec![
        vec![
            oxyn_llm::ChatEvent::ToolCallComplete(oxyn_llm::ToolCall::new(
                "call-1",
                oxyn_ai::tools::REQUEST_SAMPLE,
                serde_json::json!({ "relation": "customers", "columns": ["email"] }),
            )),
            oxyn_llm::ChatEvent::Done {
                stop_reason: oxyn_llm::StopReason::ToolCalls,
            },
        ],
        vec![
            oxyn_llm::ChatEvent::TextDelta("Lowercase addresses.".to_owned()),
            oxyn_llm::ChatEvent::Done {
                stop_reason: oxyn_llm::StopReason::EndTurn,
            },
        ],
    ]);
    let spec = oxyn_ai::sql_agent();
    let engine = oxyn_ai::AgentRuntime::new(
        spec.clone(),
        provider,
        oxyn_llm::Reach::Local,
        oxyn_ai::ToolRegistry::builtin(),
        "llama3",
    )
    .expect("a valid declaration");
    let scope = oxyn_ai::ToolScope::new(fixture.connection, fixture.session, SQLITE);
    let context =
        oxyn_ai::ContextBuilder::new(&oxyn_catalog::CatalogCache::new(), config.privacy_tier)
            .build();
    let mut session = oxyn_ai::AgentSession::new(&spec, &context, scope);
    session.ask("how are emails written?");
    let (thread, node, received) = open_question(&fixture, None);
    let sink = sink(
        &fixture,
        &thread,
        node,
        (spec.id, session.id()),
        fixture.local(),
        "Local model · llama3",
    );
    let observer = Observer {
        thread: Arc::clone(&thread),
        node,
    };
    let running = fixture.runtime.spawn(async move {
        engine
            .run(&mut session, &sink, &observer, &CancelToken::new())
            .await
            .map(|_| session)
    });
    let request = screen(&fixture, &received);
    fixture
        .backend
        .ai_answer_sample(
            fixture.connection,
            request["id"].as_str().expect("an id"),
            Some(&["email".to_owned()]),
        )
        .map_err(|error| error.message)
        .expect("answered");
    let session = fixture
        .runtime
        .block_on(running)
        .expect("joined")
        .expect("the conversation runs");
    let tool = session
        .messages()
        .iter()
        .rev()
        .find(|message| message.role == oxyn_llm::Role::Tool)
        .expect("the tool result reached the conversation")
        .content
        .clone();
    assert!(tool.contains("user1@example.com"), "{tool}");
    assert!(!tool.contains("SECRET-"), "{tool}");
    assert!(thread.is_withheld(node));
    assert!(
        thread.memory_from(Some(node)).is_none(),
        "no session remembered after it"
    );
    thread.finish(node);

    // The next question, following that answer.
    let (channel, next_events) = recording();
    let next = fixture.begin(&thread, Some(node), channel);
    let cancel = CancelToken::new();
    let run = Run {
        mentions: &crate::backend::ai::mentions::NO_MENTIONS,
        inner: &fixture.backend.inner,
        thread: &thread,
        node: next,
        parent: Some(node),
        connection: &config,
        cancel: &cancel,
    };
    let (dialogue, _) = run
        .prepare_dialogue(
            &spec,
            fixture.session,
            "and the domains?",
            None,
            PrivacyTier::Sampled,
        )
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    let prompt = prompt_of(&dialogue);
    assert!(!prompt.contains("example.com"), "{prompt}");
    assert!(
        next_events
            .lock()
            .join("\n")
            .contains(r#""reason":"sampleNotKept""#),
        "the panel says why"
    );
}

/// An ACP agent that records every prompt it is sent, as text.
pub(super) fn scripted_agent(fixture: &Fixture) -> (ExternalSession, Arc<Mutex<Vec<String>>>) {
    use agent_client_protocol::schema::v1::{
        ContentBlock, InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse,
        PromptRequest, PromptResponse, SessionId as AcpSession, StopReason,
    };
    use agent_client_protocol::{Agent, Channel as Duplex};
    use futures::FutureExt as _;

    let prompts = Arc::new(Mutex::new(Vec::<String>::new()));
    let kept = Arc::clone(&prompts);
    let (ours, theirs) = Duplex::duplex();
    let (session, driver) = ExternalSession::over(ours, std::env::temp_dir().join("unused"));
    fixture.runtime.spawn(driver);
    fixture.runtime.spawn(
        Agent
            .builder()
            .on_receive_request(
                async |request: InitializeRequest, responder, _cx| {
                    responder.respond(InitializeResponse::new(request.protocol_version))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_: NewSessionRequest, responder, _cx| {
                    responder.respond(NewSessionResponse::new(AcpSession::new("s")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: PromptRequest, responder, _cx| {
                    let text: Vec<String> = request
                        .prompt
                        .into_iter()
                        .filter_map(|block| match block {
                            ContentBlock::Text(text) => Some(text.text),
                            _ => None,
                        })
                        .collect();
                    kept.lock().push(text.join("\n"));
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(theirs)
            .boxed(),
    );
    (session, prompts)
}

/// Keeps `session` for the connection's next question, as the panel's start
/// leaves it.
pub(super) fn waiting(fixture: &Fixture, declared: &ExternalAgentConfig, session: ExternalSession) {
    fixture.backend.inner.ai.wait(
        fixture.connection,
        Waiting {
            link: AgentLink {
                agent: declared.clone(),
                tier: PrivacyTier::Sampled,
                leaf: None,
                session: Arc::new(session),
                tools: ToolTurns::new(8, Arc::new(|| false)),
                actor: (AgentId::new(), AgentSessionId::new()),
                _requests: WithdrawOnRelease::new(
                    Arc::clone(&fixture.backend.inner.executor),
                    Arc::clone(&fixture.backend.inner.ai.decisions),
                    Actor::agent(AgentId::new(), AgentSessionId::new()),
                ),
            },
            session: fixture.session,
            stop: CancelToken::new(),
        },
    );
}

/// ADR-0034, part A: the sample the user approved for an external agent
/// enters its opening prompt through the `ContextBuilder`; the process that
/// saw it is released, and the next question starts another, which never
/// sees the rows — and the panel says why.
#[test]
fn an_approved_sample_reaches_an_external_agent_once_then_the_agent_starts_over() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let config = fixture.config();
    let declared = agent("unused");
    let (first_agent, first_prompts) = scripted_agent(&fixture);
    waiting(&fixture, &declared, first_agent);

    let thread = fixture
        .backend
        .inner
        .ai
        .thread_for(fixture.connection, None)
        .expect("a conversation");
    let source = fixture.customers.to_path().expect("a relation path");
    let token = fixture.backend.inner.ai.samples.issue(Offer {
        connection: fixture.connection,
        thread: None,
        parent: None,
        source,
        offered: vec!["id".to_owned(), "email".to_owned(), "secret".to_owned()],
        recipient: Recipient::agent(&declared),
    });
    let request = SampleRequest {
        id: token,
        requested_by: None,
        source: String::new(),
        address: fixture.customers.clone(),
        rows: 5,
        fields: Vec::new(),
        destination: declared.label.clone(),
        reach: Reach::Unresolved.into(),
    };

    let (channel, first_events) = recording();
    let first = fixture.begin(&thread, None, channel);
    let cancel = CancelToken::new();
    let run = Run {
        mentions: &crate::backend::ai::mentions::NO_MENTIONS,
        inner: &fixture.backend.inner,
        thread: &thread,
        node: first,
        parent: None,
        connection: &config,
        cancel: &cancel,
    };
    let grant = fixture.backend.inner.ai.samples.take(&request.id);
    let sample = fixture
        .runtime
        .block_on(run.take_sample(
            fixture.session,
            &approval(&request, &["email"]),
            grant,
            None,
            &Recipient::agent(&declared),
            &fixture.stored(),
        ))
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    fixture
        .runtime
        .block_on(run.ask_agent(
            fixture.session,
            &declared,
            "how are emails written?",
            Some(sample),
        ))
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    thread.finish(first);

    let opening = first_prompts.lock().clone();
    let [opening] = opening.as_slice() else {
        panic!("one prompt: {opening:?}");
    };
    assert!(opening.contains("user1@example.com"), "{opening}");
    assert!(
        !opening.contains("SECRET-"),
        "an unticked column left: {opening}"
    );
    assert!(opening.contains(r#"table "main"."customers""#), "{opening}");
    let recorded = fixture.egress();
    let [entry] = recorded.as_slice() else {
        panic!("one send, one entry: {recorded:?}");
    };
    assert_eq!(entry.recipient, declared.id);
    assert_eq!(entry.reach, EgressReach::Unresolved);
    assert!(
        first_events
            .lock()
            .join("\n")
            .contains(r#""kind":"sampleApproved","rows":5,"columns":1"#)
    );
    assert!(
        !thread.has_agent_link(),
        "the process that saw the rows is released"
    );

    // The next question: another process, told the structure again, and no
    // row.
    let (second_agent, second_prompts) = scripted_agent(&fixture);
    waiting(&fixture, &declared, second_agent);
    let (channel, second_events) = recording();
    let second = fixture.begin(&thread, Some(first), channel);
    let run = Run {
        mentions: &crate::backend::ai::mentions::NO_MENTIONS,
        inner: &fixture.backend.inner,
        thread: &thread,
        node: second,
        parent: Some(first),
        connection: &config,
        cancel: &cancel,
    };
    fixture
        .runtime
        .block_on(run.ask_agent(fixture.session, &declared, "and the domains?", None))
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    thread.finish(second);

    assert_eq!(
        first_prompts.lock().len(),
        1,
        "the first process heard nothing more"
    );
    let after = second_prompts.lock().clone();
    let [after] = after.as_slice() else {
        panic!("one prompt: {after:?}");
    };
    assert!(
        after.contains(r#"table "main"."customers""#),
        "a fresh session: {after}"
    );
    assert!(
        !after.contains("example.com"),
        "the sample did not follow: {after}"
    );
    assert!(
        second_events
            .lock()
            .join("\n")
            .contains(r#""reason":"sampleNotKept""#),
        "the panel says why"
    );
    assert!(
        thread.has_agent_link(),
        "an exchange without a sample is kept"
    );
}
