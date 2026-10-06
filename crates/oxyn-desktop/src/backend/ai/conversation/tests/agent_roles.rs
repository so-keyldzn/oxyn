//! A conversation runs one agent, rendered for the destination that answers
//! ([ADR-0049](../../../../../../../docs/adr/0049-agents-declared-as-markdown-files.md)).

use oxyn_ai::{AgentCatalog, UserAgentFile, parse_agent_file, schema_agent, sql_agent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

/// A provider on the loopback that reads one request whole, hands it over,
/// and answers `400`: what the provider was sent is the subject, not what it
/// says back.
async fn capturing() -> (u16, tokio::sync::oneshot::Receiver<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port is free");
    let port = listener.local_addr().expect("an address").port();
    let (sent, received) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut read = Vec::new();
        let mut buffer = [0_u8; 8192];
        while let Ok(n) = socket.read(&mut buffer).await {
            if n == 0 {
                break;
            }
            read.extend_from_slice(buffer.get(..n).unwrap_or_default());
            let text = String::from_utf8_lossy(&read).into_owned();
            let Some(end) = text.find("\r\n\r\n") else {
                continue;
            };
            let expected = text
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if read.len() >= end + 4 + expected {
                break;
            }
        }
        let _ = sent.send(String::from_utf8_lossy(&read).into_owned());
        let _ = socket
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            )
            .await;
        let _ = socket.shutdown().await;
    });
    (port, received)
}

/// Declares an OpenAI-compatible provider on `port`.
fn local_provider(runtime: &tokio::runtime::Runtime, backend: &Backend, port: u16) -> String {
    runtime
        .block_on(
            backend.save_ai_provider(
                serde_json::from_value(serde_json::json!({
                    "kind": "openai_compatible",
                    "label": "Local model",
                    "baseUrl": format!("http://127.0.0.1:{port}/v1"),
                    "model": "a-model",
                }))
                .expect("a valid draft"),
            ),
        )
        .expect("declared")
        .id
}

fn question(open: &OpenConnection, provider: &str, agent: Option<String>) -> AskRequest {
    AskRequest {
        mentions: Vec::new(),
        connection: open.connection.clone(),
        session: open.session.clone(),
        thread: None,
        parent: None,
        question: "how many clients?".to_owned(),
        agent_id: agent,
        destination: DestinationChoice::Provider {
            id: provider.to_owned(),
            model: None,
            effort: None,
        },
        sample: None,
    }
}

/// Waits, bounded, for the run of the conversation to end.
fn ended(runtime: &tokio::runtime::Runtime, received: &Arc<Mutex<Vec<String>>>) {
    for _ in 0..400 {
        let events = received.lock().join("\n");
        if events.contains(r#""kind":"failed""#) || events.contains(r#""kind":"finished""#) {
            return;
        }
        runtime.block_on(tokio::time::sleep(std::time::Duration::from_millis(25)));
    }
    panic!("the run never ended: {}", received.lock().join("\n"));
}

/// A user agent, valid, offered only to `recipients`.
fn user_agent(id: &str, name: &str, recipients: &str) -> UserAgentFile {
    let file = format!("{name}.md");
    let text = format!(
        "---\nid: {id}\nname: {name}\nrecipients: [{recipients}]\ntools: [describe_schema]\n---\nYou are the {name} test agent.\n"
    );
    UserAgentFile {
        result: parse_agent_file(&file, &text),
        file_name: file,
    }
}

const ANTHROPIC_ONLY: &str = "0199a3c0-0000-7000-8000-0000000000e1";
const LOCAL_ONLY: &str = "0199a3c0-0000-7000-8000-0000000000e2";

fn with_user_agents(backend: &Backend) {
    backend.inner.ai.agents.set(AgentCatalog::new(
        oxyn_ai::shipped_agents(),
        vec![
            user_agent(ANTHROPIC_ONLY, "Hosted", "anthropic"),
            user_agent(LOCAL_ONLY, "Local", "openai_compatible"),
        ],
    ));
}

/// The system message a provider receives is the conversation's agent with
/// the dialect's and the protocol's fragments — never the bare role.
#[test]
fn a_provider_receives_the_agent_rendered_for_its_dialect_and_protocol() {
    let sql = "You help a data professional write and fix queries";
    let schema = "You help a data professional understand the structure";
    for (agent, role_text, absent) in [
        (None, sql, schema),
        (Some(schema_agent().id.to_string()), schema, sql),
    ] {
        let runtime = runtime();
        let _guard = runtime.enter();
        let (port, request) = runtime.block_on(capturing());
        let backend = Backend::open_temporary().expect("temporary backend");
        let open = open(&runtime, &backend, Environment::Local);
        let provider = local_provider(&runtime, &backend, port);
        let (channel, received) = recording();
        runtime
            .block_on(backend.ai_ask(question(&open, &provider, agent), channel))
            .map_err(|error| error.message)
            .expect("the question starts");
        let sent = runtime.block_on(request).expect("the provider was called");
        ended(&runtime, &received);

        assert!(sent.contains(role_text), "the role is sent: {sent}");
        assert!(!sent.contains(absent), "only the chosen role is sent");
        assert!(
            sent.contains("The dialect: SQLite"),
            "the SQLite fragment is sent"
        );
        assert!(
            sent.contains("Oxyn calls you through an OpenAI-compatible endpoint"),
            "the openai_compatible fragment is sent"
        );
        assert!(!sent.contains("{{"), "every variable is filled: {sent}");
    }
}

/// A destination the conversation's agent is not written for refuses the
/// question before anything is sent, and switches no agent.
#[test]
fn a_destination_the_agent_is_not_offered_for_refuses_the_question() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    with_user_agents(&backend);
    let open = open(&runtime, &backend, Environment::Local);
    // Nothing listens there: a question that reached the provider would fail
    // on the connection, not with the refusal below.
    let provider = local_provider(&runtime, &backend, 9);
    let (channel, received) = recording();

    let refused = runtime
        .block_on(backend.ai_ask(
            question(&open, &provider, Some(ANTHROPIC_ONLY.to_owned())),
            channel,
        ))
        .expect_err("refused");

    assert!(
        refused
            .message
            .contains("not written for openai_compatible"),
        "{}",
        refused.message
    );
    assert!(received.lock().is_empty(), "no run started");
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    assert!(
        runtime.block_on(backend.ai_threads(connection)).is_empty(),
        "no conversation was opened"
    );
}

/// The picker lists what is offered for the connection's destinations, and
/// says where each agent is not offered.
#[test]
fn the_list_follows_the_declared_destinations() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    with_user_agents(&backend);
    let open = open(&runtime, &backend, Environment::Local);
    let provider = local_provider(&runtime, &backend, 9);
    let connection: ConnectionId = open.connection.parse().expect("connection id");

    let listed = runtime
        .block_on(backend.ai_list_agents(connection, None))
        .expect("listed");
    let names: Vec<&str> = listed.iter().map(|option| option.name.as_str()).collect();
    assert_eq!(
        names,
        ["SQL", "Schema", "Local"],
        "Hosted has no destination"
    );
    assert!(
        listed
            .iter()
            .all(|option| option.disabled_destinations.is_empty())
    );

    let chosen = DestinationChoice::Provider {
        id: provider,
        model: None,
        effort: None,
    };
    let for_it = runtime
        .block_on(backend.ai_list_agents(connection, Some(&chosen)))
        .expect("listed");
    assert_eq!(for_it, listed);
}

/// Reopened after its agent left the catalog, a conversation runs the SQL
/// agent and names the one it lost.
#[test]
fn a_conversation_whose_agent_is_gone_reopens_with_sql_and_says_so() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (port, request) = runtime.block_on(capturing());
    let backend = Backend::open_temporary().expect("temporary backend");
    with_user_agents(&backend);
    let open = open(&runtime, &backend, Environment::Local);
    let provider = local_provider(&runtime, &backend, port);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let (channel, received) = recording();
    let started = runtime
        .block_on(backend.ai_ask(
            question(&open, &provider, Some(LOCAL_ONLY.to_owned())),
            channel,
        ))
        .map_err(|error| error.message)
        .expect("the question starts");
    let sent = runtime.block_on(request).expect("the provider was called");
    assert!(sent.contains("You are the Local test agent."));
    ended(&runtime, &received);
    let live = runtime.block_on(backend.ai_threads(connection));
    assert_eq!(
        live.first().and_then(|summary| summary.agent_id.as_deref()),
        Some(LOCAL_ONLY)
    );

    // The file is gone, and the window forgot the conversation: what follows
    // is read back from the workspace.
    backend.inner.ai.agents.set(AgentCatalog::shipped_only());
    backend.close_ai_conversation(connection);

    let sql = sql_agent().id.to_string();
    let saved = runtime.block_on(backend.ai_threads(connection));
    let summary = saved.first().expect("the conversation was saved");
    assert_eq!(summary.agent_id.as_deref(), Some(sql.as_str()));
    assert_eq!(
        summary
            .missing_agent
            .as_ref()
            .map(|missing| missing.name.as_str()),
        Some(LOCAL_ONLY)
    );
    let (channel, _) = recording();
    let view = runtime
        .block_on(backend.ai_open_thread(connection, &started.thread, channel))
        .map_err(|error| error.message)
        .expect("reopens");
    assert_eq!(view.agent_id.as_deref(), Some(sql.as_str()));
    assert_eq!(
        view.missing_agent.map(|missing| missing.name),
        Some(LOCAL_ONLY.to_owned())
    );
}

/// An external agent is served the MCP tools of its conversation's agent,
/// and no other (ADR-0030, ADR-0049 § 7).
#[test]
fn an_external_agent_is_served_its_roles_tools_only() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let session: SessionId = open.session.parse().expect("session id");
    let config = backend.config(connection).expect("a connection");
    let actor = Actor::agent(AgentId::new(), AgentSessionId::new());

    let served = |role: &oxyn_ai::AgentSpec| {
        tool_service(&backend.inner, &config, session, role, actor).served()
    };

    let mut narrow = sql_agent();
    narrow.allowed_tools = vec!["describe_schema".to_owned()];
    assert_eq!(served(&narrow), ["describe_schema"]);
    let schema = served(&schema_agent());
    assert!(schema.contains(&"refresh_catalog".to_owned()), "{schema:?}");
    assert!(!schema.contains(&"request_sample".to_owned()), "{schema:?}");
    let sql = served(&sql_agent());
    assert!(sql.contains(&"request_sample".to_owned()), "{sql:?}");
    assert!(!sql.contains(&"refresh_catalog".to_owned()), "{sql:?}");
}
