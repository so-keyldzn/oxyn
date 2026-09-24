use std::sync::Mutex;

use async_trait::async_trait;
use oxyn_core::{AgentId, AgentSessionId, Command, ConnectionId, QueryLanguage, SessionId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;
use oxyn_core::CancelToken;

use crate::runtime::{CommandSink, DispatchOutcome};
use crate::tools::{EXECUTE_QUERY, REFRESH_CATALOG};

/// A bus that records whether anything reached it at all.
#[derive(Debug, Default)]
struct Bus {
    seen: Mutex<usize>,
}

impl Bus {
    fn reached(&self) -> usize {
        *self.seen.lock().expect("no panic held the lock")
    }
}

#[async_trait]
impl CommandSink for Bus {
    async fn dispatch(
        &self,
        _actor: oxyn_core::Actor,
        _command: Command,
        _cancel: &CancelToken,
    ) -> DispatchOutcome {
        *self.seen.lock().expect("no panic held the lock") += 1;
        DispatchOutcome::Completed {
            summary: "1 rows, 1 batches".to_owned(),
        }
    }
}

/// An endpoint, its bus, and the task serving it.
struct Open {
    endpoint: Endpoint,
    bus: Arc<Bus>,
    serving: tokio::task::JoinHandle<()>,
    _question: crate::external::mcp::OpenTurn,
}

async fn open() -> Open {
    open_within(Limits::DEFAULT).await
}

async fn open_within(limits: Limits) -> Open {
    let bus = Arc::new(Bus::default());
    let service = Arc::new(ToolService::new(
        crate::tools::ToolRegistry::builtin(),
        vec![EXECUTE_QUERY.to_owned(), REFRESH_CATALOG.to_owned()],
        crate::tools::ToolScope::new(ConnectionId::new(), SessionId::new(), QueryLanguage::SQL),
        crate::external::mcp::TierCell::holding(crate::privacy::PrivacyTier::Metadata),
        oxyn_core::Actor::agent(AgentId::new(), AgentSessionId::new()),
    ));
    // A question in progress for the whole test: these tests are about the
    // socket, and `super::turn` about what a question allows.
    let turns = ToolTurns::new(64, Arc::new(|| false));
    let question = turns.open(
        Arc::clone(&bus) as Arc<dyn CommandSink>,
        Arc::new(()),
        CancelToken::new(),
    );
    let (endpoint, driver) = serve_within(service, turns, limits)
        .await
        .expect("the loopback socket binds");
    let serving = tokio::spawn(driver);
    Open {
        endpoint,
        bus,
        serving,
        _question: question,
    }
}

fn port_of(url: &str) -> u16 {
    url.trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .and_then(|port| port.parse().ok())
        .expect("the url carries a port")
}

/// Sends a raw request and returns the status line plus the body.
///
/// Written by hand rather than with a client crate: a test that needs an HTTP
/// client to check a header is a test that cannot send a malformed one.
async fn send(port: u16, headers: &str, body: &str) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("the endpoint accepts a connection");
    let request = format!(
        "POST /mcp HTTP/1.1\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("the request is written");
    let mut answer = String::new();
    let _ignored = stream.read_to_string(&mut answer).await;

    let status = answer
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = answer
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

const LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;

fn allowed(port: u16, token: &str) -> String {
    format!("Host: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n")
}

#[tokio::test]
async fn the_agent_reaches_the_tools_with_its_token() {
    let open = open().await;
    let port = port_of(open.endpoint.url());
    let (status, body) = send(port, &allowed(port, open.endpoint.token()), LIST).await;

    assert_eq!(status, 200);
    assert!(body.contains("execute_query"), "{body}");
    open.serving.abort();
}

#[tokio::test]
async fn nothing_is_served_without_the_token_not_even_initialize() {
    // An open handshake tells whoever knocks that Oxyn is listening and what it
    // serves. The token is required before the first word.
    let open = open().await;
    let port = port_of(open.endpoint.url());

    let (status, body) = send(port, &format!("Host: 127.0.0.1:{port}\r\n"), INITIALIZE).await;
    assert_eq!(status, 404, "{body}");
    assert!(body.is_empty(), "a refusal says nothing: {body}");

    let (status, _) = send(port, &format!("Host: 127.0.0.1:{port}\r\n"), LIST).await;
    assert_eq!(status, 404);
    open.serving.abort();
}

#[tokio::test]
async fn a_wrong_token_is_refused_exactly_like_a_missing_one() {
    // Distinguishing the two teaches the caller what it nearly got right.
    let open = open().await;
    let port = port_of(open.endpoint.url());

    let wrong = format!(
        "Host: 127.0.0.1:{port}\r\nAuthorization: Bearer {}\r\n",
        "0".repeat(open.endpoint.token().len())
    );
    let (wrong_status, wrong_body) = send(port, &wrong, LIST).await;
    let (missing_status, missing_body) =
        send(port, &format!("Host: 127.0.0.1:{port}\r\n"), LIST).await;

    assert_eq!(wrong_status, missing_status);
    assert_eq!(wrong_body, missing_body);
    assert_eq!(wrong_status, 404);
    open.serving.abort();
}

#[tokio::test]
async fn a_browser_tricked_onto_the_port_is_refused() {
    // DNS rebinding: a page makes a name it controls resolve to 127.0.0.1, then
    // talks to this port. The browser sends the name it believes it reached.
    let open = open().await;
    let port = port_of(open.endpoint.url());
    let token = open.endpoint.token();

    let rebound = format!("Host: evil.example:{port}\r\nAuthorization: Bearer {token}\r\n");
    let (status, _) = send(port, &rebound, LIST).await;
    assert_eq!(status, 404, "a non-loopback Host must be refused");

    let origin = format!(
        "Host: 127.0.0.1:{port}\r\nOrigin: https://evil.example\r\nAuthorization: Bearer {token}\r\n"
    );
    let (status, _) = send(port, &origin, LIST).await;
    assert_eq!(status, 404, "a foreign Origin must be refused");

    // And the request that is genuinely local still works, so the rule above is
    // a rule and not a wall.
    let (status, _) = send(port, &allowed(port, token), LIST).await;
    assert_eq!(status, 200);
    open.serving.abort();
}

#[tokio::test]
async fn a_request_without_a_host_is_refused() {
    let open = open().await;
    let port = port_of(open.endpoint.url());
    let headers = format!("Authorization: Bearer {}\r\n", open.endpoint.token());
    let (status, _) = send(port, &headers, LIST).await;
    assert_eq!(status, 404);
    open.serving.abort();
}

#[tokio::test]
async fn only_one_path_and_one_method_are_served() {
    let open = open().await;
    let port = port_of(open.endpoint.url());
    let token = open.endpoint.token();

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connects");
    let request = format!(
        "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.expect("written");
    let mut answer = String::new();
    let _ignored = stream.read_to_string(&mut answer).await;
    assert!(answer.contains(" 404 "), "{answer}");
    open.serving.abort();
}

#[tokio::test]
async fn a_tool_call_from_the_agent_reaches_the_bus() {
    // The whole point of the endpoint: the agent can finally query the database
    // the user has open — through the bus, as `Actor::Agent`.
    let open = open().await;
    let port = port_of(open.endpoint.url());
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call",
                   "params":{"name":"execute_query",
                             "arguments":{"statement":"SELECT count(*) FROM clients"}}}"#;
    let (status, body) = send(port, &allowed(port, open.endpoint.token()), call).await;

    assert_eq!(status, 200);
    assert_eq!(open.bus.reached(), 1, "the command must cross the bus");
    assert!(body.contains("1 rows"), "{body}");
    open.serving.abort();
}

#[tokio::test]
async fn an_oversized_body_is_refused_rather_than_held() {
    let open = open().await;
    let port = port_of(open.endpoint.url());
    let huge = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"execute_query","arguments":{{"statement":"{}"}}}}}}"#,
        "x".repeat(usize::try_from(MAX_BODY_BYTES).unwrap_or(usize::MAX) + 1)
    );
    let (status, _) = send(port, &allowed(port, open.endpoint.token()), &huge).await;
    assert_eq!(status, 413);
    assert_eq!(open.bus.reached(), 0, "nothing ran from an oversized body");
    open.serving.abort();
}

#[tokio::test]
async fn closing_the_conversation_closes_the_socket() {
    // The endpoint lives exactly as long as the conversation that opened it.
    let open = open().await;
    let port = port_of(open.endpoint.url());
    let Open {
        endpoint, serving, ..
    } = open;
    drop(endpoint);

    // The driver returns on its own once the endpoint is gone.
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(2), serving).await;
    assert!(stopped.is_ok(), "the server should stop with the endpoint");

    let refused = tokio::net::TcpStream::connect(("127.0.0.1", port)).await;
    assert!(refused.is_err(), "the port should no longer accept");
}

/// Reads one whole response from a connection kept open, without waiting for
/// the peer to close it. `None` when the connection ended or said nothing in
/// time.
async fn one_response(stream: &mut tokio::net::TcpStream) -> Option<String> {
    let mut answer = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let text = String::from_utf8_lossy(&answer).into_owned();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if body.len() >= length {
                return Some(text);
            }
        }
        let read =
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.read(&mut buffer)).await;
        match read {
            Ok(Ok(0) | Err(_)) | Err(_) => return None,
            Ok(Ok(count)) => answer.extend_from_slice(&buffer[..count]),
        }
    }
}

/// Did the server close this connection within `limit`?
async fn closed_within(stream: &mut tokio::net::TcpStream, limit: std::time::Duration) -> bool {
    let mut buffer = [0_u8; 64];
    loop {
        match tokio::time::timeout(limit, stream.read(&mut buffer)).await {
            // Still open when the limit passed.
            Err(_) => return false,
            Ok(Ok(0) | Err(_)) => return true,
            // A response (a 408, say) before the close: keep reading.
            Ok(Ok(_)) => {}
        }
    }
}

#[tokio::test]
async fn a_connection_opened_during_the_conversation_dies_with_it() {
    // The regression: stopping the listener refused *new* connections, while
    // one kept alive went on reaching the bus after the conversation ended —
    // for a process that left the agent's group, say.
    let open = open().await;
    let port = port_of(open.endpoint.url());
    let headers = allowed(port, open.endpoint.token());
    let request = format!(
        "POST /mcp HTTP/1.1\r\n{headers}Content-Length: {}\r\n\r\n{LIST}",
        LIST.len()
    );

    let mut kept = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connects");
    kept.write_all(request.as_bytes()).await.expect("written");
    let first = one_response(&mut kept).await.expect("served while open");
    assert!(first.contains(" 200 "), "{first}");

    let Open {
        endpoint, serving, ..
    } = open;
    drop(endpoint);
    tokio::time::timeout(std::time::Duration::from_secs(2), serving)
        .await
        .expect("the server stops with the endpoint")
        .expect("the server task ends cleanly");

    // Same connection, same token: nothing may answer any more.
    let _ignored = kept.write_all(request.as_bytes()).await;
    let after = one_response(&mut kept).await;
    assert!(
        after.is_none(),
        "a kept-alive connection was served after the conversation: {after:?}"
    );
}

#[tokio::test]
async fn a_silent_connection_is_closed_at_the_header_deadline() {
    let open = open_within(Limits {
        headers: std::time::Duration::from_millis(200),
        ..Limits::DEFAULT
    })
    .await;
    let port = port_of(open.endpoint.url());
    let mut silent = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connects");
    assert!(
        closed_within(&mut silent, std::time::Duration::from_secs(3)).await,
        "a connection that sends nothing must not hold a slot forever"
    );
    open.serving.abort();
}

#[tokio::test]
async fn a_body_that_never_arrives_is_given_up() {
    let open = open_within(Limits {
        body: std::time::Duration::from_millis(200),
        ..Limits::DEFAULT
    })
    .await;
    let port = port_of(open.endpoint.url());
    let mut slow = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connects");
    // Authenticated headers, then a body announced and never sent.
    let head = format!(
        "POST /mcp HTTP/1.1\r\n{}Content-Length: 100\r\n\r\n{{",
        allowed(port, open.endpoint.token())
    );
    slow.write_all(head.as_bytes()).await.expect("written");
    assert!(
        closed_within(&mut slow, std::time::Duration::from_secs(3)).await,
        "a body sent too slowly must not hold a slot forever"
    );
    assert_eq!(open.bus.reached(), 0);
    open.serving.abort();
}

#[tokio::test]
async fn connections_beyond_the_bound_are_closed_on_accept() {
    let open = open_within(Limits {
        connections: 2,
        ..Limits::DEFAULT
    })
    .await;
    let port = port_of(open.endpoint.url());
    let connect = || tokio::net::TcpStream::connect(("127.0.0.1", port));

    let first = connect().await.expect("connects");
    let second = connect().await.expect("connects");
    let mut third = connect().await.expect("the kernel accepts it");
    assert!(
        closed_within(&mut third, std::time::Duration::from_secs(2)).await,
        "a third connection must be closed while two are held"
    );

    // A bound, not a wall: once the holders leave, the agent is served again.
    drop((first, second));
    let token = open.endpoint.token().to_owned();
    let mut served = false;
    for _ in 0..50 {
        let (status, _) = send(port, &allowed(port, &token), LIST).await;
        if status == 200 {
            served = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(served, "the slots must free when their connections close");
    open.serving.abort();
}

#[test]
fn the_default_bounds_are_the_argued_ones() {
    // Fixed by a test because they are invisible otherwise: removing the timer
    // or loosening a bound fails nothing else.
    assert_eq!(
        Limits::DEFAULT,
        Limits {
            connections: 16,
            headers: std::time::Duration::from_secs(30),
            body: std::time::Duration::from_secs(30),
        }
    );
}

#[test]
fn the_token_is_not_in_what_gets_printed() {
    // I-03: a token in a `Debug` is a token in a log, a crash report, or a bug
    // report pasted into a chat.
    let rendered = format!(
        "{:?}",
        Endpoint {
            url: "http://127.0.0.1:1/mcp".to_owned(),
            token: "the-secret-token".to_owned(),
            tools: Vec::new(),
            stop: None,
        }
    );
    assert!(!rendered.contains("the-secret-token"), "{rendered}");
}

#[test]
fn the_comparison_reads_every_byte() {
    assert!(constant_time_eq(b"abcdef", b"abcdef"));
    assert!(!constant_time_eq(b"abcdef", b"abcdeg"));
    assert!(!constant_time_eq(b"abcdef", b"abcde"));
    assert!(constant_time_eq(b"", b""));
}

/// A bus that holds a call until it is cancelled, and says whether it was.
#[derive(Default)]
struct Patient {
    cancelled: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl CommandSink for Patient {
    async fn dispatch(
        &self,
        _actor: oxyn_core::Actor,
        _command: Command,
        cancel: &CancelToken,
    ) -> DispatchOutcome {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), cancel.cancelled()).await;
        self.cancelled
            .store(cancel.is_cancelled(), std::sync::atomic::Ordering::SeqCst);
        DispatchOutcome::Completed {
            summary: "0 rows, 0 batches".to_owned(),
        }
    }
}

#[tokio::test]
async fn stopping_the_endpoint_cancels_a_call_before_dropping_it() {
    // An aborted future cancels nothing on the server: a long `SELECT` on
    // production would go on running after the conversation ended.
    let bus = Arc::new(Patient::default());
    let turns = ToolTurns::new(8, Arc::new(|| false));
    let _question = turns.open(
        Arc::clone(&bus) as Arc<dyn CommandSink>,
        Arc::new(()),
        CancelToken::new(),
    );
    let service = Arc::new(ToolService::new(
        crate::tools::ToolRegistry::builtin(),
        vec![EXECUTE_QUERY.to_owned()],
        crate::tools::ToolScope::new(ConnectionId::new(), SessionId::new(), QueryLanguage::SQL),
        crate::external::mcp::TierCell::holding(crate::privacy::PrivacyTier::Metadata),
        oxyn_core::Actor::agent(AgentId::new(), AgentSessionId::new()),
    ));
    let (endpoint, driver) = serve_within(service, turns, Limits::DEFAULT)
        .await
        .expect("the loopback socket binds");
    let serving = tokio::spawn(driver);
    let port = port_of(endpoint.url());
    let headers = allowed(port, endpoint.token());
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call",
                   "params":{"name":"execute_query","arguments":{"statement":"SELECT pg_sleep(600)"}}}"#;
    let request = tokio::spawn(async move { send(port, &headers, call).await });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    drop(endpoint);
    tokio::time::timeout(std::time::Duration::from_secs(4), serving)
        .await
        .expect("the server stops")
        .expect("cleanly");
    assert!(
        bus.cancelled.load(std::sync::atomic::Ordering::SeqCst),
        "the call must see its cancellation before its task is aborted"
    );
    request.abort();
}
