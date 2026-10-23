//! A session classifies the requests of a language that is not SQL
//! (ADR-0064).
//!
//! `oxyn-query` reads SQL only: any other language comes out `Unknown`, which
//! counts as mutating. Without the session's answer, a Redis `GET` on
//! production would ask a human for a named confirmation, be refused to an
//! agent, and no preview could ever run. These tests prove the answer is used
//! where it must be — and only there: a SQL request never asks the session,
//! and a session that cannot prove a read leaves the guard exactly as it was.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use async_trait::async_trait;
use oxyn_catalog::CatalogProvider;
use oxyn_core::{
    AgentId, AgentSessionId, Capabilities, DefaultPolicy, DriverId, ExecLimits, PreviewShape,
    QueryLanguage, SqlDialect, StatementHandle, StatementIntent,
};
use oxyn_driver::Session;

/// What the fake session was asked, and what reached it.
struct Fake {
    /// What `classify_intent` answers.
    answer: Option<StatementIntent>,
    /// How many times `classify_intent` was called.
    consulted: AtomicUsize,
    /// The language of the request `preview_request` composes.
    preview_language: QueryLanguage,
    executed: Mutex<Vec<ExecRequest>>,
    catalog: catalog_tests::Probe,
}

impl Fake {
    fn answering(answer: Option<StatementIntent>) -> Self {
        Self {
            answer,
            consulted: AtomicUsize::new(0),
            preview_language: QueryLanguage::RedisCommand,
            executed: Mutex::new(Vec::new()),
            catalog: catalog_tests::Probe::default(),
        }
    }
}

struct FakeSession(Arc<Fake>);

#[async_trait]
impl Session for FakeSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::REDIS_COMMAND | Capabilities::SQL
    }
    async fn execute(&self, request: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.0.executed.lock().push(request);
        // What reached the driver is all these tests look at: failing here
        // spares a cursor.
        Err(OxynError::Query("recorded".into()))
    }
    async fn preview_request(
        &self,
        _: &oxyn_catalog::CatalogPath,
        _: u32,
        _: &PreviewShape,
        _: &CancelToken,
    ) -> Result<ExecRequest> {
        let text = match self.0.preview_language {
            QueryLanguage::Sql(_) => "SELECT * FROM \"keys\" LIMIT 2",
            _ => "SCAN 0 COUNT 2",
        };
        Ok(ExecRequest::new(self.0.preview_language, text))
    }
    fn classify_intent(&self, _: &ExecRequest) -> Option<StatementIntent> {
        self.0.consulted.fetch_add(1, Ordering::SeqCst);
        self.0.answer
    }
    async fn cancel(&self, _: StatementHandle) -> Result<()> {
        Ok(())
    }
    fn catalog(&self) -> &dyn CatalogProvider {
        &self.0.catalog
    }
    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::ZERO)
    }
    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

struct Bench {
    executor: Executor,
    connection: ConnectionId,
    session: SessionId,
    fake: Arc<Fake>,
}

fn redis() -> DriverId {
    DriverId::new("redis").expect("`redis` is a well-formed driver identifier")
}

/// A production connection with one open session on it.
fn bench(fake: Fake) -> Bench {
    let connection =
        ConnectionConfig::new("cache", redis()).with_environment(Environment::Production);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("session classification")
        .expect("workspace")
        .id;
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(&connection);
    let fake = Arc::new(fake);
    let slot = executor.sessions.insert(SessionSlot::new(
        connection.id,
        Box::new(FakeSession(Arc::clone(&fake))),
    ));
    Bench {
        executor,
        connection: connection.id,
        session: slot.id(),
        fake,
    }
}

fn actors() -> [Actor; 2] {
    [
        Actor::Human,
        Actor::agent(AgentId::new(), AgentSessionId::new()),
    ]
}

/// What a console sends: the bound lifted, the intent the caller's own.
fn execution(bench: &Bench, language: QueryLanguage, text: &str) -> Command {
    Command::Execute {
        connection: bench.connection,
        session: bench.session,
        request: Box::new(
            ExecRequest::new(language, text)
                .with_intent(StatementIntent::Read)
                .with_limits(ExecLimits::default().writable()),
        ),
    }
}

fn preview(bench: &Bench) -> Command {
    Command::PreviewRelation {
        connection: bench.connection,
        session: bench.session,
        catalog: None,
        namespace: Some("db0".into()),
        relation: "keys".into(),
        limit: 2,
        shape: PreviewShape::unordered(),
    }
}

/// The intent the journal recorded for the last decision.
fn journaled_intent(bench: &Bench) -> StatementIntent {
    bench
        .executor
        .store
        .journal()
        .recent(1)
        .expect("reading the journal")
        .pop()
        .expect("one entry")
        .record
        .intent
}

#[tokio::test]
async fn a_proven_read_passes_production_for_both_actors_bounded_read_only() {
    for actor in actors() {
        let bench = bench(Fake::answering(Some(StatementIntent::Read)));
        let outcome = bench
            .executor
            .dispatch(
                actor,
                execution(&bench, QueryLanguage::RedisCommand, "GET user:1"),
                &CancelToken::new(),
            )
            .await;

        assert!(
            !matches!(
                outcome,
                Ok(Outcome::NeedsApproval { .. } | Outcome::Denied { .. })
            ),
            "a proven read needs no confirmation: {outcome:?}"
        );
        let executed = bench.fake.executed.lock();
        let request = executed.first().expect("the read reached the session");
        assert_eq!(request.intent, StatementIntent::Read);
        assert!(
            request.limits.read_only,
            "a production read leaves bounded to read-only"
        );
        assert_eq!(bench.fake.consulted.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn a_proven_read_preview_runs_for_both_actors() {
    for actor in actors() {
        let bench = bench(Fake::answering(Some(StatementIntent::Read)));
        let outcome = bench
            .executor
            .dispatch(actor, preview(&bench), &CancelToken::new())
            .await;

        assert!(
            !matches!(outcome, Err(OxynError::PolicyDenied { .. })),
            "a proven read preview is not refused: {outcome:?}"
        );
        let executed = bench.fake.executed.lock();
        let request = executed.first().expect("the preview reached the session");
        assert_eq!(request.intent, StatementIntent::Read);
        assert!(request.limits.read_only);
        assert_eq!(request.limits.max_rows, Some(3));
    }
}

#[tokio::test]
async fn an_unproven_request_stays_unknown_and_keeps_the_production_guard() {
    let bench = bench(Fake::answering(None));
    let human = bench
        .executor
        .dispatch(
            Actor::Human,
            execution(&bench, QueryLanguage::RedisCommand, "GET user:1"),
            &CancelToken::new(),
        )
        .await
        .expect("an approval request is not a failure");
    let Outcome::NeedsApproval { reason, .. } = human else {
        panic!("a human confirms an unproven request on production: {human:?}");
    };
    assert!(
        reason.contains("cache"),
        "the confirmation names the connection: {reason}"
    );
    assert_eq!(journaled_intent(&bench), StatementIntent::Unknown);

    let agent = bench
        .executor
        .dispatch(
            actors()[1].clone(),
            execution(&bench, QueryLanguage::RedisCommand, "GET user:1"),
            &CancelToken::new(),
        )
        .await
        .expect("a denial is not a failure");
    assert!(
        agent.is_denied(),
        "an agent is refused an unproven request: {agent:?}"
    );
    assert_eq!(journaled_intent(&bench), StatementIntent::Unknown);

    for actor in actors() {
        assert!(matches!(
            bench
                .executor
                .dispatch(actor, preview(&bench), &CancelToken::new())
                .await,
            Err(OxynError::PolicyDenied { .. })
        ));
    }
    assert!(
        bench.fake.executed.lock().is_empty(),
        "nothing unproven reached the session"
    );
}

#[tokio::test]
async fn a_proven_write_stays_a_write_for_the_guard() {
    let bench = bench(Fake::answering(Some(StatementIntent::Write)));
    let human = bench
        .executor
        .dispatch(
            Actor::Human,
            execution(&bench, QueryLanguage::RedisCommand, "SET user:1 x"),
            &CancelToken::new(),
        )
        .await
        .expect("an approval request is not a failure");
    assert!(human.needs_approval(), "{human:?}");
    assert_eq!(journaled_intent(&bench), StatementIntent::Write);

    let agent = bench
        .executor
        .dispatch(
            actors()[1].clone(),
            execution(&bench, QueryLanguage::RedisCommand, "SET user:1 x"),
            &CancelToken::new(),
        )
        .await
        .expect("a denial is not a failure");
    assert!(agent.is_denied(), "{agent:?}");
    assert_eq!(journaled_intent(&bench), StatementIntent::Write);
    assert!(bench.fake.executed.lock().is_empty());
}

#[tokio::test]
async fn a_sql_request_never_consults_the_session() {
    // The fake would call anything a read: only `oxyn-query` may say so of SQL.
    let mut fake = Fake::answering(Some(StatementIntent::Read));
    fake.preview_language = QueryLanguage::Sql(SqlDialect::Postgres);
    let bench = bench(fake);

    let outcome = bench
        .executor
        .dispatch(
            Actor::Human,
            execution(
                &bench,
                QueryLanguage::Sql(SqlDialect::Postgres),
                "DELETE FROM sessions",
            ),
            &CancelToken::new(),
        )
        .await
        .expect("an approval request is not a failure");
    assert!(outcome.needs_approval(), "{outcome:?}");
    assert_eq!(journaled_intent(&bench), StatementIntent::Write);

    let _ = bench
        .executor
        .dispatch(Actor::Human, preview(&bench), &CancelToken::new())
        .await;
    assert_eq!(bench.fake.executed.lock().len(), 1, "the SQL preview ran");
    assert_eq!(
        bench.fake.consulted.load(Ordering::SeqCst),
        0,
        "SQL is classified by oxyn-query alone"
    );
}

#[tokio::test]
async fn a_session_of_another_connection_has_no_say() {
    let bench = bench(Fake::answering(Some(StatementIntent::Read)));
    let other = ConnectionConfig::new("other", redis()).with_environment(Environment::Production);
    bench.executor.register_connection(&other);
    let command = Command::Execute {
        connection: bench.connection,
        session: bench
            .executor
            .sessions
            .insert(SessionSlot::new(
                other.id,
                Box::new(FakeSession(Arc::clone(&bench.fake))),
            ))
            .id(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::RedisCommand, "GET user:1")
                .with_limits(ExecLimits::default().writable()),
        ),
    };

    let outcome = bench
        .executor
        .dispatch(Actor::Human, command, &CancelToken::new())
        .await
        .expect("an approval request is not a failure");
    assert!(outcome.needs_approval(), "{outcome:?}");
    assert_eq!(bench.fake.consulted.load(Ordering::SeqCst), 0);
}
