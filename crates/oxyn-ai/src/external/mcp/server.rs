//! The loopback endpoint that serves Oxyn's tools to one external agent.
//!
//! # Why a socket at all
//!
//! Because the agents refuse the alternative. The protocol can carry MCP inside
//! the ACP connection, with no socket and no port — and neither Claude Agent
//! 0.78.0 nor Codex 1.12.0 announces that capability; Codex answers `"acp":
//! false`. The measurement is dated in
//! [RESEARCH-NOTES](../../../../../docs/RESEARCH-NOTES.md), and the choice is
//! argued in
//! [ADR-0030](../../../../../docs/adr/0030-outils-oxyn-exposes-a-un-agent-externe.md).
//!
//! # What guards it
//!
//! A listening socket on a shared machine is reachable by every process on it,
//! and — through a browser — by pages the user did not open deliberately. Four
//! rules, each defeating a specific attack rather than a feeling:
//!
//! * **bound to `127.0.0.1`**, never `0.0.0.0`: nothing off the machine can
//!   reach it, whatever the firewall does;
//! * **a bearer token on every request, `initialize` included.** An open
//!   handshake tells whoever knocks that Oxyn is listening and what it serves;
//! * **`Host` and `Origin` restricted to the loopback.** A page can make a name
//!   it controls resolve to `127.0.0.1` and then talk to this port — *DNS
//!   rebinding*. The browser sends the name it believes it reached, so
//!   demanding a literal loopback `Host` ends it;
//! * **one refusal for every cause.** A message that separates « wrong token »
//!   from « wrong origin » teaches the caller what it nearly got right.
//!
//! Two more, for what a local program can do **without** the token:
//!
//! * **bounded before authentication.** A few connections at once, a deadline
//!   on the headers and on the body. Without them any program of the user's
//!   opens thousands of silent connections and exhausts Oxyn's file
//!   descriptors — then no database connects and no draft is saved;
//! * **every accepted connection dies with the conversation**, not only the
//!   listener. A connection kept alive would otherwise go on reaching the bus
//!   after the user closed the conversation, for a process that escaped the
//!   agent's group.
//!
//! The token lives in memory, for the length of one conversation. It is never
//! written to the workspace, never logged, and never put in an error shown to
//! the user ([I-03](../../../../../CLAUDE.md#i-03)).

use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::header::{HOST, HeaderMap, ORIGIN};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::{SharedService, ToolService, ToolTurns};

/// The path the agent posts to. One path, one method: nothing else is served.
const PATH: &str = "/mcp";

/// The largest JSON-RPC message accepted, in bytes.
///
/// A tool call is a statement and a name. Anything larger is either a mistake
/// or an attempt to make Oxyn hold a body it will never use.
const MAX_BODY_BYTES: u64 = 256 * 1024;

/// How long a stopping endpoint lets a cancelled call wind down.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// Closes the open question when the serving future is dropped whole.
///
/// That happens only if nobody stopped the endpoint first — the normal end goes
/// through `stopped` and waits (`session::live`). Here the calls' tokens are
/// cancelled, then their tasks aborted right after, without waiting: the
/// executor cancels on the database server once its `drain` returns, so an
/// aborted call reaches the server only through the executor's abandon guard.
// TODO(2026-09-30, unblocked by the executor's abandon guard, now in oxyn-exec):
// prove end to end that a call aborted here cancels its query on the server.
struct CloseOnDrop(ToolTurns);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close_now();
    }
}

/// What a peer may hold before it has proved anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Connections served at once. Beyond, a new one is closed on accept.
    pub(crate) connections: usize,
    /// From the moment a connection waits for a request to the end of its
    /// headers. It also bounds an idle kept-alive connection, which waits the
    /// same way (`hyper` 1.11.1, `proto/h1/conn.rs`, `poll_read_head`).
    pub(crate) headers: Duration,
    /// To receive a whole body once the headers are in.
    pub(crate) body: Duration,
}

impl Limits {
    /// One agent needs a handful of connections: its HTTP client pools a few,
    /// and calls rarely overlap. Thirty seconds is longer than the idle timeout
    /// of the clients measured (undici keeps a socket 4 s by default), so the
    /// client closes first and never posts onto a socket we are closing.
    ///
    /// A local program can still take the sixteen slots and starve **this**
    /// agent; it can no longer starve Oxyn. That trade is deliberate.
    pub(crate) const DEFAULT: Self = Self {
        connections: 16,
        headers: Duration::from_secs(30),
        body: Duration::from_secs(30),
    };
}

/// Where an agent reaches Oxyn's tools, and with what.
///
/// Dropping it stops the listener: the endpoint lives exactly as long as the
/// conversation that opened it.
pub struct Endpoint {
    url: String,
    token: String,
    /// The tools announced on it, so the session can tell their calls apart
    /// in what the agent streams.
    tools: Vec<String>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Written by hand, and it must stay so: `#[derive(Debug)]` on a type holding a
/// secret is the `tracing::debug!("{endpoint:?}")` added six months later
/// ([I-03](../../../../../CLAUDE.md#i-03)).
impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("url", &self.url)
            .field("open", &self.stop.is_some())
            .finish_non_exhaustive()
    }
}

impl Endpoint {
    /// The URL to hand the agent in its session declaration.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The bearer token to hand the agent, once.
    ///
    /// Held by value and never rendered: the caller puts it in a header and
    /// forgets it. It is deliberately **not** in this type's `Debug`.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The names of the tools served, as the agent's `tools/list` gives them.
    #[must_use]
    pub fn tools(&self) -> &[String] {
        &self.tools
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ignored = stop.send(());
        }
    }
}

/// Everything one conversation's calls need, shared by every request.
struct Context {
    service: SharedService,
    /// The question a call runs within — never the one that opened the
    /// endpoint.
    turns: ToolTurns,
    token: String,
    port: u16,
    body_deadline: Duration,
}

/// Opens the endpoint. The returned future serves it until the endpoint drops.
///
/// # Errors
/// Any failure to bind the loopback socket. There is no fallback: serving on
/// another interface is not a lesser version of this, it is a different and
/// unacceptable thing.
pub async fn serve(
    service: Arc<ToolService>,
    turns: ToolTurns,
) -> std::io::Result<(Endpoint, futures::future::BoxFuture<'static, ()>)> {
    serve_within(service, turns, Limits::DEFAULT).await
}

/// [`serve`], with the bounds stated rather than defaulted, so a test can make
/// a deadline expire in milliseconds.
pub(crate) async fn serve_within(
    service: Arc<ToolService>,
    turns: ToolTurns,
    limits: Limits,
) -> std::io::Result<(Endpoint, futures::future::BoxFuture<'static, ()>)> {
    // Port 0: the operating system picks a free one. A fixed port would be
    // guessable and would collide between two windows.
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
    let port = listener.local_addr()?.port();

    // Two v4 UUIDs: 244 bits from the same source the rest of the workspace
    // trusts for identifiers, rather than a new generator for one string.
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let tools = service.served();

    let context = Arc::new(Context {
        service,
        turns,
        token: token.clone(),
        port,
        body_deadline: limits.body,
    });

    let slots = Arc::new(Semaphore::new(limits.connections));
    let driver = Box::pin(async move {
        let mut stopped = stopped;
        // Every connection's task is owned here, and nowhere else: when this
        // future ends, the set drops and aborts every connection still open. A
        // detached `tokio::spawn` would keep serving the token.
        let mut connections = JoinSet::new();
        // Declared **after** the set, so it drops **before** it: locals drop in
        // reverse order. When this future is dropped whole rather than stopped,
        // the open question's calls are cancelled, then their tasks aborted at
        // once — there is no waiting in a `Drop`. Their cancellation reaches the
        // database server only through the executor's own abandon guard.
        let _closing = CloseOnDrop(context.turns.clone());
        loop {
            let accepted = tokio::select! {
                // The endpoint was dropped: the conversation is over, and the
                // socket goes with it — the open connections too.
                _ = &mut stopped => {
                    // Cancel first, so a call at the executor reaches the
                    // server with its cancellation; abort only what is left.
                    // An aborted future cancels nothing on the server.
                    context.turns.close(STOP_GRACE).await;
                    connections.abort_all();
                    return;
                }
                // Reap finished connections, so the set does not grow with
                // every request the agent ever made.
                Some(_) = connections.join_next(), if !connections.is_empty() => continue,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, peer)) = accepted else {
                // A failed accept is not a reason to stop serving the ones that
                // follow; a failing listener ends with the loop above.
                continue;
            };
            // Belt and braces: bound to the loopback already, refused again
            // here. A rule that holds in one place only holds until someone
            // edits that place.
            if !peer.ip().is_loopback() {
                continue;
            }
            // Full: closed at once, before a byte is read. Holding it open
            // "until a slot frees" is the exhaustion the bound exists to stop.
            let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
                continue;
            };
            let context = Arc::clone(&context);
            connections.spawn(async move {
                let _slot = slot;
                let serving = http1::Builder::new()
                    // Without a timer, `hyper` silently ignores every deadline
                    // (1.11.1, `common/time.rs`, `check`): the header deadline
                    // below would be a comment, not a bound.
                    .timer(TokioTimer::new())
                    .header_read_timeout(limits.headers)
                    .serve_connection(
                        TokioIo::new(stream),
                        service_fn(move |request| {
                            let context = Arc::clone(&context);
                            async move { Ok::<_, Infallible>(answer(request, context).await) }
                        }),
                    );
                let _ignored = serving.await;
            });
        }
    });

    Ok((
        Endpoint {
            url: format!("http://127.0.0.1:{port}{PATH}"),
            token,
            tools,
            stop: Some(stop),
        },
        driver,
    ))
}

/// One HTTP request, from a peer that has proved nothing yet.
async fn answer(
    request: Request<hyper::body::Incoming>,
    context: Arc<Context>,
) -> Response<Full<Bytes>> {
    if request.method() != Method::POST || request.uri().path() != PATH {
        return refused();
    }
    if !loopback_only(request.headers(), context.port) {
        return refused();
    }
    if !authorised(request.headers(), &context.token) {
        return refused();
    }

    let body = request.into_body();
    // A body sent one byte a minute holds a slot as surely as a silent socket.
    let Ok(read) = tokio::time::timeout(context.body_deadline, limited(body)).await else {
        return status(StatusCode::REQUEST_TIMEOUT);
    };
    let Ok(collected) = read else {
        return status(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let Ok(message) = String::from_utf8(collected.to_vec()) else {
        return status(StatusCode::BAD_REQUEST);
    };

    match context.service.respond(&message, &context.turns).await {
        Some(reply) => json(StatusCode::OK, reply),
        // A notification: accepted, nothing to say back.
        None => status(StatusCode::ACCEPTED),
    }
}

/// Reads at most [`MAX_BODY_BYTES`], refusing rather than growing.
async fn limited(body: hyper::body::Incoming) -> Result<Bytes, ()> {
    use http_body_util::Limited;
    Limited::new(body, usize::try_from(MAX_BODY_BYTES).unwrap_or(usize::MAX))
        .collect()
        .await
        .map(|collected| collected.to_bytes())
        .map_err(|_| ())
}

/// Is this request from the loopback, by its own account?
///
/// `Host` must be a **literal** loopback address with our port: a browser
/// tricked into talking to this socket sends the name it thinks it reached, and
/// a name is not `127.0.0.1`. `Origin`, when present, must match too — an MCP
/// client sends none, so its presence is already a sign.
fn loopback_only(headers: &HeaderMap, port: u16) -> bool {
    let expected = [format!("127.0.0.1:{port}"), format!("[::1]:{port}")];
    let host = headers.get(HOST).and_then(|value| value.to_str().ok());
    let Some(host) = host else {
        // HTTP/1.1 requires `Host`. Its absence is malformed, not permissive.
        return false;
    };
    if !expected.iter().any(|allowed| allowed == host) {
        return false;
    }
    match headers.get(ORIGIN).and_then(|value| value.to_str().ok()) {
        None => true,
        Some(origin) => expected
            .iter()
            .any(|allowed| origin == format!("http://{allowed}")),
    }
}

/// Does the request carry the exact token?
///
/// Compared in constant time: a comparison that stops at the first differing
/// byte is measurable, and a token that can be measured can be guessed one byte
/// at a time.
fn authorised(headers: &HeaderMap, token: &str) -> bool {
    let offered = headers
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let Some(offered) = offered else {
        return false;
    };
    constant_time_eq(offered.as_bytes(), token.as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    // The length is not secret — the token's is fixed — but the bytes are, so
    // every one of them is read whatever the first says.
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// The single answer to every rejected request.
///
/// Same status, same empty body, whatever the cause: a caller must not learn
/// which of the token, the origin or the path it got right.
fn refused() -> Response<Full<Bytes>> {
    status(StatusCode::NOT_FOUND)
}

fn status(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

fn json(status: StatusCode, body: String) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    response
}

#[cfg(test)]
mod tests;
