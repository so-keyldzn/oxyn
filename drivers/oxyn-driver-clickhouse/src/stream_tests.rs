//! An execution against a hostile server: what it answers is never shown
//! when it could quote a secret the statement gave it.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use oxyn_core::{CancelToken, DriverId, OxynError, StatementIntent};
use tokio::sync::{mpsc, oneshot};

use super::*;
use crate::error::ClickhouseError;
use crate::http::Call;
use crate::http::tests::{canned, http_to};

/// The password a statement carries, which the hostile server quotes back.
const SECRET: &str = "oxyn-hostile-pw";

/// A syntax error quoting the whole statement, password included.
const QUOTING: &str = "HTTP/1.1 500 Internal Server Error\r\n\
    X-ClickHouse-Query-Id: hostile\r\n\
    X-ClickHouse-Exception-Code: 62\r\n\
    Content-Length: 128\r\n\r\n\
    Code: 62. DB::Exception: Syntax error: failed at position 1 \
    (CREATE USER u IDENTIFIED BY 'oxyn-hostile-pw' oops). (SYNTAX_ERROR)";

/// What the execution of `text` opens with against `port`.
async fn opening(port: u16, text: &str) -> OxynError {
    let execution = Execution {
        http: Arc::new(http_to(port)),
        driver: DriverId::clickhouse(),
        call: Call {
            text: text.to_owned(),
            query_id: "hostile".to_owned(),
            ..Call::default()
        },
        intent: StatementIntent::Ddl,
        can_kill: false,
        json_as_text: true,
        max_rows: None,
        truncated: Arc::new(AtomicBool::new(false)),
    };
    let (opened_tx, opened_rx) = oneshot::channel();
    let (batches_tx, _batches_rx) = mpsc::channel(1);
    run(execution, CancelToken::new(), opened_tx, batches_tx).await;
    match opened_rx.await.expect("the task answers") {
        Ok(_) => panic!("the server's refusal was taken for a success"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn a_server_quoting_a_password_is_not_shown() {
    assert_eq!(QUOTING.split("\r\n\r\n").nth(1).map(str::len), Some(128));
    let (port, _) = canned(QUOTING).await;
    for text in [
        "CREATE USER u IDENTIFIED BY 'oxyn-hostile-pw' oops",
        "ALTER USER u IDENTIFIED WITH sha256_password BY 'oxyn-hostile-pw'",
        "CREATE USER u IDENTIFIED WITH plaintext_password BY 'oxyn-hostile-pw'",
    ] {
        let error = opening(port, text).await;
        for shown in [error.to_string(), format!("{error:?}")] {
            assert!(!shown.contains(SECRET), "{text}: {shown}");
            assert!(shown.contains("withheld"), "{text}: {shown}");
        }
        let OxynError::Driver { source, .. } = &error else {
            panic!("a driver error: {error:?}");
        };
        let source = source
            .downcast_ref::<ClickhouseError>()
            .expect("the driver's own error type");
        assert_eq!(source.code(), Some(62), "the code is kept");
    }
}

#[tokio::test]
async fn a_statement_without_secret_keeps_the_servers_words() {
    let (port, _) = canned(QUOTING).await;
    let error = opening(port, "CREATE TABLE t (a UInt8) ENGINE = Memory oops").await;
    assert!(error.to_string().contains("Syntax error"), "{error}");
}
