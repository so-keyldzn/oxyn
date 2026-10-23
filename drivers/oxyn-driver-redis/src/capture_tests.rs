//! Captures against a fake server that keeps the stream open, as a
//! subscribed Redis connection does, and says when the driver closes it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use arrow::array::{Array as _, AsArray as _};
use oxyn_core::{CancelToken, ErrorClass, ExecLimits, ExecRequest, OxynError, QueryLanguage};
use oxyn_driver::Session as _;
use tokio::io::{AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;

use crate::resp::{Reader, Value};
use crate::wire_tests::{COMMAND, HELLO, session_on};

/// A server that answers the handshake, sends `stream` to the first other
/// command, then holds the connection until the client closes it — and sets
/// `closed` then.
async fn streaming_server(stream: &'static [u8], closed: Arc<AtomicBool>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let port = listener.local_addr().expect("bound").port();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let closed = Arc::clone(&closed);
            tokio::spawn(async move {
                let mut reader = Reader::new(BufReader::new(socket));
                let mut streaming = false;
                loop {
                    let mut budget = 1 << 20;
                    let Ok(Value::List(_, words)) = reader.value(&mut budget).await else {
                        // Only the capture's connection ever streams.
                        if streaming {
                            closed.store(true, Ordering::SeqCst);
                        }
                        return;
                    };
                    let answer: &[u8] = match words.first() {
                        Some(Value::Bulk(word)) if word.bytes.eq_ignore_ascii_case(b"HELLO") => {
                            HELLO
                        }
                        Some(Value::Bulk(word)) if word.bytes.eq_ignore_ascii_case(b"COMMAND") => {
                            COMMAND
                        }
                        Some(Value::Bulk(word))
                            if word.bytes.eq_ignore_ascii_case(b"SUBSCRIBE")
                                || word.bytes.eq_ignore_ascii_case(b"MONITOR") =>
                        {
                            streaming = true;
                            stream
                        }
                        _ => b":0\r\n",
                    };
                    if reader.get_mut().get_mut().write_all(answer).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    port
}

const MESSAGES: &[u8] = b">3\r\n$9\r\nsubscribe\r\n$4\r\nnews\r\n:1\r\n\
>3\r\n$7\r\nmessage\r\n$4\r\nnews\r\n$5\r\nhello\r\n\
>3\r\n$7\r\nmessage\r\n$4\r\nnews\r\n$5\r\nworld\r\n";

fn request(text: &str, limits: ExecLimits) -> ExecRequest {
    ExecRequest::new(QueryLanguage::RedisCommand, text).with_limits(limits.writable())
}

async fn drain(
    session: &crate::RedisSession,
    request: ExecRequest,
    cancel: &CancelToken,
) -> Result<Vec<Option<String>>, OxynError> {
    let mut cursor = session.execute(request, cancel).await?;
    let mut values = Vec::new();
    while let Some(batch) = cursor.next_batch().await? {
        let column = batch
            .column_by_name("value")
            .expect("a value column")
            .as_string::<i32>();
        values.extend(
            (0..column.len()).map(|i| column.is_valid(i).then(|| column.value(i).to_owned())),
        );
    }
    Ok(values)
}

async fn closes(flag: &AtomicBool) -> bool {
    for _ in 0..100 {
        if flag.load(Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn a_subscription_ends_at_its_row_limit_and_closes_its_connection() {
    let closed = Arc::new(AtomicBool::new(false));
    let session = session_on(streaming_server(MESSAGES, Arc::clone(&closed)).await).await;
    let limits = ExecLimits::default().with_max_rows(2);
    let values = drain(
        &session,
        request("SUBSCRIBE news", limits),
        &CancelToken::new(),
    )
    .await
    .expect("a capture");
    assert_eq!(values, [Some("1".to_owned()), Some("hello".to_owned())]);
    assert!(closes(&closed).await, "the capture's connection is closed");
    // The console was never the capture's: it still answers.
    let after = session
        .execute(request("GET k", ExecLimits::default()), &CancelToken::new())
        .await;
    assert!(after.is_ok(), "{:?}", after.err());
}

#[tokio::test]
async fn a_subscription_ends_with_its_window_before_the_timeout() {
    let closed = Arc::new(AtomicBool::new(false));
    let session = session_on(streaming_server(MESSAGES, Arc::clone(&closed)).await).await;
    let limits = ExecLimits::default().with_timeout(Duration::from_secs(3));
    let started = Instant::now();
    let values = drain(
        &session,
        request("SUBSCRIBE news", limits),
        &CancelToken::new(),
    )
    .await
    .expect("a capture ends as a result");
    let took = started.elapsed();
    assert_eq!(values.len(), 3);
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(3),
        "{took:?}"
    );
    assert!(closes(&closed).await, "the capture's connection is closed");
}

#[tokio::test]
async fn a_stopped_capture_is_cancelled_and_closes_its_connection() {
    let closed = Arc::new(AtomicBool::new(false));
    let session = session_on(streaming_server(MESSAGES, Arc::clone(&closed)).await).await;
    let token = CancelToken::new();
    let stopping = async {
        tokio::time::sleep(Duration::from_millis(400)).await;
        token.cancel();
    };
    let (ended, ()) = futures::future::join(
        drain(
            &session,
            request("SUBSCRIBE news", ExecLimits::default()),
            &token,
        ),
        stopping,
    )
    .await;
    assert!(
        matches!(ended, Err(OxynError::Cancelled)),
        "a capture applies nothing: {ended:?}"
    );
    assert!(closes(&closed).await, "the capture's connection is closed");
}

#[tokio::test]
async fn a_refused_subscription_is_the_servers_error() {
    let closed = Arc::new(AtomicBool::new(false));
    let session = session_on(
        streaming_server(
            b"-NOPERM User default has no permissions to access the 'news' channel\r\n",
            Arc::clone(&closed),
        )
        .await,
    )
    .await;
    let error = drain(
        &session,
        request("SUBSCRIBE news", ExecLimits::default()),
        &CancelToken::new(),
    )
    .await
    .expect_err("refused");
    assert_eq!(error.class(), ErrorClass::Permanent, "{error:?}");
    assert!(error.to_string().contains("NOPERM"), "{error}");
}

#[tokio::test]
async fn monitor_lines_become_rows_and_its_ok_does_not() {
    let closed = Arc::new(AtomicBool::new(false));
    let session = session_on(
        streaming_server(
            b"+OK\r\n+1792773499.957991 [0 127.0.0.1:40342] \"PING\"\r\n",
            Arc::clone(&closed),
        )
        .await,
    )
    .await;
    let limits = ExecLimits::default().with_max_rows(1);
    let mut cursor = session
        .execute(request("MONITOR", limits), &CancelToken::new())
        .await
        .expect("a capture");
    let batch = cursor
        .next_batch()
        .await
        .expect("a batch")
        .expect("one row");
    assert_eq!(batch.num_rows(), 1);
    let command = batch
        .column_by_name("command")
        .expect("a command column")
        .as_string::<i32>();
    assert_eq!(command.value(0), "\"PING\"");
    assert!(cursor.next_batch().await.expect("the end").is_none());
    assert!(cursor.stats().truncated, "more may have come");
}
