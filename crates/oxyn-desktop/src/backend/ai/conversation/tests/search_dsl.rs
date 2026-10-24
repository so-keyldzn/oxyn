//! The assistant on an Elasticsearch connection
//! ([ADR-0062](../../../../../../../docs/adr/0062-elasticsearch-opensearch-rest-json.md),
//! [ADR-0065](../../../../../../../docs/adr/0065-agents-for-non-sql-query-languages.md)).
//!
//! The shipped driver runs against a loopback server that answers the few
//! REST calls a connection makes — `GET /`, the index list, a mapping — and
//! records every request it receives. What these tests hold is that the
//! shipped Elasticsearch agent is the one offered, that its prompt carries
//! the language and the context under the connection's tier, and that what
//! the agent sends is classified by the real driver before the gate.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use oxyn_core::{Capabilities, DriverId};
use oxyn_driver::{Credentials, Driver, DriverFamily, DriverMetadata, Session};
use oxyn_driver_elasticsearch::ElasticsearchDriver;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::agent_roles::{capturing, ended, local_provider, question};
use super::*;

/// The request lines the loopback server received, in order.
type Received = Arc<Mutex<Vec<String>>>;

/// A text no prompt may carry: it is a document value, and the connection's
/// tier is `metadata`.
const ROW_VALUE: &str = "row-value-never-in-a-prompt";

/// Serves the REST calls of one connection on the loopback, one response per
/// TCP connection.
async fn search_server() -> (u16, Received) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port is free");
    let port = listener.local_addr().expect("an address").port();
    let received: Received = Arc::default();
    let log = Arc::clone(&received);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut read = Vec::new();
            let mut buffer = [0_u8; 8192];
            let head = loop {
                let Ok(n) = socket.read(&mut buffer).await else {
                    break None;
                };
                if n == 0 {
                    break None;
                }
                read.extend_from_slice(buffer.get(..n).unwrap_or_default());
                let text = String::from_utf8_lossy(&read).into_owned();
                if text.contains("\r\n\r\n") {
                    break text.lines().next().map(str::to_owned);
                }
            };
            let Some(head) = head else { continue };
            let mut words = head.split_whitespace();
            let (method, path) = (
                words.next().unwrap_or_default().to_owned(),
                words.next().unwrap_or_default().to_owned(),
            );
            log.lock().push(format!("{method} {path}"));
            let body = match (method.as_str(), path.split('?').next().unwrap_or_default()) {
                ("GET", "/") => serde_json::json!({
                    "name": "node-1",
                    "cluster_name": "regression",
                    "version": {"number": "9.1.0", "build_flavor": "default"},
                    "tagline": "You Know, for Search"
                }),
                ("GET", "/_cat/indices") => serde_json::json!([{"index": "orders"}]),
                ("GET", "/orders/_mapping") => serde_json::json!({"orders": {"mappings": {
                    "properties": {
                        "customer": {"type": "keyword"},
                        "total": {"type": "scaled_float", "scaling_factor": 100}
                    }
                }}}),
                _ => serde_json::json!({"hits": {"hits": [
                    {"_index": "orders", "_id": "1", "_source": {"customer": ROW_VALUE}}
                ]}}),
            }
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    (port, received)
}

fn search_draft(driver: &str, port: u16, environment: Environment) -> ConnectionDraft {
    ConnectionDraft {
        driver: driver.into(),
        name: "search regression".into(),
        environment,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("url".to_owned(), format!("http://127.0.0.1:{port}"))]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    }
}

fn open_draft(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    draft: ConnectionDraft,
) -> OpenConnection {
    runtime.block_on(async {
        match backend
            .connect(CommandId::new(), draft)
            .await
            .expect("connects")
        {
            ConnectResponse::Open(open) => open,
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

/// The shipped driver on a connection Oxyn marks as `production`.
///
/// ADR-0052 refuses plain HTTP outside `Local`, and a loopback test server
/// has no certificate: this driver hands the shipped one the loopback
/// address under `Local`, while the backend, the policy and the gate see the
/// `production` connection the user saved. Classification is the shipped
/// session's own.
struct ProductionCluster {
    metadata: DriverMetadata,
    port: u16,
}

const PRODUCTION_CLUSTER: &str = "fakeproductionsearch";

#[async_trait]
impl Driver for ProductionCluster {
    fn id(&self) -> DriverId {
        self.metadata.id.clone()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    fn capabilities(&self) -> Capabilities {
        ElasticsearchDriver::new().capabilities()
    }

    async fn connect(
        &self,
        _config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> oxyn_core::Result<Box<dyn Session>> {
        let local = ConnectionConfig::new("loopback", DriverId::elasticsearch())
            .with_environment(Environment::Local)
            .with_param("url", format!("http://127.0.0.1:{}", self.port));
        ElasticsearchDriver::new()
            .connect(&local, credentials, cancel)
            .await
    }
}

/// What `execute_query` makes of `statement` for an agent on `open`.
fn agent_request(backend: &Backend, open: &OpenConnection, statement: &str) -> Command {
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let session: SessionId = open.session.parse().expect("session id");
    let config = backend.config(connection).expect("a connection");
    let language = backend
        .session_language(&config, session)
        .expect("the session speaks one language");
    assert_eq!(language, QueryLanguage::SearchDsl);
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
fn the_picker_offers_the_elasticsearch_agent_only_on_an_elasticsearch_connection() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (port, _) = runtime.block_on(search_server());
    let backend = Backend::open_temporary().expect("temporary backend");
    let search = open_draft(
        &runtime,
        &backend,
        search_draft(DriverId::ELASTICSEARCH, port, Environment::Local),
    );
    let sqlite = open(&runtime, &backend, Environment::Local);
    local_provider(&runtime, &backend, 9);

    let listed = |open: &OpenConnection| {
        let connection: ConnectionId = open.connection.parse().expect("connection id");
        runtime
            .block_on(backend.ai_list_agents(connection, None))
            .expect("listed")
            .into_iter()
            .map(|option| option.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(listed(&search), ["Elasticsearch"], "no SQL agent");
    assert!(
        !listed(&sqlite).iter().any(|name| name == "Elasticsearch"),
        "never on a SQL connection"
    );
}

#[test]
fn a_question_on_an_elasticsearch_session_runs_its_agent_under_the_tier() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (search_port, _) = runtime.block_on(search_server());
    let (port, request) = runtime.block_on(capturing());
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open_draft(
        &runtime,
        &backend,
        search_draft(DriverId::ELASTICSEARCH, search_port, Environment::Local),
    );
    let provider = local_provider(&runtime, &backend, port);
    let (channel, received) = recording();

    runtime
        .block_on(backend.ai_ask(question(&open, &provider, None), channel))
        .map_err(|error| error.message)
        .expect("the question starts");
    let sent = runtime.block_on(request).expect("the provider was called");
    ended(&runtime, &received);

    assert!(
        sent.contains("Elasticsearch or OpenSearch cluster they have open"),
        "the Elasticsearch agent's role: {sent}"
    );
    // The language's own fragment, and proposals tagged in its name.
    assert!(
        sent.contains("## Elasticsearch and OpenSearch Search DSL"),
        "{sent}"
    );
    assert!(sent.contains("tagged `search-dsl`"), "{sent}");
    assert!(!sent.contains("The dialect"), "no SQL fragment: {sent}");
    assert!(
        !sent.contains("You help a data professional write and fix queries"),
        "never the SQL agent: {sent}"
    );
    // The context came through the one gateway, under the connection's tier
    // (I-04): names and types, never a document value.
    assert!(sent.contains("privacy tier `metadata`"), "{sent}");
    assert!(sent.contains("query language: search-dsl"), "{sent}");
    assert!(sent.contains("server: Elasticsearch 9.1.0"), "{sent}");
    assert!(sent.contains(r#"index \"orders\""#), "{sent}");
    assert!(
        !sent.contains(ROW_VALUE),
        "no row value at `metadata`: {sent}"
    );
    assert!(!sent.contains("{{"), "every variable is filled: {sent}");
}

#[test]
fn an_agent_write_on_a_production_cluster_is_refused_and_a_search_runs_as_a_read() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (port, received) = runtime.block_on(search_server());
    let driver = ProductionCluster {
        metadata: DriverMetadata::new(
            DriverId::new(PRODUCTION_CLUSTER).expect("a valid driver name"),
            "Production cluster",
            DriverFamily::Search,
        ),
        port,
    };
    let backend = Backend::open_temporary_with_driver(Arc::new(driver)).expect("backend");
    let draft = ConnectionDraft {
        values: BTreeMap::new(),
        ..search_draft(PRODUCTION_CLUSTER, port, Environment::Production)
    };
    let production = open_draft(&runtime, &backend, draft);
    let (sink, _thread, actor, _) = sink_on(&backend, &production);

    for write in [
        "DELETE /orders",
        "POST /orders/_delete_by_query\n{\"query\":{\"match_all\":{}}}",
        "POST /_future_api\n{}",
    ] {
        let outcome = runtime.block_on(sink.dispatch(
            &CallHandle::new(CallId::fresh()),
            actor,
            agent_request(&backend, &production, write),
            &CancelToken::new(),
        ));
        assert!(
            matches!(outcome, DispatchOutcome::Denied { .. }),
            "{write}: {outcome:?}"
        );
    }
    let before = received.lock().len();
    assert!(
        received.lock().iter().all(|line| line.starts_with("GET ")),
        "no write reached the cluster: {:?}",
        received.lock()
    );

    // `POST /_search` reads: it runs at once, even for an agent on production.
    let outcome = runtime.block_on(sink.dispatch(
        &CallHandle::new(CallId::fresh()),
        actor,
        agent_request(&backend, &production, "POST /orders/_search\n{\"size\":1}"),
        &CancelToken::new(),
    ));
    assert!(
        !matches!(outcome, DispatchOutcome::Denied { .. }),
        "{outcome:?}"
    );
    for _ in 0..200 {
        if received.lock().len() > before {
            break;
        }
        runtime.block_on(tokio::time::sleep(Duration::from_millis(10)));
    }
    assert!(
        received
            .lock()
            .iter()
            .any(|line| line == "POST /orders/_search"),
        "the search reached the cluster: {:?}",
        received.lock()
    );
}
