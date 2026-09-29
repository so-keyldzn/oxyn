//! The driver: what it says about itself, and how it opens a session.
//!
//! # One driver per protocol
//!
//! This crate is the one for **everything that speaks the PostgreSQL
//! protocol**: PostgreSQL, Amazon Redshift, TimescaleDB, pgvector, Citus. There
//! is no `oxyn-driver-redshift` crate, and there will not be one: the difference
//! between these products is a set of capabilities, not one more protocol
//! decoder ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! # The connection in three steps
//!
//! 1. the configuration becomes a [`ConnectSpec`], which refuses any persisted
//!    secret and has no talkative `Debug`;
//! 2. the pool opens, racing against cancellation — a TLS handshake to an
//!    unreachable host lasts a minute, and `Esc` must cut it short;
//! 3. the variant is **detected**: `version()` then `pg_extension`. That is
//!    where, and only where, the session's capabilities are fixed.
//!
//! If detection fails, the connection fails. Opening a session without knowing
//! what it can do would mean declaring capabilities by guesswork — and a wrong
//! capability brings up a surface that does not work.

use async_trait::async_trait;
use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result, StatementIntent,
};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::{ConnectOptions as _, Connection as _, Row as _};

use crate::error::{map_connect_error, map_exec_error};
use crate::options::{
    ACQUIRE_TIMEOUT, ConnectSpec, DEFAULT_APPLICATION_NAME, DEFAULT_PORT, MAX_CONNECTIONS,
};
use crate::session::{PostgresSession, race_cancel};
use crate::variant::{PostgresVariant, driver_capabilities};

/// Server identity and current database. A literal, nothing composed.
const SQL_IDENTITY: &str = "SELECT version(), current_database()";
/// The extensions installed in the current database.
///
/// `::text` because `extname` is of type `name`: the cast avoids having to
/// decode a type whose OID not all compatible dialects share.
const SQL_EXTENSIONS: &str = "SELECT extname::text FROM pg_catalog.pg_extension ORDER BY extname";

/// The PostgreSQL driver.
#[derive(Debug, Clone)]
pub struct PostgresDriver {
    metadata: DriverMetadata,
}

impl PostgresDriver {
    /// Builds the driver and its connection form.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: postgres_metadata(),
        }
    }
}

impl Default for PostgresDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// PostgreSQL's connection form.
///
/// No driver codes its own screen: it describes its fields, the interface
/// renders them. That is what guarantees that a password field stays a password
/// field in all fourteen drivers, and that it never ends up in a workspace
/// file.
///
/// The `password` field **has no default value**: it would be written in clear
/// in the binary, and `DriverMetadata::check` would refuse it.
#[must_use]
pub fn postgres_metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
        .with_default_port(DEFAULT_PORT)
        .with_fields([
            ConnectionField::new("host", "Host", FieldKind::Text)
                .required()
                .with_default("localhost")
                .with_help("Server host name or address."),
            ConnectionField::new("port", "Port", FieldKind::Number)
                .with_default(DEFAULT_PORT.to_string()),
            ConnectionField::new("database", "Database", FieldKind::Text)
                .required()
                .with_default("postgres")
                .with_help("A connection sees one database: PostgreSQL does not allow cross-database introspection."),
            ConnectionField::new("user", "User", FieldKind::Text).required(),
            ConnectionField::new("password", "Password", FieldKind::Password)
                .with_help("Kept in the system keyring, never in the workspace."),
            ConnectionField::new(
                "sslmode",
                "TLS mode",
                FieldKind::Choice(vec![
                    "disable".to_owned(),
                    "allow".to_owned(),
                    "prefer".to_owned(),
                    "require".to_owned(),
                    "verify-ca".to_owned(),
                    "verify-full".to_owned(),
                ]),
            )
            .with_default("prefer")
            .with_help("`verify-full` is the only mode that authenticates the server."),
            ConnectionField::new("application_name", "Application name", FieldKind::Text)
                .with_default(DEFAULT_APPLICATION_NAME)
                .with_help("Shown in `pg_stat_activity` on the server."),
        ])
}

#[async_trait]
impl Driver for PostgresDriver {
    fn id(&self) -> DriverId {
        DriverId::postgres()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    /// The ceiling of what the driver can offer.
    ///
    /// Indicative: what is authoritative is [`Session::capabilities`], evaluated
    /// after the variant is detected.
    fn capabilities(&self) -> Capabilities {
        driver_capabilities()
    }

    /// Opens a pool, detects the variant, and returns the session.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the configuration is incomplete or carries a
    /// secret; [`OxynError::Authentication`] if the server refuses the
    /// credentials; [`OxynError::Connection`] if it is unreachable, with the
    /// network's reason rather than the pool's timeout;
    /// [`OxynError::Cancelled`] if `cancel` fires during the handshake.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let spec = ConnectSpec::from_config(&self.metadata, config, credentials)?;
        let base = spec.database().to_owned();

        let opened = race_cancel(cancel, async {
            Ok(PgPoolOptions::new()
                .max_connections(MAX_CONNECTIONS)
                .min_connections(0)
                .acquire_timeout(ACQUIRE_TIMEOUT)
                // A connection taken from the pool after a network cut must
                // fail here, not in the middle of the user's query.
                .test_before_acquire(true)
                .connect_with(spec.options().clone())
                .await)
        })
        .await?;
        let pool = match opened {
            Ok(pool) => pool,
            // `sqlx` retries a refused connection until the deadline, then
            // reports only the timeout: « pool timed out » when the tunnel is
            // down tells the user nothing they can act on.
            Err(sqlx::Error::PoolTimedOut) => {
                return Err(race_cancel(cancel, timeout_cause(spec.options())).await?);
            }
            Err(error) => return Err(map_connect_error(&error)),
        };

        let variant = match race_cancel(cancel, detect_variant(&pool)).await {
            Ok(variant) => variant,
            Err(error) => {
                // An open pool nobody will hold must be closed: otherwise the
                // connection stays established on the server.
                pool.close().await;
                return Err(error);
            }
        };

        tracing::debug!(
            target: "oxyn::driver::postgres",
            product = %variant.product(),
            version = %variant.server_version,
            "session opened"
        );

        Ok(Box::new(PostgresSession::new(
            self.id(),
            pool,
            spec,
            variant,
            base,
        )))
    }
}

/// Why opening the pool timed out: one direct attempt, without the pool's
/// retries, whose error is the server's or the network's own.
///
/// Only on the failure path: probing on every connect would cost a second
/// handshake to everyone for the benefit of an error message.
async fn timeout_cause(options: &PgConnectOptions) -> Result<OxynError> {
    let attempt = tokio::time::timeout(ACQUIRE_TIMEOUT, options.connect()).await;
    Ok(match attempt {
        Ok(Err(error)) => map_connect_error(&error),
        Ok(Ok(connection)) => {
            // The server answers now: the timeout was a moment's congestion.
            let _ = connection.close().await;
            map_connect_error(&sqlx::Error::PoolTimedOut)
        }
        Err(_) => map_connect_error(&sqlx::Error::PoolTimedOut),
    })
}

/// Asks the server for its identity and its extensions.
///
/// Two round trips, once per session. The second is **optional**:
/// `pg_extension` does not exist on Redshift and is not readable by an account
/// with restricted rights. An empty extension list says "I found nothing", not
/// "there are none" — and no optional capability is then enabled, which is the
/// cautious side.
async fn detect_variant(pool: &PgPool) -> Result<PostgresVariant> {
    let identity = sqlx::query(SQL_IDENTITY)
        .fetch_one(pool)
        .await
        .map_err(|error| map_connect_error(&error))?;

    let banner: String = identity
        .try_get(0)
        .map_err(|error| map_exec_error(&DriverId::postgres(), StatementIntent::Read, error))?;

    let extensions: Vec<String> = match sqlx::query(SQL_EXTENSIONS).fetch_all(pool).await {
        Ok(rows) => rows
            .iter()
            .filter_map(|row| row.try_get::<String, _>(0).ok())
            .collect(),
        Err(error) => {
            // Translated before being logged: `sqlx` sometimes composes its
            // messages with the connection URL, and "no `sqlx::Error` goes out
            // untranslated" is a rule that only holds without exception.
            tracing::debug!(
                target: "oxyn::driver::postgres",
                error = %map_connect_error(&error),
                "pg_extension is unreadable: no extension capability will be declared"
            );
            Vec::new()
        }
    };

    let version = server_version_from_banner(&banner);
    Ok(PostgresVariant::detect(&banner, &version, extensions))
}

/// Extracts the version number from a `version()` banner.
///
/// The banner has the form `PostgreSQL 17.2 on aarch64-apple-darwin…`: the
/// second word is the version, for PostgreSQL as for the products that imitate
/// its banner. Nothing more is guessed; if the form changes, the version is
/// empty and [`PostgresVariant::major_version`] is `None`, which is an honest
/// answer.
fn server_version_from_banner(banner: &str) -> String {
    banner
        .split_whitespace()
        .nth(1)
        .filter(|word| word.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use oxyn_driver::DriverRegistry;

    use super::*;

    #[test]
    fn the_driver_declares_itself_consistently() {
        // `DriverRegistry::register` refuses a divergence between `id()` and the
        // metadata, because it would make the driver impossible to find.
        let driver = PostgresDriver::new();
        assert_eq!(driver.id(), driver.metadata().id);
        driver
            .metadata()
            .check()
            .expect("the connection form is consistent");

        let mut registry = DriverRegistry::new();
        registry
            .register(std::sync::Arc::new(PostgresDriver::new()))
            .expect("registration");
        assert!(registry.contains(&DriverId::postgres()));
    }

    #[test]
    fn the_form_carries_the_expected_fields() {
        let metadata = postgres_metadata();
        for key in [
            "host",
            "port",
            "database",
            "user",
            "password",
            "sslmode",
            "application_name",
        ] {
            assert!(metadata.field(key).is_some(), "field `{key}` missing");
        }
        assert_eq!(metadata.default_port, Some(5432));
    }

    #[test]
    fn the_password_is_the_only_secret_field_and_has_no_default() {
        // A default value on a secret field would be written in clear in the
        // binary (I-03).
        let metadata = postgres_metadata();
        let secrets: Vec<&str> = metadata
            .secret_fields()
            .map(|field| field.key.as_str())
            .collect();
        assert_eq!(secrets, ["password"]);

        let field = metadata
            .field("password")
            .expect("the password field exists");
        assert!(field.default.is_none());
        assert_eq!(field.kind, FieldKind::Password);
    }

    #[test]
    fn the_tls_mode_offers_the_six_libpq_values() {
        let metadata = postgres_metadata();
        let field = metadata.field("sslmode").expect("the TLS field exists");
        let FieldKind::Choice(values) = &field.kind else {
            panic!("`sslmode` must be a closed choice: {:?}", field.kind);
        };
        assert_eq!(
            values,
            &[
                "disable",
                "allow",
                "prefer",
                "require",
                "verify-ca",
                "verify-full"
            ]
        );
    }

    #[test]
    fn a_valid_connection_configuration_passes_driver_validation() {
        let metadata = postgres_metadata();
        let connection = ConnectionConfig::new("shop", DriverId::postgres())
            .with_param("host", "internal.example")
            .with_param("database", "shop")
            .with_param("user", "app");
        metadata
            .validate(&connection)
            .expect("the three required fields are filled in");
    }

    #[test]
    fn the_driver_ceiling_contains_sql_and_server_side_cancel() {
        let capabilities = PostgresDriver::new().capabilities();
        assert!(capabilities.contains(Capabilities::SQL));
        assert!(capabilities.contains(Capabilities::SERVER_SIDE_CANCEL));
        assert!(
            capabilities.contains(Capabilities::VECTOR_SEARCH),
            "pgvector"
        );
        assert!(
            capabilities.contains(Capabilities::TIME_SERIES),
            "TimescaleDB"
        );
    }

    #[test]
    fn the_version_is_read_from_the_banner() {
        assert_eq!(
            server_version_from_banner("PostgreSQL 17.2 on aarch64-apple-darwin, 64-bit"),
            "17.2"
        );
        assert_eq!(
            server_version_from_banner(
                "PostgreSQL 8.0.2 on i686-pc-linux-gnu, compiled by GCC, Redshift 1.0.75008"
            ),
            "8.0.2"
        );
    }

    #[test]
    fn an_unexpected_banner_produces_no_invented_version() {
        // "Nothing is guessed": better announce nothing than announce wrong.
        assert_eq!(server_version_from_banner("bonjour"), "");
        assert_eq!(server_version_from_banner("CockroachDB CCL v23.1.0"), "");
        assert_eq!(server_version_from_banner(""), "");
    }

    #[test]
    fn sql_composed_at_connection_concatenates_nothing() {
        // I-10: these two queries are all the driver composes here.
        for compose in [SQL_IDENTITY, SQL_EXTENSIONS] {
            assert!(!compose.contains('{'), "{compose}");
            assert!(!compose.contains("||"), "{compose}");
        }
    }
}
