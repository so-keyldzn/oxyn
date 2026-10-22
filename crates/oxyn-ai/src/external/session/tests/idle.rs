//! An agent that goes silent, or never confirms a stop, does not hold the
//! session: the bounds of the `idle` module, with milliseconds for minutes.

use std::time::Duration;

use super::*;
use crate::external::settings::SettingChange;

/// The bounds, short enough for a test and long enough for a loaded machine
/// to deliver a message within them.
const SHORT: Limits = Limits {
    idle: Duration::from_millis(200),
    cancel_grace: Duration::from_millis(150),
    user_request: Duration::from_millis(200),
};

/// Safety bound: past it, something still waits — the defect these tests
/// close.
const GUARD: Duration = Duration::from_secs(10);

/// Runs a scenario against an in-memory agent, the driver and the agent on
/// their own tasks: unlike [`run`], either may end first — a session torn down
/// on purpose ends its driver.
fn run_limited<T>(
    agent: impl FnOnce(Channel) -> BoxFuture<'static, Result<(), Error>>,
    scenario: impl AsyncFnOnce(ExternalSession) -> T,
) -> T {
    run_limited_with_environment(Environment::empty(), agent, scenario)
}

/// [`run_limited`] with an environment the session redacts from the agent's
/// stream, so a test can exercise its held-back final fragment.
fn run_limited_with_environment<T>(
    environment: Environment,
    agent: impl FnOnce(Channel) -> BoxFuture<'static, Result<(), Error>>,
    scenario: impl AsyncFnOnce(ExternalSession) -> T,
) -> T {
    let (ours, theirs) = Channel::duplex();
    let (session, driver) =
        ExternalSession::over_with(ours, private(), None, None, environment, SHORT);
    block_on(async move {
        tokio::spawn(driver);
        tokio::spawn(agent(theirs));
        tokio::time::timeout(GUARD, scenario(session))
            .await
            .expect("the scenario ends within its guard")
    })
}

#[test]
fn stopping_a_taken_prompt_flushes_its_held_back_stream_before_returning() {
    let cancel = CancelToken::new();
    let stop = cancel.clone();
    let seen = Arc::new(Seen::default());
    let observer: Arc<dyn AgentObserver> = seen.clone();
    let stopped = run_limited_with_environment(
        Environment::empty().with_declared([("AGENT_TOKEN", "synthetic-agent-token")]),
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
                        cx.send_notification(SessionNotification::new(
                            session_id(),
                            text("synthetic"),
                        ))?;
                        stop.cancel();
                        cx.spawn(async move {
                            futures::future::pending::<()>().await;
                            responder.respond(PromptResponse::new(StopReason::Cancelled))
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            session
                .prompt(
                    &prompt("stop after a held-back fragment"),
                    observer,
                    &cancel,
                )
                .await
        },
    );

    assert_eq!(stopped, Ok(TurnEnd::Cancelled));
    assert_eq!(seen.lines(), vec!["text:<AGENT_TOKEN redacted>".to_owned()]);
}

fn text(t: &str) -> SessionUpdate {
    SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(t))))
}

#[test]
fn a_stop_the_agent_never_confirms_frees_the_session_for_the_next_question() {
    let token = CancelToken::new();
    let stopper = token.clone();

    let (first, closed, next) = run_limited(
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
                        stopper.cancel();
                        // Never answers, `session/cancel` or not.
                        cx.spawn(async move {
                            futures::future::pending::<()>().await;
                            responder.respond(PromptResponse::new(StopReason::Cancelled))
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            let first = session
                .prompt(&prompt("a long one"), Arc::new(()), &token)
                .await;
            let closing = async {
                while session.is_open() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            };
            let closed = tokio::time::timeout(GUARD, closing).await.is_ok();
            let next = session
                .prompt(&prompt("the next one"), Arc::new(()), &CancelToken::new())
                .await;
            (first, closed, next)
        },
    );

    assert_eq!(first, Ok(TurnEnd::Cancelled), "the stop answers at once");
    assert!(
        closed,
        "past the grace, the agent is let go rather than waited for"
    );
    assert_eq!(
        next,
        Err(ExternalError::Exited),
        "the next question is not stuck behind the stopped one: it is told to \
         start a fresh agent"
    );
}

#[test]
fn an_agent_that_goes_silent_ends_the_question_with_a_clear_error() {
    let told = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&told);
    let waiting: Arc<Mutex<Option<oneshot::Sender<()>>>> = Arc::default();
    let armed = Arc::clone(&waiting);

    let (end, open) = run_limited(
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
                        // One word, then nothing: its model connection hung.
                        cx.send_notification(SessionNotification::new(
                            session_id(),
                            text("let me"),
                        ))?;
                        let (tx, rx) = oneshot::channel();
                        *armed.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
                        cx.spawn(async move {
                            let _ = rx.await;
                            responder.respond(PromptResponse::new(StopReason::Cancelled))
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_notification(
                    async move |_: CancelNotification, _cx| {
                        counted.fetch_add(1, Ordering::SeqCst);
                        if let Some(tx) = waiting
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .take()
                        {
                            let _ = tx.send(());
                        }
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            let end = session
                .prompt(&prompt("a question"), Arc::new(()), &CancelToken::new())
                .await;
            // Past the grace: a confirmed stop keeps the agent.
            tokio::time::sleep(SHORT.cancel_grace * 2).await;
            (end, session.is_open())
        },
    );

    assert_eq!(end, Err(ExternalError::Silent { after: SHORT.idle }));
    let message = ExternalError::Silent {
        after: Duration::from_secs(300),
    }
    .to_string();
    assert!(message.contains("300 seconds"), "{message}");
    assert_eq!(told.load(Ordering::SeqCst), 1, "the agent is told to stop");
    assert!(
        open,
        "an agent that confirms the stop is kept for the next question"
    );
}

#[test]
fn an_agent_that_keeps_talking_or_works_in_a_tool_is_not_cut() {
    let (first, second) = run_limited(
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
                        let talking = cx.clone();
                        cx.spawn(async move {
                            // Talks for twice the bound, never pausing as long.
                            for _ in 0..8 {
                                talking.send_notification(SessionNotification::new(
                                    session_id(),
                                    text("."),
                                ))?;
                                tokio::time::sleep(SHORT.idle / 4).await;
                            }
                            // Then waits three times the bound inside a tool —
                            // an approval being read, a long query.
                            talking.send_notification(SessionNotification::new(
                                session_id(),
                                SessionUpdate::ToolCall(
                                    ToolCall::new(ToolCallId::new("t1"), "describe")
                                        .status(ToolCallStatus::InProgress),
                                ),
                            ))?;
                            tokio::time::sleep(SHORT.idle * 3).await;
                            talking.send_notification(SessionNotification::new(
                                session_id(),
                                SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                                    ToolCallId::new("t1"),
                                    ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
                                )),
                            ))?;
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            let first = session
                .prompt(&prompt("one"), Arc::new(()), &CancelToken::new())
                .await;
            // A call left open by the first question does not hold the
            // second's watch: this one is answered the same way.
            let second = session
                .prompt(&prompt("two"), Arc::new(()), &CancelToken::new())
                .await;
            (first, second)
        },
    );

    assert_eq!(first, Ok(TurnEnd::Answered { truncated: false }));
    assert_eq!(second, Ok(TurnEnd::Answered { truncated: false }));
}

#[test]
fn an_unanswered_sign_in_does_not_block_the_question_queued_after_it() {
    let (sign_in, question) = run_limited(
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
                    async |_: AuthenticateRequest, responder, cx| {
                        cx.spawn(async move {
                            futures::future::pending::<()>().await;
                            responder.respond(AuthenticateResponse::new())
                        })
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
        async move |session| {
            let question = prompt("what changed?");
            let cancel = CancelToken::new();
            tokio::join!(
                session.authenticate("browser"),
                session.prompt(&question, Arc::new(()), &cancel),
            )
        },
    );

    assert_eq!(
        sign_in,
        Err(ExternalError::RequestTimedOut {
            request: "the sign-in request",
            after: SHORT.user_request,
        })
    );
    assert_eq!(question, Ok(TurnEnd::Answered { truncated: false }));
}

#[test]
fn an_unanswered_setting_ends_the_session_before_a_queued_question_runs() {
    use agent_client_protocol::schema::v1::{SessionMode, SessionModeState, SetSessionModeRequest};

    let (setting, question) = run_limited(
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
                        responder.respond(NewSessionResponse::new(session_id()).modes(
                            SessionModeState::new(
                                "ask",
                                vec![
                                    SessionMode::new("ask", "Ask"),
                                    SessionMode::new("plan", "Plan"),
                                ],
                            ),
                        ))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async |_: SetSessionModeRequest, responder, cx| {
                        cx.spawn(async move {
                            futures::future::pending::<()>().await;
                            responder.respond(
                                agent_client_protocol::schema::v1::SetSessionModeResponse::new(),
                            )
                        })
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
        async move |session| {
            let started = session.start().await;
            assert!(
                started.is_ok(),
                "the session starts before its setting changes"
            );
            let question = prompt("what changed?");
            let cancel = CancelToken::new();
            tokio::join!(
                session.change_setting(SettingChange::Mode {
                    id: "plan".to_owned(),
                }),
                session.prompt(&question, Arc::new(()), &cancel),
            )
        },
    );

    assert_eq!(
        setting,
        Err(ExternalError::RequestTimedOut {
            request: "the setting change",
            after: SHORT.user_request,
        })
    );
    assert_eq!(question, Err(ExternalError::Exited));
}

#[test]
fn stopping_a_question_queued_behind_sign_in_returns_without_waiting_for_it() {
    let (started, wait_for_start) = oneshot::channel();
    let started = Arc::new(Mutex::new(Some(started)));
    let told = Arc::clone(&started);
    let stopped = run_limited(
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
                    async move |_: AuthenticateRequest, responder, cx| {
                        if let Some(started) =
                            told.lock().unwrap_or_else(PoisonError::into_inner).take()
                        {
                            let _ = started.send(());
                        }
                        cx.spawn(async move {
                            futures::future::pending::<()>().await;
                            responder.respond(AuthenticateResponse::new())
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(channel)
                .boxed()
        },
        async move |session| {
            let session = Arc::new(session);
            let signing_in = tokio::spawn({
                let session = Arc::clone(&session);
                async move { session.authenticate("browser").await }
            });
            let _ = wait_for_start.await;
            let cancel = CancelToken::new();
            let question = tokio::spawn({
                let session = Arc::clone(&session);
                let cancel = cancel.clone();
                async move {
                    session
                        .prompt(&prompt("stop before this starts"), Arc::new(()), &cancel)
                        .await
                }
            });
            tokio::task::yield_now().await;
            cancel.cancel();
            let stopped = tokio::time::timeout(SHORT.user_request, question)
                .await
                .expect("Stop must not wait behind sign-in")
                .expect("the question task completes");
            signing_in.abort();
            stopped
        },
    );

    assert_eq!(stopped, Ok(TurnEnd::Cancelled));
}
