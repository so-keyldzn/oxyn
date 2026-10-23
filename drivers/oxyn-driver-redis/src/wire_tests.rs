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

const HELLO: &[u8] = b"%5\r\n$6\r\nserver\r\n$5\r\nredis\r\n$7\r\nversion\r\n$5\r\n8.0.0\r\n$2\r\nid\r\n:7\r\n$4\r\nmode\r\n$10\r\nstandalone\r\n$4\r\nrole\r\n$6\r\nmaster\r\n";

const COMMAND: &[u8] = b"*2\r\n\
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

async fn session_on(port: u16) -> RedisSession {
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
