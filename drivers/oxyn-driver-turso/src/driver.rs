//! The driver: what it says about itself, and how it opens a session.
//!
//! # Opening, in one request
//!
//! 1. the configuration becomes a [`ConnectSpec`], which accepts only an
//!    `https://` or `libsql://` URL and has no talkative `Debug`;
//! 2. `SELECT sqlite_version()` proves the URL and the token at once — a
//!    wrong token is `Authentication`, a wrong URL `Config` — and says what the
//!    server is.
//!
//! No state is opened on the server: a Hrana stream lasts one statement.

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result, StatementIntent,
};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};

use crate::http::Http;
use crate::options::{ConnectSpec, TOKEN_FIELD, URL_FIELD};
use crate::session::TursoSession;
use crate::variant::capabilities;
use crate::wire::{self, Cell};

/// What the server is. A literal.
const SQL_PROBE: &str = "SELECT sqlite_version()";

/// The Turso Cloud driver.
#[derive(Debug, Clone)]
pub struct TursoDriver {
    metadata: DriverMetadata,
}

impl TursoDriver {
    /// Builds the driver and its connection form.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: turso_metadata(),
        }
    }
}

impl Default for TursoDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Turso's connection form: the database URL and the database token.
///
/// The token is the database's (`turso db tokens create <db>`), **not** the
/// Platform API token, which administers the organization and opens no
/// database. It has no default: it would be written in clear in the binary.
#[must_use]
pub fn turso_metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::turso(), "Turso Cloud", DriverFamily::Relational)
        .with_alias("libSQL")
        .with_fields([
            ConnectionField::new(URL_FIELD, "Database URL", FieldKind::Text)
                .required()
                .with_help(
                    "As `turso db show <db> --url` prints it: libsql://<db>-<org>.turso.io \
                     (https:// is accepted too).",
                ),
            ConnectionField::new(TOKEN_FIELD, "Database token", FieldKind::Password)
                .required()
                .with_help(
                    "From `turso db tokens create <db>` — not the Platform API token. \
                     Stored in the system keyring.",
                ),
        ])
}

#[async_trait]
impl Driver for TursoDriver {
    fn id(&self) -> DriverId {
        DriverId::turso()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    /// The ceiling: every session declares the same.
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }

    /// Proves the URL and the token, and reads the engine's version.
    ///
    /// # Errors
    /// [`OxynError::Config`] for an incomplete configuration, a URL that is
    /// not `https://` or `libsql://`, or a URL no database answers at;
    /// [`OxynError::Authentication`] if Turso refuses the token;
    /// [`OxynError::Connection`] if it is unreachable;
    /// [`OxynError::Cancelled`] if `cancel` fires.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let spec = ConnectSpec::from_config(&self.metadata, config, credentials)?;
        let http = Arc::new(Http::new(spec)?);
        open(http, self.id(), cancel).await
    }
}

/// Opens a session on a client: the probe, then the session.
pub(crate) async fn open(
    http: Arc<Http>,
    driver: DriverId,
    cancel: &CancelToken,
) -> Result<Box<dyn Session>> {
    let body = wire::request_body(SQL_PROBE, &[])?;
    let answer = http
        .run(
            &driver,
            StatementIntent::Read,
            body,
            Some(std::time::Duration::from_secs(20)),
            cancel,
        )
        .await?;
    let version = answer
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|cell| match cell {
            // The server's text is logged and kept for the session's life: a
            // version is a few printable characters, nothing else is one.
            Cell::Text(version)
                if version.len() <= 64 && version.chars().all(|c| c.is_ascii_graphic()) =>
            {
                Some(version.clone())
            }
            _ => None,
        })
        .ok_or_else(|| OxynError::Connection("the server did not say its version".to_owned()))?;
    tracing::debug!(
        target: "oxyn::driver::turso",
        sqlite_version = %version,
        "session opened"
    );
    Ok(Box::new(TursoSession::new(http, driver, version)))
}

#[cfg(test)]
mod tests {
    use oxyn_driver::DriverRegistry;

    use super::*;

    #[test]
    fn the_driver_declares_itself_consistently() {
        let driver = TursoDriver::new();
        assert_eq!(driver.id(), driver.metadata().id);
        driver.metadata().check().expect("consistent form");
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::new(TursoDriver::new()))
            .expect("registration");
        assert!(registry.contains(&DriverId::turso()));
    }

    #[test]
    fn the_form_is_the_url_and_the_database_token() {
        let metadata = turso_metadata();
        assert_eq!(metadata.id.as_str(), "turso");
        assert_eq!(metadata.display_name, "Turso Cloud");
        assert_eq!(metadata.family, DriverFamily::Relational);
        assert_eq!(metadata.default_port, None);
        let fields: Vec<(&str, &FieldKind, bool, Option<&str>)> = metadata
            .connection_fields
            .iter()
            .map(|field| {
                (
                    field.key.as_str(),
                    &field.kind,
                    field.required,
                    field.default.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            fields,
            [
                ("url", &FieldKind::Text, true, None),
                ("token", &FieldKind::Password, true, None),
            ]
        );
    }

    #[test]
    fn the_token_is_the_only_secret_and_has_no_default() {
        let metadata = turso_metadata();
        let secrets: Vec<&str> = metadata
            .secret_fields()
            .map(|field| field.key.as_str())
            .collect();
        assert_eq!(secrets, ["token"]);
    }
}
