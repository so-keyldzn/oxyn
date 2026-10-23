//! The assistant on a session that does not speak SQL
//! ([ADR-0065](../../../../../../../docs/adr/0065-agents-for-non-sql-query-languages.md)).
//!
//! No shipped driver speaks another language yet: a fake one stands for
//! Redis, declaring `REDIS_COMMAND` and nothing else, and proving `GET` and
//! `SCAN` read-only as a real driver proves it from its command table
//! (ADR-0064). What these tests hold is the boundary — the agent offered,
//! the prompt and the context written for the language, every request the
//! agent sends classified by the session and gated —, never Redis itself.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use oxyn_ai::{AgentCatalog, UserAgentFile, parse_agent_file};
use oxyn_catalog::{CatalogPath, CatalogProvider, Relation, RelationKind, RelationRef, ServerInfo};
use oxyn_core::{Capabilities, DriverId, StatementHandle, StatementIntent};
use oxyn_driver::{Credentials, Driver, DriverFamily, DriverMetadata, Session};

use super::agent_roles::{capturing, ended, local_provider, question};
use super::*;

/// What the fake session was sent: each text, with whether it was bounded
/// to a read-only execution.
#[derive(Default)]
struct Seen(Mutex<Vec<(String, bool)>>);

struct KeyDriver {
    metadata: DriverMetadata,
    seen: Arc<Seen>,
}

const KEY_DRIVER: &str = "fakekeys";

impl KeyDriver {
    fn new(seen: Arc<Seen>) -> Self {
        Self {
            metadata: DriverMetadata::new(
                DriverId::new(KEY_DRIVER).expect("a valid driver name"),
                "Fake keys",
                DriverFamily::KeyValue,
            ),
            seen,
        }
    }
}

#[async_trait]
impl Driver for KeyDriver {
    fn id(&self) -> DriverId {
        self.metadata.id.clone()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::REDIS_COMMAND
    }

    async fn connect(
        &self,
        _config: &ConnectionConfig,
        _credentials: &Credentials,
        _cancel: &CancelToken,
    ) -> oxyn_core::Result<Box<dyn Session>> {
        Ok(Box::new(KeySession(Arc::clone(&self.seen))))
    }
}

struct KeySession(Arc<Seen>);

#[async_trait]
impl Session for KeySession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::REDIS_COMMAND
    }

    async fn execute(
        &self,
        request: ExecRequest,
        _cancel: &CancelToken,
    ) -> oxyn_core::Result<Box<dyn oxyn_driver::Cursor>> {
        self.0
            .0
            .lock()
            .push((request.text.clone(), request.limits.read_only));
        // What reached the session is all these tests look at.
        Err(OxynError::Query("recorded".into()))
    }

    fn classify_intent(&self, request: &ExecRequest) -> Option<StatementIntent> {
        let command = request.text.split_whitespace().next()?;
        ["GET", "SCAN"]
            .iter()
            .any(|read| command.eq_ignore_ascii_case(read))
            .then_some(StatementIntent::Read)
    }

    async fn cancel(&self, _statement: StatementHandle) -> oxyn_core::Result<()> {
        Ok(())
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &KeyCatalog
    }

    async fn ping(&self) -> oxyn_core::Result<Duration> {
        Ok(Duration::from_millis(1))
    }

    async fn close(self: Box<Self>) -> oxyn_core::Result<()> {
        Ok(())
    }
}

struct KeyCatalog;

#[async_trait]
impl CatalogProvider for KeyCatalog {
    async fn server_info(&self, _cancel: &CancelToken) -> oxyn_core::Result<ServerInfo> {
        Ok(ServerInfo::new(
            "Fake keys",
            "1.0",
            Capabilities::REDIS_COMMAND,
        ))
    }

    async fn list_relations(
        &self,
        _namespace: &CatalogPath,
        _cancel: &CancelToken,
    ) -> oxyn_core::Result<Vec<RelationRef>> {
        Ok(Vec::new())
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        _cancel: &CancelToken,
    ) -> oxyn_core::Result<Relation> {
        Ok(Relation::new(
            relation.relation().unwrap_or(""),
            RelationKind::KeyPattern,
        ))
    }
}

/// The fixture agent of the language: a role, no product knowledge.
const KEYS_AGENT: &str = "0199a3c0-0000-7000-8000-0000000000f1";

fn keys_agent() -> UserAgentFile {
    let text = format!(
        "---\nid: {KEYS_AGENT}\nname: Keys\nlanguages: [redis]\n\
         tools: [execute_query, describe_schema]\n---\nYou help with keys.\n"
    );
    UserAgentFile {
        result: parse_agent_file("keys.md", &text),
        file_name: "keys.md".to_owned(),
    }
}

/// A backend that knows the fake driver, and the agents `users` declares
/// besides the shipped ones.
fn backend_with(users: Vec<UserAgentFile>) -> (Backend, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let backend = Backend::open_temporary_with_driver(Arc::new(KeyDriver::new(Arc::clone(&seen))))
        .expect("temporary backend");
    backend
        .inner
        .ai
        .agents
        .set(AgentCatalog::new(oxyn_ai::shipped_agents(), users));
    (backend, seen)
}

fn open_keys(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    environment: Environment,
) -> OpenConnection {
    let draft = ConnectionDraft {
        driver: KEY_DRIVER.into(),
        name: "keys regression".into(),
        environment,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: BTreeMap::new(),
        secrets: BTreeMap::new(),
    };
    runtime.block_on(async {
        match backend
            .connect(CommandId::new(), draft)
            .await
            .expect("connects")
        {
            ConnectResponse::Open(open) => open,
            // Production asks first, as for any driver.
            ConnectResponse::Approval { command, .. } => match backend
                .decide_connection(command.parse().expect("minted id"), true)
                .await
                .expect("approved")
            {
                Some(ConnectResponse::Open(open)) => open,
                _ => panic!("an approved connection opens"),
            },
            ConnectResponse::Saved { message, .. } => panic!("not opened: {message}"),
        }
    })
}

#[test]
fn a_question_on_a_redis_session_runs_its_agent_written_for_redis() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (port, request) = runtime.block_on(capturing());
    let (backend, _) = backend_with(vec![keys_agent()]);
    let open = open_keys(&runtime, &backend, Environment::Local);
    let provider = local_provider(&runtime, &backend, port);
    let (channel, received) = recording();

    // No agent named: the default is the agent of the session's language.
    runtime
        .block_on(backend.ai_ask(question(&open, &provider, None), channel))
        .map_err(|error| error.message)
        .expect("the question starts");
    let sent = runtime.block_on(request).expect("the provider was called");
    ended(&runtime, &received);

    assert!(sent.contains("You help with keys."), "the role: {sent}");
    assert!(sent.contains("The language: `redis`"), "{sent}");
    assert!(sent.contains("tagged `redis`"), "{sent}");
    assert!(!sent.contains("The dialect"), "no SQL fragment: {sent}");
    assert!(
        !sent.contains("You help a data professional write and fix queries"),
        "never the SQL agent: {sent}"
    );
    // The context came through the one gateway, under the connection's tier,
    // written for the language (I-04).
    assert!(sent.contains("privacy tier `metadata`"), "{sent}");
    assert!(sent.contains("query language: redis"), "{sent}");
    assert!(!sent.contains("{{"), "every variable is filled: {sent}");
}

#[test]
fn without_an_agent_for_its_language_a_redis_question_is_refused_before_anything_starts() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (backend, _) = backend_with(Vec::new());
    let open = open_keys(&runtime, &backend, Environment::Local);
    // Nothing listens there: a question that reached it would fail later.
    let provider = local_provider(&runtime, &backend, 9);

    let (channel, received) = recording();
    let refused = runtime
        .block_on(backend.ai_ask(question(&open, &provider, None), channel))
        .expect_err("no agent writes Redis commands");
    assert!(
        refused.message.contains("No agent writes redis"),
        "{}",
        refused.message
    );

    let (channel, _) = recording();
    let sql = oxyn_ai::sql_agent().id.to_string();
    let refused = runtime
        .block_on(backend.ai_ask(question(&open, &provider, Some(sql)), channel))
        .expect_err("the SQL agent does not write Redis commands");
    assert!(
        refused.message.contains("does not write redis"),
        "{}",
        refused.message
    );
    assert!(received.lock().is_empty(), "no run started");
}

#[test]
fn the_picker_of_a_redis_connection_offers_only_the_agents_of_its_language() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (backend, _) = backend_with(vec![keys_agent()]);
    let open = open_keys(&runtime, &backend, Environment::Local);
    local_provider(&runtime, &backend, 9);
    let connection: ConnectionId = open.connection.parse().expect("connection id");

    let listed = runtime
        .block_on(backend.ai_list_agents(connection, None))
        .expect("listed");
    let names: Vec<&str> = listed.iter().map(|option| option.name.as_str()).collect();
    assert_eq!(names, ["Keys"]);
}

/// What `execute_query` makes of `statement` for an agent on `open`, in the
/// language the session speaks — the scope a conversation builds.
fn agent_request(backend: &Backend, open: &OpenConnection, statement: &str) -> Command {
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let session: SessionId = open.session.parse().expect("session id");
    let config = backend.config(connection).expect("a connection");
    let language = backend
        .session_language(&config, session)
        .expect("the session speaks one language");
    assert_eq!(language, QueryLanguage::RedisCommand);
    let registry = ToolRegistry::builtin();
    let allowed: Vec<String> = registry.names().into_iter().map(str::to_owned).collect();
    registry
        .translate(
            &oxyn_llm::ToolCall::new(
                "call-1",
                "execute_query",
                serde_json::json!({ "statement": statement }),
            ),
            &allowed,
            &ToolScope::new(connection, session, language),
        )
        .expect("a valid call")
}

#[test]
fn an_agent_request_on_production_is_classified_by_the_session_and_a_write_refused() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (backend, seen) = backend_with(vec![keys_agent()]);
    let production = open_keys(&runtime, &backend, Environment::Production);
    let (sink, _thread, actor, _) = sink_on(&backend, &production);

    // A command the session cannot prove read-only is a write, and an
    // agent's write on production is a refusal (I-02, I-07).
    let outcome = runtime.block_on(sink.dispatch(
        &CallHandle::new(CallId::fresh()),
        actor,
        agent_request(&backend, &production, "DEL session:1"),
        &CancelToken::new(),
    ));
    assert!(
        matches!(outcome, DispatchOutcome::Denied { .. }),
        "{outcome:?}"
    );
    assert!(seen.0.lock().is_empty(), "nothing reached the session");

    // A command the session proves read-only runs, bounded to a read.
    let outcome = runtime.block_on(sink.dispatch(
        &CallHandle::new(CallId::fresh()),
        actor,
        agent_request(&backend, &production, "GET session:1"),
        &CancelToken::new(),
    ));
    assert!(
        matches!(outcome, DispatchOutcome::Failed { .. }),
        "the fake session refuses what it records: {outcome:?}"
    );
    assert_eq!(*seen.0.lock(), [("GET session:1".to_owned(), true)]);
}

#[test]
fn an_agent_write_on_a_redis_staging_session_waits_for_the_user() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (backend, seen) = backend_with(vec![keys_agent()]);
    let staging = open_keys(&runtime, &backend, Environment::Staging);
    let actor = Actor::agent(AgentId::new(), AgentSessionId::new());

    let outcome = runtime.block_on(backend.inner.executor.dispatch_as(
        CommandId::new(),
        actor,
        agent_request(&backend, &staging, "DEL session:1"),
        &CancelToken::new(),
    ));
    assert!(
        matches!(outcome, Ok(oxyn_exec::Outcome::NeedsApproval { .. })),
        "{outcome:?}"
    );
    assert!(seen.0.lock().is_empty(), "nothing ran before the approval");
}
