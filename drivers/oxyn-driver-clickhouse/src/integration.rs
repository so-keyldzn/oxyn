//! Tests against a real ClickHouse server.
//!
//! Every test is `#[ignore]`: run with `--ignored` and
//! `OXYN_CLICKHOUSE_TEST_URL=http://user:password@host:port` (see the crate
//! documentation). Without the variable a test returns at once and prints
//! `not set: test skipped` — `make test-serveurs` refuses a CI run where that
//! line appears.

use oxyn_core::{CancelToken, ConnectionConfig, DriverId, Environment};
use oxyn_driver::{Credentials, Driver, Session};
use secrecy::SecretString;
use url::Url;

use crate::ClickhouseDriver;

/// The test server, or `None` when the variable is absent.
fn target() -> Option<(ConnectionConfig, Credentials)> {
    let Ok(raw) = std::env::var("OXYN_CLICKHOUSE_TEST_URL") else {
        eprintln!("OXYN_CLICKHOUSE_TEST_URL is not set: test skipped");
        return None;
    };
    let url = Url::parse(&raw).expect("OXYN_CLICKHOUSE_TEST_URL is a URL");
    let config = ConnectionConfig::new("clickhouse-it", DriverId::clickhouse())
        .with_environment(Environment::Local)
        .with_param("host", url.host_str().expect("the URL names a host"))
        .with_param(
            "port",
            url.port().expect("the URL names a port").to_string(),
        )
        .with_param("user", url.username())
        .with_param("sslmode", "disable");
    let credentials = Credentials::new().with_password(SecretString::from(
        url.password()
            .expect("the URL carries a password")
            .to_owned(),
    ));
    Some((config, credentials))
}

async fn open() -> Option<Box<dyn Session>> {
    let (config, credentials) = target()?;
    Some(
        ClickhouseDriver::new()
            .connect(&config, &credentials, &CancelToken::new())
            .await
            .expect("the test server accepts the connection"),
    )
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn a_session_opens_and_says_what_it_can_do() {
    let Some(session) = open().await else { return };
    assert!(session.capabilities().supports_language(crate::LANGUAGE));
    session.ping().await.expect("ping");
    session.close().await.expect("close");
}
