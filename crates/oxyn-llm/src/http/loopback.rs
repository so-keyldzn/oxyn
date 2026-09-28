//! A test HTTP server on the loopback.
//!
//! Enough to replay what a hostile provider can do — redirect, answer a body
//! that never ends, stream frames —, and nothing more: each connection
//! receives the raw response it was given, and what was received remains
//! available.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// What a connection received: headers and body, as text.
pub(crate) type Received = Arc<Mutex<Vec<String>>>;

/// A server that answers `reply` to every connection.
///
/// As long as `hold` is not released, the connection stays open after the
/// response: that is what simulates a body that never ends.
pub(crate) struct Server {
    /// `http://127.0.0.1:<port>`, without a trailing slash.
    pub(crate) origin: String,
    /// Everything received, one entry per request.
    pub(crate) received: Received,
    /// Released when the server is dropped: the held connections close
    /// then.
    _release: oneshot::Sender<()>,
}

impl Server {
    /// Starts a server that answers `reply`, then closes — or holds the
    /// connection if `hold` is true.
    pub(crate) async fn start(reply: impl Into<Vec<u8>>, hold: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port is free");
        let origin = format!("http://{}", listener.local_addr().expect("a bound address"));
        let received: Received = Arc::new(Mutex::new(Vec::new()));
        let (release, released) = oneshot::channel::<()>();
        let released = futures::future::FutureExt::shared(released);
        let reply = reply.into();
        let recorded = Arc::clone(&received);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let reply = reply.clone();
                let recorded = Arc::clone(&recorded);
                let released = released.clone();
                tokio::spawn(async move {
                    let request = read_request(&mut socket).await;
                    recorded.lock().expect("test lock").push(request);
                    let _ = socket.write_all(&reply).await;
                    let _ = socket.flush().await;
                    if hold {
                        let _ = released.await;
                    }
                });
            }
        });
        Self {
            origin,
            received,
            _release: release,
        }
    }

    /// Every request received, as a single text.
    pub(crate) fn seen(&self) -> String {
        self.received.lock().expect("test lock").join("\n")
    }

    /// Number of requests received.
    pub(crate) fn hits(&self) -> usize {
        self.received.lock().expect("test lock").len()
    }
}

/// Reads the headers, then the body announced by `content-length`.
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut read = Vec::new();
    let mut buffer = [0_u8; 4096];
    while let Ok(n) = socket.read(&mut buffer).await {
        if n == 0 {
            break;
        }
        read.extend_from_slice(buffer.get(..n).unwrap_or_default());
        let text = String::from_utf8_lossy(&read).into_owned();
        if let Some(end) = text.find("\r\n\r\n") {
            let expected = text
                .lines()
                .find_map(|line| {
                    let (header_name, value) = line.split_once(':')?;
                    header_name
                        .eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if read.len() >= end + 4 + expected {
                break;
            }
        }
    }
    String::from_utf8_lossy(&read).into_owned()
}

/// A redirection response to `location`.
pub(crate) fn redirect(status: u16, location: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} Redirect\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    )
    .into_bytes()
}
