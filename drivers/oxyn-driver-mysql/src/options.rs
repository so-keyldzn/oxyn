//! Building the connection parameters without ever making them displayable.
//!
//! # The trap this module closes
//!
//! `mysql_async`'s `Opts` and `OptsBuilder` derive `Debug` **and carry the
//! password in clear**, and so does `Conn`, whose `Debug` prints its options. A
//! `tracing::debug!("{opts:?}")` added six months later would write a
//! production password to a log file ([I-03](../../../CLAUDE.md#i-03)).
//!
//! [`ConnectSpec`] wraps the options and **has no derived `Debug`**: its own
//! shows the host, the port, the database, the user and the TLS mode. The only
//! way out is `ConnectSpec::attempts`, which hands the options to
//! `Conn::new`. For the same reason no type of this crate that holds a `Conn`
//! derives `Debug`.
//!
//! # TLS is required, verified, outside a local database
//!
//! ADR-0050 §6: `verify-full` — certificate chain and host name verified — is
//! the only mode accepted on a connection whose environment is not
//! [`Environment::Local`]. A connection with no environment counts as
//! production ([I-02](../../../CLAUDE.md#i-02)), so a forgotten marking asks for
//! more, never less. The certificate is checked against the bundled Mozilla
//! roots (`webpki-roots`, through `mysql_async`'s `rustls-tls`): a server signed
//! by a private authority fails to connect until the form can name a CA file.

use std::fmt;

use mysql_async::{Opts, OptsBuilder, SslOpts};
use oxyn_core::{ConnectionConfig, Environment, OxynError, Result};
use oxyn_driver::{Credentials, DriverMetadata};
use secrecy::ExposeSecret as _;

/// Default port of the protocol.
pub const DEFAULT_PORT: u16 = 3306;

/// The program name announced in the connection attributes.
///
/// Shows up in `performance_schema.session_connect_attrs`: that is what lets a
/// DBA know a query comes from Oxyn rather than from a batch job.
const PROGRAM_NAME: &str = "oxyn";

/// The four TLS modes of the connection form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// TLS required, chain and host name verified. The default, and the only
    /// mode outside [`Environment::Local`].
    VerifyFull,
    /// TLS required, certificate not verified: the traffic is encrypted, the
    /// server is not authenticated.
    Require,
    /// TLS when the server offers it, clear text otherwise.
    Prefer,
    /// Clear text.
    Disable,
}

impl TlsMode {
    /// The mode's name, as the form spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::VerifyFull => "verify-full",
            Self::Require => "require",
            Self::Prefer => "prefer",
            Self::Disable => "disable",
        }
    }

    /// Every mode, in the order the form offers them.
    pub const ALL: [Self; 4] = [Self::VerifyFull, Self::Require, Self::Prefer, Self::Disable];

    fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.name().eq_ignore_ascii_case(value))
            .ok_or_else(|| {
                OxynError::Config(format!(
                    "parameter `sslmode` expects one of verify-full, require, prefer, disable; \
                     got `{value}`"
                ))
            })
    }
}

/// The parameters of a MySQL connection, secret included, not displayable.
#[derive(Clone)]
pub struct ConnectSpec {
    /// The options of the first attempt.
    primary: Opts,
    /// The options of the second attempt, in `prefer` mode only: clear text,
    /// when the server offers no TLS.
    clear_text_fallback: Option<Opts>,
    /// What can be shown, frozen at construction so that `Debug` has nothing to
    /// extract from the options.
    overview: Overview,
}

/// The showable part of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Overview {
    host: String,
    port: u16,
    database: Option<String>,
    user: String,
    tls: TlsMode,
}

impl ConnectSpec {
    /// Builds the parameters from a configuration and resolved credentials.
    ///
    /// The recognized keys are `host`, `port`, `database`, `user` and `sslmode`,
    /// with their usual aliases. **Any other key is refused**: a MySQL client
    /// option the driver does not apply — a CA file, a socket — would otherwise
    /// be silently ignored while the user believes it holds.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the configuration does not designate this
    /// driver, if a required parameter is missing, if the port is not a 16-bit
    /// integer, if a key is unknown, or if a TLS mode other than `verify-full`
    /// is asked for on a connection not marked [`Environment::Local`].
    pub fn from_config(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
        credentials: &Credentials,
    ) -> Result<Self> {
        metadata.validate(config)?;

        let mut host = "localhost".to_owned();
        let mut port = metadata.default_port.unwrap_or(DEFAULT_PORT);
        let mut database: Option<String> = None;
        let mut user = String::new();
        let mut tls = TlsMode::VerifyFull;

        for (key, value) in &config.params {
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "host" | "hostname" | "server" => value.clone_into(&mut host),
                "port" => {
                    port = value
                        .parse::<u16>()
                        .ok()
                        .filter(|p| *p > 0)
                        .ok_or_else(|| {
                            OxynError::Config(
                                "parameter `port` expects an integer between 1 and 65535"
                                    .to_owned(),
                            )
                        })?;
                }
                "database" | "dbname" | "db" => {
                    database = (!value.is_empty()).then(|| value.to_owned());
                }
                "user" | "username" => value.clone_into(&mut user),
                "sslmode" | "ssl_mode" | "ssl-mode" => tls = TlsMode::parse(value)?,
                other => {
                    return Err(OxynError::Config(format!(
                        "parameter `{other}` is not a MySQL connection option Oxyn applies"
                    )));
                }
            }
        }

        if host.is_empty() {
            return Err(OxynError::Config("parameter `host` is required".to_owned()));
        }
        if user.is_empty() {
            return Err(OxynError::Config("parameter `user` is required".to_owned()));
        }
        if tls != TlsMode::VerifyFull && config.environment != Environment::Local {
            return Err(OxynError::Config(format!(
                "`sslmode = {}` is only accepted on a connection marked Local: elsewhere the \
                 server must be authenticated (`verify-full`)",
                tls.name()
            )));
        }

        let base = OptsBuilder::default()
            .ip_or_hostname(host.clone())
            .tcp_port(port)
            .user(Some(user.clone()))
            .pass(
                credentials
                    .password()
                    .map(|secret| secret.expose_secret().to_owned()),
            )
            .db_name(database.clone())
            // ADR-0050 §1-2: every statement is prepared then closed by the
            // driver. A cache would keep statements open behind its back and
            // count them against `max_prepared_stmt_count`.
            .stmt_cache_size(0)
            // With `true`, a TCP connection to localhost asks the server for
            // its socket path and reconnects through it: a query and a second
            // connection the user never asked for.
            .prefer_socket(false)
            // The password never travels in clear, whatever the server asks.
            .enable_cleartext_plugin(false)
            .connect_attribute("program_name", PROGRAM_NAME);
        // No `local_infile_handler`: a `LOAD DATA LOCAL INFILE` request from
        // the server is then refused and no file of this machine is read
        // (ADR-0050 §1).

        let (primary, clear_text_fallback) = match tls {
            TlsMode::VerifyFull => (base.ssl_opts(SslOpts::default()), None),
            TlsMode::Require => (base.ssl_opts(unverified()), None),
            TlsMode::Prefer => (
                base.clone().ssl_opts(unverified()),
                Some(Opts::from(base.ssl_opts(None::<SslOpts>))),
            ),
            TlsMode::Disable => (base.ssl_opts(None::<SslOpts>), None),
        };

        Ok(Self {
            primary: Opts::from(primary),
            clear_text_fallback,
            overview: Overview {
                host,
                port,
                database,
                user,
                tls,
            },
        })
    }

    /// The options to try, in order: the configured ones, then clear text in
    /// `prefer` mode.
    ///
    /// The name is neutral, the use is not: what comes out of here carries the
    /// password. The only legitimate call is opening a connection.
    pub(crate) fn attempts(&self) -> (&Opts, Option<&Opts>) {
        (&self.primary, self.clear_text_fallback.as_ref())
    }

    /// The host, as the configuration gave it.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.overview.host
    }

    /// The port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.overview.port
    }

    /// The database the connection opens on, if the form named one.
    #[must_use]
    pub fn database(&self) -> Option<&str> {
        self.overview.database.as_deref()
    }

    /// The user.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.overview.user
    }

    /// The requested TLS mode.
    #[must_use]
    pub const fn tls(&self) -> TlsMode {
        self.overview.tls
    }
}

/// TLS without authenticating the server: `require` and `prefer`, local only.
fn unverified() -> SslOpts {
    SslOpts::default()
        .with_danger_accept_invalid_certs(true)
        .with_danger_skip_domain_validation(true)
}

impl fmt::Debug for ConnectSpec {
    /// Written by hand: this type carries a secret, and a derived `Debug` is
    /// the `tracing::debug!` that leaks it six months later.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectSpec")
            .field("host", &self.overview.host)
            .field("port", &self.overview.port)
            .field("database", &self.overview.database)
            .field("user", &self.overview.user)
            .field("tls", &self.overview.tls.name())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;

    use super::*;
    use crate::driver::mysql_metadata;

    fn config(environment: Environment) -> ConnectionConfig {
        ConnectionConfig::new("shop", DriverId::mysql())
            .with_param("host", "db.internal")
            .with_param("user", "app")
            .with_environment(environment)
    }

    #[test]
    fn debug_never_shows_the_password() {
        // I-03: the options inside do carry it.
        let secret = "hunter2-mysql";
        let spec = ConnectSpec::from_config(
            &mysql_metadata(),
            &config(Environment::Production),
            &Credentials::new().with_password(secret),
        )
        .expect("complete configuration");
        let shown = format!("{spec:?}");
        assert!(!shown.contains(secret), "leak: {shown}");
        assert!(shown.contains("db.internal"), "{shown}");
        let (primary, _) = spec.attempts();
        assert_eq!(
            primary.pass(),
            Some(secret),
            "the password reaches the client"
        );
    }

    #[test]
    fn the_default_verifies_the_server_and_disables_the_statement_cache() {
        let spec = ConnectSpec::from_config(
            &mysql_metadata(),
            &config(Environment::Production),
            &Credentials::new(),
        )
        .expect("complete configuration");
        assert_eq!(spec.tls(), TlsMode::VerifyFull);
        let (primary, fallback) = spec.attempts();
        let ssl = primary.ssl_opts().expect("TLS is required");
        assert!(!ssl.accept_invalid_certs() && !ssl.skip_domain_validation());
        assert!(fallback.is_none(), "no clear-text second chance");
        assert_eq!(primary.stmt_cache_size(), 0);
        assert!(primary.local_infile_handler().is_none());
        assert!(!primary.enable_cleartext_plugin());
        assert!(!primary.prefer_socket());
        assert_eq!(primary.tcp_port(), DEFAULT_PORT);
        assert_eq!(spec.database(), None, "the database is optional");
    }

    #[test]
    fn a_weaker_tls_mode_is_refused_outside_a_local_connection() {
        // ADR-0050 §6. A connection with no environment is production.
        for mode in ["require", "prefer", "disable"] {
            for environment in [
                Environment::Production,
                Environment::Staging,
                Environment::Development,
            ] {
                let error = ConnectSpec::from_config(
                    &mysql_metadata(),
                    &config(environment).with_param("sslmode", mode),
                    &Credentials::new(),
                )
                .expect_err("refusal expected");
                assert!(matches!(error, OxynError::Config(_)), "{error:?}");
                assert!(error.to_string().contains("Local"), "{error}");
            }
            let local = ConnectSpec::from_config(
                &mysql_metadata(),
                &config(Environment::Local).with_param("sslmode", mode),
                &Credentials::new(),
            )
            .expect("accepted on a local database");
            assert_eq!(local.tls().name(), mode);
        }
    }

    #[test]
    fn prefer_tries_tls_first_then_clear_text() {
        let spec = ConnectSpec::from_config(
            &mysql_metadata(),
            &config(Environment::Local).with_param("sslmode", "prefer"),
            &Credentials::new(),
        )
        .expect("local");
        let (primary, fallback) = spec.attempts();
        assert!(primary.ssl_opts().is_some());
        assert!(fallback.expect("second attempt").ssl_opts().is_none());
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_ignored() {
        let error = ConnectSpec::from_config(
            &mysql_metadata(),
            &config(Environment::Local).with_param("ssl_ca", "/etc/ca.pem"),
            &Credentials::new(),
        )
        .expect_err("refusal expected");
        assert!(error.to_string().contains("ssl_ca"), "{error}");
    }

    #[test]
    fn a_port_outside_sixteen_bits_is_refused() {
        for port in ["0", "65536", "-1", "abc"] {
            assert!(
                ConnectSpec::from_config(
                    &mysql_metadata(),
                    &config(Environment::Local).with_param("port", port),
                    &Credentials::new(),
                )
                .is_err(),
                "{port}"
            );
        }
    }
}
