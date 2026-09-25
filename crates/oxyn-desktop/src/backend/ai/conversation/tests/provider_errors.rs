//! What a provider says after its `200` reaches the webview without the key.
//!
//! I-03 counts six channels; this is the displayed-error one, followed to its
//! end: the streamed error becomes the run's failure, and the failure is an
//! [`AiUpdate`] the webview renders.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

/// A made-up key: finding it in what leaves proves the leak, and it is valid
/// nowhere.
const SENTINEL: &str = "sk-sentinel-7c1a-must-not-leak";

/// A provider on the loopback that answers `reply` to one request, read in
/// full first.
async fn served_once(reply: String) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port is free");
    let port = listener.local_addr().expect("an address").port();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut read = Vec::new();
        let mut buffer = [0_u8; 4096];
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
        let _ = socket.write_all(reply.as_bytes()).await;
        let _ = socket.shutdown().await;
    });
    port
}

/// A `200` whose only SSE frame is an error repeating the key.
fn streamed_error(frame: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{frame}",
        frame.len()
    )
}

/// Asks a question of a provider of `kind` that answers `reply`, declared
/// with the sentinel key at `path` on its port, and returns everything the
/// webview received.
fn asked(kind: &str, path: &str, reply: String) -> String {
    let runtime = runtime();
    let _guard = runtime.enter();
    let port = runtime.block_on(served_once(reply));
    let base_url = format!("http://127.0.0.1:{port}{path}");
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let provider = runtime
        .block_on(
            backend.save_ai_provider(
                serde_json::from_value(serde_json::json!({
                    "kind": kind,
                    "label": "Echoing gateway",
                    "baseUrl": base_url,
                    "model": "a-model",
                    "key": SENTINEL,
                }))
                .expect("a valid draft"),
            ),
        )
        .expect("declared")
        .id;
    let (channel, received) = recording();
    runtime
        .block_on(backend.ai_ask(
            AskRequest {
                mentions: Vec::new(),
                connection: open.connection.clone(),
                session: open.session.clone(),
                thread: None,
                parent: None,
                question: "how many clients?".to_owned(),
                destination: DestinationChoice::Provider {
                    id: provider,
                    model: None,
                    effort: None,
                },
                sample: None,
            },
            channel,
        ))
        .map_err(|error| error.message)
        .expect("the question starts");
    // The run ends on its own once the one frame is read: this waits for
    // that, bounded, and never for a fixed time.
    for _ in 0..400 {
        let events = received.lock().join("\n");
        if events.contains(r#""kind":"failed""#) || events.contains(r#""kind":"finished""#) {
            return events;
        }
        runtime.block_on(tokio::time::sleep(std::time::Duration::from_millis(25)));
    }
    panic!("the run never ended: {}", received.lock().join("\n"));
}

/// The failure is there, and says what happened without the key.
fn assert_redacted(events: &str) {
    assert!(events.contains(r#""kind":"failed""#), "{events}");
    assert!(!events.contains(SENTINEL), "{events}");
    assert!(events.contains("redacted API key"), "{events}");
}

#[test]
fn an_openai_compatible_streamed_error_reaches_the_webview_without_the_key() {
    let frame = format!("data: {{\"error\":{{\"message\":\"invalid key {SENTINEL}\"}}}}\n\n");
    assert_redacted(&asked("openai_compatible", "/v1", streamed_error(&frame)));
}

#[test]
fn an_anthropic_streamed_error_reaches_the_webview_without_the_key() {
    let frame = format!(
        "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"api_error\",\"message\":\"echo {SENTINEL}\"}}}}\n\n"
    );
    assert_redacted(&asked("anthropic", "", streamed_error(&frame)));
}
