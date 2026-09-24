//! A write an agent proposes waits for the user's decision, inside its call.
//!
//! The regression these tests hold: the call used to come back at once with
//! « awaiting approval ». The model went on writing its answer, the answer
//! ended, and whatever withdrew the request afterwards — the agent released
//! after an exchange that carried a sample (ADR-0034 §5), a new agent launched,
//! the connection edited — withdrew it silently. The card still offered
//! « Review… », and approving said « no command is awaiting approval under
//! this identifier ».

use std::time::Duration;

use super::*;

/// The request the executor holds for `actor`, once it appears.
fn pending_of(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    actor: Actor,
) -> oxyn_exec::PendingCommand {
    for _ in 0..500 {
        if let Some(found) = backend
            .inner
            .executor
            .approvals()
            .pending()
            .into_iter()
            .find(|pending| pending.actor == actor)
        {
            return found;
        }
        runtime.block_on(tokio::time::sleep(Duration::from_millis(10)));
    }
    panic!("no request for approval appeared");
}

/// Waits for a call that must end, without hanging the suite if it does not.
fn ended(
    runtime: &tokio::runtime::Runtime,
    call: tokio::task::JoinHandle<DispatchOutcome>,
) -> DispatchOutcome {
    runtime
        .block_on(tokio::time::timeout(Duration::from_secs(10), call))
        .expect("the call ended")
        .expect("the call did not panic")
}

/// A write as `execute_query` translates it: its limits lifted, since the text
/// writes — the approval is what stands in its way.
fn write(open: &OpenConnection, sql: &str) -> Command {
    Command::Execute {
        connection: open.connection.parse().expect("connection id"),
        session: open.session.parse().expect("session id"),
        request: Box::new(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), sql)
                .with_limits(oxyn_core::ExecLimits::default().writable()),
        ),
    }
}

fn reported(received: &Mutex<Vec<String>>) -> Vec<serde_json::Value> {
    received
        .lock()
        .iter()
        .filter_map(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .filter_map(|update| update.get("event").cloned())
        .filter(|event| {
            event.get("kind").and_then(serde_json::Value::as_str) == Some("toolReported")
        })
        .collect()
}

#[test]
fn an_agent_write_waits_for_the_users_decision_and_the_model_learns_its_outcome() {
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
    let sink = Arc::new(sink);
    let call = runtime.spawn({
        let sink = Arc::clone(&sink);
        let command = write(&staging, "CREATE TABLE t (a INTEGER)");
        async move { sink.dispatch(actor, command, &CancelToken::new()).await }
    });

    let request = pending_of(&runtime, &backend, actor);
    runtime.block_on(tokio::time::sleep(Duration::from_millis(200)));
    assert!(
        !call.is_finished(),
        "the call came back before the user answered: the model would go on writing"
    );

    let decided = runtime.block_on(backend.decide(request.id, true));
    assert!(
        matches!(decided, Ok(crate::ipc::CommandOutcome::Executed { .. })),
        "{decided:?}"
    );
    let outcome = ended(&runtime, call);
    assert!(
        matches!(outcome, DispatchOutcome::Completed { .. }),
        "the model must learn what the approved write did: {outcome:?}"
    );
    assert!(thread.wrote(), "an approved write may have taken effect");
}

#[test]
fn a_rejected_write_tells_the_model_nothing_ran() {
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
    let sink = Arc::new(sink);
    let call = runtime.spawn({
        let sink = Arc::clone(&sink);
        let command = write(&staging, "CREATE TABLE t (a INTEGER)");
        async move { sink.dispatch(actor, command, &CancelToken::new()).await }
    });

    let request = pending_of(&runtime, &backend, actor);
    let decided = runtime.block_on(backend.decide(request.id, false));
    assert!(decided.is_ok(), "{decided:?}");
    let outcome = ended(&runtime, call);
    assert!(
        matches!(outcome, DispatchOutcome::Denied { reason } if reason.contains("rejected")),
        "the model must learn that the user said no"
    );
}

/// A request held back for an agent, straight from the executor.
fn held(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    staging: &OpenConnection,
) -> CommandId {
    let id = CommandId::new();
    let actor = Actor::agent(AgentId::new(), AgentSessionId::new());
    let outcome = runtime.block_on(backend.inner.executor.dispatch_as(
        id,
        actor,
        execute(
            staging,
            "DELETE FROM app_settings WHERE \"key\" = 'oxyn_demo'",
        ),
        &CancelToken::new(),
    ));
    assert!(
        matches!(outcome, Ok(oxyn_exec::Outcome::NeedsApproval { .. })),
        "{outcome:?}"
    );
    id
}

#[test]
fn an_unanswered_request_expires_and_a_late_approval_says_expired() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let staging = open(&runtime, &backend, Environment::Staging);
    let request = held(&runtime, &backend, &staging);

    let awaited = backend.inner.ai.decisions.expect(request);
    let ending = runtime.block_on(decisions::wait(
        &backend.inner.executor,
        awaited,
        tokio::time::Instant::now() + Duration::from_millis(50),
        &CancelToken::new(),
    ));
    let RequestEnd::Withdrawn(withdrawn) = ending else {
        panic!("an unanswered request must end withdrawn");
    };
    assert!(
        withdrawn.detail.starts_with("Expired"),
        "{}",
        withdrawn.detail
    );
    assert!(withdrawn.reason.contains("expired"), "{}", withdrawn.reason);
    assert!(backend.inner.executor.approvals().pending().is_empty());

    // The capture's words were « no command is awaiting approval under this
    // identifier »: a request that expired says so.
    let late = runtime.block_on(backend.decide(request, true));
    let Err(error) = late else {
        panic!("an expired request must not run: {late:?}");
    };
    assert!(error.message.contains("expired"), "{}", error.message);
    assert!(!error.message.contains("no command is awaiting"));
}

#[test]
fn a_stopped_question_withdraws_its_request() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let staging = open(&runtime, &backend, Environment::Staging);
    let request = held(&runtime, &backend, &staging);

    let stop = CancelToken::new();
    stop.cancel();
    let awaited = backend.inner.ai.decisions.expect(request);
    let ending = runtime.block_on(decisions::wait(
        &backend.inner.executor,
        awaited,
        tokio::time::Instant::now() + Duration::from_secs(300),
        &stop,
    ));
    let RequestEnd::Withdrawn(withdrawn) = ending else {
        panic!("a stopped question's request must end withdrawn");
    };
    assert_eq!(withdrawn.status, ToolStatus::Cancelled);
    assert!(backend.inner.executor.approvals().pending().is_empty());
}

#[test]
fn a_decision_cut_off_is_ambiguous_never_nothing_ran() {
    // I-13: once `decide` began, the command may have run.
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let staging = open(&runtime, &backend, Environment::Staging);
    let request = held(&runtime, &backend, &staging);

    let awaited = backend.inner.ai.decisions.expect(request);
    drop(backend.inner.ai.decisions.answer(request));
    let ending = runtime.block_on(decisions::wait(
        &backend.inner.executor,
        awaited,
        tokio::time::Instant::now() + Duration::from_secs(300),
        &CancelToken::new(),
    ));
    assert!(
        matches!(
            ending,
            RequestEnd::Answered(DispatchReport::Failed {
                class: ErrorClass::Ambiguous,
                ..
            })
        ),
        "a decision that said nothing may have applied"
    );
}

/// The capture's scenario: the agent released while its request waited.
#[test]
fn releasing_an_agent_withdraws_its_waiting_request_and_says_so_on_its_card() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let staging = open(&runtime, &backend, Environment::Staging);
    let (sink, thread, actor, received) = sink_on(&backend, &staging);
    thread.open_call(
        "execute_query",
        "Execute",
        staging.connection.parse().ok(),
        true,
    );
    let sink = Arc::new(sink);
    let call = runtime.spawn({
        let sink = Arc::clone(&sink);
        let command = write(&staging, "CREATE TABLE t (a INTEGER)");
        async move { sink.dispatch(actor, command, &CancelToken::new()).await }
    });
    let request = pending_of(&runtime, &backend, actor);

    // An unrelated agent's request is left alone.
    let (other_sink, other_thread, other_actor, _other) = sink_on(&backend, &staging);
    other_thread.open_call(
        "execute_query",
        "Execute",
        staging.connection.parse().ok(),
        true,
    );
    let other_sink = Arc::new(other_sink);
    let _other_call = runtime.spawn({
        let command = write(&staging, "CREATE TABLE u (a INTEGER)");
        async move {
            other_sink
                .dispatch(other_actor, command, &CancelToken::new())
                .await
        }
    });
    let other = pending_of(&runtime, &backend, other_actor);

    let (ours, _theirs) = agent_client_protocol::Channel::duplex();
    let (session, _driver) = ExternalSession::over(ours, std::env::temp_dir().join("unused"));
    thread.link_agent(Some(AgentLink {
        agent: agent("unused"),
        tier: PrivacyTier::Metadata,
        leaf: None,
        session: Arc::new(session),
        tools: ToolTurns::new(8, Arc::new(|| false)),
        actor: (AgentId::new(), AgentSessionId::new()),
        _requests: WithdrawOnRelease::new(
            Arc::clone(&backend.inner.executor),
            Arc::clone(&backend.inner.ai.decisions),
            actor,
        ),
    }));
    thread.link_agent(None);

    let outcome = ended(&runtime, call);
    assert!(
        matches!(outcome, DispatchOutcome::Denied { .. }),
        "the model must learn that nothing ran: {outcome:?}"
    );
    let cards = reported(&received);
    assert!(
        cards.iter().any(|card| {
            card.get("status").and_then(serde_json::Value::as_str) == Some("cancelled")
                && card
                    .get("detail")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|detail| detail.contains("Nothing ran"))
        }),
        "the card must stop offering « Review… »: {cards:?}"
    );
    let left = backend.inner.executor.approvals().pending();
    assert_eq!(left.len(), 1, "only the other agent's request remains");
    assert_eq!(left[0].id, other.id);
    let approved = runtime.block_on(backend.inner.executor.approve(
        "human",
        request.id,
        &CancelToken::new(),
    ));
    assert!(approved.is_err(), "a withdrawn request is not approvable");
}
