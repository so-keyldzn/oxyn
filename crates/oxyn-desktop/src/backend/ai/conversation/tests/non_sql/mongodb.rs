//! The assistant on a MongoDB session: the agent the MongoDB driver ships
//! (`crates/oxyn-ai/agents/mongodb.md`) runs there by default and only there,
//! and every command it sends is classified by the MongoDB session's reading
//! — on production, a write, an unknown command or server-side JavaScript is
//! refused, never confirmed (I-02, I-07).

use super::*;

#[test]
fn a_question_on_a_mongodb_session_runs_the_shipped_mongodb_agent() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (port, request) = runtime.block_on(capturing());
    let (backend, _) = backend_speaking(DOCUMENTS, Vec::new());
    let open = open_speaking(&runtime, &backend, DOCUMENTS, Environment::Local);
    let provider = local_provider(&runtime, &backend, port);
    let (channel, received) = recording();

    // No agent named, none written by the user: the shipped one runs.
    runtime
        .block_on(backend.ai_ask(question(&open, &provider, None), channel))
        .map_err(|error| error.message)
        .expect("the question starts");
    let sent = runtime.block_on(request).expect("the provider was called");
    ended(&runtime, &received);

    assert!(sent.contains("senior MongoDB engineer"), "the role: {sent}");
    assert!(
        sent.contains("The language: MongoDB database commands (`mongo`)"),
        "the MongoDB fragment: {sent}"
    );
    assert!(
        !sent.contains("Oxyn has no notes of its own"),
        "not the generic fragment: {sent}"
    );
    assert!(sent.contains("tagged `mongo`"), "{sent}");
    assert!(!sent.contains("The dialect"), "no SQL fragment: {sent}");
    // The context came through the one gateway, under the connection's tier,
    // written for the language (I-04).
    assert!(sent.contains("privacy tier `metadata`"), "{sent}");
    assert!(sent.contains("query language: mongo"), "{sent}");
    assert!(!sent.contains("{{"), "every variable is filled: {sent}");
}

#[test]
fn the_picker_of_a_mongodb_connection_offers_the_mongodb_agent_only() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (backend, _) = backend_speaking(DOCUMENTS, vec![keys_agent()]);
    let open = open_speaking(&runtime, &backend, DOCUMENTS, Environment::Local);
    local_provider(&runtime, &backend, 9);
    let connection: ConnectionId = open.connection.parse().expect("connection id");

    let listed = runtime
        .block_on(backend.ai_list_agents(connection, None))
        .expect("listed");
    let names: Vec<&str> = listed.iter().map(|option| option.name.as_str()).collect();
    assert_eq!(names, ["MongoDB"], "no SQL agent, no Redis agent");
}

/// A user command: an agent never manages privileges, on any connection.
const GRANT: &str = r#"{"dropUser": "reporter"}"#;

/// Commands an agent might send that the MongoDB session cannot prove
/// read-only, each for a different reason.
const UNPROVEN: [&str; 10] = [
    // Writes.
    r#"{"delete": "orders", "deletes": [{"q": {}, "limit": 0}]}"#,
    r#"{"insert": "orders", "documents": [{"total": {"$numberDecimal": "9.99"}}]}"#,
    // A read that writes, explained or not.
    r#"{"aggregate": "orders", "pipeline": [{"$match": {}}, {"$out": "orders_copy"}], "cursor": {}}"#,
    r#"{"explain": {"aggregate": "orders", "pipeline": [{"$merge": "totals"}], "cursor": {}}}"#,
    // Structure.
    r#"{"createIndexes": "orders", "indexes": [{"key": {"customer": 1}, "name": "customer_1"}]}"#,
    r#"{"dropDatabase": 1}"#,
    // Server-side JavaScript, an unknown command, a transaction verb.
    r#"{"find": "orders", "filter": {"$where": "this.total > 0"}}"#,
    r#"{"shutdown": 1}"#,
    r#"{"commitTransaction": 1}"#,
    // Shell syntax is not a command.
    "db.orders.find({})",
];

#[test]
fn an_agent_command_on_a_mongodb_production_session_is_refused_unless_proven_read() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (backend, seen) = backend_speaking(DOCUMENTS, Vec::new());
    let production = open_speaking(&runtime, &backend, DOCUMENTS, Environment::Production);
    let (sink, _thread, actor, _) = sink_on(&backend, &production);
    let dispatch = |statement: &str| {
        runtime.block_on(sink.dispatch(
            &CallHandle::new(CallId::fresh()),
            actor,
            agent_request_in(DOCUMENTS, &backend, &production, statement),
            &CancelToken::new(),
        ))
    };

    for command in UNPROVEN.into_iter().chain([GRANT]) {
        let outcome = dispatch(command);
        assert!(
            matches!(outcome, DispatchOutcome::Denied { .. }),
            "{command}: {outcome:?}"
        );
    }
    assert!(seen.0.lock().is_empty(), "nothing reached the session");

    // A command the session proves read-only runs, bounded to a read.
    let read =
        r#"{"find": "orders", "filter": {"status": "open"}, "limit": 100, "maxTimeMS": 5000}"#;
    let outcome = dispatch(read);
    assert!(
        matches!(outcome, DispatchOutcome::Failed { .. }),
        "the fake session refuses what it records: {outcome:?}"
    );
    assert_eq!(*seen.0.lock(), [(read.to_owned(), true)]);
}

#[test]
fn an_agent_write_on_a_mongodb_staging_session_waits_for_the_user() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (backend, seen) = backend_speaking(DOCUMENTS, Vec::new());
    let staging = open_speaking(&runtime, &backend, DOCUMENTS, Environment::Staging);

    let dispatch = |command: &str| {
        runtime.block_on(backend.inner.executor.dispatch_as(
            CommandId::new(),
            Actor::agent(AgentId::new(), AgentSessionId::new()),
            agent_request_in(DOCUMENTS, &backend, &staging, command),
            &CancelToken::new(),
        ))
    };
    for command in UNPROVEN {
        let outcome = dispatch(command);
        assert!(
            matches!(outcome, Ok(oxyn_exec::Outcome::NeedsApproval { .. })),
            "{command}: {outcome:?}"
        );
    }
    // Privileges are not a write to approve: they are refused outright.
    let outcome = dispatch(GRANT);
    assert!(
        matches!(outcome, Ok(oxyn_exec::Outcome::Denied { .. })),
        "{outcome:?}"
    );
    assert!(seen.0.lock().is_empty(), "nothing ran before an approval");
}
