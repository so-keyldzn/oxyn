//! Driver metadata and the connection entry point.

use async_trait::async_trait;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, Result, StatementIntent};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};

use crate::api::Api;
use crate::session::ElasticsearchSession;

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
            .with_help("One Elasticsearch or OpenSearch HTTP endpoint."),
        ConnectionField::new(
            "authentication",
            "Authentication",
            FieldKind::Choice(vec![
                "none".into(),
                "basic".into(),
                "api-key".into(),
                "bearer".into(),
            ]),
        )
        .with_default("none"),
        ConnectionField::new("username", "Username", FieldKind::Text)
            .with_help("Used only with basic authentication."),
        ConnectionField::new("password", "Password", FieldKind::Password)
            .with_help("Stored in the system keyring for basic authentication."),
        ConnectionField::new("token", "API key or bearer token", FieldKind::Password)
            .with_help("Stored in the system keyring for API-key or bearer authentication."),
    ])
}

#[async_trait]
impl Driver for ElasticsearchDriver {
    fn id(&self) -> DriverId {
        DriverId::elasticsearch()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::SEARCH_DSL
            | Capabilities::DOCUMENT
            | Capabilities::SCHEMALESS
            | Capabilities::INFERRED_SCHEMA
            | Capabilities::TABLES
            | Capabilities::INDEXES
            | Capabilities::STREAMING
            | Capabilities::SERVER_SIDE_CANCEL
            | Capabilities::DDL
            | Capabilities::DML
    }

    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        self.metadata.validate(config)?;
        if cancel.is_cancelled() {
            return Err(oxyn_core::OxynError::Cancelled);
        }
        let api = Api::from_config(config, credentials)?;
        let root = api
            .json("GET", "/", None, StatementIntent::Read, None)
            .await?;
        let version = root
            .pointer("/version/number")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                oxyn_core::OxynError::Connection(
                    "the server did not return a version number".to_owned(),
                )
            })?;
        let distribution = root
            .pointer("/version/distribution")
            .and_then(serde_json::Value::as_str);
        let product = match distribution {
            Some("opensearch") => "OpenSearch",
            _ if root
                .get("tagline")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|tagline| tagline.contains("OpenSearch")) =>
            {
                "OpenSearch"
            }
            _ => "Elasticsearch",
        };
        if !supported_version(product, version) {
            return Err(oxyn_core::OxynError::NotSupported {
                capability: format!(
                    "{product} {version}: this driver supports Elasticsearch 8.19 and 9.x, \
                     or OpenSearch 2.10 and 3.x"
                ),
            });
        }
        let capabilities = self.capabilities() & !Capabilities::SERVER_SIDE_CANCEL;
        Ok(Box::new(ElasticsearchSession::new(
            api,
            product.to_owned(),
            version.to_owned(),
            capabilities,
        )))
    }
}

fn supported_version(product: &str, version: &str) -> bool {
    let mut numbers = version.split('.').map(str::parse::<u16>);
    let (Some(Ok(major)), Some(Ok(minor))) = (numbers.next(), numbers.next()) else {
        return false;
    };
    match product {
        "Elasticsearch" => major > 8 || (major == 8 && minor >= 19),
        "OpenSearch" => major > 2 || (major == 2 && minor >= 10),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
    fn support_floors_are_explicit_and_not_guessed_from_a_url() {
        assert!(supported_version("Elasticsearch", "8.19.0"));
        assert!(supported_version("Elasticsearch", "9.5.5"));
        assert!(!supported_version("Elasticsearch", "8.18.0"));
        assert!(supported_version("OpenSearch", "2.10.0"));
        assert!(supported_version("OpenSearch", "3.0.0"));
        assert!(!supported_version("OpenSearch", "2.9.0"));
        assert!(!supported_version("OpenSearch", "version unknown"));
    }
}
