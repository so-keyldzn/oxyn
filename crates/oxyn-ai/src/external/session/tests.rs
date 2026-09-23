//! The session against an in-memory agent: no process, the real protocol.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AuthMethodAgent, AuthenticateResponse, ContentChunk, InitializeResponse, McpCapabilities,
    NewSessionResponse, PermissionOption, PermissionOptionId, PermissionOptionKind, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, ToolCall, ToolCallId, ToolCallUpdate,
    ToolCallUpdateFields, UsageUpdate,
};
use agent_client_protocol::{Agent, Channel, Error};
use futures::FutureExt;
use futures::executor::block_on;
use futures::future::{join, select};

use super::*;
use crate::observer::AgentEvent;

/// What the panel would have drawn.
#[derive(Default)]
struct Seen(Mutex<Vec<String>>);

impl AgentObserver for Seen {
    fn observe(&self, event: AgentEvent<'_>) {
        let line = match event {
            AgentEvent::TextDelta { text } => format!("text:{text}"),
            AgentEvent::ThinkingDelta { text } => format!("thinking:{text}"),
            AgentEvent::ExternalToolCall { kind, status, .. } => {
                format!("tool:{}:{status:?}", kind.unwrap_or("?"))
            }
            AgentEvent::PermissionRefused { kind, .. } => format!("refused:{kind}"),
            AgentEvent::ContextWindow { used, size, cost } => {
                format!("context:{used}/{size}:{cost:?}")
            }
            AgentEvent::CallRejected { .. } => return,
            AgentEvent::AgentSettings(settings) => format!(
                "settings:mode={}:options={}",
                settings.current_mode.as_deref().unwrap_or("-"),
                settings
                    .options
                    .iter()
                    .map(|option| match &option.value {
                        crate::external::settings::OptionValue::Select { current, .. } => {
                            format!("{}={current}", option.id)
                        }
                        crate::external::settings::OptionValue::Boolean(on) => {
                            format!("{}={on}", option.id)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            other => format!("{other:?}"),
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
    }
}

impl Seen {
    fn lines(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn prompt(text: &str) -> AgentPrompt {
    AgentPrompt::from_user(PrivacyTier::Metadata, text).expect("a question under metadata")
}

/// Runs a scenario: the session, its driver and the fake agent together, until
/// the scenario returns.
fn run<T>(
    agent: impl FnOnce(Channel) -> BoxFuture<'static, Result<(), Error>>,
    scenario: impl AsyncFnOnce(ExternalSession) -> T,
) -> T {
    run_with(None, agent, scenario)
}

/// [`run`], for a session that also declares Oxyn's tool endpoint.
fn run_with<T>(
    tools: Option<ToolEndpoint>,
    agent: impl FnOnce(Channel) -> BoxFuture<'static, Result<(), Error>>,
    scenario: impl AsyncFnOnce(ExternalSession) -> T,
) -> T {
    let (ours, theirs) = Channel::duplex();
    let (session, driver) = ExternalSession::over_with(ours, private(), tools, None);
    let background = join(driver, agent(theirs));
    block_on(async move {
        let scenario = pin!(scenario(session));
        match select(scenario, pin!(background)).await {
            Either::Left((value, _)) => value,
            Either::Right(_) => panic!("the agent stopped before the scenario ended"),
        }
    })
}

/// The directory a test session announces; never read, only compared.
fn private() -> std::path::PathBuf {
    std::path::PathBuf::from("/oxyn-test/private-agent-directory")
}

fn session_id() -> SessionId {
    SessionId::new("s-1")
}

#[test]
fn a_conversation_keeps_one_session_and_streams_what_can_be_shown() {
    let sessions = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&sessions);
    let seen = Arc::new(Seen::default());
    let observer: Arc<dyn AgentObserver> = seen.clone();

    let ends = run(
        move |channel| {
            Agent
                .builder()
                .on_receive_request(
                    async |request: InitializeRequest, responder, _cx| {
                        responder.respond(InitializeResponse::new(request.protocol_version))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: NewSessionRequest, responder, _cx| {
                        counted.fetch_add(1, Ordering::SeqCst);
                        responder.respond(NewSessionResponse::new(session_id()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async |_: PromptRequest, responder, cx| {
                        let text =
                            |t: &str| ContentChunk::new(ContentBlock::Text(TextContent::new(t)));
                        for update in [
                            SessionUpdate::AgentThoughtChunk(text("looking")),
                            SessionUpdate::ToolCall(
                                ToolCall::new(ToolCallId::new("t1"), "Read /etc/hosts")
                                    .kind(ToolKind::Read),
                            ),
                            SessionUpdate::AgentMessageChunk(text("hello")),
                            SessionUpdate::UsageUpdate(UsageUpdate::new(1200, 200_000)),
                        ] {
                            cx.send_notification(SessionNotification::new(session_id(), update))?;
                        }
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            let first = session
                .prompt(&prompt("one"), Arc::clone(&observer), &CancelToken::new())
                .await;
            let second = session
                .prompt(&prompt("two"), observer, &CancelToken::new())
                .await;
            (first, second)
        },
    );

    assert_eq!(ends.0, Ok(TurnEnd::Answered { truncated: false }));
    assert_eq!(ends.1, Ok(TurnEnd::Answered { truncated: false }));
    assert_eq!(
        sessions.load(Ordering::SeqCst),
        1,
        "the agent remembers: one session"
    );
    let lines = seen.lines();
    assert!(lines.contains(&"thinking:looking".to_owned()), "{lines:?}");
    assert!(lines.contains(&"text:hello".to_owned()), "{lines:?}");
    assert!(lines.contains(&"tool:read:Pending".to_owned()), "{lines:?}");
    assert!(
        lines.contains(&"context:1200/200000:None".to_owned()),
        "{lines:?}"
    );
    // The agent's title quotes a path of the machine: it is not relayed.
    assert!(
        lines.iter().all(|line| !line.contains("/etc/hosts")),
        "{lines:?}"
    );
}

#[test]
fn the_agents_settings_follow_it_and_nothing_it_does_not_show_is_relayed() {
    use agent_client_protocol::schema::v1::{
        AvailableCommand, AvailableCommandsUpdate, ConfigOptionUpdate, CurrentModeUpdate,
        SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelect,
        SessionConfigSelectOption, SessionInfoUpdate, SessionMode, SessionModeState,
    };

    let effort = |current: &str| {
        SessionConfigOption::new(
            "effort".to_owned(),
            "Effort",
            SessionConfigKind::Select(SessionConfigSelect::new(
                current.to_owned(),
                vec![
                    SessionConfigSelectOption::new("low", "Low"),
                    SessionConfigSelectOption::new("high", "High"),
                ],
            )),
        )
        .category(SessionConfigOptionCategory::ThoughtLevel)
    };
    let prompts = Arc::new(AtomicUsize::new(0));
    let first = Arc::new(Seen::default());
    let second = Arc::new(Seen::default());
    let observers: (Arc<dyn AgentObserver>, Arc<dyn AgentObserver>) =
        (first.clone(), second.clone());

    let between = run(
        move |channel| {
            Agent
                .builder()
                .on_receive_request(
                    async |request: InitializeRequest, responder, _cx| {
                        responder.respond(InitializeResponse::new(request.protocol_version))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: NewSessionRequest, responder, _cx| {
                        responder.respond(
                            NewSessionResponse::new(session_id())
                                .modes(SessionModeState::new(
                                    "ask",
                                    vec![
                                        SessionMode::new("ask", "Ask"),
                                        SessionMode::new("plan", "Plan"),
                                    ],
                                ))
                                .config_options(vec![effort("high")]),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: PromptRequest, responder, cx| {
                        let send = |update| {
                            cx.send_notification(SessionNotification::new(session_id(), update))
                        };
                        if prompts.fetch_add(1, Ordering::SeqCst) == 0 {
                            send(SessionUpdate::UserMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new("replayed question")),
                            )))?;
                            send(SessionUpdate::SessionInfoUpdate(
                                SessionInfoUpdate::new().title("/Users/someone/secret".to_owned()),
                            ))?;
                            send(SessionUpdate::AvailableCommandsUpdate(
                                AvailableCommandsUpdate::new(vec![AvailableCommand::new(
                                    "review",
                                    "Reviews the project",
                                )]),
                            ))?;
                            send(SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
                                vec![effort("low")],
                            )))?;
                            send(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(
                                "plan",
                            )))?;
                        }
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: agent_client_protocol::schema::v1::SetSessionModeRequest,
                                responder,
                                cx| {
                        // Between two questions for certain: a change is only
                        // handled when none is being answered.
                        cx.send_notification(SessionNotification::new(
                            session_id(),
                            SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(vec![
                                effort("high"),
                            ])),
                        ))?;
                        responder.respond(
                            agent_client_protocol::schema::v1::SetSessionModeResponse::new(),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            let live = session.settings();
            let _ = session
                .prompt(&prompt("one"), observers.0, &CancelToken::new())
                .await;
            // The agent notifies before answering: the answer is dispatched
            // after the notification, so that one has been kept by now.
            let _ = session
                .change_setting(crate::external::settings::SettingChange::Mode {
                    id: "ask".to_owned(),
                })
                .await;
            let between = (
                live.borrow().current_mode.clone(),
                live.borrow()
                    .options
                    .iter()
                    .map(|option| format!("{:?}", option.value))
                    .collect::<Vec<_>>(),
            );
            let _ = session
                .prompt(&prompt("two"), observers.1, &CancelToken::new())
                .await;
            between
        },
    );

    assert_eq!(
        first.lines(),
        vec!["settings:mode=ask:options=effort=high".to_owned()],
        "the settings a question starts from — and neither the replay, the title nor the \
         commands. Changes go to the session's `watch`, not to the question"
    );
    assert!(
        between.1.len() == 1 && between.1[0].contains(r#"current: "high""#),
        "a change sent between two questions reaches the watch: {between:?}"
    );
    assert_eq!(
        second.lines(),
        vec!["settings:mode=ask:options=effort=high".to_owned()],
        "a new question starts from the settings in force, not from the declaration"
    );
}

#[test]
fn a_session_that_needs_a_sign_in_says_how_and_recovers_after_it() {
    let signed_in = Arc::new(AtomicUsize::new(0));
    let checked = Arc::clone(&signed_in);
    let marked = Arc::clone(&signed_in);

    let (first, after) = run(
        move |channel| {
            Agent
                .builder()
                .on_receive_request(
                    async |request: InitializeRequest, responder, _cx| {
                        responder.respond(
                            InitializeResponse::new(request.protocol_version).auth_methods(vec![
                                AuthMethod::Agent(AuthMethodAgent::new("chat-gpt", "ChatGPT")),
                            ]),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: NewSessionRequest, responder, _cx| {
                        if checked.load(Ordering::SeqCst) == 0 {
                            return responder.respond_with_error(Error::auth_required());
                        }
                        responder.respond(NewSessionResponse::new(session_id()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: AuthenticateRequest, responder, _cx| {
                        marked.fetch_add(1, Ordering::SeqCst);
                        responder.respond(AuthenticateResponse::new())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async |session| {
            let first = session.start().await;
            let signed = session.authenticate("chat-gpt").await;
            assert_eq!(signed, Ok(()));
            (first, session.start().await)
        },
    );

    assert_eq!(
        first,
        Err(ExternalError::AuthRequired {
            methods: vec![AuthOption {
                id: "chat-gpt".into(),
                name: "ChatGPT".into(),
                description: None,
                kind: AuthKind::Agent,
            }],
        })
    );
    assert!(after.is_ok(), "{after:?}");
    assert_eq!(signed_in.load(Ordering::SeqCst), 1);
}

/// What the agent reads on the wire, end to end: the structure of the open
/// database, fenced, and no row value — the failure the user met was an agent
/// that knew nothing of the base and proposed `SELECT * FROM your_table`.
#[test]
fn the_agent_receives_the_structure_of_the_database_and_no_value() {
    use oxyn_catalog::model::{Field, LogicalType, Relation, RelationKind};
    use oxyn_catalog::{CatalogCache, CatalogPath};

    let mut cache = CatalogCache::new();
    let orders = CatalogPath::for_relation(None, Some("main"), "orders").expect("a valid path");
    cache
        .set_relation(
            &orders,
            Relation::new("orders", RelationKind::Table)
                .with_estimated_rows(11)
                .with_fields(vec![
                    Field::new("id", 0, LogicalType::INT64, "INTEGER").primary_key(),
                    Field::new("placed_at", 1, LogicalType::Text, "TEXT"),
                ]),
        )
        .expect("the path names a relation");

    let received = Arc::new(Mutex::new(Vec::<String>::new()));
    let kept = Arc::clone(&received);
    let invite = AgentPrompt::with_schema(
        PrivacyTier::Sampled,
        "the last 10 rows",
        &cache,
        oxyn_core::QueryLanguage::Sql(oxyn_core::SqlDialect::Sqlite),
        Vec::new(),
    )
    .expect("an external agent is allowed under sampled");

    let end = run(
        move |channel| {
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
                        responder.respond(NewSessionResponse::new(session_id()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: PromptRequest, responder, _cx| {
                        for block in request.prompt {
                            if let ContentBlock::Text(text) = block {
                                kept.lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .push(text.text);
                            }
                        }
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            session
                .prompt(&invite, Arc::new(()), &CancelToken::new())
                .await
        },
    );
    assert_eq!(end, Ok(TurnEnd::Answered { truncated: false }));

    let text = received
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .join("\n");
    assert!(
        text.contains(r#"table "main"."orders""#),
        "the agent is told the table: {text}"
    );
    assert!(
        text.contains(r#""placed_at" TEXT"#),
        "and its columns: {text}"
    );
    assert!(
        text.contains(crate::untrusted::FENCE_OPEN),
        "fenced: {text}"
    );
    assert!(
        !text.contains("row sample approved by the user"),
        "no row value reaches an external agent, even under sampled: {text}"
    );
    assert!(text.ends_with("the last 10 rows"), "{text}");
}

#[test]
fn a_refusal_is_not_an_answer() {
    let end = run(
        |channel| {
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
                        responder.respond(NewSessionResponse::new(session_id()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async |_: PromptRequest, responder, _cx| {
                        responder.respond(PromptResponse::new(StopReason::Refusal))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async |session| {
            session
                .prompt(
                    &prompt("drop everything"),
                    Arc::new(()),
                    &CancelToken::new(),
                )
                .await
        },
    );
    assert_eq!(end, Ok(TurnEnd::Refused));
}

#[test]
fn another_protocol_version_is_named_not_guessed() {
    let started = run(
        |channel| {
            Agent
                .builder()
                .on_receive_request(
                    async |_: InitializeRequest, responder, _cx| {
                        responder.respond(InitializeResponse::new(ProtocolVersion::V0))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async |session| session.start().await,
    );
    assert_eq!(
        started,
        Err(ExternalError::Incompatible {
            agent: "0".into(),
            client: "1".into(),
        })
    );
}

#[test]
fn stopping_answers_at_once_and_tells_the_agent() {
    let cancelled = Arc::new(AtomicUsize::new(0));
    let told = Arc::clone(&cancelled);
    let waiting: Arc<Mutex<Option<oneshot::Sender<()>>>> = Arc::default();
    let armed = Arc::clone(&waiting);
    let token = CancelToken::new();
    let stopper = token.clone();
    let late: Arc<Mutex<Option<RequestPermissionOutcome>>> = Arc::default();
    let recorded = Arc::clone(&late);

    let end = run(
        move |channel| {
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
                        responder.respond(NewSessionResponse::new(session_id()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: PromptRequest, responder, cx| {
                        let (tx, rx) = oneshot::channel();
                        *armed.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
                        // The prompt is being worked on: the client may stop it.
                        stopper.cancel();
                        cx.spawn(async move {
                            let _ = rx.await;
                            responder.respond(PromptResponse::new(StopReason::Cancelled))
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_notification(
                    async move |_: CancelNotification, cx| {
                        told.fetch_add(1, Ordering::SeqCst);
                        let waiting = Arc::clone(&waiting);
                        let recorded = Arc::clone(&recorded);
                        let asking = cx.clone();
                        cx.spawn(async move {
                            // Asked after the stop, for a kind Oxyn grants
                            // while a question is followed.
                            let answer = asking
                                .send_request(RequestPermissionRequest::new(
                                    session_id(),
                                    ToolCallUpdate::new(
                                        ToolCallId::new("late"),
                                        ToolCallUpdateFields::new().kind(ToolKind::Think),
                                    ),
                                    vec![PermissionOption::new(
                                        PermissionOptionId::new("allow"),
                                        "allow",
                                        PermissionOptionKind::AllowOnce,
                                    )],
                                ))
                                .block_task()
                                .await?;
                            *recorded.lock().unwrap_or_else(PoisonError::into_inner) =
                                Some(answer.outcome);
                            if let Some(tx) = waiting
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .take()
                            {
                                let _ = tx.send(());
                            }
                            Ok(())
                        })
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            let end = session
                .prompt(&prompt("a long one"), Arc::new(()), &token)
                .await;
            // The next start waits for the cancelled prompt to close.
            let _ = session.start().await;
            end
        },
    );
    assert_eq!(end, Ok(TurnEnd::Cancelled));
    assert_eq!(cancelled.load(Ordering::SeqCst), 1, "the agent was told");
    assert_eq!(
        *late.lock().unwrap_or_else(PoisonError::into_inner),
        Some(RequestPermissionOutcome::Cancelled),
        "after `session/cancel`, every permission is answered `Cancelled`"
    );
}

#[test]
fn a_request_to_act_on_the_machine_is_refused_and_shown() {
    let seen = Arc::new(Seen::default());
    let observer: Arc<dyn AgentObserver> = seen.clone();
    let chosen: Arc<Mutex<Option<RequestPermissionOutcome>>> = Arc::default();
    let recorded = Arc::clone(&chosen);

    let end = run(
        move |channel| {
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
                        responder.respond(NewSessionResponse::new(session_id()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: PromptRequest, responder, cx| {
                        let asking = cx.clone();
                        let recorded = Arc::clone(&recorded);
                        cx.spawn(async move {
                            let options = [
                                (PermissionOptionKind::AllowOnce, "allow"),
                                (PermissionOptionKind::RejectOnce, "reject"),
                            ]
                            .into_iter()
                            .map(|(kind, id)| {
                                PermissionOption::new(PermissionOptionId::new(id), id, kind)
                            })
                            .collect();
                            let answer = asking
                                .send_request(RequestPermissionRequest::new(
                                    session_id(),
                                    ToolCallUpdate::new(
                                        ToolCallId::new("t9"),
                                        ToolCallUpdateFields::new().kind(ToolKind::Execute),
                                    ),
                                    options,
                                ))
                                .block_task()
                                .await?;
                            *recorded.lock().unwrap_or_else(PoisonError::into_inner) =
                                Some(answer.outcome);
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            session
                .prompt(&prompt("rm -rf"), observer, &CancelToken::new())
                .await
        },
    );

    assert_eq!(end, Ok(TurnEnd::Answered { truncated: false }));
    assert_eq!(seen.lines(), vec!["refused:execute".to_owned()]);
    assert_eq!(
        *chosen.lock().unwrap_or_else(PoisonError::into_inner),
        Some(RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new(PermissionOptionId::new("reject"))
        )),
        "no button grants it, and neither does Oxyn"
    );
}

#[test]
fn a_local_only_connection_launches_nothing() {
    let agent =
        ExternalAgentConfig::new(oxyn_core::ProviderId::for_new_agent(), "Claude Code", "npx");
    let refused = ExternalSession::launch(&agent, PrivacyTier::Local).map(|_| ());
    assert_eq!(refused, Err(ExternalError::RefusedByTier));
}

#[test]
fn oxyn_asks_for_nothing_on_the_machine_and_believes_only_what_the_agent_declares() {
    // The refusal that matters is the one the protocol enforces: an agent that
    // was never told Oxyn can read files cannot ask to read one. Checking the
    // declaration is therefore checking the refusal.
    let announced = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&announced);

    let ready = run(
        move |channel| {
            Agent
                .builder()
                .on_receive_request(
                    async move |request: InitializeRequest, responder, _cx| {
                        *recorded.lock().expect("no panic held the lock") =
                            Some(request.client_capabilities.clone());
                        responder.respond(
                            InitializeResponse::new(request.protocol_version)
                                .agent_capabilities(
                                    AgentCapabilities::new()
                                        .load_session(true)
                                        .mcp_capabilities(
                                            McpCapabilities::new().http(true).sse(true),
                                        ),
                                )
                                .agent_info(Implementation::new("Agent facti", "9.9.9")),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async |_: NewSessionRequest, responder, _cx| {
                        responder.respond(NewSessionResponse::new(session_id()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| session.start().await,
    )
    .expect("the agent started");

    let asked = announced
        .lock()
        .expect("no panic held the lock")
        .clone()
        .expect("the agent was initialized");
    assert!(!asked.fs.read_text_file, "Oxyn must not offer file reads");
    assert!(!asked.fs.write_text_file, "Oxyn must not offer file writes");
    assert!(!asked.terminal, "Oxyn must not offer a terminal");
    assert!(
        !asked.auth.terminal,
        "Oxyn cannot host an interactive sign-in"
    );
    assert!(
        asked.elicitation.is_none(),
        "Oxyn opens no form for an agent"
    );

    // And what the agent says about itself is read, not guessed.
    assert_eq!(ready.name.as_deref(), Some("Agent facti"));
    assert_eq!(ready.version.as_deref(), Some("9.9.9"));
    assert!(ready.can.mcp_http, "this agent accepts MCP over HTTP");
    assert!(ready.can.load_session);
    assert!(
        !ready.can.resume_session,
        "an undeclared capability is absent, never assumed"
    );
    assert!(!ready.can.close_session);
    assert!(!ready.can.embedded_context);
}

/// A log sink the test can read back.
#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Journal {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Journal {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

thread_local! {
    /// The journal this thread's test is writing, and whether it is capped.
    static CAPTURE: std::cell::RefCell<Option<(Journal, bool)>> =
        const { std::cell::RefCell::new(None) };
}

/// Writes into the journal of the test running on this thread, if any.
struct ThisThread;

impl std::io::Write for ThisThread {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        CAPTURE.with(|capture| {
            if let Some((journal, _)) = capture.borrow_mut().as_mut() {
                let _ = journal.write(bytes);
            }
        });
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ThisThread {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        Self
    }
}

/// Lets an event through only on a thread whose test is capturing, and never
/// lets `tracing` cache a callsite as disabled.
///
/// That cache is **global**: a `with_default` subscriber per test was not
/// enough. Another test hitting the same `warn!` with no subscriber cached it
/// as « never », and the journal of the test that looked came back empty — in
/// the full suite only.
struct Capturing;

impl<S> tracing_subscriber::layer::Filter<S> for Capturing {
    fn enabled(
        &self,
        meta: &tracing::Metadata<'_>,
        _cx: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        CAPTURE.with(|capture| {
            capture.borrow().as_ref().is_some_and(|(_, capped)| {
                !(*capped && crate::external::is_protocol_chatter(meta.target(), meta.level()))
            })
        })
    }

    fn callsite_enabled(
        &self,
        _meta: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }
}

/// Runs `scenario` with a journal open at `trace` on this thread, with or
/// without the host's cap, and returns what was written.
fn journal_while(capped: bool, scenario: impl FnOnce()) -> String {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;

    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(ThisThread)
                .with_ansi(false)
                .with_filter(Capturing),
        );
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
    // A callsite first reached before the subscriber existed kept « never ».
    tracing::callsite::rebuild_interest_cache();

    let journal = Journal::default();
    CAPTURE.with(|capture| *capture.borrow_mut() = Some((journal.clone(), capped)));
    scenario();
    CAPTURE.with(|capture| *capture.borrow_mut() = None);

    let bytes = journal
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

const TOKEN: &str = "oxyn-test-token-4f1d9c0b7e";
const QUESTION: &str = "how many invoices did Dupont SARL leave unpaid";

/// A whole conversation — tools declared, one question — under a journal open
/// at `trace`, with or without the host's cap. Returns what was written.
fn journal_of_a_conversation(capped: bool) -> String {
    journal_while(capped, || {
        let tools = ToolEndpoint {
            url: "http://127.0.0.1:9/mcp".to_owned(),
            token: TOKEN.to_owned(),
            over_acp: true,
        };
        let ended = run_with(
            Some(tools),
            |channel| {
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
                            responder.respond(NewSessionResponse::new(session_id()))
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .on_receive_request(
                        async |_: PromptRequest, responder, _cx| {
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .connect_to(channel)
                    .boxed()
            },
            async |session| {
                session
                    .prompt(
                        &prompt(QUESTION),
                        Arc::new(Seen::default()),
                        &CancelToken::new(),
                    )
                    .await
            },
        );
        assert_eq!(ended, Ok(TurnEnd::Answered { truncated: false }));
    })
}

#[test]
fn the_protocol_crate_writes_the_token_and_the_question_when_left_alone() {
    // The reason the cap exists, measured rather than asserted: without it, a
    // journal at `debug` holds the bearer token and the user's question. If
    // this ever stops holding, the crate changed — re-read it before removing
    // the cap, do not remove the cap because this failed.
    let journal = journal_of_a_conversation(false);
    assert!(
        journal.contains(TOKEN),
        "the crate no longer logs messages whole"
    );
    assert!(journal.contains(QUESTION));
}

#[test]
fn the_host_cap_keeps_the_token_and_the_question_out_of_a_debug_journal() {
    let journal = journal_of_a_conversation(true);
    assert!(!journal.contains(TOKEN), "the token reached the journal");
    assert!(
        !journal.contains(QUESTION),
        "the question reached the journal"
    );
}

#[test]
fn the_cap_is_on_the_crate_not_on_a_level() {
    use crate::external::is_protocol_chatter;
    use tracing::Level;

    for level in [Level::WARN, Level::INFO, Level::DEBUG, Level::TRACE] {
        assert!(is_protocol_chatter("agent_client_protocol", &level));
        assert!(is_protocol_chatter(
            "agent_client_protocol::jsonrpc::outgoing_actor",
            &level
        ));
    }
    // Its `warn` lines quote what the agent sent: capped too.
    assert!(is_protocol_chatter("agent_client_protocol", &Level::WARN));
    // A crate failure still reaches the journal.
    assert!(!is_protocol_chatter("agent_client_protocol", &Level::ERROR));
    // Oxyn's own logs, and a crate that merely shares the prefix, are untouched.
    assert!(!is_protocol_chatter("oxyn_ai::external", &Level::DEBUG));
    assert!(!is_protocol_chatter(
        "agent_client_protocol_rmcp",
        &Level::DEBUG
    ));
}

#[test]
fn the_session_is_announced_in_the_directory_the_process_owns() {
    // An agent loads its project's instructions and extra MCP servers from the
    // session's working directory. The shared temporary directory, writable by
    // every program of the user's, would let any of them inject both.
    let announced = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&announced);

    run(
        move |channel| {
            Agent
                .builder()
                .on_receive_request(
                    async |request: InitializeRequest, responder, _cx| {
                        responder.respond(InitializeResponse::new(request.protocol_version))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: NewSessionRequest, responder, _cx| {
                        *recorded.lock().unwrap_or_else(PoisonError::into_inner) =
                            Some(request.cwd.clone());
                        responder.respond(NewSessionResponse::new(session_id()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async |session| session.start().await,
    )
    .expect("the agent started");

    let cwd = announced
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .expect("a session was opened");
    assert_eq!(cwd, private());
    assert_ne!(cwd, std::env::temp_dir());
}

#[test]
fn a_refused_tier_opens_no_port() {
    // Run where no socket can exist — no Tokio reactor: binding one here would
    // panic. The refusal must therefore fall before the endpoint is served,
    // as it falls before the process is started (ADR-0026).
    let agent =
        ExternalAgentConfig::new(oxyn_core::ProviderId::for_new_agent(), "Claude Code", "npx");
    let service = Arc::new(crate::external::mcp::ToolService::new(
        crate::tools::ToolRegistry::builtin(),
        vec![crate::tools::EXECUTE_QUERY.to_owned()],
        crate::tools::ToolScope::new(
            oxyn_core::ConnectionId::new(),
            oxyn_core::SessionId::new(),
            oxyn_core::QueryLanguage::SQL,
        ),
        crate::external::mcp::TierCell::holding(PrivacyTier::Local),
        oxyn_core::Actor::agent(oxyn_core::AgentId::new(), oxyn_core::AgentSessionId::new()),
    ));
    let bridge = ToolBridge {
        service,
        turns: crate::external::mcp::ToolTurns::new(8, Arc::new(|| false)),
    };
    let refused = block_on(ExternalSession::launch_with_tools(
        &agent,
        PrivacyTier::Local,
        bridge,
    ))
    .map(|_| ());
    assert_eq!(refused, Err(ExternalError::RefusedByTier));
}

#[test]
fn an_agent_that_says_the_token_back_does_not_show_it() {
    // The agent received the token in `session/new`; an error quoting its MCP
    // configuration would put a live token in the panel.
    let failed = run_with(
        Some(ToolEndpoint {
            url: "http://127.0.0.1:9/mcp".to_owned(),
            token: TOKEN.to_owned(),
            over_acp: true,
        }),
        |channel| {
            Agent
                .builder()
                .on_receive_request(
                    async |request: InitializeRequest, responder, _cx| {
                        responder.respond(InitializeResponse::new(request.protocol_version))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async |request: NewSessionRequest, responder, _cx| {
                        responder.respond_with_error(Error::new(
                            -32603,
                            format!("cannot reach MCP server: {:?}", request.mcp_servers),
                        ))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async |session| session.start().await,
    );

    let Err(ExternalError::Protocol(message)) = failed else {
        panic!("expected the agent's error, got {failed:?}");
    };
    assert!(!message.contains(TOKEN), "{message}");
    assert!(message.contains("<tool token redacted>"), "{message}");
}

#[test]
fn a_token_is_redacted_wherever_it_appears_and_nothing_else_is() {
    assert_eq!(
        redact_token(&format!("Bearer {TOKEN} and {TOKEN}"), Some(TOKEN)),
        "Bearer <tool token redacted> and <tool token redacted>"
    );
    assert_eq!(
        redact_token("nothing to hide", Some(TOKEN)),
        "nothing to hide"
    );
    assert_eq!(redact_token("no endpoint", None), "no endpoint");
    // An empty token would « redact » between every character.
    assert_eq!(redact_token("abc", Some("")), "abc");
}

/// A conversation with an agent that sends a malformed update quoting the
/// question, then fails the prompt with a message quoting it too.
fn journal_of_a_misbehaving_agent(capped: bool) -> String {
    journal_while(capped, || {
        let ended = run(
            |channel| {
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
                            responder.respond(NewSessionResponse::new(session_id()))
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .on_receive_request(
                        async |_: PromptRequest, responder, cx| {
                            // `content` must be a block: a bare string is
                            // invalid, and serde's error quotes it.
                            let malformed = agent_client_protocol::UntypedMessage::new(
                                "session/update",
                                serde_json::json!({
                                    "sessionId": "s-1",
                                    "update": {
                                        "sessionUpdate": "agent_message_chunk",
                                        "content": QUESTION,
                                    },
                                }),
                            )?;
                            cx.send_notification(malformed)?;
                            responder.respond_with_error(Error::new(
                                -32603,
                                format!("could not answer « {QUESTION} »"),
                            ))
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .connect_to(channel)
                    .boxed()
            },
            async |session| {
                session
                    .prompt(
                        &prompt(QUESTION),
                        Arc::new(Seen::default()),
                        &CancelToken::new(),
                    )
                    .await
            },
        );
        assert!(
            matches!(ended, Err(ExternalError::Protocol(_))),
            "{ended:?}"
        );
    })
}

#[test]
fn the_protocol_crate_quotes_what_the_agent_sent_in_its_warnings() {
    // Measured, not assumed: without the cap, a `warn` line holds the string the
    // agent sent — here the user's question.
    let journal = journal_of_a_misbehaving_agent(false);
    let quoted = journal
        .lines()
        .any(|line| line.contains(" WARN ") && line.contains(QUESTION));
    assert!(
        quoted,
        "the crate no longer quotes it at warn: re-read before lifting the cap"
    );
}

#[test]
fn capped_at_error_the_journal_keeps_oxyns_own_warning_and_no_content() {
    let journal = journal_of_a_misbehaving_agent(true);
    assert!(!journal.contains(QUESTION), "{journal}");
    assert!(
        journal.contains("the agent answered a request with an error")
            && journal.contains("session/prompt")
            && journal.contains("-32603"),
        "the diagnosis survives, in Oxyn's words: {journal}"
    );
}

/// A bus that holds a call until it is cancelled, and says whether it was.
#[derive(Default)]
struct Patient(std::sync::atomic::AtomicBool);

#[async_trait::async_trait]
impl crate::runtime::CommandSink for Patient {
    async fn dispatch(
        &self,
        _actor: oxyn_core::Actor,
        _command: oxyn_core::Command,
        cancel: &CancelToken,
    ) -> crate::runtime::DispatchOutcome {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), cancel.cancelled()).await;
        self.0
            .store(cancel.is_cancelled(), std::sync::atomic::Ordering::SeqCst);
        crate::runtime::DispatchOutcome::Completed {
            summary: "0 rows, 0 batches".to_owned(),
        }
    }
}

#[tokio::test]
async fn an_agent_that_dies_mid_query_has_its_query_cancelled() {
    // The session raced its tool server against the protocol and the child:
    // the agent died, the server was dropped in the race, and the call at the
    // executor was aborted without being cancelled — a long `SELECT` went on
    // running on the database server.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let bus = Arc::new(Patient::default());
    let turns = crate::external::mcp::ToolTurns::new(8, Arc::new(|| false));
    let _question = turns.open(
        Arc::clone(&bus) as Arc<dyn crate::runtime::CommandSink>,
        Arc::new(()),
        CancelToken::new(),
    );
    let service = Arc::new(crate::external::mcp::ToolService::new(
        crate::tools::ToolRegistry::builtin(),
        vec![crate::tools::EXECUTE_QUERY.to_owned()],
        crate::tools::ToolScope::new(
            oxyn_core::ConnectionId::new(),
            oxyn_core::SessionId::new(),
            oxyn_core::QueryLanguage::SQL,
        ),
        crate::external::mcp::TierCell::holding(PrivacyTier::Metadata),
        oxyn_core::Actor::agent(oxyn_core::AgentId::new(), oxyn_core::AgentSessionId::new()),
    ));
    let (endpoint, serving) = crate::external::mcp::server::serve(service, turns)
        .await
        .expect("the loopback socket binds");
    let url = endpoint.url().to_owned();
    let token = endpoint.token().to_owned();

    // The agent's life: a protocol that goes on, and a child we kill.
    let (died, death) = oneshot::channel::<()>();
    let watch = death.map(|_| ExitReport::default()).boxed();
    let protocol = futures::future::pending::<()>().boxed();
    let exit = Arc::new(Mutex::new(None));
    let life = tokio::spawn(live(protocol, watch, Some((endpoint, serving)), exit));

    // A long query, from the agent, over HTTP.
    let address = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .expect("host and port")
        .to_owned();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute_query","arguments":{"statement":"SELECT pg_sleep(600)"}}}"#;
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let calling = tokio::spawn(async move {
        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connects");
        stream.write_all(request.as_bytes()).await.expect("sent");
        let mut answer = Vec::new();
        let _ = stream.read_to_end(&mut answer).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // The agent dies.
    died.send(()).expect("the session is watching");
    tokio::time::timeout(std::time::Duration::from_secs(4), life)
        .await
        .expect("the session ends")
        .expect("cleanly");

    assert!(
        bus.0.load(std::sync::atomic::Ordering::SeqCst),
        "the query must be cancelled, not merely abandoned"
    );
    calling.abort();
}

mod setting_changes {
    use agent_client_protocol::schema::v1::{
        SessionConfigKind, SessionConfigOption, SessionConfigSelect, SessionConfigSelectOption,
        SessionMode, SessionModeState, SetSessionConfigOptionRequest,
        SetSessionConfigOptionResponse, SetSessionModeRequest, SetSessionModeResponse,
    };

    use super::*;
    use crate::external::settings::{SettingChange, SettingRefused, SettingValue};

    /// What reached the agent, in order.
    type Sent = Arc<Mutex<Vec<String>>>;

    fn effort(current: &str) -> SessionConfigOption {
        SessionConfigOption::new(
            "effort".to_owned(),
            "Effort",
            SessionConfigKind::Select(SessionConfigSelect::new(
                current.to_owned(),
                vec![
                    SessionConfigSelectOption::new("low", "Low"),
                    SessionConfigSelectOption::new("high", "High"),
                ],
            )),
        )
    }

    fn low() -> SettingChange {
        SettingChange::Option {
            id: "effort".to_owned(),
            value: SettingValue::Choice("low".to_owned()),
        }
    }

    /// How the fake agent says an option changed: its answer carries `low`,
    /// and a notification about the same option carries `high`.
    #[derive(Clone, Copy)]
    enum Said {
        /// The answer only.
        Answer,
        /// The notification, then the answer.
        NotifiedThenAnswered,
        /// The answer, then the notification.
        AnsweredThenNotified,
    }

    /// An agent with two modes and one option. `refuse` answers every change
    /// with an error quoting a secret; `held` fires when a prompt arrives,
    /// which is then held until its receiver fires.
    fn agent(
        sent: Sent,
        refuse: bool,
        held: Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>,
    ) -> impl FnOnce(Channel) -> BoxFuture<'static, Result<(), Error>> {
        scripted(sent, refuse, held, Said::Answer)
    }

    fn scripted(
        sent: Sent,
        refuse: bool,
        held: Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>,
        said: Said,
    ) -> impl FnOnce(Channel) -> BoxFuture<'static, Result<(), Error>> {
        let held = Arc::new(Mutex::new(held));
        let modes = Arc::clone(&sent);
        move |channel| {
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
                            NewSessionResponse::new(session_id())
                                .modes(SessionModeState::new(
                                    "ask",
                                    vec![
                                        SessionMode::new("ask", "Ask"),
                                        SessionMode::new("plan", "Plan"),
                                    ],
                                ))
                                .config_options(vec![effort("high")]),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: SetSessionModeRequest, responder, _cx| {
                        modes
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(format!("mode:{}", request.mode_id));
                        if refuse {
                            responder.respond_with_error(Error::new(-32000, "no: SECRET-ROW"))
                        } else {
                            responder.respond(SetSessionModeResponse::new())
                        }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: SetSessionConfigOptionRequest, responder, cx| {
                        sent.lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(format!("option:{}", request.config_id));
                        let notify = || {
                            cx.send_notification(SessionNotification::new(
                                session_id(),
                                SessionUpdate::ConfigOptionUpdate(
                                    agent_client_protocol::schema::v1::ConfigOptionUpdate::new(
                                        vec![effort("high")],
                                    ),
                                ),
                            ))
                        };
                        let answer = SetSessionConfigOptionResponse::new(vec![effort("low")]);
                        match said {
                            Said::Answer => responder.respond(answer),
                            Said::NotifiedThenAnswered => {
                                notify()?;
                                responder.respond(answer)
                            }
                            Said::AnsweredThenNotified => {
                                responder.respond(answer)?;
                                notify()
                            }
                        }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: PromptRequest, responder, cx| {
                        let pair = held.lock().unwrap_or_else(PoisonError::into_inner).take();
                        cx.spawn(async move {
                            if let Some((started, release)) = pair {
                                let _ = started.send(());
                                let _ = release.await;
                            }
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        }
    }

    fn sent(sent: &Sent) -> Vec<String> {
        sent.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    #[test]
    fn a_change_the_agent_did_not_declare_is_never_sent() {
        let reached: Sent = Arc::default();
        let refused = run(agent(Arc::clone(&reached), false, None), async |session| {
            let before_any_session = session.change_setting(low()).await;
            let _ = session.start().await;
            let unknown_mode = session
                .change_setting(SettingChange::Mode {
                    id: "yolo".to_owned(),
                })
                .await;
            let unknown_level = session
                .change_setting(SettingChange::Option {
                    id: "effort".to_owned(),
                    value: SettingValue::Choice("max".to_owned()),
                })
                .await;
            (before_any_session, unknown_mode, unknown_level)
        });
        assert_eq!(
            refused,
            (
                Err(ExternalError::Setting(SettingRefused::UnknownOption)),
                Err(ExternalError::Setting(SettingRefused::UnknownMode)),
                Err(ExternalError::Setting(SettingRefused::UnknownValue)),
            )
        );
        assert!(sent(&reached).is_empty(), "{:?}", sent(&reached));
    }

    /// The settings a new question starts from, after `scenario`.
    fn in_force_after(
        agent: impl FnOnce(Channel) -> BoxFuture<'static, Result<(), Error>>,
        scenario: impl AsyncFnOnce(&ExternalSession) + 'static,
    ) -> Vec<String> {
        let seen = Arc::new(Seen::default());
        let observer: Arc<dyn AgentObserver> = seen.clone();
        run(agent, async move |session| {
            let _ = session.start().await;
            scenario(&session).await;
            let _ = session
                .prompt(&prompt("next"), observer, &CancelToken::new())
                .await;
        });
        seen.lines()
    }

    fn mode_and_effort(settings: &AgentSettings) -> String {
        let effort = settings
            .options
            .iter()
            .find_map(|option| match &option.value {
                crate::external::settings::OptionValue::Select { current, .. } => {
                    Some(current.clone())
                }
                crate::external::settings::OptionValue::Boolean(_) => None,
            });
        format!(
            "{}/{}",
            settings.current_mode.as_deref().unwrap_or("-"),
            effort.unwrap_or_default()
        )
    }

    #[test]
    fn a_declared_change_takes_what_the_agent_answers_not_what_was_asked() {
        let reached: Sent = Arc::default();
        let answers: Arc<Mutex<Vec<String>>> = Arc::default();
        let kept = Arc::clone(&answers);
        let in_force = in_force_after(
            agent(Arc::clone(&reached), false, None),
            async move |session| {
                for change in [
                    SettingChange::Mode {
                        id: "plan".to_owned(),
                    },
                    // Asked `low`; the fake agent answers `low` too, through the
                    // same conversion as a notification.
                    low(),
                ] {
                    let answer = session
                        .change_setting(change)
                        .await
                        .map(|settings| mode_and_effort(&settings));
                    kept.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(format!("{answer:?}"));
                }
            },
        );
        assert_eq!(sent(&reached), vec!["mode:plan", "option:effort"]);
        assert_eq!(
            *answers.lock().unwrap_or_else(PoisonError::into_inner),
            vec![r#"Ok("plan/high")"#, r#"Ok("plan/low")"#]
        );
        assert_eq!(
            in_force,
            vec!["settings:mode=plan:options=effort=low".to_owned()]
        );
    }

    #[test]
    fn a_notification_before_the_answer_is_overtaken_by_it() {
        // Received in this order: the notification (`high`), then the answer
        // (`low`). The last word received wins.
        let in_force = in_force_after(
            scripted(Arc::default(), false, None, Said::NotifiedThenAnswered),
            async |session| {
                let _ = session.change_setting(low()).await;
            },
        );
        assert_eq!(
            in_force,
            vec!["settings:mode=ask:options=effort=low".to_owned()]
        );
    }

    #[test]
    fn a_notification_after_the_answer_overtakes_it() {
        // The other order: the answer (`low`), then the notification (`high`).
        // Applied after `block_task` instead of in the ordered callback, the
        // answer could land last and win.
        let in_force = in_force_after(
            scripted(Arc::default(), false, None, Said::AnsweredThenNotified),
            async |session| {
                let _ = session.change_setting(low()).await;
            },
        );
        assert_eq!(
            in_force,
            vec!["settings:mode=ask:options=effort=high".to_owned()]
        );
    }

    #[test]
    fn a_change_during_a_question_is_refused_without_sending() {
        let reached: Sent = Arc::default();
        let (started, on_start) = oneshot::channel();
        let (release, on_release) = oneshot::channel();
        let refused = run(
            agent(Arc::clone(&reached), false, Some((started, on_release))),
            async move |session| {
                let _ = session.start().await;
                let question = prompt("long");
                let cancel = CancelToken::new();
                let asking = session.prompt(&question, Arc::new(()), &cancel);
                let changing = async {
                    let _ = on_start.await;
                    // Polled once: a refusal is immediate, and a change that
                    // was queued instead would wait behind the question
                    // forever — a failure, not a hang.
                    let refused = session.change_setting(low()).now_or_never();
                    let _ = release.send(());
                    refused
                };
                let (_, refused) = join(asking, changing).await;
                refused
            },
        );
        assert_eq!(
            refused,
            Some(Err(ExternalError::Setting(
                SettingRefused::QuestionInProgress
            )))
        );
        assert!(sent(&reached).is_empty(), "{:?}", sent(&reached));
    }

    #[test]
    fn a_refused_mode_leaves_the_settings_as_they_were() {
        let in_force = in_force_after(agent(Arc::default(), true, None), async |session| {
            let _ = session
                .change_setting(SettingChange::Mode {
                    id: "plan".to_owned(),
                })
                .await;
        });
        assert_eq!(
            in_force,
            vec!["settings:mode=ask:options=effort=high".to_owned()]
        );
    }

    #[test]
    fn an_agents_refusal_carries_its_code_and_none_of_its_words() {
        let refused = run(agent(Arc::default(), true, None), async |session| {
            let _ = session.start().await;
            session
                .change_setting(SettingChange::Mode {
                    id: "plan".to_owned(),
                })
                .await
        });
        assert_eq!(
            refused,
            Err(ExternalError::Setting(SettingRefused::ByAgent {
                code: -32000
            }))
        );
        let Err(error) = refused else {
            panic!("refused");
        };
        let shown = error.to_string();
        assert!(!shown.contains("SECRET-ROW"), "{shown}");
    }
}

mod confined {
    use agent_client_protocol::schema::v1::{
        CurrentModeUpdate, SessionMode, SessionModeState, SetSessionModeResponse,
    };

    use super::*;
    use crate::external::confine::Confinement;

    type Log = Arc<Mutex<Vec<String>>>;

    fn log(log: &Log, line: String) {
        log.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
    }

    /// An agent that starts in `free`, offers `locked`, and goes back to
    /// `free` on its own while answering the first question.
    fn wandering(said: Log) -> impl FnOnce(Channel) -> BoxFuture<'static, Result<(), Error>> {
        let (opened, moved, asked) = (Arc::clone(&said), Arc::clone(&said), said);
        move |channel| {
            Agent
                .builder()
                .on_receive_request(
                    async |request: InitializeRequest, responder, _cx| {
                        responder.respond(InitializeResponse::new(request.protocol_version))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: NewSessionRequest, responder, _cx| {
                        let meta = request.meta.map(serde_json::Value::Object);
                        log(&opened, format!("meta:{}", meta.unwrap_or_default()));
                        responder.respond(NewSessionResponse::new(session_id()).modes(
                            SessionModeState::new(
                                "free",
                                vec![
                                    SessionMode::new("locked", "Locked"),
                                    SessionMode::new("free", "Free"),
                                ],
                            ),
                        ))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: SetSessionModeRequest, responder, _cx| {
                        log(&moved, format!("mode:{}", request.mode_id));
                        responder.respond(SetSessionModeResponse::new())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: PromptRequest, responder, cx| {
                        log(&asked, "prompt".to_owned());
                        cx.send_notification(SessionNotification::new(
                            session_id(),
                            SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("free")),
                        ))?;
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        }
    }

    #[test]
    fn a_confined_agent_is_put_in_its_mode_first_and_cut_off_as_soon_as_it_leaves() {
        let said: Log = Arc::default();
        let seen = Arc::new(Seen::default());
        let observer: Arc<dyn AgentObserver> = seen.clone();
        let mut meta = serde_json::Map::new();
        meta.insert("confined".to_owned(), serde_json::Value::Bool(true));
        let confinement = Confinement {
            env: Vec::new(),
            session_meta: Some(meta),
            mode: "locked",
            tools_in_env: false,
        };

        let (ours, theirs) = Channel::duplex();
        let (session, driver) =
            ExternalSession::over_with(ours, private(), None, Some(confinement));
        let background = join(driver, wandering(Arc::clone(&said))(theirs));
        let ends = block_on(async move {
            let scenario = pin!(async {
                let first = session
                    .prompt(&prompt("one"), Arc::clone(&observer), &CancelToken::new())
                    .await;
                let second = session
                    .prompt(&prompt("two"), observer, &CancelToken::new())
                    .await;
                (first, second)
            });
            match select(scenario, pin!(background)).await {
                Either::Left((ends, _)) => ends,
                Either::Right(_) => panic!("the agent ended before the scenario"),
            }
        });

        // The answer during which it left is cut and said as such, not taken
        // as a full answer; the next question is refused before it is sent.
        assert_eq!(ends.0, Err(ExternalError::Unconfined));
        assert_eq!(ends.1, Err(ExternalError::Unconfined));
        assert_eq!(
            said.lock().unwrap_or_else(PoisonError::into_inner).clone(),
            vec![
                r#"meta:{"confined":true}"#.to_owned(),
                "mode:locked".to_owned(),
                // One question only: the second never reached an agent that
                // left its mode.
                "prompt".to_owned(),
            ]
        );
        // No mode is offered to the user of a confined agent.
        assert!(
            seen.lines()
                .iter()
                .filter(|line| line.starts_with("settings:"))
                .all(|line| line.starts_with("settings:mode=-")),
            "{:?}",
            seen.lines()
        );
    }
}
