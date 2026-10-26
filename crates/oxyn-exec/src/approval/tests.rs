use super::*;
use oxyn_core::{
    AgentId, AgentSessionId, ConnectionId, ExecRequest, QueryLanguage, SessionId, StatementIntent,
};

fn sample_command() -> Command {
    Command::Execute {
        connection: ConnectionId::new(),
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::SQL, "DELETE FROM commandes")
                .with_intent(StatementIntent::Write),
        ),
    }
}

fn agent() -> Actor {
    Actor::agent(AgentId::new(), AgentSessionId::new())
}

#[test]
fn an_approval_is_used_only_once() {
    // A replayable approval is an approval an agent can replay.
    let registry = ApprovalRegistry::new();
    let id = CommandId::new();
    registry
        .submit(id, agent(), sample_command(), "write by an agent", None)
        .expect("the queue is empty");

    assert!(registry.take(id).is_ok());
    assert_eq!(registry.take(id), Err(ApprovalError::Unknown));
    assert!(registry.is_empty());
}

#[test]
fn a_stale_request_executes_nothing() {
    let registry = ApprovalRegistry::with_ttl(Duration::ZERO);
    let id = CommandId::new();
    registry
        .submit(id, agent(), sample_command(), "write by an agent", None)
        .expect("the queue is empty");

    let outcome = registry.take(id);
    assert!(
        matches!(outcome, Err(ApprovalError::Expired { .. })),
        "{outcome:?}"
    );
    // And it was removed: it cannot be "caught up".
    assert!(registry.is_empty());
}

#[test]
fn a_stale_approval_becomes_a_denial_not_a_failure() {
    let error: OxynError = ApprovalError::Expired {
        after: Duration::from_secs(300),
    }
    .into();
    assert!(matches!(error, OxynError::PolicyDenied { .. }));
    assert!(error.is_user_error(), "{error:?}");
    assert!(!error.is_retryable(), "a stale approval is not replayed");
}

#[test]
fn the_command_set_aside_is_the_one_that_will_run() {
    // It is stored as is: approving one text and executing another would
    // reopen the gap that reclassification closes.
    let registry = ApprovalRegistry::new();
    let id = CommandId::new();
    let cmd = sample_command();
    registry
        .submit(id, Actor::Human, cmd.clone(), "production", None)
        .expect("the queue is empty");

    let resumption = registry.take(id).expect("approval given");
    assert_eq!(resumption.command, cmd);
    assert_eq!(resumption.actor, Actor::Human);
}

#[test]
fn a_pending_request_is_not_replaced() {
    // Replaced while a dialog shows it, the approval given to what was read
    // would execute something else (ADR-0037).
    let registry = ApprovalRegistry::new();
    let id = CommandId::new();
    let read_back = sample_command();
    registry
        .submit(id, Actor::Human, read_back.clone(), "production", None)
        .expect("the queue is empty");
    let other = Command::Execute {
        connection: ConnectionId::new(),
        session: SessionId::new(),
        request: Box::new(ExecRequest::new(QueryLanguage::SQL, "DROP TABLE audit")),
    };
    assert_eq!(
        registry
            .submit(id, Actor::Human, other, "production", None)
            .err(),
        Some(ApprovalError::AlreadyPending)
    );
    assert_eq!(
        registry.take(id).expect("approval given").command,
        read_back
    );
}

#[test]
fn approving_does_not_change_the_actor() {
    // An approved agent command stays an agent command: the log must keep
    // saying who wrote it.
    let registry = ApprovalRegistry::new();
    let id = CommandId::new();
    let who = agent();
    registry
        .submit(id, who, sample_command(), "write by an agent", None)
        .expect("the queue is empty");

    let resumption = registry.take(id).expect("approval given");
    assert!(resumption.actor.is_agent());
    assert_eq!(resumption.actor, who);
}

#[test]
fn the_queue_is_bounded() {
    let registry = ApprovalRegistry::new().with_capacity(2);
    for _ in 0..2 {
        registry
            .submit(CommandId::new(), agent(), sample_command(), "motif", None)
            .expect("under the bound");
    }
    let outcome = registry.submit(CommandId::new(), agent(), sample_command(), "motif", None);
    assert_eq!(
        outcome.err(),
        Some(ApprovalError::QueueFull { limit: 2 }),
        "the bound must refuse the new request"
    );
    assert_eq!(registry.len(), 2, "no old one was evicted");
}

#[test]
fn stale_requests_do_not_block_the_queue() {
    let registry = ApprovalRegistry::with_ttl(Duration::ZERO).with_capacity(1);
    registry
        .submit(CommandId::new(), agent(), sample_command(), "motif", None)
        .expect("queue not empty");
    // The previous one is stale: it no longer counts in the bound.
    registry
        .submit(CommandId::new(), agent(), sample_command(), "motif", None)
        .expect("the stale one made room");
}

#[test]
fn the_sweep_removes_only_stale_ones() {
    let live = ApprovalRegistry::new();
    let id = CommandId::new();
    live.submit(id, Actor::Human, sample_command(), "motif", None)
        .expect("queue not empty");
    assert!(live.sweep().is_empty());
    assert_eq!(live.len(), 1);

    let dead = ApprovalRegistry::with_ttl(Duration::ZERO);
    dead.submit(
        CommandId::new(),
        Actor::Human,
        sample_command(),
        "motif",
        None,
    )
    .expect("queue not empty");
    assert_eq!(dead.sweep().len(), 1);
    assert!(dead.is_empty());
}

#[test]
fn a_request_purged_by_the_next_is_still_reported_stale() {
    // The regression: the next request purged the stale one, and the late
    // approval read "no command is awaiting approval" — a request that
    // would never have existed.
    let registry = ApprovalRegistry::with_ttl(Duration::ZERO);
    let stale = CommandId::new();
    registry
        .submit(stale, agent(), sample_command(), "motif", None)
        .expect("queue not empty");
    registry
        .submit(CommandId::new(), agent(), sample_command(), "motif", None)
        .expect("the stale one made room");

    let outcome = registry.take(stale);
    assert!(
        matches!(outcome, Err(ApprovalError::Expired { .. })),
        "{outcome:?}"
    );
    // Said once: afterwards, nothing waits under this identifier any more.
    assert_eq!(registry.take(stale), Err(ApprovalError::Unknown));
}

#[test]
fn expiring_removes_the_request_and_the_late_approval_says_so() {
    let registry = ApprovalRegistry::new();
    let id = CommandId::new();
    registry
        .submit(id, agent(), sample_command(), "motif", None)
        .expect("queue not empty");

    assert!(registry.expire(id).is_some());
    assert!(registry.is_empty());
    assert!(registry.expire(id).is_none(), "already removed");
    let outcome = registry.take(id);
    assert!(
        matches!(outcome, Err(ApprovalError::Expired { .. })),
        "{outcome:?}"
    );
}

#[test]
fn a_request_purged_as_stale_can_be_told_from_one_taken() {
    // A call waiting on a request purged by the next submission found it
    // gone, took it for approved, and waited for an outcome nobody owed.
    let registry = ApprovalRegistry::with_ttl(Duration::ZERO);
    let purged = CommandId::new();
    registry
        .submit(purged, agent(), sample_command(), "motif", None)
        .expect("queue not empty");
    registry
        .submit(CommandId::new(), agent(), sample_command(), "motif", None)
        .expect("the stale one made room");
    assert!(registry.was_expired(purged));
    assert!(registry.was_expired(purged), "read without forgetting");
    assert!(matches!(
        registry.take(purged),
        Err(ApprovalError::Expired { .. })
    ));

    let live = ApprovalRegistry::new();
    let taken = CommandId::new();
    live.submit(taken, agent(), sample_command(), "motif", None)
        .expect("queue not empty");
    assert!(live.take(taken).is_ok());
    assert!(!live.was_expired(taken));
}

#[test]
fn the_memory_of_stale_ones_is_bounded() {
    let registry = ApprovalRegistry::new().with_capacity(2);
    let ids: Vec<CommandId> = (0..3).map(|_| CommandId::new()).collect();
    for id in &ids {
        registry
            .submit(*id, agent(), sample_command(), "motif", None)
            .expect("under the bound");
        registry.expire(*id);
    }
    // The oldest is forgotten: the bound also applies to memory.
    assert_eq!(registry.take(ids[0]), Err(ApprovalError::Unknown));
    assert!(matches!(
        registry.take(ids[2]),
        Err(ApprovalError::Expired { .. })
    ));
}

#[test]
fn an_explicit_denial_removes_the_request() {
    let registry = ApprovalRegistry::new();
    let id = CommandId::new();
    registry
        .submit(id, agent(), sample_command(), "motif", None)
        .expect("queue not empty");
    assert!(registry.reject(id).is_some());
    assert_eq!(registry.take(id), Err(ApprovalError::Unknown));
}
