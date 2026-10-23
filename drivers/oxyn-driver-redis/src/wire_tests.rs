//! The driver against a server that lies: a reply cut mid-stream, bytes that
//! are not RESP, an integer out of range, an error where a value belongs.
//!
//! A real Redis never sends these; a proxy, a broken network or a hostile
//! endpoint does. The fake server below answers the handshake like Redis 8 —
//! `HELLO`, `COMMAND` with `GET` flagged `readonly` and `SET` flagged `write`,
//! `CLIENT UNBLOCK` — and answers every other command with the bytes a test
//! scripts, then closes the connection. These tests need no server and are
//! not ignored.

use std::sync::Arc;

use oxyn_core::{
    CancelToken, ConnectionConfig, Environment, ErrorClass, ExecLimits, ExecRequest, OxynError,
    QueryLanguage,
};
use oxyn_driver::{Credentials, Session};
use tokio::io::{AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;

use crate::RedisSession;
use crate::driver::{driver_id, redis_metadata};
use crate::options::ConnectSpec;
use crate::resp::{Reader, Value};

pub(crate) const HELLO: &[u8] = b"%5\r\n$6\r\nserver\r\n$5\r\nredis\r\n$7\r\nversion\r\n$5\r\n8.0.0\r\n$2\r\nid\r\n:7\r\n$4\r\nmode\r\n$10\r\nstandalone\r\n$4\r\nrole\r\n$6\r\nmaster\r\n";

pub(crate) const COMMAND: &[u8] = b"*2\r\n\
*10\r\n$3\r\nget\r\n:2\r\n~1\r\n+readonly\r\n:1\r\n:1\r\n:1\r\n~1\r\n+@read\r\n*0\r\n*0\r\n*0\r\n\
*10\r\n$3\r\nset\r\n:-3\r\n~1\r\n+write\r\n:1\r\n:1\r\n:1\r\n~1\r\n+@write\r\n*0\r\n*0\r\n*0\r\n";

/// Starts a server that answers the handshake and then `reply` to the first
/// other command, before closing. Returns its port.
async fn lying_server(reply: &'static [u8]) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let port = listener.local_addr().expect("bound").port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut reader = Reader::new(BufReader::new(stream));
                loop {
                    let mut budget = 1 << 20;
                    let Ok(Value::List(_, words)) = reader.value(&mut budget).await else {
                        return;
                    };
                    let name = match words.first() {
                        Some(Value::Bulk(word)) => word.bytes.to_ascii_uppercase(),
                        _ => return,
                    };
                    let (answer, last): (&[u8], bool) = match name.as_slice() {
                        b"HELLO" => (HELLO, false),
                        b"COMMAND" => (COMMAND, false),
                        b"CLIENT" | b"SELECT" => (b":0\r\n", false),
                        _ => (reply, true),
                    };
                    let stream = reader.get_mut().get_mut();
                    if stream.write_all(answer).await.is_err() || last {
                        return;
                    }
                }
            });
        }
    });
    port
}

pub(crate) async fn session_on(port: u16) -> RedisSession {
    let config = ConnectionConfig::new("liar", driver_id())
        .with_environment(Environment::Local)
        .with_param("host", "127.0.0.1")
        .with_param("port", port.to_string())
        .with_param("tls", "disable");
    let spec =
        ConnectSpec::from_config(&redis_metadata(), &config, &Credentials::new()).expect("valid");
    RedisSession::open(Arc::new(spec), false, &CancelToken::new())
        .await
        .expect("the handshake is answered")
}

async fn outcome(port: u16, text: &str) -> Result<usize, OxynError> {
    let session = session_on(port).await;
    let request = ExecRequest::new(QueryLanguage::RedisCommand, text)
        .with_limits(ExecLimits::default().writable());
    let mut cursor = session.execute(request, &CancelToken::new()).await?;
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch().await? {
        rows += batch.num_rows();
    }
    Ok(rows)
}

#[tokio::test]
async fn the_fake_handshake_classifies_like_redis() {
    let session = session_on(lying_server(b"+OK\r\n").await).await;
    let intent =
        |text: &str| session.intent_of(&ExecRequest::new(QueryLanguage::RedisCommand, text));
    assert_eq!(intent("GET k"), Some(oxyn_core::StatementIntent::Read));
    assert_eq!(intent("SET k v"), Some(oxyn_core::StatementIntent::Write));
    assert_eq!(intent("DEL k"), None);
}

#[tokio::test]
async fn a_read_cut_mid_stream_is_a_transient_error_not_a_short_result() {
    let error = outcome(lying_server(b"*3\r\n$1\r\na\r\n$1\r\nb").await, "GET k")
        .await
        .expect_err("cut");
    assert_eq!(error.class(), ErrorClass::Transient, "{error:?}");
}

#[tokio::test]
async fn a_write_cut_mid_stream_has_an_unknown_outcome() {
    let error = outcome(lying_server(b"*3\r\n:1\r\n").await, "SET k v")
        .await
        .expect_err("cut");
    assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn a_command_of_unknown_intent_cut_before_its_reply_has_an_unknown_outcome() {
    let error = outcome(lying_server(b"").await, "DEL k")
        .await
        .expect_err("no reply");
    assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
}

#[tokio::test]
async fn bytes_that_are_not_resp_fail_the_command_without_a_panic() {
    for garbage in [
        b"?what is this\r\n".as_slice(),
        b"HTTP/1.1 400 Bad Request\r\n",
        b":99999999999999999999\r\n",
        b"*2\r\n:1\r\n@nope\r\n",
        b"$-7\r\n",
        b"$3\r\nabcde\r\n",
    ] {
        let leaked: &'static [u8] = Box::leak(garbage.to_vec().into_boxed_slice());
        let error = outcome(lying_server(leaked).await, "GET k")
            .await
            .expect_err("garbage");
        assert!(
            matches!(error, OxynError::Driver { .. }),
            "{:?}: {error:?}",
            String::from_utf8_lossy(garbage)
        );
    }
}

#[tokio::test]
async fn an_error_reply_is_the_servers_error() {
    let error = outcome(
        lying_server(b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n")
            .await,
        "GET k",
    )
    .await
    .expect_err("an error");
    assert_eq!(error.class(), ErrorClass::Permanent);
    assert!(error.to_string().contains("WRONGTYPE"), "{error}");
}

#[tokio::test]
async fn a_null_where_a_value_is_expected_is_a_null_row() {
    assert_eq!(
        outcome(lying_server(b"_\r\n").await, "GET k")
            .await
            .expect("a null"),
        1
    );
    assert_eq!(
        outcome(lying_server(b"*2\r\n_\r\n$-1\r\n").await, "GET k")
            .await
            .expect("nulls"),
        2
    );
}

#[tokio::test]
async fn a_server_in_cluster_mode_is_refused_at_connect() {
    const CLUSTER: &[u8] =
        b"%3\r\n$6\r\nserver\r\n$5\r\nredis\r\n$2\r\nid\r\n:1\r\n$4\r\nmode\r\n$7\r\ncluster\r\n";
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let port = listener.local_addr().expect("bound").port();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buffer = [0_u8; 256];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buffer).await;
            let _ = stream.write_all(CLUSTER).await;
        }
    });
    let config = ConnectionConfig::new("cluster", driver_id())
        .with_environment(Environment::Local)
        .with_param("host", "127.0.0.1")
        .with_param("port", port.to_string())
        .with_param("tls", "disable");
    let spec =
        ConnectSpec::from_config(&redis_metadata(), &config, &Credentials::new()).expect("valid");
    let error = RedisSession::open(Arc::new(spec), false, &CancelToken::new())
        .await
        .expect_err("a cluster is refused");
    assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
}

/// A cursor read to its end gives the console's connection back at once: the
/// session's next command does not wait for the caller to drop it.
#[tokio::test]
async fn a_drained_cursor_does_not_hold_the_console() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let port = listener.local_addr().expect("bound").port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut reader = Reader::new(BufReader::new(stream));
                loop {
                    let mut budget = 1 << 20;
                    let Ok(Value::List(_, words)) = reader.value(&mut budget).await else {
                        return;
                    };
                    let answer: &[u8] = match words.first() {
                        Some(Value::Bulk(word)) if word.bytes.eq_ignore_ascii_case(b"HELLO") => {
                            HELLO
                        }
                        Some(Value::Bulk(word)) if word.bytes.eq_ignore_ascii_case(b"COMMAND") => {
                            COMMAND
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
    let session = session_on(port).await;
    let request = |text: &str| {
        ExecRequest::new(QueryLanguage::RedisCommand, text)
            .with_limits(ExecLimits::default().writable())
    };
    let mut first = session
        .execute(request("GET a"), &CancelToken::new())
        .await
        .expect("runs");
    while first.next_batch().await.expect("a batch").is_some() {}
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        session.execute(request("GET b"), &CancelToken::new()),
    )
    .await;
    assert!(
        matches!(second, Ok(Ok(_))),
        "the drained cursor still held the console"
    );
    drop(first);
}

/// An executor deadline or a closed tab drops `next_batch` in the middle of a
/// reply. The console's next command must read its own reply, on a fresh
/// connection — never the rest of the abandoned one.
#[tokio::test]
async fn a_reply_abandoned_mid_read_never_answers_the_next_command() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let port = listener.local_addr().expect("bound").port();
    let commands = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&commands);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut reader = Reader::new(BufReader::new(stream));
                loop {
                    let mut budget = 1 << 20;
                    let Ok(Value::List(_, words)) = reader.value(&mut budget).await else {
                        return;
                    };
                    let name = match words.first() {
                        Some(Value::Bulk(word)) => word.bytes.to_ascii_uppercase(),
                        _ => return,
                    };
                    let answer: &[u8] = match name.as_slice() {
                        b"HELLO" => HELLO,
                        b"COMMAND" => COMMAND,
                        b"CLIENT" | b"SELECT" => b":0\r\n",
                        // The first user command gets two of its three
                        // elements, then nothing; every later one, its own.
                        _ if seen.fetch_add(1, Ordering::SeqCst) == 0 => {
                            b"*3\r\n$5\r\nstale\r\n$5\r\nstale\r\n"
                        }
                        _ => b"+FRESH\r\n",
                    };
                    if reader.get_mut().get_mut().write_all(answer).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    let session = session_on(port).await;
    let request = |text: &str| {
        ExecRequest::new(QueryLanguage::RedisCommand, text)
            .with_limits(ExecLimits::default().writable())
    };
    let mut cursor = session
        .execute(request("GET big"), &CancelToken::new())
        .await
        .expect("the reply started");
    let abandoned =
        tokio::time::timeout(std::time::Duration::from_millis(200), cursor.next_batch()).await;
    assert!(abandoned.is_err(), "the third element never comes");
    drop(cursor);

    let mut next = session
        .execute(request("GET k"), &CancelToken::new())
        .await
        .expect("the next command runs");
    let batch = next
        .next_batch()
        .await
        .expect("its own reply")
        .expect("one row");
    let value = arrow::util::display::array_value_to_string(batch.column(0), 0).expect("text");
    assert_eq!(value, "FRESH");
}
