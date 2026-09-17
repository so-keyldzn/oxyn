use std::collections::BTreeMap;

use oxyn_core::{AgentId, AgentSessionId, CommandId, ExecRequest, ProviderId, SqlDialect};
use parking_lot::Mutex;
use tauri::ipc::InvokeResponseBody;

use super::*;
use crate::ipc::{ConnectResponse, ConnectionDraft, OpenConnection};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// A channel that records what the webview would receive, as JSON.
fn recording() -> (Channel<AiUpdate>, Arc<Mutex<Vec<String>>>) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(json) = body {
            sink.lock().push(json);
        }
        Ok(())
    });
    (channel, received)
}

fn open(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    environment: Environment,
) -> OpenConnection {
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "assistant regression".into(),
        environment,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    };
    runtime.block_on(async {
        match backend
            .connect(CommandId::new(), draft)
            .await
            .expect("connects")
        {
            ConnectResponse::Open(open) => open,
            ConnectResponse::Approval { command, .. } => {
                match backend
                    .decide_connection(command.parse().expect("minted id"), true)
                    .await
                    .expect("approved")
                {
                    Some(ConnectResponse::Open(open)) => open,
                    _ => panic!("an approved connection opens"),
                }
            }
        }
    })
}

/// A running node on a new conversation, and a sink that reports into it.
fn sink_on(
    backend: &Backend,
    open: &OpenConnection,
) -> (AgentSink, Arc<Thread>, Actor, Arc<Mutex<Vec<String>>>) {
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let thread = backend
        .inner
        .ai
        .thread_for(connection, None)
        .expect("a conversation");
    let (channel, received) = recording();
    let (node, _) = thread
        .begin(
            None,
            "copy the audit table",
            channel,
            Scope {
                connection,
                name: open.name.clone(),
                environment: open.environment,
            },
        )
        .expect("begins");
    let (agent, session) = (AgentId::new(), AgentSessionId::new());
    let sink = AgentSink {
        sink: ExecutorSink::for_agent(Arc::clone(&backend.inner.executor), agent, session),
        executor: Arc::clone(&backend.inner.executor),
        thread: Arc::clone(&thread),
        node,
        question: QuestionOpen::new(),
    };
    (sink, thread, Actor::agent(agent, session), received)
}

fn execute(open: &OpenConnection, sql: &str) -> Command {
    Command::Execute {
        connection: open.connection.parse().expect("connection id"),
        session: open.session.parse().expect("session id"),
        request: Box::new(ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Sqlite),
            sql,
        )),
    }
}

#[test]
fn an_agent_write_on_production_is_refused_and_shown_with_its_statement() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Production);
    let (sink, thread, actor, received) = sink_on(&backend, &open);

    thread.open_call(
        "execute_query",
        "Execute",
        open.connection.parse().ok(),
        true,
    );
    let outcome = runtime.block_on(sink.dispatch(
        actor,
        execute(&open, "CREATE TABLE audit_copy (id INTEGER)"),
        &CancelToken::new(),
    ));
    // I-02: for an agent, a refusal — not a confirmation.
    assert!(
        matches!(outcome, DispatchOutcome::Denied { .. }),
        "{outcome:?}"
    );

    let sent = received.lock().join("\n");
    assert!(sent.contains("CREATE TABLE audit_copy"), "{sent}");
    assert!(sent.contains("assistant regression"), "{sent}");
    assert!(
        !sent.contains(&open.connection),
        "a connection id reached the webview: {sent}"
    );
    assert!(!sent.contains("approvalRequested"), "{sent}");
}

#[test]
fn an_agent_cannot_present_itself_as_the_user() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let (sink, _thread, _actor, _) = sink_on(&backend, &open);
    let outcome = runtime.block_on(sink.dispatch(
        Actor::Human,
        execute(&open, "SELECT 1"),
        &CancelToken::new(),
    ));
    assert!(matches!(outcome, DispatchOutcome::Denied { .. }));
}

#[test]
fn a_tool_call_cancelled_with_its_conversation_is_not_a_failure() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let (_sink, thread, _actor, received) = sink_on(&backend, &open);
    thread.open_call("execute_query", "Execute", None, false);
    let (call, _, _) = announce_call(&thread, 0, Some("SELECT pg_sleep(60)".into()));
    thread.mark_cancelled(call);
    report_call(
        &thread,
        0,
        &DispatchOutcome::Failed {
            class: oxyn_core::ErrorClass::Ambiguous,
            message: "cancelled".into(),
        },
        false,
    );
    let sent = received.lock().join("\n");
    assert!(sent.contains(r#""status":"cancelled""#), "{sent}");
}

fn agent(command: &str) -> ExternalAgentConfig {
    ExternalAgentConfig::new(ProviderId::for_new_agent(), "Codex", command)
        .with_args(["-y", "@agentclientprotocol/codex-acp@1.12.0"])
}

#[test]
fn a_missing_agent_program_is_named_not_a_protocol_error() {
    let runtime = runtime();
    let failure = runtime
        .block_on(locate(&agent("oxyn-no-such-program-4f1c")))
        .expect_err("not found");
    assert_eq!(failure.category, FailureCategory::AgentNotFound);
    assert!(
        failure.message.contains("oxyn-no-such-program-4f1c"),
        "{}",
        failure.message
    );
}

#[test]
fn a_sign_in_names_the_agent_and_the_documented_command_without_running_it() {
    let failure = agent_failure(
        &agent("npx"),
        ExternalError::AuthRequired {
            methods: vec![oxyn_ai::external::session::AuthOption {
                id: "chat-gpt".into(),
                name: "ChatGPT".into(),
                description: None,
                kind: oxyn_ai::external::session::AuthKind::Agent,
            }],
        },
    );
    assert_eq!(failure.category, FailureCategory::AgentSignIn);
    let help = failure.sign_in.expect("help");
    assert_eq!(help.agent, "Codex");
    assert_eq!(help.terminal_command, Some("codex login"));
    let method = help.methods.first().expect("one method");
    assert_eq!((method.kind, method.command.as_deref()), ("agent", None));
}

#[test]
fn an_agent_on_a_local_only_connection_is_refused_before_launch() {
    let failure = agent_failure(&agent("npx"), ExternalError::RefusedByTier);
    assert_eq!(failure.category, FailureCategory::Refused);
}

/// A provider that answers with what the test wrote, turn by turn.
///
/// The point of this fake is narrow: it removes the model, and nothing else.
/// Everything after it — the registry, the `PolicyGate`, the executor, the
/// driver — is the real thing, on a real SQLite database.
#[derive(Debug)]
struct ScriptedProvider {
    turns: Mutex<Vec<Vec<oxyn_llm::ChatEvent>>>,
}

impl ScriptedProvider {
    fn new(turns: Vec<Vec<oxyn_llm::ChatEvent>>) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(turns),
        })
    }
}

#[async_trait::async_trait]
impl oxyn_llm::LlmProvider for ScriptedProvider {
    fn id(&self) -> ProviderId {
        ProviderId::for_new_agent()
    }

    async fn models(&self) -> oxyn_core::Result<Vec<oxyn_llm::ModelInfo>> {
        Ok(Vec::new())
    }

    async fn stream(
        &self,
        _request: oxyn_llm::ChatRequest,
        _cancel: &CancelToken,
    ) -> oxyn_core::Result<futures::stream::BoxStream<'static, oxyn_llm::ChatEvent>> {
        let mut turns = self.turns.lock();
        let events = if turns.is_empty() {
            Vec::new()
        } else {
            turns.remove(0)
        };
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

#[test]
fn a_tool_call_really_runs_the_query_on_the_database() {
    // The regression this guards: every other test of this loop stops at a
    // fake bus, so a break between the registry and the driver would show up
    // only in the running application, as an assistant that answers about a
    // database it never read.
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);

    let provider = ScriptedProvider::new(vec![
        vec![
            oxyn_llm::ChatEvent::ToolCallComplete(oxyn_llm::ToolCall::new(
                "call-1",
                "execute_query",
                serde_json::json!({
                    "statement": "SELECT 41 + 1 AS answer, 'oxyn' AS who"
                }),
            )),
            oxyn_llm::ChatEvent::Done {
                stop_reason: oxyn_llm::StopReason::ToolCalls,
            },
        ],
        vec![
            oxyn_llm::ChatEvent::TextDelta("The answer is 42.".to_owned()),
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
        "scripted",
    )
    .expect("a valid declaration");

    let context = oxyn_ai::ContextBuilder::new(
        &oxyn_catalog::CatalogCache::new(),
        oxyn_core::PrivacyTier::Metadata,
    )
    .build();
    let scope = oxyn_ai::ToolScope::new(
        open.connection.parse().expect("connection id"),
        open.session.parse().expect("session id"),
        QueryLanguage::Sql(SqlDialect::Sqlite),
    );
    let mut session = oxyn_ai::AgentSession::new(&spec, &context, scope);
    session.ask("what is the answer?");

    // Bound to the same agent and the same conversation the runtime speaks
    // for: the executor refuses any other actor, and that refusal is what a
    // sink built on a fresh identity would earn.
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let thread = backend
        .inner
        .ai
        .thread_for(connection, None)
        .expect("a conversation");
    let (channel, received) = recording();
    let (node, _) = thread
        .begin(
            None,
            "what is the answer?",
            channel,
            Scope {
                connection,
                name: open.name.clone(),
                environment: open.environment,
            },
        )
        .expect("begins");
    let sink = AgentSink {
        sink: ExecutorSink::for_agent(Arc::clone(&backend.inner.executor), spec.id, session.id()),
        executor: Arc::clone(&backend.inner.executor),
        thread: Arc::clone(&thread),
        node,
        question: QuestionOpen::new(),
    };

    let observer = Observer {
        thread: Arc::clone(&thread),
        node,
    };
    let outcome = runtime
        .block_on(engine.run(&mut session, &sink, &observer, &CancelToken::new()))
        .expect("the conversation runs");
    assert!(
        matches!(outcome, oxyn_ai::AgentOutcome::Answered { .. }),
        "{outcome:?}"
    );

    // The proof: the driver's own result came back through the bus, and the
    // model was told about it as a tool result.
    let tool = session
        .messages()
        .iter()
        .rev()
        .find(|message| message.role == oxyn_llm::Role::Tool)
        .expect("the tool result reached the conversation");
    assert!(
        tool.content.contains("1 row"),
        "the query did not run: {:?}",
        tool.content
    );
    // The citation's count is the executor's measurement, carried as a field:
    // a panel that parsed "1 rows" would break on the first rewording.
    let reports: Vec<String> = received
        .lock()
        .iter()
        .filter(|json| json.contains(r#""kind":"toolReported""#))
        .cloned()
        .collect();
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(reports[0].contains(r#""rows":1"#), "{reports:?}");
}

/// A conversation holding an agent link, as a launch leaves it. The returned
/// driver is kept, unpolled, so the session stays open for the test.
fn linked(
    backend: &Backend,
    connection: ConnectionId,
) -> (Arc<Thread>, oxyn_ai::external::session::SessionDriver) {
    let thread = backend
        .inner
        .ai
        .thread_for(connection, None)
        .expect("a conversation");
    let (ours, _theirs) = agent_client_protocol::Channel::duplex();
    let (session, driver) = ExternalSession::over(ours, std::env::temp_dir().join("unused"));
    thread.link_agent(Some(AgentLink {
        agent: ProviderId::for_new_agent(),
        tier: PrivacyTier::Sampled,
        leaf: None,
        session: Arc::new(session),
        tools: ToolTurns::new(8, Arc::new(|| false)),
        actor: (AgentId::new(), AgentSessionId::new()),
        _requests: WithdrawOnRelease::new(
            Arc::clone(&backend.inner.executor),
            Actor::agent(AgentId::new(), AgentSessionId::new()),
        ),
    }));
    assert!(thread.has_agent_link());
    (thread, driver)
}

fn edited(tier: PrivacyTier) -> crate::ipc::settings::ConnectionEdit {
    crate::ipc::settings::ConnectionEdit {
        name: "assistant regression".into(),
        environment: Environment::Local,
        privacy_tier: tier,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    }
}

#[test]
fn a_tool_call_reads_the_tier_the_connection_has_now() {
    // I-04: the agent's process outlives the question it was launched for;
    // the tier its calls obey is the connection's, read at the call.
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let source = StoredTier {
        executor: Arc::clone(&backend.inner.executor),
        connection,
    };
    assert_eq!(
        runtime.block_on(source.current()),
        Some(PrivacyTier::Metadata)
    );

    runtime
        .block_on(backend.update_connection(
            CommandId::new(),
            connection,
            edited(PrivacyTier::Sampled),
        ))
        .expect("saved");
    assert_eq!(
        runtime.block_on(source.current()),
        Some(PrivacyTier::Sampled)
    );

    runtime
        .block_on(backend.update_connection(
            CommandId::new(),
            connection,
            edited(PrivacyTier::Local),
        ))
        .expect("saved");
    assert_eq!(runtime.block_on(source.current()), Some(PrivacyTier::Local));
}

#[test]
fn editing_the_connection_releases_the_agent_launched_under_its_old_tier() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let (thread, _driver) = linked(&backend, connection);

    runtime
        .block_on(backend.update_connection(
            CommandId::new(),
            connection,
            edited(PrivacyTier::Metadata),
        ))
        .expect("saved");
    assert!(
        !thread.has_agent_link(),
        "the next question must relaunch under the tier the connection has now"
    );
}

#[test]
fn closing_the_connection_releases_its_agents() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let (thread, _driver) = linked(&backend, connection);

    runtime
        .block_on(backend.disconnect(connection))
        .expect("disconnects");
    assert!(!thread.has_agent_link());
}

#[test]
fn a_refused_question_releases_the_agents_kept_for_the_connection() {
    // The question is refused before any conversation is touched — here, an
    // agent no longer declared. What was kept alive for this connection was
    // launched under something that no longer holds.
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let (thread, _driver) = linked(&backend, connection);

    let (channel, _received) = recording();
    let refused = runtime.block_on(backend.ai_ask(
        AskRequest {
            connection: open.connection.clone(),
            session: open.session.clone(),
            thread: None,
            parent: None,
            question: "how many clients?".to_owned(),
            destination: crate::ipc::ai::DestinationChoice::Agent {
                id: ProviderId::for_new_agent().to_string(),
            },
            sample: None,
        },
        channel,
    ));
    assert!(refused.is_err());
    assert!(!thread.has_agent_link());
}

fn retry_of(event: &AiEvent) -> (bool, String) {
    match event {
        AiEvent::Failed {
            retryable, message, ..
        } => (*retryable, message.clone()),
        _ => panic!("a failure"),
    }
}

#[test]
fn ask_again_is_decided_by_the_backend_from_the_class_and_the_writes() {
    // The front used to deduce it from the category. A stream cut after an
    // approved `INSERT` was « provider », hence « ask again », hence the same
    // `INSERT` proposed twice (I-13).
    let provider = || Failure::new("the stream was cut", FailureCategory::Provider);

    let (retryable, message) = retry_of(&provider().into_event(false));
    assert!(retryable);
    assert_eq!(message, "the stream was cut");

    let (retryable, message) = retry_of(&provider().into_event(true));
    assert!(!retryable, "a write ran: asking again may apply it twice");
    assert!(
        message.contains("may already have been applied"),
        "{message}"
    );

    let ambiguous = Failure {
        class: Some(ErrorClass::Ambiguous),
        ..provider()
    };
    assert!(!retry_of(&ambiguous.into_event(false)).0);

    let refused = Failure::new("local-only", FailureCategory::Refused);
    assert!(!retry_of(&refused.into_event(false)).0);
}

#[test]
fn a_write_that_reached_the_executor_marks_the_run_and_a_refused_one_does_not() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");

    // Refused on production: nothing ran, nothing to fear from asking again.
    let production = open(&runtime, &backend, Environment::Production);
    let (sink, thread, actor, _received) = sink_on(&backend, &production);
    thread.open_call(
        "execute_query",
        "Execute",
        production.connection.parse().ok(),
        true,
    );
    runtime.block_on(sink.dispatch(
        actor,
        execute(&production, "DELETE FROM t"),
        &CancelToken::new(),
    ));
    assert!(!thread.wrote());

    // Held for approval elsewhere: the user may approve it, so it counts.
    let staging = open(&runtime, &backend, Environment::Staging);
    let (sink, thread, actor, _received) = sink_on(&backend, &staging);
    thread.open_call(
        "execute_query",
        "Execute",
        staging.connection.parse().ok(),
        true,
    );
    runtime.block_on(sink.dispatch(
        actor,
        execute(&staging, "CREATE TABLE t (a INTEGER)"),
        &CancelToken::new(),
    ));
    assert!(thread.wrote());
}

#[test]
fn an_interrupted_stream_offers_no_retry_and_says_it_may_have_been_billed() {
    // The class is the error's, carried as data: a stream cut before the turn
    // ended may have been finished — and billed — on the provider's side.
    let interrupted = AiError::Interrupted("connection reset".to_owned());
    let failure = Failure {
        class: class_of(&interrupted),
        ..Failure::new(interrupted.to_string(), category_of(&interrupted))
    };
    let (retryable, message) = retry_of(&failure.into_event(false));
    assert!(!retryable, "asking again may pay twice for the same turn");
    assert!(
        message.contains("may have finished — and billed"),
        "{message}"
    );

    // A failure the provider announced is not ambiguous: it may be asked again.
    let announced = AiError::Provider("overloaded".to_owned());
    let failure = Failure {
        class: class_of(&announced),
        ..Failure::new(announced.to_string(), category_of(&announced))
    };
    assert!(retry_of(&failure.into_event(false)).0);
}

#[test]
fn a_request_born_after_its_question_closed_is_withdrawn_not_shown() {
    // The residual window: a call at the executor when the question ended.
    // Its request would show under a finished answer and stay approvable.
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let staging = open(&runtime, &backend, Environment::Staging);
    let (mut sink, thread, actor, received) = sink_on(&backend, &staging);
    let question = QuestionOpen::new();
    sink.question = question.clone();
    question.close();

    thread.open_call(
        "execute_query",
        "Execute",
        staging.connection.parse().ok(),
        true,
    );
    let outcome = runtime.block_on(sink.dispatch(
        actor,
        execute(&staging, "CREATE TABLE t (a INTEGER)"),
        &CancelToken::new(),
    ));

    assert!(
        matches!(outcome, DispatchOutcome::Denied { .. }),
        "{outcome:?}"
    );
    assert!(
        backend.inner.executor.approvals().pending().is_empty(),
        "the request must be withdrawn by the executor, not merely hidden"
    );
    assert!(
        received
            .lock()
            .iter()
            .all(|json| !json.contains("approvalRequested")),
        "nothing may be shown under a closed question"
    );
    assert!(!thread.wrote(), "a withdrawn request ran nothing");
}

#[test]
fn releasing_an_agent_withdraws_its_pending_requests() {
    // Its author gone, a request would stay approvable, and approving it would
    // run a command nobody reads the result of.
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let staging = open(&runtime, &backend, Environment::Staging);
    let (sink, thread, actor, _received) = sink_on(&backend, &staging);
    thread.open_call(
        "execute_query",
        "Execute",
        staging.connection.parse().ok(),
        true,
    );
    let outcome = runtime.block_on(sink.dispatch(
        actor,
        execute(&staging, "CREATE TABLE t (a INTEGER)"),
        &CancelToken::new(),
    ));
    assert!(
        matches!(outcome, DispatchOutcome::AwaitingApproval { .. }),
        "{outcome:?}"
    );
    let pending = backend.inner.executor.approvals().pending();
    assert_eq!(pending.len(), 1);
    let request = pending[0].id;

    // An unrelated agent's request is left alone.
    let (other_sink, other_thread, other_actor, _other) = sink_on(&backend, &staging);
    other_thread.open_call(
        "execute_query",
        "Execute",
        staging.connection.parse().ok(),
        true,
    );
    runtime.block_on(other_sink.dispatch(
        other_actor,
        execute(&staging, "CREATE TABLE u (a INTEGER)"),
        &CancelToken::new(),
    ));

    let (ours, _theirs) = agent_client_protocol::Channel::duplex();
    let (session, _driver) = ExternalSession::over(ours, std::env::temp_dir().join("unused"));
    thread.link_agent(Some(AgentLink {
        agent: ProviderId::for_new_agent(),
        tier: PrivacyTier::Metadata,
        leaf: None,
        session: Arc::new(session),
        tools: ToolTurns::new(8, Arc::new(|| false)),
        actor: (AgentId::new(), AgentSessionId::new()),
        _requests: WithdrawOnRelease::new(Arc::clone(&backend.inner.executor), actor),
    }));
    thread.link_agent(None);

    let left = backend.inner.executor.approvals().pending();
    assert_eq!(left.len(), 1, "only the other agent's request remains");
    assert!(left.iter().all(|pending| pending.id != request));
    let approved = runtime.block_on(backend.inner.executor.approve(
        "human",
        request,
        &CancelToken::new(),
    ));
    assert!(approved.is_err(), "a withdrawn request is not approvable");
}

#[test]
fn an_agents_settings_reach_the_panel() {
    // The conversion is tested in `ipc`; this is the line between: an event
    // the observer does not forward is a selector that never appears.
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let thread = backend
        .inner
        .ai
        .thread_for(connection, None)
        .expect("a conversation");
    let (channel, received) = recording();
    let (node, _) = thread
        .begin(
            None,
            "which mode?",
            channel,
            Scope {
                connection,
                name: open.name.clone(),
                environment: open.environment,
            },
        )
        .expect("begins");
    let observer = Observer {
        thread: Arc::clone(&thread),
        node,
    };

    let declared = oxyn_ai::external::settings::AgentSettings {
        current_mode: Some("plan".to_owned()),
        ..Default::default()
    };
    observer.observe(AgentEvent::AgentSettings(&declared));

    let settings: Vec<String> = received
        .lock()
        .iter()
        .filter(|json| json.contains(r#""kind":"agentSettings""#))
        .cloned()
        .collect();
    assert_eq!(settings.len(), 1, "{:?}", received.lock());
    assert!(
        settings[0].contains(r#""currentMode":"plan""#),
        "{settings:?}"
    );
}

#[test]
fn an_agents_settings_reach_the_panel_between_two_questions() {
    use agent_client_protocol::schema::v1::{
        InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse, SessionId,
        SessionMode, SessionModeState,
    };
    use agent_client_protocol::{Agent, Channel as Duplex};
    use futures::FutureExt as _;

    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let thread = backend
        .inner
        .ai
        .thread_for(connection, None)
        .expect("a conversation");

    // A question already answered: nothing is running when the agent speaks.
    let (channel, received) = recording();
    let (node, _) = thread
        .begin(
            None,
            "first",
            channel,
            Scope {
                connection,
                name: open.name.clone(),
                environment: open.environment,
            },
        )
        .expect("begins");
    thread.finish(node);

    let (ours, theirs) = Duplex::duplex();
    let (session, driver) = ExternalSession::over(ours, std::env::temp_dir().join("unused"));
    runtime.spawn(driver);
    runtime.spawn(
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
                    responder.respond(NewSessionResponse::new(SessionId::new("s")).modes(
                        SessionModeState::new("plan", vec![SessionMode::new("plan", "Plan")]),
                    ))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(theirs)
            .boxed(),
    );
    let session = Arc::new(session);
    thread.follow_agent_settings(&session);
    thread.link_agent(Some(AgentLink {
        agent: ProviderId::for_new_agent(),
        tier: PrivacyTier::Sampled,
        leaf: Some(node),
        session: Arc::clone(&session),
        tools: ToolTurns::new(8, Arc::new(|| false)),
        actor: (AgentId::new(), AgentSessionId::new()),
        _requests: WithdrawOnRelease::new(
            Arc::clone(&backend.inner.executor),
            Actor::agent(AgentId::new(), AgentSessionId::new()),
        ),
    }));

    runtime
        .block_on(session.start())
        .expect("the fake agent starts");
    let shown = runtime.block_on(async {
        for _ in 0..200 {
            let shown: Vec<String> = received
                .lock()
                .iter()
                .filter(|json| json.contains(r#""kind":"agentSettings""#))
                .cloned()
                .collect();
            if !shown.is_empty() {
                return shown;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Vec::new()
    });
    assert_eq!(shown.len(), 1, "{:?}", received.lock());
    assert!(shown[0].contains(r#""currentMode":"plan""#), "{shown:?}");
    assert!(shown[0].contains(&format!(r#""node":{node}"#)), "{shown:?}");

    // A session that is no longer the conversation's shows nothing.
    let (other, _driver) = ExternalSession::over(Duplex::duplex().0, std::env::temp_dir());
    let other = Arc::new(other);
    thread.agent_settings_changed(
        &Arc::downgrade(&other),
        &oxyn_ai::external::settings::AgentSettings::default(),
    );
    let after: usize = received
        .lock()
        .iter()
        .filter(|json| json.contains(r#""kind":"agentSettings""#))
        .count();
    assert_eq!(after, 1, "{:?}", received.lock());

    // Many changes keep one entry in the node's log, not one each.
    for _ in 0..3 {
        thread.agent_settings_changed(
            &Arc::downgrade(&session),
            &oxyn_ai::external::settings::AgentSettings::default(),
        );
    }
    let kept = thread
        .view(None)
        .nodes
        .iter()
        .flat_map(|node| node.events.iter())
        .filter(|event| matches!(event, AiEvent::AgentSettings(_)))
        .count();
    assert_eq!(kept, 1);
}

#[test]
fn an_effort_is_sent_only_if_the_model_declares_it() {
    use oxyn_llm::ReasoningEffort::{High, Low, Max};

    let runtime = || {
        oxyn_ai::AgentRuntime::new(
            oxyn_ai::sql_agent(),
            ScriptedProvider::new(Vec::new()),
            oxyn_llm::Reach::Local,
            oxyn_ai::ToolRegistry::builtin(),
            "m",
        )
        .expect("a valid declaration")
    };
    let declared = [
        oxyn_llm::ModelInfo::new("m").with_reasoning_efforts(vec![Low, High]),
        oxyn_llm::ModelInfo::new("other").with_reasoning_efforts(vec![Max]),
    ];

    assert!(
        with_effort(runtime(), None, "m", None).is_ok(),
        "no effort, no check"
    );
    assert!(with_effort(runtime(), Some(Low), "m", Some(&declared)).is_ok());

    for (effort, model) in [(Max, "m"), (Low, "unlisted")] {
        let Err(refused) = with_effort(runtime(), Some(effort), model, Some(&declared)) else {
            panic!("{effort:?} on {model} is not declared");
        };
        let (retryable, message) = retry_of(&refused.into_event(false));
        assert!(
            !retryable,
            "the same effort again changes nothing: {message}"
        );
        assert!(message.contains("does not declare"), "{message}");
    }
}

#[test]
fn the_effort_asked_for_travels_with_the_provider_it_was_asked_for() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let declared = runtime
        .block_on(
            backend.save_ai_provider(
                serde_json::from_value(serde_json::json!({
                    "kind": "openai_compatible",
                    "label": "Local",
                    "baseUrl": "http://127.0.0.1:11434/v1",
                    "model": "llama3",
                }))
                .expect("a valid draft"),
            ),
        )
        .expect("declared");

    let resolved = runtime
        .block_on(backend.resolve_destination(
            &DestinationChoice::Provider {
                id: declared.id.clone(),
                model: None,
                effort: Some(oxyn_llm::ReasoningEffort::Low),
            },
            PrivacyTier::Metadata,
        ))
        .map_err(|error| error.message)
        .expect("resolved");
    assert!(
        matches!(
            resolved,
            Resolved::Provider {
                effort: Some(oxyn_llm::ReasoningEffort::Low),
                ..
            }
        ),
        "the effort was dropped on the way"
    );
}

mod approved_samples {
    use super::*;
    use std::collections::VecDeque;

    use super::super::samples::Recipient;
    use crate::ipc::ai::{DestinationChoice, SampleApproval, SampleRequest};
    use crate::ipc::{CatalogAddress, CatalogNode};

    /// Tiers read in turn, as the store would answer them over time.
    struct Scripted(Mutex<VecDeque<Option<PrivacyTier>>>);

    #[async_trait]
    impl TierSource for Scripted {
        async fn current(&self) -> Option<PrivacyTier> {
            self.0.lock().pop_front().flatten()
        }
    }

    /// A `Sampled` SQLite connection with a `customers` table of twelve rows,
    /// loaded in the catalog, and a local provider declared.
    struct Fixture {
        runtime: tokio::runtime::Runtime,
        backend: Backend,
        open: OpenConnection,
        connection: ConnectionId,
        session: SessionId,
        customers: CatalogAddress,
        /// A table whose second column's name is longer than the audit
        /// records: SQLite accepts it, `ai_egress` refuses it.
        wide: CatalogAddress,
        provider: String,
    }

    /// A column name past `oxyn_store::egress::MAX_COLUMN_NAME_BYTES`.
    fn unrecordable_column() -> String {
        "n".repeat(oxyn_store::egress::MAX_COLUMN_NAME_BYTES + 44)
    }

    fn find(nodes: &[CatalogNode], name: &str) -> Option<CatalogNode> {
        nodes.iter().find_map(|node| {
            if node.name == name {
                Some(node.clone())
            } else {
                find(&node.children, name)
            }
        })
    }

    fn fixture() -> Fixture {
        let runtime = runtime();
        let backend = Backend::open_temporary().expect("temporary backend");
        let (open, connection, session, customers, wide, provider) = {
            let _guard = runtime.enter();
            let open = open(&runtime, &backend, Environment::Local);
            let connection: ConnectionId = open.connection.parse().expect("connection id");
            runtime
                .block_on(backend.update_connection(
                    CommandId::new(),
                    connection,
                    edited(PrivacyTier::Sampled),
                ))
                .expect("sampled");
            let session: SessionId = open.session.parse().expect("session id");
            let mut rows = String::from("INSERT INTO customers VALUES ");
            rows.push_str(
                &(1..=12)
                    .map(|n| format!("({n}, 'user{n}@example.com', 'SECRET-{n}')"))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            for statement in [
                "CREATE TABLE customers (id INTEGER PRIMARY KEY, email TEXT, secret TEXT)"
                    .to_owned(),
                rows,
                format!(
                    "CREATE TABLE wide (id INTEGER PRIMARY KEY, \"{}\" TEXT)",
                    unrecordable_column()
                ),
                "INSERT INTO wide (id) VALUES (1), (2)".to_owned(),
            ] {
                runtime
                    .block_on(backend.execute(CommandId::new(), connection, session, statement))
                    .expect("setup statement runs");
            }
            runtime
                .block_on(backend.refresh_catalog(CommandId::new(), connection, session, None))
                .expect("root");
            let mut tree = backend.catalog_tree(connection).expect("tree");
            for _ in 0..3 {
                let Some(level) = tree
                    .iter()
                    .find(|node| !node.loaded && node.kind != "table")
                    .cloned()
                else {
                    break;
                };
                runtime
                    .block_on(backend.refresh_catalog(
                        CommandId::new(),
                        connection,
                        session,
                        Some(level.address),
                    ))
                    .expect("level");
                tree = backend.catalog_tree(connection).expect("tree");
            }
            let customers = find(&tree, "customers").expect("listed").address;
            let wide = find(&tree, "wide").expect("listed").address;
            // Described, columns included, as the explorer does on opening it.
            for relation in [&customers, &wide] {
                runtime
                    .block_on(backend.refresh_catalog(
                        CommandId::new(),
                        connection,
                        session,
                        Some(relation.clone()),
                    ))
                    .expect("relation");
            }
            let provider = runtime
                .block_on(
                    backend.save_ai_provider(
                        serde_json::from_value(serde_json::json!({
                            "kind": "openai_compatible",
                            "label": "Local model",
                            "baseUrl": "http://127.0.0.1:11434/v1",
                            "model": "llama3",
                        }))
                        .expect("a valid draft"),
                    ),
                )
                .expect("declared")
                .id;
            (open, connection, session, customers, wide, provider)
        };
        Fixture {
            runtime,
            backend,
            open,
            connection,
            session,
            customers,
            wide,
            provider,
        }
    }

    impl Fixture {
        fn offer(&self, thread: Option<String>, parent: Option<u32>) -> SampleRequest {
            let _guard = self.runtime.enter();
            self.runtime
                .block_on(self.backend.ai_request_sample(
                    self.connection,
                    thread,
                    parent,
                    self.customers.clone(),
                    DestinationChoice::Provider {
                        id: self.provider.clone(),
                        model: None,
                        effort: None,
                    },
                ))
                .map_err(|error| error.message)
                .expect("offered under Sampled")
        }

        /// Offers `relation` for a question to `provider`, starting a
        /// conversation.
        fn offer_of(&self, relation: &CatalogAddress, provider: &str) -> SampleRequest {
            self.runtime
                .block_on(self.backend.ai_request_sample(
                    self.connection,
                    None,
                    None,
                    relation.clone(),
                    DestinationChoice::Provider {
                        id: provider.to_owned(),
                        model: None,
                        effort: None,
                    },
                ))
                .map_err(|error| error.message)
                .expect("offered under Sampled")
        }

        /// Declares a provider served on `127.0.0.1:port`.
        fn declare_at(&self, port: u16) -> String {
            self.runtime
                .block_on(
                    self.backend.save_ai_provider(
                        serde_json::from_value(serde_json::json!({
                            "kind": "openai_compatible",
                            "label": "Listening model",
                            "baseUrl": format!("http://127.0.0.1:{port}/v1"),
                            "model": "llama3",
                        }))
                        .expect("a valid draft"),
                    ),
                )
                .expect("declared")
                .id
        }

        fn egress(&self) -> Vec<EgressRecord> {
            self.backend
                .inner
                .executor
                .store()
                .egress()
                .for_connection(self.connection, None, 100)
                .expect("egress")
                .entries
                .into_iter()
                .map(|entry| entry.record)
                .collect()
        }

        /// The recipient the fixture's offers name.
        fn local(&self) -> Recipient {
            Recipient {
                provider: self.provider.parse().expect("a provider id"),
                model: "llama3".to_owned(),
                reach: Reach::Local,
            }
        }

        fn stored(&self) -> StoredTier {
            StoredTier {
                executor: Arc::clone(&self.backend.inner.executor),
                connection: self.connection,
            }
        }

        /// Presents `approval` as `ai_ask` does: the grant taken out first.
        fn present(
            &self,
            run: &Run<'_>,
            approval: &SampleApproval,
            asked_in: Option<&str>,
            tiers: &dyn TierSource,
        ) -> Result<ApprovedSample, Failure> {
            let grant = self.backend.inner.ai.samples.take(&approval.request);
            self.runtime.block_on(run.take_sample(
                self.session,
                approval,
                grant,
                asked_in,
                Some(&self.local()),
                tiers,
            ))
        }

        fn ask(
            &self,
            request: AskRequest,
        ) -> (Result<AskStarted, IpcError>, Arc<Mutex<Vec<String>>>) {
            let (channel, received) = recording();
            (
                self.runtime.block_on(self.backend.ai_ask(request, channel)),
                received,
            )
        }

        fn config(&self) -> ConnectionConfig {
            self.backend.config(self.connection).expect("config")
        }

        fn reads(&self) -> usize {
            self.backend
                .inner
                .executor
                .store()
                .journal()
                .recent(256)
                .expect("journal")
                .into_iter()
                .filter(|entry| entry.record.command_kind == "PreviewRelation")
                .count()
        }

        fn begin(
            &self,
            thread: &Arc<Thread>,
            parent: Option<u32>,
            channel: Channel<AiUpdate>,
        ) -> u32 {
            thread
                .begin(
                    parent,
                    "which plans do customers use?",
                    channel,
                    Scope {
                        connection: self.connection,
                        name: self.open.name.clone(),
                        environment: self.open.environment,
                    },
                )
                .expect("begins")
                .0
        }
    }

    fn approval(request: &SampleRequest, columns: &[&str]) -> SampleApproval {
        SampleApproval {
            request: request.id.clone(),
            source: request.address.clone(),
            columns: columns.iter().map(|&column| column.to_owned()).collect(),
        }
    }

    fn prompt_of(dialogue: &AgentSession) -> String {
        dialogue
            .messages()
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn an_offer_names_columns_and_a_bound_never_a_value() {
        let fixture = fixture();
        let request = fixture.offer(None, None);
        assert_eq!(request.rows, 5);
        assert_eq!(
            request
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["id", "email", "secret"]
        );
        let json = serde_json::to_string(&request).expect("serializable");
        assert!(
            !json.contains("example.com") && !json.contains("SECRET-"),
            "{json}"
        );
        assert_eq!(fixture.reads(), 0, "offering reads nothing");
    }

    #[test]
    fn an_offer_does_not_exist_below_sampled() {
        let fixture = fixture();
        let _guard = fixture.runtime.enter();
        fixture
            .runtime
            .block_on(fixture.backend.update_connection(
                CommandId::new(),
                fixture.connection,
                edited(PrivacyTier::Metadata),
            ))
            .expect("lowered");
        let refused = fixture.runtime.block_on(fixture.backend.ai_request_sample(
            fixture.connection,
            None,
            None,
            fixture.customers.clone(),
            DestinationChoice::Provider {
                id: fixture.provider.clone(),
                model: None,
                effort: None,
            },
        ));
        assert!(refused.is_err());
    }

    #[test]
    fn a_tier_lowered_between_offer_and_question_refuses_and_reads_nothing() {
        let fixture = fixture();
        let _guard = fixture.runtime.enter();
        let request = fixture.offer(None, None);
        let config = fixture.config();
        fixture
            .runtime
            .block_on(fixture.backend.update_connection(
                CommandId::new(),
                fixture.connection,
                edited(PrivacyTier::Metadata),
            ))
            .expect("lowered");

        let thread = fixture
            .backend
            .inner
            .ai
            .thread_for(fixture.connection, None)
            .expect("a conversation");
        let (channel, _received) = recording();
        let node = fixture.begin(&thread, None, channel);
        let cancel = CancelToken::new();
        let run = Run {
            inner: &fixture.backend.inner,
            thread: &thread,
            node,
            parent: None,
            // As the panel captured it when the question left: still Sampled.
            connection: &config,
            cancel: &cancel,
        };
        let Err(refused) = fixture.present(
            &run,
            &approval(&request, &["email"]),
            None,
            &fixture.stored(),
        ) else {
            panic!("the tier was lowered");
        };
        assert!(
            refused.message.contains("privacy tier"),
            "{}",
            refused.message
        );
        assert_eq!(fixture.reads(), 0, "refused before any read");
    }

    #[test]
    fn a_column_not_offered_is_refused_and_a_grant_is_not_replayed() {
        let fixture = fixture();
        let _guard = fixture.runtime.enter();
        let request = fixture.offer(None, None);
        let config = fixture.config();
        let thread = fixture
            .backend
            .inner
            .ai
            .thread_for(fixture.connection, None)
            .expect("a conversation");
        let (channel, _received) = recording();
        let node = fixture.begin(&thread, None, channel);
        let cancel = CancelToken::new();
        let run = Run {
            inner: &fixture.backend.inner,
            thread: &thread,
            node,
            parent: None,
            connection: &config,
            cancel: &cancel,
        };

        let smuggled = approval(&request, &["email", "rowid"]);
        assert!(
            fixture
                .present(&run, &smuggled, None, &fixture.stored())
                .is_err()
        );
        // Spent by the refusal: the honest columns do not get a second try.
        let honest = approval(&request, &["email"]);
        let Err(replayed) = fixture.present(&run, &honest, None, &fixture.stored()) else {
            panic!("a grant presented twice");
        };
        assert!(
            replayed.message.contains("already used"),
            "{}",
            replayed.message
        );
        assert_eq!(fixture.reads(), 0);
    }

    #[test]
    fn a_sample_reaches_one_prompt_with_its_ticked_columns_and_nothing_else_keeps_it() {
        let fixture = fixture();
        let _guard = fixture.runtime.enter();
        let config = fixture.config();
        let thread = fixture
            .backend
            .inner
            .ai
            .thread_for(fixture.connection, None)
            .expect("a conversation");
        let (channel, received) = recording();
        let agent = sql_agent();
        let cancel = CancelToken::new();
        let run_at = |node, parent| Run {
            inner: &fixture.backend.inner,
            thread: &thread,
            node,
            parent,
            connection: &config,
            cancel: &cancel,
        };

        // An earlier exchange, remembered as a provider conversation.
        let first = fixture.begin(&thread, None, channel.clone());
        let (mut remembered, _) = run_at(first, None)
            .prepare_dialogue(&agent, fixture.session, "first", None, PrivacyTier::Sampled)
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        remembered.ask("EARLIER-EXCHANGE-MARK");
        thread.remember(
            first,
            Memory {
                session: remembered,
                tier: PrivacyTier::Sampled,
            },
        );
        thread.finish(first);

        // The sampled question.
        let request = fixture.offer(Some(thread.id().to_string()), Some(first));
        let sampled = fixture.begin(&thread, Some(first), channel.clone());
        let run = run_at(sampled, Some(first));
        let sample = fixture
            .present(
                &run,
                &approval(&request, &["id", "email"]),
                Some(&thread.id().to_string()),
                &fixture.stored(),
            )
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert!(fixture.reads() > 0, "the audit shows the real read");
        assert_eq!(
            sample.rows.rows.len(),
            5,
            "the backend's bound, whatever the table holds"
        );
        let (with_sample, _) = run
            .prepare_dialogue(
                &agent,
                fixture.session,
                "which plans?",
                Some(sample.rows),
                sample.tier,
            )
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let prompt = prompt_of(&with_sample);
        assert!(
            prompt.contains("user1@example.com"),
            "the ticked column reaches it"
        );
        assert!(
            !prompt.contains("SECRET-"),
            "an unticked column does not: {prompt}"
        );
        assert!(
            !prompt.contains("EARLIER-EXCHANGE-MARK"),
            "a sampled question starts fresh"
        );
        // Whatever the outcome, `converse` would not remember it.
        thread.finish(sampled);

        // The next question.
        let next = fixture.begin(&thread, Some(sampled), channel);
        let (after, _) = run_at(next, Some(sampled))
            .prepare_dialogue(
                &agent,
                fixture.session,
                "and then?",
                None,
                PrivacyTier::Sampled,
            )
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let prompt = prompt_of(&after);
        assert!(
            !prompt.contains("example.com"),
            "the sample did not follow: {prompt}"
        );
        assert!(
            !prompt.contains("EARLIER-EXCHANGE-MARK"),
            "nor did the memory from before it"
        );

        // What the conversation keeps: the size of the sample, and why the
        // memory was reset — never a value.
        let events = received.lock().join("\n");
        assert!(
            events.contains(r#""kind":"sampleApproved","rows":5,"columns":2"#),
            "{events}"
        );
        assert_eq!(
            events.matches(r#""reason":"sampleNotKept""#).count(),
            2,
            "{events}"
        );
        let kept = serde_json::to_string(&thread.view(None)).expect("serializable");
        assert!(
            !kept.contains("example.com") && !kept.contains("SECRET-"),
            "a value is kept in the conversation: {kept}"
        );
    }

    fn sampled_question(
        fixture: &Fixture,
        request: &SampleRequest,
        destination: DestinationChoice,
        question: &str,
    ) -> AskRequest {
        AskRequest {
            connection: fixture.open.connection.clone(),
            session: fixture.open.session.clone(),
            thread: None,
            parent: None,
            question: question.to_owned(),
            destination,
            sample: Some(approval(request, &["email"])),
        }
    }

    /// Waits for the run's last word, `finished` or `failed`.
    fn ending(fixture: &Fixture, received: &Mutex<Vec<String>>) -> String {
        for _ in 0..200 {
            let events = received.lock().join("\n");
            if events.contains(r#""kind":"failed""#) || events.contains(r#""kind":"finished""#) {
                return events;
            }
            fixture
                .runtime
                .block_on(tokio::time::sleep(std::time::Duration::from_millis(25)));
        }
        panic!("the run never ended: {}", received.lock().join("\n"));
    }

    #[test]
    fn a_sample_approved_for_a_local_model_is_refused_for_a_remote_provider_and_reads_nothing() {
        let fixture = fixture();
        let _guard = fixture.runtime.enter();
        let remote = fixture
            .runtime
            .block_on(
                fixture.backend.save_ai_provider(
                    serde_json::from_value(serde_json::json!({
                        "kind": "openai_compatible",
                        "label": "Remote model",
                        "baseUrl": "https://api.example.invalid/v1",
                        "model": "llama3",
                    }))
                    .expect("a valid draft"),
                ),
            )
            .expect("declared")
            .id;
        // Approved on a screen that said « Local model, on this machine ».
        let request = fixture.offer(None, None);
        assert_eq!(request.destination, "Local model");

        let (started, received) = fixture.ask(sampled_question(
            &fixture,
            &request,
            DestinationChoice::Provider {
                id: remote,
                model: None,
                effort: None,
            },
            "which plans?",
        ));
        started
            .map_err(|error| error.message)
            .expect("the question starts");
        let events = ending(&fixture, &received);
        assert!(events.contains("another provider"), "{events}");
        assert_eq!(fixture.reads(), 0, "refused before any read");
        assert!(fixture.egress().is_empty(), "a refusal records no send");
    }

    #[test]
    fn a_grant_is_spent_by_a_question_refused_before_it_starts() {
        let fixture = fixture();
        let _guard = fixture.runtime.enter();
        let request = fixture.offer(None, None);
        let local = DestinationChoice::Provider {
            id: fixture.provider.clone(),
            model: None,
            effort: None,
        };
        let (refused, _) = fixture.ask(sampled_question(&fixture, &request, local, "   "));
        assert!(refused.is_err(), "an empty question");
        assert_eq!(
            fixture
                .backend
                .inner
                .ai
                .samples
                .take(&request.id)
                .map(|_| ()),
            Err(SampleRefused::UnknownOrUsed)
        );
    }

    #[test]
    fn a_declined_offer_can_no_longer_be_presented() {
        let fixture = fixture();
        let request = fixture.offer(None, None);
        fixture
            .backend
            .ai_withdraw_sample(fixture.connection, &request.id);
        assert_eq!(
            fixture
                .backend
                .inner
                .ai
                .samples
                .take(&request.id)
                .map(|_| ()),
            Err(SampleRefused::UnknownOrUsed)
        );
    }

    #[test]
    fn a_tier_lowered_during_the_read_sends_nothing_and_governs_the_prompt() {
        let fixture = fixture();
        let _guard = fixture.runtime.enter();
        let request = fixture.offer(None, None);
        let config = fixture.config();
        let thread = fixture
            .backend
            .inner
            .ai
            .thread_for(fixture.connection, None)
            .expect("a conversation");
        let (channel, _received) = recording();
        let node = fixture.begin(&thread, None, channel);
        let cancel = CancelToken::new();
        let run = Run {
            inner: &fixture.backend.inner,
            thread: &thread,
            node,
            parent: None,
            connection: &config,
            cancel: &cancel,
        };

        // Sampled when the grant is checked, Metadata once the preview ran.
        let tiers = Scripted(Mutex::new(VecDeque::from([
            Some(PrivacyTier::Sampled),
            Some(PrivacyTier::Metadata),
        ])));
        let Err(refused) = fixture.present(&run, &approval(&request, &["email"]), None, &tiers)
        else {
            panic!("the tier was lowered during the read");
        };
        assert!(
            refused.message.contains("privacy tier"),
            "{}",
            refused.message
        );
        assert!(fixture.reads() > 0, "it was read, and not sent");
        assert!(fixture.egress().is_empty(), "a refusal records no send");

        // The prompt is built under the tier it is given, not the one the
        // question captured: under Metadata, no value enters it.
        let sample = oxyn_ai::context::RowSample::new(
            fixture.customers.to_path().expect("a path"),
            vec!["email".to_owned()],
            vec![vec![oxyn_core::ScalarValue::Text(
                "user1@example.com".to_owned(),
            )]],
        );
        let (dialogue, _) = run
            .prepare_dialogue(
                &sql_agent(),
                fixture.session,
                "which plans?",
                Some(sample),
                PrivacyTier::Metadata,
            )
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let prompt = prompt_of(&dialogue);
        assert!(!prompt.contains("example.com"), "{prompt}");
    }

    /// A provider that accepts connections and never answers: whether a
    /// request reached it is whether a connection was accepted.
    fn listening() -> (std::net::TcpListener, u16) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        listener.set_nonblocking(true).expect("non-blocking");
        let port = listener.local_addr().expect("an address").port();
        (listener, port)
    }

    fn approved(request: &SampleRequest, columns: &[String]) -> SampleApproval {
        SampleApproval {
            request: request.id.clone(),
            source: request.address.clone(),
            columns: columns.to_vec(),
        }
    }

    fn question_with(fixture: &Fixture, provider: &str, sample: SampleApproval) -> AskRequest {
        AskRequest {
            connection: fixture.open.connection.clone(),
            session: fixture.open.session.clone(),
            thread: None,
            parent: None,
            question: "which plans?".to_owned(),
            destination: DestinationChoice::Provider {
                id: provider.to_owned(),
                model: None,
                effort: None,
            },
            sample: Some(sample),
        }
    }

    #[test]
    fn a_sample_is_recorded_before_it_leaves_keyed_on_the_preview_that_read_it() {
        let fixture = fixture();
        let _guard = fixture.runtime.enter();
        let (listener, port) = listening();
        let provider = fixture.declare_at(port);
        let request = fixture.offer_of(&fixture.customers, &provider);

        let (started, received) = fixture.ask(question_with(
            &fixture,
            &provider,
            approved(&request, &["email".to_owned()]),
        ));
        let started = started
            .map_err(|error| error.message)
            .expect("the question starts");
        let mut reached = false;
        for _ in 0..400 {
            if listener.accept().is_ok() {
                reached = true;
                break;
            }
            fixture
                .runtime
                .block_on(tokio::time::sleep(std::time::Duration::from_millis(25)));
        }
        assert!(reached, "{}", received.lock().join("\n"));

        let recorded = fixture.egress();
        assert_eq!(recorded.len(), 1, "one send, one entry");
        let entry = &recorded[0];
        assert!(entry.source.contains("customers"), "{}", entry.source);
        assert_eq!(entry.columns, vec!["email".to_owned()]);
        assert_eq!(entry.rows, 5);
        assert_eq!(entry.recipient.as_str(), provider);
        assert_eq!(entry.model.as_deref(), Some("llama3"));
        assert_eq!(entry.reach, EgressReach::Local);
        let previews: Vec<_> = fixture
            .backend
            .inner
            .executor
            .store()
            .journal()
            .recent(256)
            .expect("journal")
            .into_iter()
            .filter(|logged| logged.record.command_kind == "PreviewRelation")
            .filter_map(|logged| logged.record.command_id)
            .collect();
        assert!(
            entry.command.is_some_and(|read| previews.contains(&read)),
            "the entry names the journaled preview"
        );
        fixture
            .backend
            .ai_cancel(fixture.connection, &started.thread)
            .expect("cancelled");
    }

    #[test]
    fn a_send_that_cannot_be_recorded_does_not_leave() {
        let fixture = fixture();
        let _guard = fixture.runtime.enter();
        let (listener, port) = listening();
        let provider = fixture.declare_at(port);
        let request = fixture.offer_of(&fixture.wide, &provider);

        let (started, received) = fixture.ask(question_with(
            &fixture,
            &provider,
            approved(&request, &[unrecordable_column()]),
        ));
        started
            .map_err(|error| error.message)
            .expect("the question starts");
        let events = ending(&fixture, &received);
        assert!(events.contains("audit trail"), "{events}");
        assert!(
            !events.contains(&unrecordable_column()),
            "the refusal quotes no name"
        );
        assert!(fixture.reads() > 0, "read, then held back");
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "nothing reached the provider"
        );
        assert!(fixture.egress().is_empty());
    }
}
