use super::*;
use oxyn_core::{
    AgentSessionId, ErrorClass, ExecRequest, ScalarValue, SessionId, SqlDialect, StatementHandle,
};

#[test]
fn direct_records_cannot_persist_password_literals() {
    let store = Store::open_in_memory().expect("in-memory store");
    let sql = "ALTER ROLE app PASSWORD 'witness-secret'";
    let command = Command::Execute {
        connection: ConnectionId::new(),
        session: SessionId::new(),
        request: Box::new(ExecRequest::new(QueryLanguage::SQL, sql)),
    };
    let history = HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, sql);
    let mut journal =
        crate::JournalRecord::new(&Actor::Human, &command, &oxyn_core::Decision::Allow);
    journal.statement = Some(sql.into());
    store.history().record(&history).expect("history insert");
    store.journal().append(&journal).expect("journal append");
    store
        .with_connection(|conn| {
            for table in ["query_history", "audit_journal"] {
                let statement: String =
                    conn.query_row(&format!("SELECT statement FROM {table}"), [], |row| {
                        row.get(0)
                    })?;
                assert!(!statement.contains("witness-secret"));
                assert!(statement.contains("<redacted>"));
            }
            Ok(())
        })
        .expect("raw SQL reads");
    assert_eq!(history.statement, sql);
    assert_eq!(journal.statement.as_deref(), Some(sql));
}

/// The same for a Redis credential command: the history knows its
/// language, the journal entry written back does not and applies every
/// grammar.
#[test]
fn direct_records_cannot_persist_redis_credentials() {
    let store = Store::open_in_memory().expect("in-memory store");
    for text in [
        "CONFIG SET requirepass witness-secret",
        "\"A\\x55TH\" app witness-secret",
        "ACL SETUSER app on \"\\x3ewitness-secret\"",
    ] {
        let command = Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(ExecRequest::new(QueryLanguage::RedisCommand, text)),
        };
        let history = HistoryRecord::new(&Actor::Human, QueryLanguage::RedisCommand, text);
        let built = crate::JournalRecord::new(&Actor::Human, &command, &oxyn_core::Decision::Allow);
        assert!(
            built
                .statement
                .as_deref()
                .is_some_and(|statement| !statement.contains("witness-secret")),
            "{text}"
        );
        let mut direct = built.clone();
        direct.statement = Some(text.into());
        store.history().record(&history).expect("history insert");
        store.journal().append(&built).expect("journal append");
        store.journal().append(&direct).expect("journal append");
    }
    store
        .with_connection(|conn| {
            for table in ["query_history", "audit_journal"] {
                let mut select = conn.prepare(&format!("SELECT statement FROM {table}"))?;
                let statements = select
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert!(!statements.is_empty(), "{table}");
                for statement in statements {
                    assert!(
                        !statement.contains("witness-secret"),
                        "{table}: {statement}"
                    );
                }
            }
            Ok(())
        })
        .expect("raw SQL reads");
}

fn read_record(text: &str) -> HistoryRecord {
    HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, text).with_intent(StatementIntent::Read)
}

/// An ambiguous error survives the round trip **as data** (I-13).
///
/// The message is not enough: "timed out after 30 s" does not say the
/// server may have applied the write. And parsing it would be exactly what
/// `rust.md` forbids — a message gets reworded, and a caller that read it
/// then breaks without anything failing. So this test locks the column,
/// not the text.
#[test]
fn an_error_class_survives_reading_back() {
    let store = Store::open_in_memory().expect("open");
    let expire =
        read_record("INSERT INTO orders (customer) VALUES (1)").failed(&OxynError::Timeout {
            after: Duration::from_secs(30),
        });
    let clear_cut = read_record("SELECT * FROM absente").failed(&OxynError::Query(
        "relation \"absente\" does not exist".into(),
    ));

    store.history().record(&clear_cut).expect("write");
    store.history().record(&expire).expect("write");
    let read_back = store.history().recent(10).expect("read back");

    let ambiguous = read_back
        .iter()
        .find(|entry| entry.record.statement.starts_with("INSERT"))
        .expect("the expired write");
    assert_eq!(ambiguous.record.error_class, Some(ErrorClass::Ambiguous));
    assert!(
        !ambiguous
            .record
            .error_class
            .expect("a family")
            .is_retryable(),
        "an expired `INSERT` is not replayed: the server may have applied it"
    );

    let permanent = read_back
        .iter()
        .find(|entry| entry.record.statement.starts_with("SELECT"))
        .expect("the rejected query");
    assert_eq!(permanent.record.error_class, Some(ErrorClass::Permanent));

    // The status does not tell them apart: the class carries the
    // information, and it alone.
    assert_eq!(ambiguous.record.status, HistoryStatus::Failed);
    assert_eq!(permanent.record.status, HistoryStatus::Failed);
}

/// Not knowing means not replaying — in both places where an error's
/// class can be unknown.
#[test]
fn an_unknown_class_forbids_retry() {
    // When reading back a value this binary does not know.
    assert_eq!(
        crate::encoding::error_class_from_text("vaporised"),
        ErrorClass::Ambiguous
    );

    // And on a row written before the column existed: its `None` does not
    // mean "no error", it means "unknown class". Replaying the expired
    // `INSERT` it may carry would create a duplicate.
    let mut inherited = read_record("INSERT INTO orders (customer) VALUES (1)");
    inherited.status = HistoryStatus::Failed;
    inherited.error = Some("timed out after 30s".to_owned());
    assert_eq!(inherited.error_class, None);
    assert!(!inherited.is_retryable());

    // Only the transient class opens a retry.
    let disconnection =
        read_record("SELECT 1").failed(&OxynError::Connection("disconnected".into()));
    assert!(disconnection.is_retryable());
    for forbidden in [
        read_record("x").failed(&OxynError::Timeout {
            after: Duration::from_secs(1),
        }),
        read_record("x").failed(&OxynError::Query("syntax".into())),
        read_record("x").denied("read-only"),
        read_record("x").succeeded(Duration::from_millis(1), Some(0)),
    ] {
        assert!(!forbidden.is_retryable(), "{:?}", forbidden.error_class);
    }
}

/// A refusal returned by the last barrier reads as a refusal.
///
/// That barrier returns an `Err`, where the `PolicyGate` returns a
/// decision. Without this classification, two refusals identical for the
/// user would show one as "refused", the other as "failed" — and the
/// second would send them looking for a server incident that did not
/// happen.
#[test]
fn a_policy_refusal_is_not_a_failure() {
    let refusal = read_record("DELETE FROM customers").failed(&OxynError::PolicyDenied {
        reason: "read-only".to_owned(),
    });
    assert_eq!(refusal.status, HistoryStatus::Denied);
    assert_eq!(refusal.error_class, Some(ErrorClass::Permanent));
}

#[test]
fn an_execution_round_trips() {
    let store = Store::open_in_memory().expect("open");
    let connection = ConnectionId::new();
    let record = HistoryRecord::new(
        &Actor::Human,
        QueryLanguage::Sql(SqlDialect::Postgres),
        "SELECT * FROM customers",
    )
    .on_connection(connection, "customer db")
    .with_intent(StatementIntent::Read)
    .succeeded(Duration::from_millis(87), Some(1_204));

    let id = store.history().record(&record).expect("write");
    let read_back = store.history().recent(10).expect("read back").remove(0);

    assert_eq!(read_back.id, id);
    assert_eq!(read_back.record.connection, Some(connection));
    assert_eq!(
        read_back.record.connection_name.as_deref(),
        Some("customer db")
    );
    assert_eq!(
        read_back.record.language,
        QueryLanguage::Sql(SqlDialect::Postgres),
        "the dialect must survive the round trip"
    );
    assert_eq!(read_back.record.status, HistoryStatus::Succeeded);
    assert_eq!(read_back.record.duration, Some(Duration::from_millis(87)));
    assert_eq!(read_back.record.rows, Some(1_204));
    assert_eq!(read_back.record.actor_kind, ActorKind::Human);
}

#[test]
fn an_execution_is_recorded_running_then_completed() {
    let store = Store::open_in_memory().expect("open");
    let in_progress = read_record("SELECT count(*) FROM sales");
    assert_eq!(in_progress.status, HistoryStatus::Running);

    let id = store.history().record(&in_progress).expect("write");
    assert_eq!(
        store.history().recent(1).expect("read back")[0]
            .record
            .status,
        HistoryStatus::Running
    );

    let finished = in_progress.succeeded(Duration::from_millis(410), Some(3));
    assert!(store.history().finish(id, &finished).expect("completion"));

    let read_back = store.history().recent(1).expect("read back").remove(0);
    assert_eq!(read_back.id, id);
    assert_eq!(read_back.record.status, HistoryStatus::Succeeded);
    assert_eq!(read_back.record.rows, Some(3));
    assert_eq!(read_back.record.statement, "SELECT count(*) FROM sales");

    // The same move on the audit journal is impossible: there is no
    // method, and the file itself would refuse it.
    assert!(
        !store
            .history()
            .finish(id + 1_000, &finished)
            .expect("no row"),
        "a missing row is not silently completed"
    );
}

#[test]
fn a_cancellation_is_not_a_failure() {
    let store = Store::open_in_memory().expect("open");
    store
        .history()
        .record(&read_record("SELECT pg_sleep(60)").failed(&OxynError::Cancelled))
        .expect("write");
    store
        .history()
        .record(&read_record("SELECT 1/0").failed(&OxynError::Query("division by zero".into())))
        .expect("write");

    let entries = store.history().recent(10).expect("read back");
    let statuses: Vec<HistoryStatus> = entries.iter().map(|e| e.record.status).collect();
    assert!(statuses.contains(&HistoryStatus::Cancelled));
    assert!(statuses.contains(&HistoryStatus::Failed));
}

#[test]
fn a_search_does_not_interpret_metacharacters() {
    let store = Store::open_in_memory().expect("open");
    for text in [
        "SELECT taux FROM remises WHERE taux = '100%'",
        "SELECT * FROM customers",
        "SELECT a_b FROM t",
    ] {
        store.history().record(&read_record(text)).expect("write");
    }

    let on_percent = store.history().search("100%", 50).expect("search");
    assert_eq!(on_percent.len(), 1, "`%` must be literal, not a wildcard");

    let on_underscore = store.history().search("a_b", 50).expect("search");
    assert_eq!(on_underscore.len(), 1, "`_` must be literal");

    let nothing = store.history().search("a%b", 50).expect("search");
    assert!(nothing.is_empty(), "`a%b` must find nothing literally");
}

#[test]
fn purging_the_history_does_not_touch_the_journal() {
    // That is the guarantee that makes it possible to offer "clear my
    // history" without opening a way to erase an agent's trace.
    use crate::journal::JournalRecord;
    use oxyn_core::Decision;

    let store = Store::open_in_memory().expect("open");
    let connection = ConnectionId::new();
    let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
    let command = Command::Execute {
        connection,
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::SQL, "DELETE FROM customers")
                .with_intent(StatementIntent::Write),
        ),
    };

    store
        .journal()
        .append(&JournalRecord::new(
            &agent,
            &command,
            &Decision::approval("write by an agent", None),
        ))
        .expect("journal");
    store
        .history()
        .record(&HistoryRecord::from_command(&agent, &command).expect("an Execute has a text"))
        .expect("history");

    assert_eq!(store.history().count().expect("count"), 1);
    assert_eq!(store.journal().count().expect("count"), 1);

    let erased = store.history().clear().expect("purge");
    assert_eq!(erased, 1);
    assert_eq!(store.history().count().expect("count"), 0);
    assert_eq!(
        store.journal().count().expect("count"),
        1,
        "the audit journal is not purged"
    );
}

#[test]
fn purging_by_date_only_takes_older_entries() {
    let store = Store::open_in_memory().expect("open");
    let mut former = read_record("SELECT 'vieux'");
    former.ts = Utc::now() - chrono::Duration::days(30);
    store.history().record(&former).expect("write");
    store
        .history()
        .record(&read_record("SELECT 'recent'"))
        .expect("write");

    let disconnection = Utc::now() - chrono::Duration::days(7);
    assert_eq!(
        store.history().purge_before(disconnection).expect("purge"),
        1
    );

    let remaining = store.history().recent(10).expect("read back");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].record.statement, "SELECT 'recent'");
}

#[test]
fn only_executions_become_history_entries() {
    let non_executions = [
        Command::Connect {
            connection: ConnectionId::new(),
        },
        Command::Cancel {
            connection: ConnectionId::new(),
            statement: StatementHandle::new(),
        },
        Command::RefreshCatalog {
            connection: ConnectionId::new(),
        },
    ];
    for command in &non_executions {
        assert!(
            HistoryRecord::from_command(&Actor::Human, command).is_none(),
            "`{}` is not a query",
            command.name()
        );
    }
}

#[test]
fn bound_values_do_not_enter_the_history() {
    let store = Store::open_in_memory().expect("open");
    let command = Command::Execute {
        connection: ConnectionId::new(),
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(
                QueryLanguage::SQL,
                "SELECT * FROM accounts WHERE token = $1",
            )
            .with_intent(StatementIntent::Read)
            .with_params(vec![ScalarValue::Text("hunter2".to_owned())]),
        ),
    };
    store
        .history()
        .record(&HistoryRecord::from_command(&Actor::Human, &command).expect("an Execute"))
        .expect("write");

    let all: String = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT group_concat(statement || COALESCE(error, '')) FROM query_history",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("raw read");
    assert!(!all.contains("hunter2"), "a bound value leaked: {all}");
}

#[test]
fn the_history_survives_the_connection_deletion() {
    let store = Store::open_in_memory().expect("open");
    let connection = ConnectionId::new();
    store
        .history()
        .record(&read_record("SELECT 1").on_connection(connection, "base disparue"))
        .expect("write");

    // No foreign key: the row remains and stays readable.
    let read_back = store
        .history()
        .for_connection(connection, 10)
        .expect("read back");
    assert_eq!(read_back.len(), 1);
    assert_eq!(
        read_back[0].record.connection_name.as_deref(),
        Some("base disparue")
    );
}

/// A MongoDB command's credentials are redacted on the decoded command
/// in every record: the history row and the journal entry built from the
/// command, which know its language, and a journal entry written back,
/// which does not. What the driver receives is left whole.
#[test]
fn mongo_records_cannot_persist_a_credential() {
    // The scheme apart, so that no line here reads as a connection string.
    let scheme = "mongodb";
    records_cannot_persist_a_credential(
        QueryLanguage::MongoQuery,
        &[
            r#"{"createUser": "app", "pwd": "witness-secret", "roles": []}"#.to_owned(),
            r#"{"updateUser": "app", "pwd": "witness-secret"}"#.to_owned(),
            r#"{"createUser":"app","pwd":"witness-secret"}"#.to_owned(),
            r#"{"saslStart": 1, "mechanism": "PLAIN", "payload": "witness-secret"}"#.to_owned(),
            format!(r#"{{"addShard": "{scheme}://admin:witness-secret@h1/"}}"#),
        ],
    );
}

/// The same for a search DSL request: the decoded body decides, and a
/// pasted shell line keeps no header nor user password.
#[test]
fn search_dsl_records_cannot_persist_a_credential() {
    let scheme = "https";
    records_cannot_persist_a_credential(
        QueryLanguage::SearchDsl,
        &[
            "PUT /_security/user/app\n{\"password\": \"witness-secret\"}".to_owned(),
            "POST /_reindex\n{\"source\": {\"remote\": {\"headers\": \
                 {\"Authorization\": \"ApiKey witness-secret\"}}}}"
                .to_owned(),
            "POST /_nodes/reload_secure_settings\n\
                 {\"secure_settings_password\": \"witness-secret\"}"
                .to_owned(),
            format!("curl -u elastic:witness-secret {scheme}://h:9200/_search"),
            format!("GET {scheme}://elastic:witness-secret@h:9200/_search"),
        ],
    );
}

/// Every record of `texts` in `language` — the history row and the
/// journal entry built from the command, which know its language, and a
/// journal entry written back, which does not — keeps no credential; what
/// the driver receives is left whole.
fn records_cannot_persist_a_credential(language: QueryLanguage, texts: &[String]) {
    let store = Store::open_in_memory().expect("in-memory store");
    for text in texts {
        let text = text.as_str();
        let request = ExecRequest::new(language, text);
        let command = Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(request),
        };
        let history = HistoryRecord::from_command(&Actor::Human, &command).expect("an execution");
        let journal =
            crate::JournalRecord::new(&Actor::Human, &command, &oxyn_core::Decision::Allow);
        store.history().record(&history).expect("history insert");
        store
            .history()
            .record(&HistoryRecord::new(&Actor::Human, language, text))
            .expect("history insert");
        let mut direct = journal.clone();
        direct.statement = Some(text.into());
        store.journal().append(&journal).expect("journal append");
        store.journal().append(&direct).expect("journal append");
        assert_eq!(command.statement_text(), Some(text));
    }
    store
        .with_connection(|conn| {
            for table in ["query_history", "audit_journal"] {
                let mut select = conn.prepare(&format!("SELECT statement FROM {table}"))?;
                let statements = select
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert!(!statements.is_empty(), "{table}");
                for statement in statements {
                    assert!(
                        !statement.contains("witness-secret"),
                        "{table}: {statement}"
                    );
                    assert!(statement.contains("<redacted>"), "{table}: {statement}");
                }
            }
            Ok(())
        })
        .expect("raw SQL reads");
}
