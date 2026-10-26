use super::*;
use oxyn_core::{
    ConnectionConfig, DriverId, Environment, ExecRequest, Preview, QueryLanguage, ScalarValue,
    SessionId,
};

fn execution(connection: ConnectionId, text: &str, intent: StatementIntent) -> Command {
    Command::Execute {
        connection,
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::SQL, text)
                .with_intent(intent)
                .with_params(vec![ScalarValue::Text("hunter2".to_owned())]),
        ),
    }
}

#[test]
fn the_journal_refuses_updates() {
    let store = Store::open_in_memory().expect("open");
    let command = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
    let id = store
        .journal()
        .append(&JournalRecord::new(
            &Actor::Human,
            &command,
            &Decision::Allow,
        ))
        .expect("insert");

    let refusal = store.with_connection(|conn| {
        Ok(conn.execute(
            "UPDATE audit_journal SET statement = 'SELECT 2' WHERE id = ?1",
            params![id],
        )?)
    });
    let error = refusal.expect_err("an UPDATE must fail");
    assert!(
        error.to_string().contains("append-only"),
        "the trigger must be the cause: {error}"
    );

    let read_back = store.journal().recent(1).expect("read back");
    assert_eq!(
        read_back[0].record.statement.as_deref(),
        Some("SELECT 1"),
        "the row must not have moved"
    );
}

#[test]
fn the_journal_refuses_deletions() {
    let store = Store::open_in_memory().expect("open");
    let command = execution(
        ConnectionId::new(),
        "DROP TABLE customers",
        StatementIntent::Ddl,
    );
    store
        .journal()
        .append(&JournalRecord::new(
            &Actor::Human,
            &command,
            &Decision::deny("read-only connection"),
        ))
        .expect("insert");

    for sql in [
        "DELETE FROM audit_journal",
        "DELETE FROM audit_journal WHERE id = 1",
    ] {
        let refusal = store.with_connection(|conn| Ok(conn.execute(sql, [])?));
        let error = refusal.expect_err("a DELETE must fail");
        assert!(error.to_string().contains("append-only"), "{sql}: {error}");
    }

    assert_eq!(store.journal().count().expect("count"), 1);
}

#[test]
fn a_refused_command_is_logged_with_its_reason() {
    // A journal that only records what worked says nothing of what an
    // agent attempted.
    let store = Store::open_in_memory().expect("open");
    let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
    let command = execution(
        ConnectionId::new(),
        "GRANT ALL ON customers TO PUBLIC",
        StatementIntent::Grant,
    );

    store
        .journal()
        .append(&JournalRecord::new(
            &agent,
            &command,
            &Decision::deny("an agent does not change privileges"),
        ))
        .expect("insert");

    let entry = store.journal().recent(10).expect("read back").remove(0);
    assert_eq!(entry.record.decision, PolicyOutcome::Denied);
    assert!(entry.record.actor_kind.is_agent());
    assert_eq!(
        entry.record.decision_reason.as_deref(),
        Some("an agent does not change privileges")
    );
    assert_eq!(entry.record.intent, StatementIntent::Grant);
    assert!(entry.record.duration.is_none(), "it was not executed");
}

#[test]
fn bound_values_do_not_enter_the_journal() {
    // I-03: the query's text is audit material, its bound values are not.
    let store = Store::open_in_memory().expect("open");
    let command = execution(
        ConnectionId::new(),
        "SELECT * FROM accounts WHERE password = $1",
        StatementIntent::Read,
    );
    store
        .journal()
        .append(&JournalRecord::new(
            &Actor::Human,
            &command,
            &Decision::Allow,
        ))
        .expect("insert");

    let all: String = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT group_concat(COALESCE(statement, '') || COALESCE(error, '')) \
                     FROM audit_journal",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("raw read");
    assert!(!all.contains("hunter2"), "a bound value leaked: {all}");
}

#[test]
fn an_entry_round_trips_completely() {
    let store = Store::open_in_memory().expect("open");
    let connection = ConnectionId::new();
    let agent_id = AgentId::new();
    let session = AgentSessionId::new();
    let command_id = CommandId::new();
    let actor = Actor::agent(agent_id, session);
    let command = execution(connection, "DELETE FROM orders", StatementIntent::Write);

    let record = JournalRecord::new(
        &actor,
        &command,
        &Decision::approval(
            "write requested by an agent",
            Some(Preview::new("DELETE FROM orders", "customer db")),
        ),
    )
    .with_command_id(command_id)
    .approved_by("nicolas")
    .completed(Duration::from_millis(1_234), Some(42));

    let id = store.journal().append(&record).expect("insert");
    assert!(id > 0);

    let read_back = store
        .journal()
        .for_connection(connection, 10)
        .expect("read back")
        .remove(0);

    assert_eq!(read_back.id, id);
    assert_eq!(read_back.record.command_id, Some(command_id));
    assert_eq!(read_back.record.actor_id, Some(agent_id));
    assert_eq!(read_back.record.agent_session, Some(session));
    assert_eq!(read_back.record.connection, Some(connection));
    assert_eq!(read_back.record.command_kind, "Execute");
    assert_eq!(
        read_back.record.statement.as_deref(),
        Some("DELETE FROM orders")
    );
    assert_eq!(read_back.record.intent, StatementIntent::Write);
    assert_eq!(read_back.record.decision, PolicyOutcome::ApprovalRequired);
    assert_eq!(read_back.record.approved_by.as_deref(), Some("nicolas"));
    assert_eq!(
        read_back.record.duration,
        Some(Duration::from_millis(1_234))
    );
    assert_eq!(read_back.record.rows_affected, Some(42));
    assert!(read_back.record.error.is_none());
}

#[test]
fn the_declared_risk_is_preserved() {
    let store = Store::open_in_memory().expect("open");
    let command = Command::Execute {
        connection: ConnectionId::new(),
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::SQL, "TRUNCATE TABLE customers")
                .with_intent(StatementIntent::Ddl)
                .with_risk(MutationRisk::Truncate),
        ),
    };
    store
        .journal()
        .append(&JournalRecord::new(
            &Actor::Human,
            &command,
            &Decision::approval("TRUNCATE", None),
        ))
        .expect("insert");

    let read_back = store.journal().recent(1).expect("read back").remove(0);
    assert_eq!(read_back.record.risk, MutationRisk::Truncate);
}

#[test]
fn a_failure_is_logged_without_hiding_the_decision() {
    let store = Store::open_in_memory().expect("open");
    let command = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
    let record = JournalRecord::new(&Actor::Human, &command, &Decision::Allow)
        .completed(Duration::from_millis(5), None)
        .failed(&OxynError::Query("missing relation".into()));

    store.journal().append(&record).expect("insert");
    let read_back = store.journal().recent(1).expect("read back").remove(0);

    assert_eq!(read_back.record.decision, PolicyOutcome::Allowed);
    assert!(
        read_back
            .record
            .error
            .as_deref()
            .is_some_and(|e| e.contains("missing relation"))
    );
    assert_eq!(read_back.record.error_class, Some(ErrorClass::Permanent));
}

/// An agent write whose effect is unknown stays **expressible** in the
/// audit trail.
///
/// It is the question put to the journal after an incident: did this
/// agent modify the database? "Timed out after 30 s" does not answer;
/// `ambiguous` answers "we do not know", and it is the only honest
/// answer. The journal being append-only, a class not written at the
/// moment of the incident will never be added
/// ([I-13](../../../CLAUDE.md#i-13)).
#[test]
fn an_error_class_survives_in_the_audit_trail() {
    let store = Store::open_in_memory().expect("open");
    let command = execution(
        ConnectionId::new(),
        "INSERT INTO orders (customer) VALUES (1)",
        StatementIntent::Write,
    );
    let record = JournalRecord::new(
        &Actor::agent(AgentId::new(), AgentSessionId::new()),
        &command,
        &Decision::Allow,
    )
    .failed(&OxynError::Timeout {
        after: Duration::from_secs(30),
    });

    store.journal().append(&record).expect("insert");
    let read_back = store.journal().recent(1).expect("read back").remove(0);

    assert_eq!(read_back.record.error_class, Some(ErrorClass::Ambiguous));
    assert!(
        !read_back
            .record
            .error_class
            .expect("a family")
            .is_retryable(),
        "an expired `INSERT` is not replayed: the server may have applied it"
    );
    // The bound value did not follow here either (I-03).
    assert!(!format!("{:?}", read_back.record).contains("hunter2"));
}

/// A command that succeeds carries no class: there is no error to
/// classify, and `completed` erases what an earlier `failed` would have
/// set.
#[test]
fn a_successful_command_carries_no_class() {
    let store = Store::open_in_memory().expect("open");
    let command = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
    let record = JournalRecord::new(&Actor::Human, &command, &Decision::Allow)
        .failed(&OxynError::Query("rejected".into()))
        .completed(Duration::from_millis(3), Some(1));

    store.journal().append(&record).expect("insert");
    let read_back = store.journal().recent(1).expect("read back").remove(0);

    assert_eq!(read_back.record.error_class, None);
    assert_eq!(read_back.record.error, None);
}

#[test]
fn the_journal_survives_the_connection_deletion() {
    // That is the property that makes the audit trail useful: erasing the
    // connection does not erase what was done with it.
    let store = Store::open_in_memory().expect("open");
    let workspace = store.workspaces().create("workshop").expect("workspace");
    let config = ConnectionConfig::new("customer db", DriverId::postgres())
        .with_environment(Environment::Production);
    store
        .connections()
        .save(workspace.id, &config)
        .expect("write");

    let command = execution(config.id, "DELETE FROM customers", StatementIntent::Write);
    store
        .journal()
        .append(&JournalRecord::new(
            &Actor::Human,
            &command,
            &Decision::approval("production", None),
        ))
        .expect("insert");

    assert!(store.connections().delete(config.id).expect("deletion"));
    assert!(store.workspaces().delete(workspace.id).expect("deletion"));

    assert_eq!(store.journal().count().expect("count"), 1);
    let remaining = store
        .journal()
        .for_connection(config.id, 10)
        .expect("read back");
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].record.statement.as_deref(),
        Some("DELETE FROM customers")
    );
}

#[test]
fn an_unreadable_decision_does_not_count_as_allowed() {
    let store = Store::open_in_memory().expect("open");
    let command = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
    store
        .journal()
        .append(&JournalRecord::new(
            &Actor::Human,
            &command,
            &Decision::Allow,
        ))
        .expect("insert");

    // An UPDATE is impossible; a row written by a future version is
    // therefore simulated by directly inserting unknown tags.
    store
        .with_connection(|conn| {
            Ok(conn.execute(
                "INSERT INTO audit_journal
                         (ts, actor_kind, command_kind, intent, risk, policy_decision)
                     VALUES (?1, 'quantum', 'Execute', 'levitate', '\"none\"', 'maybe')",
                params![Utc::now()],
            )?)
        })
        .expect("insert");

    let read_back = store.journal().recent(1).expect("read back").remove(0);
    assert_eq!(read_back.record.decision, PolicyOutcome::Denied);
    assert!(read_back.record.actor_kind.is_agent());
    assert_eq!(read_back.record.intent, StatementIntent::Unknown);
    assert!(read_back.record.intent.is_mutating());
}

#[test]
fn entries_come_out_in_reverse_recording_order() {
    let store = Store::open_in_memory().expect("open");
    let connection = ConnectionId::new();
    for n in 0..5 {
        let command = execution(connection, &format!("SELECT {n}"), StatementIntent::Read);
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &command,
                &Decision::Allow,
            ))
            .expect("insert");
    }

    let recent = store.journal().recent(3).expect("read back");
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].record.statement.as_deref(), Some("SELECT 4"));
    assert_eq!(recent[2].record.statement.as_deref(), Some("SELECT 2"));
}
