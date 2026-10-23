//! The initial real-server probes for Elasticsearch.
//!
//! They are ignored because a local edit must not require a server. CI supplies
//! `OXYN_ELASTICSEARCH_TEST_URL`; the server target rejects its skip output.

use oxyn_catalog::CatalogPath;
use oxyn_core::{CancelToken, ConnectionConfig, DriverId, Environment};
use oxyn_driver::{Credentials, Driver as _};

use crate::ElasticsearchDriver;

const VARIABLE: &str = "OXYN_ELASTICSEARCH_TEST_URL";

fn config() -> Option<ConnectionConfig> {
    let url = match std::env::var(VARIABLE) {
        Ok(url) => url,
        Err(_) => {
            eprintln!("{VARIABLE} is not set: test skipped");
            return None;
        }
    };
    Some(
        ConnectionConfig::new("Elasticsearch integration", DriverId::elasticsearch())
            .with_environment(Environment::Local)
            .with_param("url", url),
    )
}

#[tokio::test]
#[ignore = "requires OXYN_ELASTICSEARCH_TEST_URL"]
async fn connects_and_lists_the_flat_index_catalog() {
    let Some(config) = config() else {
        return;
    };
    let driver = ElasticsearchDriver::new();
    let session = driver
        .connect(&config, &Credentials::new(), &CancelToken::new())
        .await
        .expect("connect to the disposable Elasticsearch server");
    let info = session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .expect("server information");
    assert_eq!(info.product, "Elasticsearch");
    let relations = session
        .catalog()
        .list_relations(&CatalogPath::empty(), &CancelToken::new())
        .await
        .expect("flat index catalog");
    assert!(
        relations.is_empty(),
        "a disposable server has no user indices"
    );
}
