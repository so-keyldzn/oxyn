//! What bounds an agent's asking for a sample, on both of its ways in.
//!
//! The internal loop has no ceiling of calls per turn, and the bridge allows
//! eight per question: without a bound of its own, a refused request could be
//! asked again until a screen is approved by reflex. The bounds live in the
//! sink, the one place both ways pass
//! ([ADR-0034](../../../../../../../../docs/adr/0034-echantillon-pour-toute-destination.md)).

use super::agent_asks::{SQLITE, bridge, open_question, screen, sink, text_of};
use super::*;

/// How many approval screens for a sample the panel was shown.
fn screens(received: &Arc<Mutex<Vec<String>>>) -> usize {
    received
        .lock()
        .iter()
        .filter(|json| json.contains(r#""kind":"sampleRequested""#))
        .count()
}

/// The internal assistant, scripted to make each of `calls` in a turn of its
/// own, then to answer. Runs until the loop ends; the tool results, in order,
/// are what the model was told.
fn assistant(
    fixture: &Fixture,
    thread: &Arc<Thread>,
    node: u32,
    calls: Vec<(&'static str, serde_json::Value)>,
) -> tokio::task::JoinHandle<Vec<String>> {
    let mut turns: Vec<Vec<oxyn_llm::ChatEvent>> = calls
        .into_iter()
        .enumerate()
        .map(|(n, (tool, arguments))| {
            vec![
                oxyn_llm::ChatEvent::ToolCallComplete(oxyn_llm::ToolCall::new(
                    format!("call-{n}"),
                    tool,
                    arguments,
                )),
                oxyn_llm::ChatEvent::Done {
                    stop_reason: oxyn_llm::StopReason::ToolCalls,
                },
            ]
        })
        .collect();
    turns.push(vec![
        oxyn_llm::ChatEvent::TextDelta("Done.".to_owned()),
        oxyn_llm::ChatEvent::Done {
            stop_reason: oxyn_llm::StopReason::EndTurn,
        },
    ]);
    let spec = oxyn_ai::sql_agent();
    let engine = oxyn_ai::AgentRuntime::new(
        spec.clone(),
        ScriptedProvider::new(turns),
        oxyn_llm::Reach::Local,
        oxyn_ai::ToolRegistry::builtin(),
        "llama3",
    )
    .expect("a valid declaration");
    let scope = oxyn_ai::ToolScope::new(fixture.connection, fixture.session, SQLITE);
    let context = oxyn_ai::ContextBuilder::new(
        &oxyn_catalog::CatalogCache::new(),
        fixture.config().privacy_tier,
    )
    .build();
    let mut session = oxyn_ai::AgentSession::new(&spec, &context, scope);
    session.ask("how are emails written?");
    let sink = sink(
        fixture,
        thread,
        node,
        (spec.id, session.id()),
        fixture.local(),
        "Local model · llama3",
    );
    let observer = Observer {
        thread: Arc::clone(thread),
        node,
    };
    fixture.runtime.spawn(async move {
        engine
            .run(&mut session, &sink, &observer, &CancelToken::new())
            .await
            .expect("the conversation runs");
        session
            .messages()
            .iter()
            .filter(|message| message.role == oxyn_llm::Role::Tool)
            .map(|message| message.content.clone())
            .collect()
    })
}

/// Waits for `task`, but not for a screen nobody answers: a regression that
/// opens one would otherwise hold the test for the request's five minutes.
fn settled<T>(fixture: &Fixture, task: tokio::task::JoinHandle<T>) -> T {
    fixture
        .runtime
        .block_on(tokio::time::timeout(
            std::time::Duration::from_secs(30),
            task,
        ))
        .expect("the call waited on a screen it should not have opened")
        .expect("joined")
}

fn request_customers() -> (&'static str, serde_json::Value) {
    (
        oxyn_ai::tools::REQUEST_SAMPLE,
        serde_json::json!({ "relation": "customers", "columns": ["email"] }),
    )
}

/// The user declines the internal assistant's request; its second request, in
/// the same answer, opens no screen and reads nothing.
#[test]
fn after_a_refusal_the_assistant_cannot_ask_again_in_the_same_answer() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let (thread, node, received) = open_question(&fixture, None);
    let running = assistant(
        &fixture,
        &thread,
        node,
        vec![request_customers(), request_customers()],
    );
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
    let told = settled(&fixture, running);

    let [first, second] = told.as_slice() else {
        panic!("two tool results: {told:?}");
    };
    assert!(first.contains("the user declined"), "{first}");
    assert!(
        second.contains("status: denied") && second.contains("only one may be"),
        "{second}"
    );
    assert_eq!(screens(&received), 1, "a second screen opened");
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
}

/// The same bound over the MCP bridge: a refusal, then a request again —
/// no second screen, and words that say nothing of the first answer.
#[test]
fn after_a_refusal_an_external_agent_cannot_ask_again_in_the_same_answer() {
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

    let arguments = serde_json::json!({ "relation": "customers", "columns": ["email"] });
    let pending = bridge.ask(&fixture, arguments.clone());
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
    let first = text_of(&fixture, pending);
    assert!(first.contains("the user declined"), "{first}");

    let again = bridge.ask(&fixture, arguments);
    let again: serde_json::Value =
        serde_json::from_str(&settled(&fixture, again).expect("a request is answered"))
            .expect("the reply is JSON");
    let again = again["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        again.contains("status: denied") && again.contains("only one may be"),
        "{again}"
    );
    assert_eq!(screens(&received), 1, "a second screen opened");
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
}

/// The internal assistant proposes a write, which waits for the user: its
/// loop waits with it, so no sample screen opens over the write. Once the
/// user decided, the model learns it — and only then asks for the sample.
#[test]
fn no_sample_screen_opens_while_a_write_of_the_assistant_waits() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let (thread, node, received) = open_question(&fixture, None);
    let running = assistant(
        &fixture,
        &thread,
        node,
        vec![
            (
                oxyn_ai::tools::EXECUTE_QUERY,
                serde_json::json!({
                    "statement": "INSERT INTO customers VALUES (99, 'new@example.com', 'S')"
                }),
            ),
            request_customers(),
        ],
    );
    let write = waiting_write(&fixture);
    fixture
        .runtime
        .block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));
    assert!(
        !running.is_finished(),
        "the loop went on past a waiting write"
    );
    assert_eq!(screens(&received), 0, "a sample screen opened over a write");

    fixture
        .runtime
        .block_on(fixture.backend.decide(write, false))
        .map_err(|error| error.message)
        .expect("rejected");
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
    let told = settled(&fixture, running);

    let [write, sample] = told.as_slice() else {
        panic!("two tool results: {told:?}");
    };
    assert!(
        write.contains("status: denied") && write.contains("rejected"),
        "{write}"
    );
    assert!(sample.contains("status: denied"), "{sample}");
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
}

/// The request for approval the assistant's write is waiting on.
fn waiting_write(fixture: &Fixture) -> oxyn_core::CommandId {
    for _ in 0..400 {
        if let Some(pending) = fixture.backend.inner.executor.approvals().pending().first() {
            return pending.id;
        }
        fixture
            .runtime
            .block_on(tokio::time::sleep(std::time::Duration::from_millis(25)));
    }
    panic!("the write asked for no approval");
}

/// The agent hangs up while the screen waits: its call's future is dropped,
/// as hyper drops a service whose connection closed. The screen closes all
/// the same, and nothing of it stays approvable.
#[test]
fn a_call_dropped_mid_wait_closes_its_screen() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let declared = agent("unused");
    let actor = (AgentId::new(), AgentSessionId::new());
    let (thread, node, received) = open_question(&fixture, None);
    let sink = sink(
        &fixture,
        &thread,
        node,
        actor,
        Recipient::agent(&declared),
        "Codex",
    );
    let request = oxyn_ai::ToolRegistry::builtin()
        .request(
            &oxyn_llm::ToolCall::new(
                "call-1",
                oxyn_ai::tools::REQUEST_SAMPLE,
                serde_json::json!({ "relation": "customers", "columns": ["email"] }),
            ),
            &oxyn_ai::sql_agent().allowed_tools,
            &oxyn_ai::ToolScope::new(fixture.connection, fixture.session, SQLITE),
        )
        .expect("a valid call");
    let oxyn_ai::tools::ToolRequest::Sample(ask) = request else {
        panic!("a sample request");
    };
    let call = fixture.runtime.spawn(async move {
        sink.request_sample(Actor::agent(actor.0, actor.1), ask, &CancelToken::new())
            .await
    });
    let shown = screen(&fixture, &received);
    let id = shown["id"].as_str().expect("an id").to_owned();

    call.abort();
    let ended = fixture.runtime.block_on(call);
    assert!(
        ended.is_err_and(|error| error.is_cancelled()),
        "the call was dropped mid-wait"
    );

    let closed = received.lock().iter().any(|json| {
        serde_json::from_str::<serde_json::Value>(json).is_ok_and(|update| {
            update["event"]["kind"] == "sampleAnswered"
                && update["event"]["request"] == id.as_str()
                && update["event"]["approved"] == false
        })
    });
    assert!(
        closed,
        "the screen stayed open: {}",
        received.lock().join("\n")
    );
    assert!(
        fixture
            .backend
            .ai_answer_sample(fixture.connection, &id, Some(&["email".to_owned()]))
            .is_err(),
        "an approval still reached a request nobody waits for"
    );
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
}

/// A table of twenty rows and thirty columns, each value at the rendering's
/// cap: approved whole, it is past the context budget.
fn furnish_wide_rows(fixture: &Fixture) {
    let columns: Vec<String> = (0..30).map(|n| format!("c{n}")).collect();
    let row = format!(
        "({})",
        columns
            .iter()
            .map(|_| format!("'{}'", "v".repeat(64)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for statement in [
        format!(
            "CREATE TABLE wide_rows ({})",
            columns
                .iter()
                .map(|column| format!("{column} TEXT"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!("INSERT INTO wide_rows VALUES {}", vec![row; 20].join(", ")),
    ] {
        fixture
            .runtime
            .block_on(fixture.backend.execute(
                CommandId::new(),
                fixture.connection,
                fixture.session,
                statement,
            ))
            .expect("setup statement runs");
    }
    // Listed, then described, as the explorer does.
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
    let wide = find(&tree, "wide_rows").expect("listed").address;
    fixture
        .runtime
        .block_on(fixture.backend.refresh_catalog(
            CommandId::new(),
            fixture.connection,
            fixture.session,
            Some(wide),
        ))
        .expect("described");
}

/// Approved, read, then dropped by the point of passage for the budget: the
/// audit stays empty, the panel is never told « sent », and the tool line says
/// what the model was told — a refusal.
#[test]
fn a_sample_dropped_for_the_budget_is_neither_recorded_nor_announced() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    furnish_wide_rows(&fixture);
    let (thread, node, received) = open_question(&fixture, None);
    let running = assistant(
        &fixture,
        &thread,
        node,
        vec![(
            oxyn_ai::tools::REQUEST_SAMPLE,
            serde_json::json!({ "relation": "wide_rows", "rows": 20 }),
        )],
    );
    let request = screen(&fixture, &received);
    let every: Vec<String> = request["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .filter_map(|field| field["name"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(every.len(), 30);
    fixture
        .backend
        .ai_answer_sample(
            fixture.connection,
            request["id"].as_str().expect("an id"),
            Some(&every),
        )
        .map_err(|error| error.message)
        .expect("approved");
    let told = settled(&fixture, running);

    let [said] = told.as_slice() else {
        panic!("one tool result: {told:?}");
    };
    assert!(said.contains("did not fit the context budget"), "{said}");
    assert!(!said.contains(&"v".repeat(64)), "{said}");
    assert!(fixture.reads() > 0, "the rows were read");
    assert!(
        fixture.egress().is_empty(),
        "a sample that did not leave was recorded"
    );
    let events = received.lock().join("\n");
    assert!(
        !events.contains(r#""kind":"sampleApproved""#),
        "announced as sent: {events}"
    );
    let reported = received
        .lock()
        .iter()
        .find(|json| json.contains(r#""kind":"toolReported""#))
        .cloned()
        .expect("the call is reported");
    assert!(reported.contains(r#""status":"denied""#), "{reported}");
    assert!(!reported.contains("sent "), "{reported}");
}
