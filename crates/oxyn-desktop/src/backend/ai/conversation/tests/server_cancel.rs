//! An external agent's query, abandoned, stops **on the PostgreSQL server**.
//!
//! Dropping a future cancels nothing remote: a `SELECT` left running keeps a
//! server process, its locks and one of the session's four connections. These
//! tests follow a call from the agent's HTTP request, through Oxyn's MCP
//! endpoint, the question's gate, the real `AgentSink` and the executor, down to
//! the driver — and read the outcome where it is decided, in
//! `pg_stat_activity`, from a connection of their own.
//!
//! Three ways a call is abandoned, each with its own path:
//!
//! * **the question closes** — the user stops it: the call's token is
//!   cancelled and the call winds down at its own pace;
//! * **the endpoint stops** — the conversation ends: the call is cancelled,
//!   then given `STOP_GRACE` before its connection is aborted;
//! * **the serving future is dropped whole** — nobody stopped the endpoint: the
//!   call is cancelled and its task aborted at once, with no grace at all. This
//!   is the grace run out, taken to its limit: only the executor's abandon guard
//!   and the cursor's `Drop` are left to reach the server.
//!
//! # The limit these tests cannot lift
//!
//! Every path ends in the driver's stream task sending `pg_cancel_backend`
//! over a **second** connection. A server that accepts no new connection —
//! `max_connections` reached, a network cut, a host that stopped answering —
//! never receives it: the query goes on until it ends, until the server's
//! `statement_timeout`, or until the dead socket is noticed. The driver contract
//! classes such an end ambiguous, never failed, and nothing replays it
//! (DRIVER-CONTRACT §4, I-13). A fixed grace cannot change that; only the
//! server can.
//!
//! The proof assumes the test server keeps `client_connection_check_interval`
//! at its default, `0`: `pg_sleep` then never notices a closed client socket,
//! so only a cancellation can end it within the bound below.
//!
//! # Running them
//!
//! `#[ignore]`, like every test that needs a server, and configured the same
//! way (see `drivers/oxyn-driver-postgres/src/integration.rs`):
//!
//! ```sh
//! docker run --rm -d -p 127.0.0.1::5432 \
//!   -e POSTGRES_PASSWORD=oxyn --name oxyn-pg postgres:17
//! OXYN_PG_TEST_URL="postgres://postgres:oxyn@$(docker port oxyn-pg 5432)/postgres?sslmode=disable" \
//!   cargo test -p oxyn-desktop server_cancel -- --ignored --test-threads=1
//! docker rm -f oxyn-pg
//! ```
//!
//! Without `OXYN_PG_TEST_URL`, each test stops without failing and says so.

use std::time::{Duration, Instant};

use arrow::array::{Array as _, Int32Array, StringArray};
use oxyn_ai::external::mcp::server::{Endpoint, serve};
use oxyn_core::{DriverId, ScalarValue};
use oxyn_driver::{Driver as _, ParsedDsn};
use oxyn_driver_postgres::PostgresDriver;
use oxyn_exec::Outcome;
use oxyn_secrets::ExposeSecret as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;

const VARIABLE: &str = "OXYN_PG_TEST_URL";

/// Far longer than any bound below: still running at the end means nothing
/// stopped it.
const SLEEP_SECONDS: u32 = 60;

/// How long the server may take to see the cancellation. Generous for a
/// loopback round trip; a hang becomes a readable failure, not a stuck suite.
const CANCEL_BOUND: Duration = Duration::from_secs(5);

/// How long the server is watched after the stop for a second execution.
const RETRY_WATCH: Duration = Duration::from_millis(1500);

/// The question's call, its agent, and the server seen from aside.
struct Bench {
    runtime: tokio::runtime::Runtime,
    backend: Backend,
    open: OpenConnection,
    thread: Arc<Thread>,
    node: u32,
    identity: AgentId,
    conversation: AgentSessionId,
    service: Arc<ToolService>,
    turns: ToolTurns,
    /// The connection's `application_name`: what tells its server processes
    /// apart from every other client of the test server.
    tag: String,
    /// An administrative session, outside the backend under test.
    admin: Box<dyn oxyn_driver::Session>,
}

fn bench() -> Option<Bench> {
    let Ok(url) = std::env::var(VARIABLE) else {
        eprintln!("{VARIABLE} is not set: test skipped");
        return None;
    };
    let parsed = ParsedDsn::parse(&url).expect("OXYN_PG_TEST_URL must be a `postgres://` URL");
    let runtime = runtime();
    let backend = {
        let _guard = runtime.enter();
        Backend::open_temporary().expect("temporary backend")
    };
    let tag = format!("oxyn_cancel_{}", uuid::Uuid::new_v4().simple());

    let parts = parsed.parts();
    // The URL's own options first — `sslmode=require` included —, secrets
    // already set apart by the parser; only `application_name` is ours.
    let mut values: BTreeMap<String, String> = parts
        .options
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    values.extend(
        [
            ("host", parts.host.clone()),
            ("port", parts.port.map(|port| port.to_string())),
            ("user", parts.user.clone()),
            ("database", parts.database().map(str::to_owned)),
            ("application_name", Some(tag.clone())),
        ]
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_owned(), value?))),
    );
    // Only when the URL names no mode: the Docker image serves no TLS.
    values
        .entry("sslmode".to_owned())
        .or_insert_with(|| "disable".to_owned());
    let secrets = parsed
        .credentials()
        .password()
        .map(|password| ("password".to_owned(), password.expose_secret().to_owned()))
        .into_iter()
        .collect();
    let draft = ConnectionDraft {
        driver: "postgres".into(),
        name: "server cancellation".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values,
        secrets,
    };
    let open = runtime.block_on(async {
        match backend
            .connect(CommandId::new(), draft)
            .await
            .expect("the test server must be reachable")
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
            ConnectResponse::Saved { message, .. } => panic!("not opened: {message}"),
        }
    });

    let admin = runtime.block_on(async {
        let (parts, credentials) = parsed.into_parts();
        PostgresDriver::new()
            .connect(
                &parts
                    .to_config("cancellation observer", DriverId::postgres())
                    .with_environment(Environment::Local),
                &credentials,
                &CancelToken::new(),
            )
            .await
            .expect("the observer connects")
    });

    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let thread = backend
        .inner
        .ai
        .thread_for(connection, None)
        .expect("a conversation");
    let (channel, _received) = recording();
    let (node, _) = thread
        .begin(
            None,
            "how long does this take?",
            channel,
            Scope {
                connection,
                name: open.name.clone(),
                environment: open.environment,
            },
        )
        .expect("begins");

    // Wired as `ask_agent` wires it, as in the SQLite test above.
    let spec = oxyn_ai::sql_agent();
    let (identity, conversation) = (AgentId::new(), AgentSessionId::new());
    let service = Arc::new(ToolService::new(
        ToolRegistry::builtin(),
        spec.allowed_tools.clone(),
        ToolScope::new(
            connection,
            open.session.parse().expect("session id"),
            QueryLanguage::Sql(SqlDialect::Postgres),
        ),
        Arc::new(StoredTier {
            executor: Arc::clone(&backend.inner.executor),
            connection,
        }),
        Actor::agent(identity, conversation),
    ));
    let turns = ToolTurns::new(spec.max_turns, Arc::new(|| false));

    Some(Bench {
        runtime,
        backend,
        open,
        thread,
        node,
        identity,
        conversation,
        service,
        turns,
        tag,
        admin,
    })
}

impl Bench {
    /// Opens the question the agent's calls run within.
    fn question(&self) -> oxyn_ai::external::mcp::OpenTurn {
        let executor = &self.backend.inner.executor;
        self.turns.open(
            Arc::new(AgentSink {
                sink: ExecutorSink::for_agent(
                    Arc::clone(executor),
                    self.identity,
                    self.conversation,
                ),
                executor: Arc::clone(executor),
                decisions: Arc::clone(&self.backend.inner.ai.decisions),
                thread: Arc::clone(&self.thread),
                node: self.node,
                question: QuestionOpen::new(),
                sampling: None,
            }),
            Arc::new(AgentObserverFilter(Observer {
                thread: Arc::clone(&self.thread),
                node: self.node,
            })),
            CancelToken::new(),
        )
    }

    /// Opens the endpoint the agent reaches.
    fn endpoint(&self) -> (Endpoint, futures::future::BoxFuture<'static, ()>) {
        self.runtime
            .block_on(serve(Arc::clone(&self.service), self.turns.clone()))
            .expect("the endpoint binds the loopback")
    }

    /// Posts a long `execute_query` as the agent would, on a task of its own:
    /// the answer, or the I/O error of a connection Oxyn cut.
    fn call_long_query(
        &self,
        endpoint: &Endpoint,
    ) -> tokio::task::JoinHandle<std::io::Result<String>> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 97,
            "method": "tools/call",
            "params": {
                "name": "execute_query",
                "arguments": {
                    "statement": format!("SELECT pg_catalog.pg_sleep({SLEEP_SECONDS}) AS slept")
                }
            }
        })
        .to_string();
        let address = endpoint
            .url()
            .trim_start_matches("http://")
            .trim_end_matches("/mcp")
            .to_owned();
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\n\
             Content-Type: application/json\r\nConnection: close\r\nContent-Length: {length}\r\n\
             \r\n{body}",
            token = endpoint.token(),
            length = body.len(),
        );
        self.runtime.spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(&address).await?;
            stream.write_all(request.as_bytes()).await?;
            let mut answer = String::new();
            tokio::time::timeout(
                Duration::from_secs(u64::from(SLEEP_SECONDS) * 2),
                stream.read_to_string(&mut answer),
            )
            .await??;
            Ok(answer)
        })
    }

    /// `(pid, query_start)` of every server process of the connection under
    /// test that is running the long query now.
    fn sleeping(&self) -> Vec<(i32, String)> {
        let request = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Postgres),
            "SELECT pid, query_start::text AS started FROM pg_catalog.pg_stat_activity \
             WHERE application_name = $1 AND state = 'active' AND query LIKE '%pg_sleep%'",
        )
        .with_params(vec![ScalarValue::Text(self.tag.clone())])
        .with_intent(oxyn_core::StatementIntent::Read);
        self.runtime.block_on(async {
            let mut cursor = self
                .admin
                .execute(request, &CancelToken::new())
                .await
                .expect("pg_stat_activity reads");
            let mut found = Vec::new();
            while let Some(batch) = cursor.next_batch().await.expect("the read drains") {
                let pids = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .expect("pid is an integer");
                let starts = batch
                    .column(1)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("query_start as text");
                for row in 0..batch.num_rows() {
                    found.push((pids.value(row), starts.value(row).to_owned()));
                }
            }
            found
        })
    }

    /// Is the server process `pid` still there, whatever it does?
    fn alive(&self, pid: i32) -> bool {
        let request = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Postgres),
            "SELECT count(*)::int4 AS found FROM pg_catalog.pg_stat_activity WHERE pid::int8 = $1",
        )
        .with_params(vec![ScalarValue::Int64(i64::from(pid))])
        .with_intent(oxyn_core::StatementIntent::Read);
        self.runtime.block_on(async {
            let mut cursor = self
                .admin
                .execute(request, &CancelToken::new())
                .await
                .expect("pg_stat_activity reads");
            let batch = cursor
                .next_batch()
                .await
                .expect("the read drains")
                .expect("one row");
            batch
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .expect("a count cast to int4")
                .value(0)
                > 0
        })
    }

    fn pause(&self, length: Duration) {
        self.runtime.block_on(tokio::time::sleep(length));
    }

    /// Waits until the long query runs on the server, and returns it.
    fn started(&self) -> (i32, String) {
        let deadline = Instant::now() + CANCEL_BOUND;
        loop {
            if let [running] = self.sleeping().as_slice() {
                return running.clone();
            }
            assert!(
                Instant::now() < deadline,
                "the long query never reached the server"
            );
            self.pause(Duration::from_millis(20));
        }
    }

    /// Asserts the server stops the query within [`CANCEL_BOUND`], then that
    /// no second execution of it appears (I-13): a replay would show a new
    /// `query_start`, or a new process.
    fn stopped_without_replay(&self, at: Instant, first: &(i32, String)) {
        let deadline = at + CANCEL_BOUND;
        loop {
            let running = self.sleeping();
            if running.is_empty() {
                break;
            }
            assert_eq!(
                running,
                std::slice::from_ref(first),
                "a second execution appeared"
            );
            assert!(
                Instant::now() < deadline,
                "the query still runs on the server {CANCEL_BOUND:?} after the call was abandoned"
            );
            self.pause(Duration::from_millis(20));
        }
        eprintln!(
            "the server stopped the query {:?} after the call was abandoned",
            at.elapsed()
        );
        let watched = Instant::now() + RETRY_WATCH;
        while Instant::now() < watched {
            assert_eq!(self.sleeping(), [], "the abandoned query ran again");
            self.pause(Duration::from_millis(50));
        }
    }

    /// The connection that ran the abandoned query is released, and the
    /// session still serves.
    ///
    /// Released means closed: the stream task cancels while holding it, then
    /// closes it rather than hand a process with a cancellation in flight to
    /// the next query. A connection still held by the abandoned call would
    /// keep that process alive, idle, with one connection fewer in the pool.
    fn session_serves(&self, first: &(i32, String)) {
        let deadline = Instant::now() + CANCEL_BOUND;
        while self.alive(first.0) {
            assert!(
                Instant::now() < deadline,
                "the abandoned query's connection is still held"
            );
            self.pause(Duration::from_millis(20));
        }
        let started = Instant::now();
        let outcome = self
            .runtime
            .block_on(tokio::time::timeout(
                CANCEL_BOUND,
                self.backend.inner.executor.dispatch(
                    Actor::Human,
                    Command::Execute {
                        connection: self.open.connection.parse().expect("connection id"),
                        session: self.open.session.parse().expect("session id"),
                        request: Box::new(ExecRequest::new(
                            QueryLanguage::Sql(SqlDialect::Postgres),
                            "SELECT 1 AS still_served",
                        )),
                    },
                    &CancelToken::new(),
                ),
            ))
            .expect("the session answers after the cancellation")
            .expect("the query runs");
        assert!(matches!(outcome, Outcome::Executed { .. }), "{outcome:?}");
        eprintln!("the session served again within {:?}", started.elapsed());
    }
}

#[test]
#[ignore = "requires an isolated PostgreSQL test server"]
fn closing_the_question_cancels_the_agents_query_on_the_server() {
    let Some(bench) = bench() else { return };
    let _guard = bench.runtime.enter();
    let (endpoint, serving) = bench.endpoint();
    let serving = bench.runtime.spawn(serving);
    let question = bench.question();

    let call = bench.call_long_query(&endpoint);
    let first = bench.started();
    let at = Instant::now();
    // The user stops the question: the call's token is cancelled, nothing is
    // aborted.
    drop(question);

    bench.stopped_without_replay(at, &first);
    let answer = bench
        .runtime
        .block_on(call)
        .expect("the agent's task")
        .expect("the agent is answered, not cut off");
    eprintln!("the agent read: {answer}");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    // Told the outcome is unknown and not to replay it — never a clean
    // failure an agent would retry (I-13).
    assert!(answer.contains("class: ambiguous"), "{answer}");
    assert!(answer.contains("retryable: false"), "{answer}");
    bench.session_serves(&first);

    drop(endpoint);
    bench
        .runtime
        .block_on(tokio::time::timeout(CANCEL_BOUND, serving))
        .expect("the endpoint stops")
        .expect("the endpoint did not panic");
}

#[test]
#[ignore = "requires an isolated PostgreSQL test server"]
fn stopping_the_endpoint_cancels_the_agents_query_within_its_grace() {
    let Some(bench) = bench() else { return };
    let _guard = bench.runtime.enter();
    let (endpoint, serving) = bench.endpoint();
    let serving = bench.runtime.spawn(serving);
    let _question = bench.question();

    let call = bench.call_long_query(&endpoint);
    let first = bench.started();
    let at = Instant::now();
    // The conversation ends with the question still open: the endpoint
    // cancels, waits its grace, then aborts what is left.
    drop(endpoint);

    bench.stopped_without_replay(at, &first);
    bench
        .runtime
        .block_on(tokio::time::timeout(CANCEL_BOUND, serving))
        .expect("the endpoint stops within its grace")
        .expect("the endpoint did not panic");
    // Answered if the call wound down within the grace, cut off otherwise:
    // either way, never a result taken for complete.
    let answer = bench.runtime.block_on(call).expect("the agent's task");
    eprintln!("the agent read: {answer:?}");
    assert!(
        !answer
            .as_deref()
            .is_ok_and(|answer| answer.contains("\"slept\"")),
        "an abandoned call handed back a result: {answer:?}"
    );
    bench.session_serves(&first);
}

#[test]
#[ignore = "requires an isolated PostgreSQL test server"]
fn dropping_the_serving_future_whole_still_cancels_the_query_on_the_server() {
    let Some(bench) = bench() else { return };
    let _guard = bench.runtime.enter();
    let (endpoint, serving) = bench.endpoint();
    let serving = bench.runtime.spawn(serving);
    let _question = bench.question();

    let call = bench.call_long_query(&endpoint);
    let first = bench.started();
    let at = Instant::now();
    // Nobody stops the endpoint: its future is dropped where it stands. The
    // call's task is aborted at once, with no grace — what is left to reach
    // the server is the executor's abandon guard and the cursor's `Drop`.
    serving.abort();

    bench.stopped_without_replay(at, &first);
    let cut = bench.runtime.block_on(call).expect("the agent's task");
    eprintln!("the agent read: {cut:?}");
    assert!(
        !cut.as_deref()
            .is_ok_and(|answer| answer.contains("\"slept\"")),
        "an aborted call handed back a result: {cut:?}"
    );
    bench.session_serves(&first);
    drop(endpoint);
}
