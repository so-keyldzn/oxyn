//! A local server that speaks Hrana over HTTP, for the tests.
//!
//! No Turso account and no secret: a `TcpListener` on the loopback reads
//! `POST /v2/pipeline` requests and answers what a script says. Nothing in the
//! tests waits on the wall clock — they wait on **events** the server raises
//! (a request arrived, the client hung up), and the test harness's own timeout
//! catches a hang.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

/// The token a test server accepts.
pub(crate) const TOKEN: &str = "eyJ-test-token-0123456789";

/// A request the server received.
#[derive(Debug, Clone)]
pub(crate) struct Recorded {
    pub(crate) path: String,
    pub(crate) authorization: Option<String>,
    pub(crate) body: Value,
}

impl Recorded {
    /// The statement of the pipeline's `execute`.
    pub(crate) fn sql(&self) -> &str {
        self.body["requests"][0]["stmt"]["sql"]
            .as_str()
            .unwrap_or_default()
    }

    /// The typed arguments of the pipeline's `execute`.
    pub(crate) fn args(&self) -> &Value {
        &self.body["requests"][0]["stmt"]["args"]
    }
}

/// What the server answers.
#[derive(Debug, Clone)]
pub(crate) enum Reply {
    /// A status and a JSON body.
    Json(u16, String),
    /// A status and raw bytes.
    Raw(u16, Vec<u8>),
    /// Deliberately split UTF-8 and JSON across HTTP chunks.
    Chunks(Vec<Vec<u8>>),
    /// Never answers; waits for the client to hang up.
    Hang,
    /// A 200 with no length, then `total` bytes as fast as the client reads.
    Flood(usize),
    /// A 200 that announces a long body, sends a few bytes and hangs up.
    Truncated,
    /// Generate a cursor without constructing its complete response in memory.
    Cursor {
        rows: usize,
        value_bytes: usize,
        before_header: Option<Arc<Notify>>,
    },
}

type Script = dyn Fn(&Recorded) -> Reply + Send + Sync;

#[derive(Default)]
struct Shared {
    requests: Mutex<Vec<Recorded>>,
    arrived: Notify,
    hung_up: Notify,
    hung_up_count: AtomicUsize,
    flooded: AtomicUsize,
}

/// A running test server.
pub(crate) struct Server {
    port: u16,
    shared: Arc<Shared>,
}

impl Server {
    /// Starts a server that answers every request with `script`.
    pub(crate) async fn start(script: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local port");
        let port = listener.local_addr().expect("bound").port();
        let shared = Arc::new(Shared::default());
        let script: Arc<Script> = Arc::new(script);
        let state = Arc::clone(&shared);
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(serve(socket, Arc::clone(&state), Arc::clone(&script)));
            }
        });
        Self { port, shared }
    }

    /// A server that checks the token like Turso and answers `database`.
    pub(crate) async fn database(
        database: impl Fn(&Recorded) -> Reply + Send + Sync + 'static,
    ) -> Self {
        Self::start(move |request| {
            if request.authorization.as_deref() != Some(&format!("Bearer {TOKEN}")) {
                return Reply::Json(401, json!({"error": "Unauthorized"}).to_string());
            }
            if request.path == "/v3" {
                return Reply::Raw(404, Vec::new());
            }
            database(request)
        })
        .await
    }

    pub(crate) fn base(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    /// Every request so far.
    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.shared
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Resolves once `count` requests arrived.
    pub(crate) async fn requests_seen(&self, count: usize) {
        loop {
            let notified = self.shared.arrived.notified();
            if self.requests().len() >= count {
                return;
            }
            notified.await;
        }
    }

    /// Resolves once `count` clients hung up on a [`Reply::Hang`].
    pub(crate) async fn hung_up(&self, count: usize) {
        loop {
            let notified = self.shared.hung_up.notified();
            if self.shared.hung_up_count.load(Ordering::SeqCst) >= count {
                return;
            }
            notified.await;
        }
    }

    /// How many bytes a [`Reply::Flood`] managed to write before it stopped.
    pub(crate) fn flooded(&self) -> usize {
        self.shared.flooded.load(Ordering::SeqCst)
    }
}

async fn serve(mut socket: TcpStream, shared: Arc<Shared>, script: Arc<Script>) {
    let Some(request) = read_request(&mut socket).await else {
        return;
    };
    shared
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request.clone());
    shared.arrived.notify_waiters();
    match script(&request) {
        Reply::Cursor {
            rows,
            value_bytes,
            before_header,
        } => {
            let head =
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            if let Some(gate) = before_header {
                gate.notified().await;
            }
            let start = concat!(
                "{\"baton\":\"cursor-private-identity\",\"base_url\":null}\n",
                "{\"type\":\"step_begin\",\"step\":0,\"cols\":[{\"name\":\"id\",\"decltype\":\"INTEGER\"},{\"name\":\"label\",\"decltype\":\"TEXT\"}]}\n"
            );
            if socket.write_all(start.as_bytes()).await.is_err() {
                return;
            }
            let payload = "x".repeat(value_bytes);
            for index in 0..rows {
                let mut line = serde_json::to_vec(&json!({"type":"row", "row":[
                    {"type":"integer", "value":index.to_string()}, {"type":"text", "value":payload}
                ]}))
                .expect("fixture JSON");
                line.push(b'\n');
                if socket.write_all(&line).await.is_err() {
                    return;
                }
            }
            let _ = socket.write_all(b"{\"type\":\"step_end\",\"affected_row_count\":0,\"last_insert_rowid\":null}\n").await;
            let _ = socket.shutdown().await;
        }
        Reply::Json(status, body) => {
            write_response(&mut socket, status, "application/json", body.as_bytes()).await;
        }
        Reply::Chunks(chunks) => {
            if socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .is_err()
            {
                return;
            }
            for chunk in chunks {
                let head = format!("{:X}\r\n", chunk.len());
                if socket.write_all(head.as_bytes()).await.is_err()
                    || socket.write_all(&chunk).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
        Reply::Raw(status, body) => {
            write_response(&mut socket, status, "text/html", &body).await;
        }
        Reply::Hang => {
            let mut sink = [0_u8; 64];
            // The request is fully read: any later byte or the EOF is the
            // client going away.
            while matches!(socket.read(&mut sink).await, Ok(n) if n > 0) {}
            shared.hung_up_count.fetch_add(1, Ordering::SeqCst);
            shared.hung_up.notify_waiters();
        }
        Reply::Truncated => {
            let head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                        Content-Length: 1000000\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(b"{\"results\": [{\"type\": ").await;
            // Dropping the socket closes it short of the announced length.
        }
        Reply::Flood(total) => {
            let head =
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            let block = vec![b' '; 64 * 1024];
            let mut written = 0;
            while written < total {
                let n = block.len().min(total - written);
                if socket.write_all(&block[..n]).await.is_err() {
                    break;
                }
                written += n;
                shared.flooded.store(written, Ordering::SeqCst);
            }
        }
    }
}

async fn write_response(socket: &mut TcpStream, status: u16, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} Status\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if socket.write_all(head.as_bytes()).await.is_ok() {
        let _ = socket.write_all(body).await;
    }
    let _ = socket.shutdown().await;
}

async fn read_request(socket: &mut TcpStream) -> Option<Recorded> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let end = loop {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let mut lines = head.lines();
    let path = lines
        .next()?
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let mut length = 0_usize;
    let mut authorization = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().ok()?,
            "authorization" => authorization = Some(value.to_owned()),
            _ => {}
        }
    }
    while buffer.len() < end + length {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
    let body = serde_json::from_slice(&buffer[end..end + length]).unwrap_or(Value::Null);
    Some(Recorded {
        path,
        authorization,
        body,
    })
}

/// A Hrana value.
pub(crate) mod value {
    use serde_json::{Value, json};

    pub(crate) fn int(value: i64) -> Value {
        json!({"type": "integer", "value": value.to_string()})
    }
    pub(crate) fn float(value: f64) -> Value {
        json!({"type": "float", "value": value})
    }
    pub(crate) fn text(value: &str) -> Value {
        json!({"type": "text", "value": value})
    }
    pub(crate) fn blob(base64: &str) -> Value {
        json!({"type": "blob", "base64": base64})
    }
    pub(crate) fn null() -> Value {
        json!({"type": "null"})
    }
}

/// The answer of a statement that returned rows.
pub(crate) fn rows(cols: &[(&str, Option<&str>)], rows: Vec<Vec<Value>>) -> Reply {
    let cols: Vec<Value> = cols
        .iter()
        .map(|(name, decltype)| json!({"name": name, "decltype": decltype}))
        .collect();
    Reply::Json(
        200,
        json!({
            "baton": null,
            "base_url": null,
            "results": [
                {"type": "ok", "response": {"type": "execute", "result": {
                    "cols": cols,
                    "rows": rows,
                    "affected_row_count": 0,
                    "last_insert_rowid": null,
                    "replication_index": "1",
                    "rows_read": 0,
                    "rows_written": 0,
                    "query_duration_ms": 1.5,
                }}},
                {"type": "ok", "response": {"type": "close"}},
            ],
        })
        .to_string(),
    )
}

/// The answer of a write.
pub(crate) fn affected(count: u64) -> Reply {
    Reply::Json(
        200,
        json!({
            "baton": null,
            "base_url": null,
            "results": [
                {"type": "ok", "response": {"type": "execute", "result": {
                    "cols": [], "rows": [], "affected_row_count": count,
                    "last_insert_rowid": "7", "replication_index": "2",
                    "rows_read": 0, "rows_written": count, "query_duration_ms": 0.5,
                }}},
                {"type": "ok", "response": {"type": "close"}},
            ],
        })
        .to_string(),
    )
}

/// The answer of a statement that failed.
pub(crate) fn failure(code: &str, message: &str) -> Reply {
    Reply::Json(
        200,
        json!({
            "baton": null,
            "base_url": null,
            "results": [
                {"type": "error", "error": {"message": message, "code": code}},
                {"type": "ok", "response": {"type": "close"}},
            ],
        })
        .to_string(),
    )
}
