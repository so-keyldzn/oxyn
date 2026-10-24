//! Driver metadata and the connection entry point.
//!
//! The product is never guessed from the URL: `GET /` says it. OpenSearch
//! answers `version.distribution: "opensearch"`; Elasticsearch has no
//! distribution and its `tagline`. What a session can do is then read from
//! the cluster, not assumed from the product (ADR-0003).

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};
use reqwest::Method;
use serde_json::Value;

use crate::catalog::{ElasticsearchCatalog, Product};
use crate::http::{Call, Http};
use crate::options::{AuthMethod, ConnectSpec, TlsMode};
use crate::session::ElasticsearchSession;
use crate::stop::tasks_of;

/// The Elasticsearch REST driver, also serving OpenSearch.
#[derive(Debug, Clone)]
pub struct ElasticsearchDriver {
    metadata: DriverMetadata,
}

impl ElasticsearchDriver {
    /// Builds the driver and its generated connection form.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: elasticsearch_metadata(),
        }
    }
}

impl Default for ElasticsearchDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// The form shared by Elasticsearch and OpenSearch.
#[must_use]
pub fn elasticsearch_metadata() -> DriverMetadata {
    DriverMetadata::new(
        DriverId::elasticsearch(),
        "Elasticsearch",
        DriverFamily::Search,
    )
    .with_alias("OpenSearch")
    .with_fields([
        ConnectionField::new("url", "URL", FieldKind::Text)
            .required()
            .with_default("https://localhost:9200")
            .with_help(
                "One node or load balancer of the cluster, with a path prefix if a proxy \
                 serves it under one. `http://` is accepted only on a Local connection.",
            ),
        ConnectionField::new(
            "sslmode",
            "TLS mode",
            FieldKind::Choice(
                TlsMode::ALL
                    .iter()
                    .map(|mode| mode.name().to_owned())
                    .collect(),
            ),
        )
        .with_default(TlsMode::VerifyFull.name())
        .with_help(
            "verify-full checks the certificate and host name. require encrypts without \
             checking them, on a Local connection only.",
        ),
        ConnectionField::new(
            "authentication",
            "Authentication",
            FieldKind::Choice(
                AuthMethod::ALL
                    .iter()
                    .map(|method| method.name().to_owned())
                    .collect(),
            ),
        )
        .with_default(AuthMethod::None.name()),
        ConnectionField::new("username", "Username", FieldKind::Text)
            .with_help("Used with basic authentication."),
        ConnectionField::new("password", "Password", FieldKind::Password)
            .with_help("Used with basic authentication. Stored in the system keyring."),
        ConnectionField::new("token", "API key or bearer token", FieldKind::Password)
            .with_help("The encoded API key, or the bearer token. Stored in the system keyring."),
    ])
}

/// What every session of this driver can do, whatever the cluster.
const BASE: Capabilities = Capabilities::SEARCH_DSL
    .union(Capabilities::DOCUMENT)
    .union(Capabilities::SCHEMALESS)
    .union(Capabilities::TABLES)
    .union(Capabilities::VIEWS)
    .union(Capabilities::STREAMING)
    .union(Capabilities::ROW_COUNT_ESTIMATE)
    .union(Capabilities::OBJECT_DEFINITION)
    .union(Capabilities::PREVIEW_SORT)
    .union(Capabilities::PREVIEW_FILTER)
    .union(Capabilities::DDL)
    .union(Capabilities::DML);

#[async_trait]
impl Driver for ElasticsearchDriver {
    fn id(&self) -> DriverId {
        DriverId::elasticsearch()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    /// The most a session can declare; a session declares server-side cancel
    /// only when its account can read the cluster's tasks.
    fn capabilities(&self) -> Capabilities {
        BASE | Capabilities::SERVER_SIDE_CANCEL
    }

    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let spec = ConnectSpec::from_config(&self.metadata, config, credentials)?;
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let http = Arc::new(Http::new(spec)?);
        let root = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(OxynError::Cancelled),
            root = http.send(Call::read(Method::GET, "/")) => root?.value()?,
        };
        let product = product(&root)?;
        // Cancelling needs the task list: an account without `monitor` cannot
        // read it, and its session does not claim a stop it cannot prove.
        let can_list_tasks = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(OxynError::Cancelled),
            tasks = tasks_of(&http, "") => tasks.is_some(),
        };
        let capabilities = if can_list_tasks {
            BASE | Capabilities::SERVER_SIDE_CANCEL
        } else {
            BASE
        };
        let catalog = ElasticsearchCatalog::new(Arc::clone(&http), product, capabilities);
        Ok(Box::new(ElasticsearchSession::new(
            http,
            catalog,
            capabilities,
        )))
    }
}

/// The product and version `GET /` reports, refused below the floor.
fn product(root: &Value) -> Result<Product> {
    let version = root
        .pointer("/version/number")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            OxynError::Connection(
                "the server at this URL is not an Elasticsearch or OpenSearch node: `GET /` \
                 reports no version number"
                    .to_owned(),
            )
        })?;
    let name = match root
        .pointer("/version/distribution")
        .and_then(Value::as_str)
    {
        Some("opensearch") => "OpenSearch",
        Some(other) => {
            return Err(OxynError::NotSupported {
                capability: format!(
                    "the `{other}` distribution: this driver speaks Elasticsearch and OpenSearch"
                ),
            });
        }
        None => "Elasticsearch",
    };
    if !supported(name, version) {
        return Err(OxynError::NotSupported {
            capability: format!(
                "{name} {version}: this driver supports Elasticsearch 8 and 9, and OpenSearch \
                 2 and 3"
            ),
        });
    }
    Ok(Product {
        name,
        version: version.to_owned(),
    })
}

fn supported(product: &str, version: &str) -> bool {
    let Some(Ok(major)) = version.split('.').next().map(str::parse::<u16>) else {
        return false;
    };
    match product {
        "Elasticsearch" => (8..=9).contains(&major),
        "OpenSearch" => (2..=3).contains(&major),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use oxyn_driver::DriverRegistry;

    use super::*;

    #[test]
    fn driver_metadata_and_identifier_agree() {
        let driver = ElasticsearchDriver::new();
        driver
            .metadata()
            .check()
            .expect("valid generated connection form");
        assert_eq!(driver.id(), driver.metadata().id);
        let mut registry = DriverRegistry::new();
        registry.register(Arc::new(driver)).expect("registration");
        assert!(registry.contains(&DriverId::elasticsearch()));
    }

    #[test]
    fn the_product_comes_from_the_server_not_the_url() {
        let opensearch =
            serde_json::json!({"version": {"distribution": "opensearch", "number": "3.0.0"}});
        assert_eq!(product(&opensearch).expect("supported").name, "OpenSearch");
        let elasticsearch =
            serde_json::json!({"version": {"number": "9.1.0", "build_flavor": "default"}});
        assert_eq!(
            product(&elasticsearch).expect("supported").name,
            "Elasticsearch"
        );
        assert!(product(&serde_json::json!({"name": "a web server"})).is_err());
        assert!(
            product(&serde_json::json!({"version": {"distribution": "other", "number": "1"}}))
                .is_err()
        );
    }

    #[test]
    fn support_floors_are_explicit() {
        assert!(supported("Elasticsearch", "8.0.0"));
        assert!(supported("Elasticsearch", "9.5.5"));
        assert!(!supported("Elasticsearch", "7.17.0"));
        assert!(!supported("Elasticsearch", "10.0.0"));
        assert!(supported("OpenSearch", "2.0.0"));
        assert!(supported("OpenSearch", "3.0.0"));
        assert!(!supported("OpenSearch", "1.3.0"));
        assert!(!supported("OpenSearch", "version unknown"));
    }
}
