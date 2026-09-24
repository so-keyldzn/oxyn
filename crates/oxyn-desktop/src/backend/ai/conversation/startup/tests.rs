use std::collections::BTreeMap;

use agent_client_protocol::schema::v1::{
    InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse, PromptRequest,
    PromptResponse, SessionId as AcpSessionId, SessionMode, SessionModeState, StopReason,
};
use agent_client_protocol::{Agent, Channel as Duplex};
use futures::FutureExt as _;
use oxyn_ai::external::mcp::ToolTurns;
use oxyn_ai::external::session::ExternalSession;
use oxyn_core::{
    Actor, AgentId, AgentSessionId, CommandId, ConnectionId, Environment, ExternalAgentConfig,
    PrivacyTier, ProviderId, SessionId,
};
use parking_lot::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};

use super::super::super::threads::{AgentLink, Waiting, WithdrawOnRelease};
use crate::backend::Backend;
use crate::ipc::ai::{AgentStart, AgentStartRequest, AiUpdate, AskRequest, DestinationChoice};
use crate::ipc::{ConnectResponse, ConnectionDraft, OpenConnection};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

fn recording() -> (Channel<AiUpdate>, std::sync::Arc<Mutex<Vec<String>>>) {
    let received = std::sync::Arc::new(Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&received);
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(json) = body {
            sink.lock().push(json);
        }
        Ok(())
    });
    (channel, received)
}

fn open(runtime: &tokio::runtime::Runtime, backend: &Backend) -> OpenConnection {
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "startup regression".into(),
        environment: Environment::Local,
        privacy_tier: PrivacyTier::Metadata,
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

/// An agent declared with a program that does not exist: any launch of it
/// fails as « not found », so a start that succeeds used the waiting one.
fn declare(backend: &Backend) -> ExternalAgentConfig {
    let agent = ExternalAgentConfig::new(
        ProviderId::for_new_agent(),
        "Fake agent",
        "oxyn-no-such-agent-7d2e",
    );
    backend
        .inner
        .executor
        .store()
        .external_agents()
        .save(&agent)
        .expect("declared");
    agent
}

/// Counts the fake agent's `initialize` and `session/new`.
#[derive(Default)]
struct Calls {
    initialize: usize,
    new_session: usize,
    prompts: usize,
}

/// A session with an in-memory agent that declares a « plan » mode and ends
/// every prompt at once.
fn fake_agent(
    runtime: &tokio::runtime::Runtime,
) -> (ExternalSession, std::sync::Arc<Mutex<Calls>>) {
    let calls = std::sync::Arc::new(Mutex::new(Calls::default()));
    let (ours, theirs) = Duplex::duplex();
    let (session, driver) = ExternalSession::over(ours, std::env::temp_dir().join("unused"));
    runtime.spawn(driver);
    let (initialized, opened, prompted) = (
        std::sync::Arc::clone(&calls),
        std::sync::Arc::clone(&calls),
        std::sync::Arc::clone(&calls),
    );
    runtime.spawn(
        Agent
            .builder()
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    initialized.lock().initialize += 1;
                    responder.respond(InitializeResponse::new(request.protocol_version))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_: NewSessionRequest, responder, _cx| {
                    opened.lock().new_session += 1;
                    responder.respond(NewSessionResponse::new(AcpSessionId::new("s")).modes(
                        SessionModeState::new("plan", vec![SessionMode::new("plan", "Plan")]),
                    ))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_: PromptRequest, responder, _cx| {
                    prompted.lock().prompts += 1;
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(theirs)
            .boxed(),
    );
    (session, calls)
}

fn waiting(
    backend: &Backend,
    agent: &ExternalAgentConfig,
    session: ExternalSession,
    scope: SessionId,
) -> Waiting {
    Waiting {
        link: AgentLink {
            agent: agent.clone(),
            tier: PrivacyTier::Metadata,
            leaf: None,
            session: std::sync::Arc::new(session),
            tools: ToolTurns::new(8, std::sync::Arc::new(|| false)),
            actor: (AgentId::new(), AgentSessionId::new()),
            _requests: WithdrawOnRelease::new(
                std::sync::Arc::clone(&backend.inner.executor),
                std::sync::Arc::clone(&backend.inner.ai.decisions),
                Actor::agent(AgentId::new(), AgentSessionId::new()),
            ),
        },
        session: scope,
        stop: oxyn_core::CancelToken::new(),
    }
}

fn start_request(open: &OpenConnection, agent: &ExternalAgentConfig) -> AgentStartRequest {
    AgentStartRequest {
        connection: open.connection.clone(),
        session: open.session.clone(),
        thread: None,
        parent: None,
        agent: agent.id.to_string(),
    }
}

#[test]
fn a_started_agent_offers_its_settings_before_any_question() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let agent = declare(&backend);
    let (session, calls) = fake_agent(&runtime);
    backend.inner.ai.wait(
        connection,
        waiting(
            &backend,
            &agent,
            session,
            open.session.parse().expect("session"),
        ),
    );

    let started = runtime
        .block_on(backend.ai_start_agent(start_request(&open, &agent)))
        .expect("started");
    let AgentStart::Ready { settings, .. } = started else {
        panic!("the waiting agent answers: {started:?}");
    };
    assert_eq!(settings.current_mode.as_deref(), Some("plan"));
    let calls = calls.lock();
    assert_eq!(
        (calls.initialize, calls.new_session, calls.prompts),
        (1, 1, 0)
    );
}

/// The shape of what Claude Agent 0.78.0 declares on `session/new`, as
/// measured on 2026-09-23: `model`, `effort` (`thought_level`), `fast`
/// (`model_config`). The choices are stand-ins, cut to a few: their names are
/// the adapter's and are not recorded.
fn claude_options(effort: &str) -> Vec<agent_client_protocol::schema::v1::SessionConfigOption> {
    use agent_client_protocol::schema::v1::{
        SessionConfigBoolean, SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
        SessionConfigSelect, SessionConfigSelectOption,
    };
    vec![
        SessionConfigOption::new(
            "model".to_owned(),
            "Model",
            SessionConfigKind::Select(SessionConfigSelect::new(
                "model-1".to_owned(),
                vec![
                    SessionConfigSelectOption::new("model-1", "Model 1"),
                    SessionConfigSelectOption::new("model-2", "Model 2"),
                    SessionConfigSelectOption::new("model-3", "Model 3"),
                ],
            )),
        )
        .category(SessionConfigOptionCategory::Model),
        SessionConfigOption::new(
            "effort".to_owned(),
            "Effort",
            SessionConfigKind::Select(SessionConfigSelect::new(
                effort.to_owned(),
                vec![
                    SessionConfigSelectOption::new("low", "Low"),
                    SessionConfigSelectOption::new("medium", "Medium"),
                    SessionConfigSelectOption::new("high", "High"),
                ],
            )),
        )
        .category(SessionConfigOptionCategory::ThoughtLevel),
        SessionConfigOption::new(
            "fast".to_owned(),
            "Fast mode",
            SessionConfigKind::Boolean(SessionConfigBoolean::new(false)),
        )
        .category(SessionConfigOptionCategory::ModelConfig),
    ]
}

#[test]
fn everything_the_agent_declares_is_offered_and_settable_before_any_question() {
    use agent_client_protocol::schema::v1::{
        SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
    };
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let agent = declare(&backend);

    let changes = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = std::sync::Arc::clone(&changes);
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
                    responder.respond(
                        NewSessionResponse::new(AcpSessionId::new("s"))
                            .config_options(claude_options("medium")),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: SetSessionConfigOptionRequest, responder, _cx| {
                    seen.lock().push(request.config_id.to_string());
                    responder.respond(SetSessionConfigOptionResponse::new(claude_options("high")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(theirs)
            .boxed(),
    );
    backend.inner.ai.wait(
        connection,
        waiting(
            &backend,
            &agent,
            session,
            open.session.parse().expect("session"),
        ),
    );

    let started = runtime
        .block_on(backend.ai_start_agent(start_request(&open, &agent)))
        .expect("started");
    let AgentStart::Ready { settings, .. } = started else {
        panic!("ready: {started:?}");
    };
    // Model, effort and the fast switch — the selector's three places.
    let categories: Vec<&str> = settings
        .options
        .iter()
        .map(|option| option.category)
        .collect();
    assert_eq!(categories, ["model", "thoughtLevel", "modelConfig"]);

    // No conversation yet: the change goes to the started agent, through
    // `change_setting`, and the panel shows what the agent answered.
    let answer = runtime
        .block_on(
            backend.ai_set_agent_setting(
                connection,
                None,
                serde_json::from_value(serde_json::json!({ "option": "effort", "value": "high" }))
                    .expect("a change"),
            ),
        )
        .expect("sent");
    let crate::ipc::ai::AgentSettingAnswer::Sent { settings } = answer else {
        panic!("accepted: {answer:?}");
    };
    let effort = settings
        .options
        .iter()
        .find(|option| option.id == "effort")
        .expect("effort");
    assert!(
        matches!(&effort.value, crate::ipc::ai::AgentOptionValue::Select { current, .. } if current == "high"),
        "{effort:?}"
    );
    assert_eq!(*changes.lock(), ["effort"]);
}

#[test]
fn the_first_question_takes_the_started_agent_rather_than_launching_one() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let agent = declare(&backend);
    let (session, calls) = fake_agent(&runtime);
    backend.inner.ai.wait(
        connection,
        waiting(
            &backend,
            &agent,
            session,
            open.session.parse().expect("session"),
        ),
    );
    let started = runtime
        .block_on(backend.ai_start_agent(start_request(&open, &agent)))
        .expect("started");
    assert!(matches!(started, AgentStart::Ready { .. }), "{started:?}");

    let (channel, received) = recording();
    let asked = runtime
        .block_on(backend.ai_ask(
            AskRequest {
                mentions: Vec::new(),
                connection: open.connection.clone(),
                session: open.session.clone(),
                thread: None,
                parent: None,
                question: "which tables?".to_owned(),
                destination: DestinationChoice::Agent {
                    id: agent.id.to_string(),
                },
                sample: None,
            },
            channel,
        ))
        .expect("accepted");
    let ended = runtime.block_on(async {
        for _ in 0..200 {
            let ended = received.lock().iter().any(|json| {
                json.contains(r#""kind":"finished""#) || json.contains(r#""kind":"failed""#)
            });
            if ended {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    });
    assert!(ended, "{:?}", received.lock());
    let events = received.lock().join("\n");
    // A launch of this agent's program would have failed as « not found ».
    assert!(!events.contains(r#""kind":"failed""#), "{events}");
    let calls = calls.lock();
    assert_eq!(
        (calls.initialize, calls.new_session, calls.prompts),
        (1, 1, 1),
        "one start, then the question on the same session"
    );
    assert!(backend.inner.ai.waiting_session(connection).is_none());
    let thread = backend
        .inner
        .ai
        .find(connection, &asked.thread)
        .expect("the conversation");
    assert!(thread.live_agent().is_some(), "the conversation keeps it");
}

#[test]
fn a_replaced_agent_is_not_served_by_the_process_of_the_command_it_replaced() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let agent = declare(&backend);
    let (session, calls) = fake_agent(&runtime);
    backend.inner.ai.wait(
        connection,
        waiting(
            &backend,
            &agent,
            session,
            open.session.parse().expect("session"),
        ),
    );
    // « Replace… » keeps the id and rewrites the command.
    let replaced = ExternalAgentConfig {
        command: "oxyn-no-such-agent-replaced-4b1c".to_owned(),
        ..agent.clone()
    };
    backend
        .inner
        .executor
        .store()
        .external_agents()
        .save(&replaced)
        .expect("replaced");

    let started = runtime
        .block_on(backend.ai_start_agent(start_request(&open, &replaced)))
        .expect("answered");
    assert!(!matches!(started, AgentStart::Ready { .. }), "{started:?}");
    assert_eq!(
        calls.lock().initialize,
        0,
        "the old process was never asked"
    );
}

#[test]
fn a_local_only_connection_starts_nothing_and_releases_what_waited() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let agent = declare(&backend);
    let (session, _calls) = fake_agent(&runtime);
    backend.inner.ai.wait(
        connection,
        waiting(
            &backend,
            &agent,
            session,
            open.session.parse().expect("session"),
        ),
    );
    runtime
        .block_on(
            backend.update_connection(
                CommandId::new(),
                connection,
                crate::ipc::settings::ConnectionEdit {
                    name: "startup regression".into(),
                    environment: Environment::Local,
                    privacy_tier: PrivacyTier::Local,
                    read_only: false,
                    values: [("path".to_owned(), ":memory:".to_owned())]
                        .into_iter()
                        .collect(),
                    secrets: BTreeMap::new(),
                },
            ),
        )
        .expect("saved");

    let refused = runtime.block_on(backend.ai_start_agent(start_request(&open, &agent)));
    let error = refused.expect_err("refused before launch");
    assert!(error.message.contains("local-only"), "{}", error.message);
    assert!(backend.inner.ai.waiting_session(connection).is_none());
}

#[test]
fn stopping_a_start_releases_the_agent_and_says_cancelled() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let agent = declare(&backend);
    // An agent that never answers `initialize`: the first `npx` download.
    let (ours, _theirs) = Duplex::duplex();
    let (session, driver) = ExternalSession::over(ours, std::env::temp_dir().join("unused"));
    runtime.spawn(driver);
    backend.inner.ai.wait(
        connection,
        waiting(
            &backend,
            &agent,
            session,
            open.session.parse().expect("session"),
        ),
    );

    let starting = {
        let backend = backend.clone();
        let request = start_request(&open, &agent);
        runtime.spawn(async move { backend.ai_start_agent(request).await })
    };
    runtime.block_on(tokio::time::sleep(std::time::Duration::from_millis(50)));
    assert!(backend.ai_stop_agent_start(connection));
    let ended = runtime
        .block_on(starting)
        .expect("the start ends")
        .expect("answered");
    assert!(matches!(ended, AgentStart::Cancelled), "{ended:?}");
    assert!(backend.inner.ai.waiting_session(connection).is_none());
    assert!(
        !backend.ai_stop_agent_start(connection),
        "nothing left to stop"
    );
}

#[test]
fn a_start_that_times_out_names_the_agent_and_the_bound() {
    let failure = super::super::timed_out(&ExternalAgentConfig::new(
        ProviderId::for_new_agent(),
        "Claude Code",
        "npx",
    ));
    assert_eq!(
        failure.category,
        crate::ipc::ai::FailureCategory::AgentTimedOut
    );
    assert!(
        failure
            .message
            .starts_with("Claude Code did not finish starting within 120 seconds")
    );
    // Never « ask again » on its own: the next start is a click, and a
    // failure after a write is never retryable (I-13).
    assert!(!failure.retryable(true));
    // Stopped by Oxyn: the next question launches a new agent.
    assert!(failure.ends_agent);
}

#[cfg(unix)]
#[test]
fn an_agent_that_dies_says_why_and_nothing_it_was_handed() {
    // A real process, so the report goes the whole way: `spawn` reads and
    // redacts the error output, the session's driver keeps it, the failure
    // carries it.
    let runtime = runtime();
    let _guard = runtime.enter();
    let secret = "sk-oxyn-test-2f8c1d";
    let mut agent = ExternalAgentConfig::new(ProviderId::for_new_agent(), "Broken agent", "sh")
        .with_args([
            "-c",
            "echo \"cannot start: key $OXYN_TEST_KEY refused\" >&2; exit 3",
        ]);
    agent.env = vec![("OXYN_TEST_KEY".to_owned(), secret.to_owned())];
    let (session, driver) =
        ExternalSession::launch(&agent, PrivacyTier::Metadata).expect("launched");
    runtime.spawn(driver);

    let failure = runtime.block_on(async {
        let error = session.start().await.expect_err("the agent died");
        super::super::agent_failure_of(&agent, error, &session).await
    });
    assert_eq!(
        failure.category,
        crate::ipc::ai::FailureCategory::AgentExited
    );
    // The conversation lets a dead agent go: « ask again » starts a new one,
    // as its message says.
    assert!(failure.ends_agent);
    let exit = failure.exit.expect("the report reached the failure");
    assert_eq!(exit.code, Some(3));
    assert!(exit.output.contains("cannot start"), "{}", exit.output);
    assert!(!exit.output.contains(secret), "{}", exit.output);
}
