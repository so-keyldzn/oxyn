//! A read aimed at production leaves bounded to read-only (issue #11).
//!
//! A read passes the gate without confirmation, and a `SELECT` that calls a
//! writing function looks exactly like one. The executor therefore bounds
//! every production read to read-only, whatever its caller asked, and the
//! driver makes the server refuse the write. The first tests prove the bound
//! at the driver boundary, with no server; the last ones prove the refusal on
//! a real PostgreSQL, through the bus.

use super::*;
use async_trait::async_trait;
use oxyn_catalog::CatalogProvider;
use oxyn_core::{
    Capabilities, DefaultPolicy, DriverId, ExecLimits, QueryLanguage, SqlDialect, StatementHandle,
};
use oxyn_driver::Session;

/// A session that records the bounds each execution reached it with.
#[derive(Default)]
struct Recorder {
    read_only: Mutex<Vec<bool>>,
    catalog: catalog_tests::Probe,
}

struct RecordingSession(Arc<Recorder>);

#[async_trait]
impl Session for RecordingSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL
    }
    async fn execute(&self, request: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.0.read_only.lock().push(request.limits.read_only);
        // What reached the driver is all these tests look at: failing here
        // spares a cursor.
        Err(OxynError::Query("recorded".into()))
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
    recorder: Arc<Recorder>,
}

fn bench(connection: &ConnectionConfig, policy: Arc<dyn PolicyGate>) -> Bench {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("production reads")
        .expect("workspace")
        .id;
    store
        .connections()
        .save(workspace, connection)
        .expect("connection");
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(connection);
    let recorder = Arc::new(Recorder::default());
    let slot = executor.sessions.insert(SessionSlot::new(
        connection.id,
        Box::new(RecordingSession(Arc::clone(&recorder))),
    ));
    Bench {
        executor,
        connection: connection.id,
        session: slot.id(),
        recorder,
    }
}

fn default_policy(connection: &ConnectionConfig) -> Arc<dyn PolicyGate> {
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(connection);
    policy
}

/// What a console sends on a writable connection: the bound lifted.
fn writable(bench: &Bench, text: &str) -> Command {
    Command::Execute {
        connection: bench.connection,
        session: bench.session,
        request: Box::new(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), text)
                .with_limits(ExecLimits::default().writable()),
        ),
    }
}

/// The bounds of every execution that reached the session, in order.
fn reached(bench: &Bench) -> Vec<bool> {
    bench.recorder.read_only.lock().clone()
}

#[tokio::test]
async fn a_production_read_reaches_the_driver_read_only_even_when_the_caller_lifted_the_bound() {
    let connection = ConnectionConfig::new("customers", DriverId::postgres())
        .with_environment(Environment::Production);
    let bench = bench(&connection, default_policy(&connection));

    // `SELECT audit_touch()` reads like any `SELECT`: nothing asks for a
    // confirmation, so the server must refuse the write.
    let _ = bench
        .executor
        .dispatch(
            Actor::Human,
            writable(&bench, "SELECT public.audit_touch()"),
            &CancelToken::new(),
        )
        .await;

    assert_eq!(reached(&bench), [true]);
}

#[tokio::test]
async fn a_connection_without_environment_counts_as_production_for_its_reads() {
    // No environment set: production, never the reverse (I-02).
    let connection = ConnectionConfig::new("unmarked", DriverId::postgres());
    let bench = bench(&connection, default_policy(&connection));

    let _ = bench
        .executor
        .dispatch(
            Actor::Human,
            writable(&bench, "SELECT * FROM v_touching"),
            &CancelToken::new(),
        )
        .await;

    assert_eq!(reached(&bench), [true]);
}

#[tokio::test]
async fn the_bound_follows_the_gates_marking_when_the_executors_lags() {
    // The executor still knows the connection as local; the gate has learned
    // it is production. The two registries are fed apart: the bound must rest
    // on what the gate judges, not on the copy that lags.
    let lagging = ConnectionConfig::new("customers", DriverId::postgres())
        .with_environment(Environment::Local);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&lagging.clone().with_environment(Environment::Production));
    let bench = bench(&lagging, policy);

    let _ = bench
        .executor
        .dispatch(
            Actor::Human,
            writable(&bench, "SELECT public.audit_touch()"),
            &CancelToken::new(),
        )
        .await;

    assert_eq!(reached(&bench), [true]);
}

#[tokio::test]
async fn a_read_outside_production_keeps_the_bound_its_caller_chose() {
    let connection = ConnectionConfig::new("workbench", DriverId::postgres())
        .with_environment(Environment::Staging);
    let bench = bench(&connection, default_policy(&connection));

    let _ = bench
        .executor
        .dispatch(
            Actor::Human,
            writable(&bench, "SELECT public.audit_touch()"),
            &CancelToken::new(),
        )
        .await;

    assert_eq!(reached(&bench), [false]);
}

#[tokio::test]
async fn a_confirmed_production_write_keeps_its_lifted_bound() {
    let connection = ConnectionConfig::new("customers", DriverId::postgres())
        .with_environment(Environment::Production);
    let bench = bench(&connection, default_policy(&connection));

    let Outcome::NeedsApproval { command, .. } = bench
        .executor
        .dispatch(
            Actor::Human,
            writable(&bench, "INSERT INTO audit VALUES (1)"),
            &CancelToken::new(),
        )
        .await
        .expect("a decision")
    else {
        panic!("a production write asks for a confirmation naming the connection");
    };
    assert!(reached(&bench).is_empty(), "nothing runs before the answer");

    let _ = bench
        .executor
        .approve("tester", command, &CancelToken::new())
        .await;

    assert_eq!(reached(&bench), [false]);
}

/// A policy that asks about everything, to reach `approve` with a read.
struct AskAlways;

impl PolicyGate for AskAlways {
    fn authorize(&self, _: &Actor, _: &Command, _: Environment) -> Decision {
        Decision::approval("fixture policy asks about everything", None)
    }
}

#[tokio::test]
async fn an_approved_read_on_production_is_bounded_too() {
    let connection = ConnectionConfig::new("customers", DriverId::postgres())
        .with_environment(Environment::Production);
    let bench = bench(&connection, Arc::new(AskAlways));

    let Outcome::NeedsApproval { command, .. } = bench
        .executor
        .dispatch(
            Actor::Human,
            writable(&bench, "SELECT public.audit_touch()"),
            &CancelToken::new(),
        )
        .await
        .expect("a decision")
    else {
        panic!("the fixture policy asks about everything");
    };
    let _ = bench
        .executor
        .approve("tester", command, &CancelToken::new())
        .await;

    assert_eq!(reached(&bench), [true]);
}

// ── Against a real server ───────────────────────────────────────────────────
//
// `#[ignore]`, like the driver's own server tests, and started the same way:
// see `drivers/oxyn-driver-postgres/src/integration.rs`. Without
// `OXYN_PG_TEST_URL`, each test stops without failing and says so.

mod server {
    use super::*;
    use oxyn_driver::{Credentials, Driver as _, ParsedDsn};
    use oxyn_driver_postgres::PostgresDriver;

    const VARIABLE: &str = "OXYN_PG_TEST_URL";

    /// The login role the connection under test uses: it may write, so that
    /// only the executor's bound stands between a read and a write. It has no
    /// password: the test server trusts local connections, as the driver's
    /// own server tests assume.
    const ROLE: &str = "oxyn_production_reader";

    /// The schema every synthetic object lives in, dropped at the start and
    /// end of each test.
    const SCHEMA: &str = "oxyn_production_reads";

    struct Server {
        executor: Executor,
        connection: ConnectionId,
        session: SessionId,
        /// The session of the administrative connection, for set-up and for
        /// counting what reached the table.
        admin: Box<dyn Session>,
    }

    fn target() -> Option<(ConnectionConfig, Credentials)> {
        let url = std::env::var(VARIABLE).ok()?;
        let (parts, credentials) = ParsedDsn::parse(&url)
            .expect("OXYN_PG_TEST_URL must be a `postgres://` URL")
            .into_parts();
        Some((
            parts.to_config("production reads", DriverId::postgres()),
            credentials,
        ))
    }

    async fn admin_run(admin: &dyn Session, sql: &str) {
        let request = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
            .with_intent(oxyn_core::StatementIntent::Write)
            .with_limits(ExecLimits::default().writable());
        let mut cursor = admin
            .execute(request, &CancelToken::new())
            .await
            .unwrap_or_else(|error| panic!("`{sql}` must be accepted: {error}"));
        while cursor.next_batch().await.expect("set-up drains").is_some() {}
    }

    /// The number of rows the writing function left behind.
    async fn touched(admin: &dyn Session) -> i64 {
        let request = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Postgres),
            format!("SELECT count(*) FROM {SCHEMA}.audit"),
        )
        .with_intent(oxyn_core::StatementIntent::Read);
        let mut cursor = admin
            .execute(request, &CancelToken::new())
            .await
            .expect("count");
        let batch = cursor
            .next_batch()
            .await
            .expect("count drains")
            .expect("one row");
        batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .expect("count(*) is a bigint")
            .value(0)
    }

    /// A fresh schema holding a table, a `VOLATILE` function that inserts into
    /// it, and a view that calls the function; a login role that may do all of
    /// it; and an executor with a session opened as that role, on a connection
    /// marked `environment`.
    async fn server(environment: Option<Environment>) -> Option<Server> {
        let Some((admin_config, credentials)) = target() else {
            eprintln!("{VARIABLE} is not set: test skipped");
            return None;
        };
        let driver = Arc::new(PostgresDriver::new());
        let admin = driver
            .connect(
                &admin_config.clone().with_environment(Environment::Local),
                &credentials,
                &CancelToken::new(),
            )
            .await
            .expect("the test server must be reachable");
        for sql in [
            format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"),
            format!("DROP ROLE IF EXISTS {ROLE}"),
            format!("CREATE ROLE {ROLE} LOGIN"),
            format!("CREATE SCHEMA {SCHEMA} AUTHORIZATION {ROLE}"),
            format!("CREATE TABLE {SCHEMA}.audit (at timestamptz DEFAULT now())"),
            format!(
                "CREATE FUNCTION {SCHEMA}.audit_touch() RETURNS integer \
                 LANGUAGE sql VOLATILE AS \
                 'INSERT INTO {SCHEMA}.audit DEFAULT VALUES RETURNING 1'"
            ),
            format!("CREATE VIEW {SCHEMA}.touching AS SELECT {SCHEMA}.audit_touch() AS touched"),
            format!("GRANT ALL ON ALL TABLES IN SCHEMA {SCHEMA} TO {ROLE}"),
            format!("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {SCHEMA} TO {ROLE}"),
        ] {
            admin_run(&*admin, &sql).await;
        }

        let base = admin_config.with_param("user", ROLE);
        let connection = match environment {
            Some(environment) => base.with_environment(environment),
            None => base,
        };
        let policy = Arc::new(DefaultPolicy::new());
        policy.register(&connection);
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let workspace = store
            .workspaces()
            .create("production reads")
            .expect("workspace")
            .id;
        store
            .connections()
            .save(workspace, &connection)
            .expect("connection");
        let mut drivers = DriverRegistry::new();
        drivers.register(driver).expect("PostgreSQL registration");
        let executor = Executor::builder(store, policy)
            .with_drivers(Arc::new(drivers))
            .with_workspace(workspace)
            .build();
        executor.register_connection(&connection);
        let Outcome::Connected { session, .. } = executor
            .dispatch(
                Actor::Human,
                Command::Connect {
                    connection: connection.id,
                },
                &CancelToken::new(),
            )
            .await
            .expect("connection as the test role")
        else {
            panic!("expected a connected session");
        };
        Some(Server {
            executor,
            connection: connection.id,
            session,
            admin,
        })
    }

    async fn tear_down(server: Server) {
        let _ = server
            .executor
            .dispatch(
                Actor::Human,
                Command::CloseSession {
                    connection: server.connection,
                    session: server.session,
                },
                &CancelToken::new(),
            )
            .await;
        admin_run(&*server.admin, &format!("DROP SCHEMA {SCHEMA} CASCADE")).await;
        admin_run(&*server.admin, &format!("DROP ROLE {ROLE}")).await;
        server.admin.close().await.expect("close");
    }

    /// What a console sends on a writable connection.
    async fn console(server: &Server, text: &str) -> Result<Outcome> {
        server
            .executor
            .dispatch(
                Actor::Human,
                Command::Execute {
                    connection: server.connection,
                    session: server.session,
                    request: Box::new(
                        ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), text)
                            .with_limits(ExecLimits::default().writable()),
                    ),
                },
                &CancelToken::new(),
            )
            .await
    }

    fn refused(issue: &Result<Outcome>) -> bool {
        match issue {
            Err(OxynError::Query(message)) => message.contains("read-only"),
            _ => false,
        }
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL test server"]
    async fn a_writing_function_does_not_write_on_production_directly_or_through_a_view() {
        for environment in [Some(Environment::Production), None] {
            let Some(server) = server(environment).await else {
                return;
            };

            let direct = console(&server, &format!("SELECT {SCHEMA}.audit_touch()")).await;
            assert!(
                refused(&direct),
                "{environment:?}: direct call must be refused"
            );
            let view = console(&server, &format!("SELECT * FROM {SCHEMA}.touching")).await;
            assert!(refused(&view), "{environment:?}: the view must be refused");
            assert_eq!(touched(&*server.admin).await, 0, "{environment:?}");

            // A plain read stays usable.
            let read = console(&server, &format!("SELECT count(*) FROM {SCHEMA}.audit")).await;
            assert!(
                matches!(read, Ok(Outcome::Executed { .. })),
                "{environment:?}: a plain read must run: {:?}",
                read.as_ref().err()
            );

            tear_down(server).await;
        }
    }

    /// The control: the role may write, and outside production nothing but
    /// the caller's bound decides. Without it, the test above could pass on a
    /// role that simply lacks the privilege.
    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL test server"]
    async fn outside_production_the_same_call_writes() {
        let Some(server) = server(Some(Environment::Local)).await else {
            return;
        };

        let direct = console(&server, &format!("SELECT {SCHEMA}.audit_touch()")).await;
        assert!(direct.is_ok(), "{:?}", direct.as_ref().err());
        let view = console(&server, &format!("SELECT * FROM {SCHEMA}.touching")).await;
        assert!(view.is_ok(), "{:?}", view.as_ref().err());
        assert_eq!(touched(&*server.admin).await, 2);

        tear_down(server).await;
    }
}
