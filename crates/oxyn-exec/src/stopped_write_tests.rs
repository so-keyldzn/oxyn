//! A Stop on a write is announced with its driver's verdict, never as a
//! harmless cancellation decided before it (issue #139, I-13).

use super::*;
use arrow::array::Int64Array;
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{DefaultPolicy, DriverId, ExecLimits, QueryLanguage};
use oxyn_driver::Session;
use tokio::sync::Notify;

/// Bounds a test that would otherwise hang on a regression.
const GIVE_UP_AFTER: Duration = Duration::from_secs(30);

/// How the scripted driver ends once its token fires.
#[derive(Debug, Clone, Copy)]
enum Verdict {
    /// The server may have committed: what the PostgreSQL driver says of a
    /// write cut while its rows were being read.
    Unknown,
    /// The server confirmed nothing was applied.
    Cancelled,
    /// The statement ended before the stop reached it.
    Finished,
    /// The driver never answers.
    Silent,
}

struct Script {
    verdict: Verdict,
    /// Notified when the cursor waits for its token, after its first batch.
    waiting: Notify,
    catalog: catalog_tests::Probe,
}

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]))
}

struct ScriptedSession(Arc<Script>);

#[async_trait]
impl Session for ScriptedSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL
    }
    async fn execute(&self, _: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        Ok(Box::new(ScriptedCursor {
            script: Arc::clone(&self.0),
            cancel: cancel.clone(),
            handle: StatementHandle::new(),
            sent: false,
            ended: false,
        }))
    }
    async fn cancel(&self, _: StatementHandle) -> Result<()> {
        Ok(())
    }
    fn catalog(&self) -> &dyn oxyn_catalog::CatalogProvider {
        &self.0.catalog
    }
    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::ZERO)
    }
    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

/// One row, then the wait for the token, then the scripted verdict — the
/// shape of a `RETURNING` stopped while its rows arrive.
struct ScriptedCursor {
    script: Arc<Script>,
    cancel: CancelToken,
    handle: StatementHandle,
    sent: bool,
    ended: bool,
}

#[async_trait]
impl Cursor for ScriptedCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }
    fn schema(&self) -> SchemaRef {
        schema()
    }
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.ended {
            return Ok(None);
        }
        if !self.sent {
            self.sent = true;
            let batch = RecordBatch::try_new(schema(), vec![Arc::new(Int64Array::from(vec![1]))])
                .expect("the column matches the schema");
            return Ok(Some(batch));
        }
        self.script.waiting.notify_one();
        self.cancel.cancelled().await;
        self.ended = true;
        match self.script.verdict {
            Verdict::Unknown => Err(OxynError::OutcomeUnknown(
                "interrupted while its rows were being read".to_owned(),
            )),
            Verdict::Cancelled => Err(OxynError::Cancelled),
            Verdict::Finished => Ok(None),
            Verdict::Silent => std::future::pending().await,
        }
    }
    fn stats(&self) -> ExecStats {
        ExecStats::default()
    }
}

/// Runs `text` on a scripted session, pressing Stop once the first row is in.
async fn stopped(verdict: Verdict, text: &str, writable: bool) -> (Result<Outcome>, Vec<Event>) {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store.workspaces().create("stop").expect("workspace").id;
    let config =
        ConnectionConfig::new("stop", DriverId::postgres()).with_environment(Environment::Local);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&config);
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(&config);
    let script = Arc::new(Script {
        verdict,
        waiting: Notify::new(),
        catalog: catalog_tests::Probe::default(),
    });
    let slot = executor.sessions.insert(SessionSlot::new(
        config.id,
        Box::new(ScriptedSession(Arc::clone(&script))),
    ));
    let mut events = executor.subscribe();

    let limits = if writable {
        ExecLimits::default().writable()
    } else {
        ExecLimits::default()
    };
    let request = ExecRequest::new(QueryLanguage::SQL, text).with_limits(limits.with_timeout(None));
    let stop = CancelToken::new();
    let run = executor.dispatch(
        Actor::Human,
        Command::Execute {
            connection: config.id,
            session: slot.id(),
            request: Box::new(request),
        },
        &stop,
    );
    let outcome = tokio::time::timeout(GIVE_UP_AFTER, async {
        tokio::pin!(run);
        tokio::select! {
            outcome = &mut run => outcome,
            () = script.waiting.notified() => {
                stop.cancel();
                run.await
            }
        }
    })
    .await
    .expect("a stopped execution must end");

    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event.event);
    }
    (outcome, seen)
}

#[tokio::test]
async fn a_write_whose_outcome_is_unknown_is_not_announced_as_cancelled() {
    let (outcome, events) = stopped(
        Verdict::Unknown,
        "INSERT INTO t SELECT g FROM generate_series(1, 1000000) g RETURNING id",
        true,
    )
    .await;

    let Err(error) = outcome else {
        panic!("expected an ambiguous failure, got {outcome:?}");
    };
    assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
    assert_eq!(error.class(), oxyn_core::ErrorClass::Ambiguous);
    assert!(!error.is_retryable(), "never offered to run again (I-13)");
    assert!(
        events.iter().any(|event| matches!(
            event,
            Event::Failed {
                retryable: false,
                ..
            }
        )),
        "{events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(event, Event::Cancelled)),
        "{events:?}"
    );
}

#[tokio::test]
async fn a_write_the_server_rolled_back_is_cancelled() {
    let (outcome, _) = stopped(
        Verdict::Cancelled,
        "UPDATE t SET n = n + 1 WHERE n > 0 RETURNING id",
        true,
    )
    .await;

    assert!(
        matches!(
            outcome,
            Ok(Outcome::Executed {
                sink: SinkOutcome::Cancelled,
                ..
            })
        ),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_write_that_ended_before_the_stop_is_completed() {
    let (outcome, events) = stopped(
        Verdict::Finished,
        "DELETE FROM t WHERE n < 10 RETURNING id",
        true,
    )
    .await;

    assert!(
        matches!(
            outcome,
            Ok(Outcome::Executed {
                sink: SinkOutcome::Exhausted,
                ..
            })
        ),
        "the write was applied, the Stop came too late: {outcome:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Completed { .. })),
        "{events:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_write_whose_driver_stays_silent_ends_ambiguous_after_the_grace() {
    let (outcome, _) = stopped(
        Verdict::Silent,
        "DELETE FROM t WHERE n > 0 RETURNING id",
        true,
    )
    .await;

    let Err(error) = outcome else {
        panic!("expected an ambiguous failure, got {outcome:?}");
    };
    assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
}

#[tokio::test]
async fn an_abandoned_write_is_announced_ambiguous() {
    // The caller stops waiting — a closed conversation — while the write waits
    // for its driver: there is no verdict, so none is invented.
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store.workspaces().create("abandon").expect("workspace").id;
    let config =
        ConnectionConfig::new("abandon", DriverId::postgres()).with_environment(Environment::Local);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&config);
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(&config);
    let script = Arc::new(Script {
        verdict: Verdict::Silent,
        waiting: Notify::new(),
        catalog: catalog_tests::Probe::default(),
    });
    let slot = executor.sessions.insert(SessionSlot::new(
        config.id,
        Box::new(ScriptedSession(Arc::clone(&script))),
    ));
    let mut events = executor.subscribe();
    let request = ExecRequest::new(QueryLanguage::SQL, "DELETE FROM t WHERE n > 0 RETURNING id")
        .with_limits(ExecLimits::default().writable().with_timeout(None));
    let never = CancelToken::new();
    let run = executor.dispatch(
        Actor::Human,
        Command::Execute {
            connection: config.id,
            session: slot.id(),
            request: Box::new(request),
        },
        &never,
    );
    tokio::select! {
        outcome = run => panic!("the write ended on its own: {outcome:?}"),
        () = script.waiting.notified() => {}
    }

    let mut terminal = None;
    while let Ok(event) = events.try_recv() {
        if event.event.is_terminal() {
            terminal = Some(event.event);
        }
    }
    assert!(
        matches!(
            terminal,
            Some(Event::Failed {
                retryable: false,
                ..
            })
        ),
        "{terminal:?}"
    );
}

#[tokio::test]
async fn a_stopped_read_is_cancelled_at_once() {
    // Even on a driver that never answers: a read has no effect to report,
    // and the grid must let go at once.
    let (outcome, events) = stopped(Verdict::Silent, "SELECT id FROM t", false).await;

    assert!(
        matches!(
            outcome,
            Ok(Outcome::Executed {
                sink: SinkOutcome::Cancelled,
                ..
            })
        ),
        "{outcome:?}"
    );
    assert!(
        events.iter().any(|event| matches!(event, Event::Cancelled)),
        "{events:?}"
    );
}

#[tokio::test]
async fn a_sqlite_returning_write_stopped_mid_stream_is_not_announced_cancelled() {
    // The real driver, not a script (issue #181): a `RETURNING` made all its
    // changes at its first step, so a Stop while its rows stream proves no
    // rollback. Terminal event and history must both stay ambiguous.
    use oxyn_driver_sqlite::{BatchLimits, SqliteDriver};

    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store
        .workspaces()
        .create("sqlite-stop")
        .expect("workspace")
        .id;
    let config = ConnectionConfig::new("sqlite-stop", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
    store
        .connections()
        .save(workspace, &config)
        .expect("connection");
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&config);
    let mut drivers = oxyn_driver::DriverRegistry::new();
    drivers
        .register(Arc::new(
            // One row per batch: the stream outlasts the Stop.
            SqliteDriver::new().with_batch_limits(BatchLimits::new().with_max_rows(1)),
        ))
        .expect("SQLite registration");
    let executor = Executor::builder(Arc::clone(&store), policy)
        .with_drivers(Arc::new(drivers))
        .with_workspace(workspace)
        .build();
    executor.register_connection(&config);
    let Ok(Outcome::Connected { session, .. }) = executor
        .dispatch(
            Actor::Human,
            Command::Connect {
                connection: config.id,
            },
            &CancelToken::new(),
        )
        .await
    else {
        panic!("an in-memory database always opens");
    };
    let writable = ExecLimits::default().writable().with_timeout(None);
    let execute = |text: &str| Command::Execute {
        connection: config.id,
        session,
        request: Box::new(ExecRequest::new(QueryLanguage::SQL, text).with_limits(writable.clone())),
    };
    executor
        .dispatch(
            Actor::Human,
            execute("CREATE TABLE t(id INTEGER)"),
            &CancelToken::new(),
        )
        .await
        .expect("fixture SQL");

    let statement = "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s \
                     WHERE n < 100000) INSERT INTO t SELECT n FROM s RETURNING id";
    let mut events = executor.subscribe();
    let stop = CancelToken::new();
    let run = executor.dispatch(Actor::Human, execute(statement), &stop);
    let outcome = tokio::time::timeout(GIVE_UP_AFTER, async {
        tokio::pin!(run);
        loop {
            tokio::select! {
                outcome = &mut run => break outcome,
                event = events.recv() => {
                    if matches!(event.map(|seen| seen.event), Ok(Event::BatchReady { .. })) {
                        stop.cancel();
                        break run.await;
                    }
                }
            }
        }
    })
    .await
    .expect("a stopped execution must end");

    let Err(error) = outcome else {
        panic!("expected an ambiguous failure, got {outcome:?}");
    };
    assert!(!error.is_cancelled(), "{error:?}");
    assert_eq!(error.class(), oxyn_core::ErrorClass::Ambiguous, "{error:?}");
    let mut terminal = None;
    while let Ok(event) = events.try_recv() {
        if event.event.is_terminal() {
            terminal = Some(event.event);
        }
    }
    assert!(
        matches!(
            terminal,
            Some(Event::Failed {
                retryable: false,
                ..
            })
        ),
        "{terminal:?}"
    );

    let recorded = store
        .history()
        .recent(10)
        .expect("history")
        .into_iter()
        .find(|entry| entry.record.statement == statement)
        .expect("the stopped write is in the history");
    assert_ne!(recorded.record.status, oxyn_store::HistoryStatus::Cancelled);
    assert_eq!(
        recorded.record.error_class,
        Some(oxyn_core::ErrorClass::Ambiguous)
    );
    assert!(recorded.record.requires_reconciliation());
}
