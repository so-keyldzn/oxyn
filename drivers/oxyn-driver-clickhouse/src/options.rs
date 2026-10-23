//! The connection parameters, never displayable with their secret.
//!
//! [`ConnectSpec`] holds the password and has **no derived `Debug`**: its own
//! shows the host, the port, the database, the user and the TLS mode. The
//! password leaves it only as the `X-ClickHouse-Key` header of a request built
//! by [`crate::http`] — never in the URL, which proxies and server logs keep
//! ([I-03](../../../CLAUDE.md#i-03)).
//!
//! # TLS is required, verified, outside a local database
//!
//! [ADR-0052](../../../docs/adr/0052-verified-tls-outside-local.md):
//! `verify-full` — HTTPS, certificate chain and host name verified against the
//! system's trust store — is the only mode accepted on a connection whose
//! environment is not [`Environment::Local`]. A connection with no environment
//! counts as production ([I-02](../../../CLAUDE.md#i-02)). HTTP cannot
//! negotiate TLS the way MySQL's `prefer` does: the modes are HTTPS verified,
//! HTTPS unverified, or plain HTTP.

use std::fmt;

use oxyn_core::{ConnectionConfig, Environment, OxynError, Result};
use oxyn_driver::{Credentials, DriverMetadata};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

/// The HTTPS port of ClickHouse's HTTP interface.
pub const HTTPS_PORT: u16 = 8443;
/// The plain-HTTP port of ClickHouse's HTTP interface.
pub const HTTP_PORT: u16 = 8123;

/// The three TLS modes of the connection form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// HTTPS, chain and host name verified. The default, and the only mode
    /// outside [`Environment::Local`].
    VerifyFull,
    /// HTTPS, certificate not verified: the traffic is encrypted, the server
    /// is not authenticated.
    Require,
    /// Plain HTTP.
    Disable,
}

impl TlsMode {
    /// Every mode, in the order the form offers them.
    pub const ALL: [Self; 3] = [Self::VerifyFull, Self::Require, Self::Disable];

    /// The mode's name, as the form spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::VerifyFull => "verify-full",
            Self::Require => "require",
            Self::Disable => "disable",
        }
    }

    /// The port a connection uses when the form names none.
    #[must_use]
    pub const fn default_port(self) -> u16 {
        match self {
            Self::VerifyFull | Self::Require => HTTPS_PORT,
            Self::Disable => HTTP_PORT,
        }
    }

    fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.name().eq_ignore_ascii_case(value))
            .ok_or_else(|| {
                OxynError::Config(
                    "parameter `sslmode` expects one of verify-full, require, disable".to_owned(),
                )
            })
    }
}

/// The parameters of a ClickHouse connection, secret included, not
/// displayable.
#[derive(Clone)]
pub struct ConnectSpec {
    base: Url,
    user: String,
    password: Option<SecretString>,
    database: Option<String>,
    tls: TlsMode,
}

impl ConnectSpec {
    /// Builds the parameters from a configuration and resolved credentials.
    ///
    /// The recognized keys are `host`, `port`, `database`, `user` and
    /// `sslmode`. **Any other key is refused**: an option the driver does not
    /// apply would otherwise be silently ignored while the user believes it
    /// holds.
    ///
    /// # Errors
    /// [`OxynError::Config`] if a required parameter is missing, the port is
    /// not a 16-bit integer, the host does not make a URL, a key is unknown,
    /// or a TLS mode other than `verify-full` is asked for on a connection not
    /// marked [`Environment::Local`].
    pub fn from_config(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
        credentials: &Credentials,
    ) -> Result<Self> {
        metadata.validate(config)?;

        let mut host = String::from("localhost");
        let mut port: Option<u16> = None;
        let mut database: Option<String> = None;
        let mut user = String::new();
        let mut tls = TlsMode::VerifyFull;

        for (key, value) in &config.params {
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "host" => value.clone_into(&mut host),
                "port" if value.is_empty() => {}
                "port" => {
                    port = Some(
                        value
                            .parse::<u16>()
                            .ok()
                            .filter(|p| *p > 0)
                            .ok_or_else(|| {
                                OxynError::Config(
                                    "parameter `port` expects an integer between 1 and 65535"
                                        .to_owned(),
                                )
                            })?,
                    );
                }
                "database" => database = (!value.is_empty()).then(|| value.to_owned()),
                "user" => value.clone_into(&mut user),
                "sslmode" => tls = TlsMode::parse(value)?,
                other => {
                    return Err(OxynError::Config(format!(
                        "parameter `{other}` is not a ClickHouse connection option Oxyn applies"
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

        let scheme = if tls == TlsMode::Disable {
            "http"
        } else {
            "https"
        };
        let mut base = Url::parse(&format!("{scheme}://localhost/"))
            .map_err(|_| OxynError::Internal("the base URL literal is valid".to_owned()))?;
        base.set_host(Some(&host)).map_err(|_| {
            OxynError::Config("parameter `host` is not a host name or an address".to_owned())
        })?;
        base.set_port(Some(port.unwrap_or_else(|| tls.default_port())))
            .map_err(|()| OxynError::Config("parameter `port` cannot be set".to_owned()))?;

        Ok(Self {
            base,
            user,
            password: credentials.password().cloned(),
            database,
            tls,
        })
    }

    /// The server's URL, without credentials or query.
    #[must_use]
    pub const fn base(&self) -> &Url {
        &self.base
    }

    /// The account.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    /// The password, for the request header only.
    pub(crate) fn password(&self) -> Option<&str> {
        self.password.as_ref().map(|secret| secret.expose_secret())
    }

    /// The database the form names, if it names one.
    #[must_use]
    pub fn database(&self) -> Option<&str> {
        self.database.as_deref()
    }

    /// The TLS mode.
    #[must_use]
    pub const fn tls(&self) -> TlsMode {
        self.tls
    }
}

impl fmt::Debug for ConnectSpec {
    /// Everything but the password, which is not even said to exist.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectSpec")
            .field("base", &self.base.as_str())
            .field("user", &self.user)
            .field("database", &self.database)
            .field("tls", &self.tls)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;

    use super::*;
    use crate::driver::clickhouse_metadata;

    fn config(params: &[(&str, &str)], environment: Environment) -> ConnectionConfig {
        let mut config =
            ConnectionConfig::new("events", DriverId::new("clickhouse").expect("valid"))
                .with_environment(environment);
        for (key, value) in params {
            config = config.with_param(*key, *value);
        }
        config
    }

    fn credentials() -> Credentials {
        Credentials::new().with_password(SecretString::from("hunter2-secret"))
    }

    #[test]
    fn the_password_never_shows_in_debug_nor_in_the_url() {
        let spec = ConnectSpec::from_config(
            &clickhouse_metadata(),
            &config(
                &[("host", "ch.internal"), ("user", "app")],
                Environment::Production,
            ),
            &credentials(),
        )
        .expect("valid");
        let rendered = format!("{spec:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!spec.base().as_str().contains("hunter2"));
        assert_eq!(spec.base().as_str(), "https://ch.internal:8443/");
        assert_eq!(spec.password(), Some("hunter2-secret"));
    }

    #[test]
    fn plain_http_or_an_unverified_certificate_is_only_for_a_local_database() {
        for mode in ["disable", "require"] {
            let error = ConnectSpec::from_config(
                &clickhouse_metadata(),
                &config(
                    &[("host", "ch.internal"), ("user", "app"), ("sslmode", mode)],
                    Environment::Staging,
                ),
                &credentials(),
            )
            .expect_err(mode);
            assert!(matches!(error, OxynError::Config(_)), "{error:?}");
        }
        let local = ConnectSpec::from_config(
            &clickhouse_metadata(),
            &config(
                &[
                    ("host", "127.0.0.1"),
                    ("user", "app"),
                    ("sslmode", "disable"),
                ],
                Environment::Local,
            ),
            &credentials(),
        )
        .expect("accepted on a local database");
        assert_eq!(local.base().as_str(), "http://127.0.0.1:8123/");
    }

    #[test]
    fn an_unknown_option_is_refused_rather_than_ignored() {
        let error = ConnectSpec::from_config(
            &clickhouse_metadata(),
            &config(
                &[("host", "h"), ("user", "u"), ("compression", "lz4")],
                Environment::Local,
            ),
            &credentials(),
        )
        .expect_err("refused");
        assert!(error.to_string().contains("compression"), "{error}");
    }

    #[test]
    fn a_host_that_is_not_a_host_is_refused() {
        for host in ["a b", "x/y?z", "user@host"] {
            let outcome = ConnectSpec::from_config(
                &clickhouse_metadata(),
                &config(&[("host", host), ("user", "u")], Environment::Production),
                &credentials(),
            );
            assert!(
                outcome
                    .as_ref()
                    .is_err_and(|error| matches!(error, OxynError::Config(_)))
                    || outcome.is_ok_and(|spec| spec.base().path() == "/"),
                "{host}"
            );
        }
    }
}
