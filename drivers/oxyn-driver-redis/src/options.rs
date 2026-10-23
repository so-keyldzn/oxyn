//! The connection parameters, built once and never displayable.
//!
//! # The trap this module closes
//!
//! A Redis URL carries its password (`redis://user:secret@host`), and so does
//! every structure that holds what `AUTH` will send. [`ConnectSpec`] has **no
//! derived `Debug`**: its own shows the address, the database, the user and
//! the TLS mode, never the password ([I-03](../../../CLAUDE.md#i-03)). The
//! password leaves only through `ConnectSpec::auth_arguments`, whose one
//! caller writes it on the connection.
//!
//! # TLS: verified outside a local database
//!
//! [ADR-0052](../../../docs/adr/0052-verified-tls-outside-local.md) applies as
//! it does to PostgreSQL and MySQL: `verify-full` — chain and host name — is the
//! default and the only mode accepted on a connection not marked
//! [`Environment::Local`]. A connection without an environment counts as
//! production. Redis has no in-protocol upgrade to TLS, hence no `prefer`: a
//! port speaks TLS or it does not.
//!
//! # What the driver reads, and nothing else
//!
//! A CA file is the one file the driver opens, because the form names it; it
//! holds certificates, which are public. Any key the form does not declare is
//! refused rather than ignored: an option the user believes applied and that
//! is not is worse than an error.

use std::fmt;
use std::path::PathBuf;

use oxyn_core::{ConnectionConfig, Environment, OxynError, Result};
use oxyn_driver::{Credentials, DriverMetadata};
use secrecy::{ExposeSecret as _, SecretString};

/// The port Redis listens on by default.
pub const DEFAULT_PORT: u16 = 6379;

/// The highest database number accepted. Redis's `databases` defaults to 16
/// and the server refuses an index it does not have; the bound only keeps a
/// typo of twenty digits from becoming an argument.
pub const MAX_DATABASE: u32 = 1 << 20;

/// The three TLS modes of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// TLS, chain and host name verified. The default, and the only mode
    /// outside [`Environment::Local`].
    VerifyFull,
    /// TLS, certificate not verified: encrypted, the server not authenticated.
    Require,
    /// Clear text.
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

    fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.name().eq_ignore_ascii_case(value))
            .ok_or_else(|| {
                OxynError::Config(format!(
                    "parameter `tls` expects one of verify-full, require, disable; got `{value}`"
                ))
            })
    }
}

/// Where the server is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// A host and a port.
    Tcp {
        /// Host name or address, as the form gives it.
        host: String,
        /// Port.
        port: u16,
    },
    /// A Unix socket on this machine.
    Unix(PathBuf),
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp { host, port } => write!(f, "{host}:{port}"),
            Self::Unix(path) => write!(f, "unix:{}", path.display()),
        }
    }
}

/// Everything needed to open a connection, the password included.
#[derive(Clone)]
pub struct ConnectSpec {
    address: Address,
    database: u32,
    user: Option<String>,
    password: Option<SecretString>,
    tls: TlsMode,
    ca_file: Option<PathBuf>,
}

impl ConnectSpec {
    /// Builds the parameters from a configuration and its resolved
    /// credentials.
    ///
    /// Keys: `host`, `port`, `socket`, `database` (or `db`), `user`
    /// (or `username`), `tls`, `ca_file`.
    ///
    /// # Errors
    /// [`OxynError::Config`] for a configuration of another driver, an unknown
    /// key, a port or database that is not a number in range, a socket named
    /// with a host other than the default, a CA file without TLS, or a TLS
    /// mode weaker than `verify-full` on a connection not marked
    /// [`Environment::Local`].
    pub fn from_config(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
        credentials: &Credentials,
    ) -> Result<Self> {
        metadata.validate(config)?;
        let mut host: Option<String> = None;
        let mut port = DEFAULT_PORT;
        let mut socket: Option<PathBuf> = None;
        let mut database = 0_u32;
        let mut user = None;
        let mut tls = TlsMode::VerifyFull;
        let mut ca_file = None;
        for (key, value) in &config.params {
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "host" | "hostname" => host = (!value.is_empty()).then(|| value.to_owned()),
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
                "socket" => socket = (!value.is_empty()).then(|| PathBuf::from(value)),
                "database" | "db" => {
                    database = if value.is_empty() {
                        0
                    } else {
                        value
                            .parse::<u32>()
                            .ok()
                            .filter(|n| *n <= MAX_DATABASE)
                            .ok_or_else(|| {
                                OxynError::Config(
                                "parameter `database` expects a database number, 0 for the first"
                                    .to_owned(),
                            )
                            })?
                    };
                }
                "user" | "username" => user = (!value.is_empty()).then(|| value.to_owned()),
                "tls" => tls = TlsMode::parse(value)?,
                "ca_file" => ca_file = (!value.is_empty()).then(|| PathBuf::from(value)),
                other => {
                    return Err(OxynError::Config(format!(
                        "parameter `{other}` is not a Redis connection option Oxyn applies"
                    )));
                }
            }
        }

        let address = match (socket, host) {
            (Some(path), None) => Address::Unix(path),
            (Some(path), Some(host)) if host == "localhost" => Address::Unix(path),
            (Some(_), Some(_)) => {
                return Err(OxynError::Config(
                    "a connection names either a socket or a host, not both".to_owned(),
                ));
            }
            (None, host) => Address::Tcp {
                host: host.unwrap_or_else(|| "localhost".to_owned()),
                port,
            },
        };
        if matches!(address, Address::Unix(_)) && tls != TlsMode::Disable {
            // A Unix socket never leaves the machine, and Redis serves no TLS
            // on it: requiring it would refuse every socket connection.
            tls = TlsMode::Disable;
        } else if tls != TlsMode::VerifyFull && config.environment != Environment::Local {
            return Err(OxynError::Config(format!(
                "`tls = {}` is only accepted on a connection marked Local: elsewhere the server \
                 must be authenticated (`verify-full`)",
                tls.name()
            )));
        }
        if ca_file.is_some() && tls != TlsMode::VerifyFull {
            return Err(OxynError::Config(
                "a CA file is only read to verify the server: it needs `tls = verify-full`"
                    .to_owned(),
            ));
        }
        Ok(Self {
            address,
            database,
            user,
            password: credentials.password().cloned(),
            tls,
            ca_file,
        })
    }

    /// Where the server is.
    #[must_use]
    pub const fn address(&self) -> &Address {
        &self.address
    }

    /// The database the session opens on.
    #[must_use]
    pub const fn database(&self) -> u32 {
        self.database
    }

    /// The ACL user, when the form names one; Redis's `default` otherwise.
    #[must_use]
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// The TLS mode.
    #[must_use]
    pub const fn tls(&self) -> TlsMode {
        self.tls
    }

    /// The CA file the server certificate is verified against, instead of the
    /// bundled Mozilla roots.
    #[must_use]
    pub fn ca_file(&self) -> Option<&PathBuf> {
        self.ca_file.as_ref()
    }

    /// The host name a certificate must carry, for TLS over TCP.
    #[must_use]
    pub fn server_name(&self) -> Option<&str> {
        match &self.address {
            Address::Tcp { host, .. } => Some(host),
            Address::Unix(_) => None,
        }
    }

    /// The `AUTH` clause of `HELLO`: user and password, when there is a
    /// password.
    ///
    /// The name is neutral, the use is not: the second element is the secret.
    /// Its one caller writes it on the connection and drops it.
    pub(crate) fn auth_arguments(&self) -> Option<[Vec<u8>; 2]> {
        let password = self.password.as_ref()?;
        let user = self.user.as_deref().unwrap_or("default");
        Some([
            user.as_bytes().to_vec(),
            password.expose_secret().as_bytes().to_vec(),
        ])
    }
}

impl fmt::Debug for ConnectSpec {
    /// Written by hand: the password is reported as present or absent, never
    /// shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectSpec")
            .field("address", &self.address)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("tls", &self.tls)
            .field("ca_file", &self.ca_file)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;
    use rstest::rstest;

    use super::*;
    use crate::driver::redis_metadata;

    fn config(params: &[(&str, &str)], environment: Environment) -> ConnectionConfig {
        params
            .iter()
            .fold(
                ConnectionConfig::new("cache", DriverId::new("redis").expect("valid")),
                |config, (key, value)| config.with_param(*key, *value),
            )
            .with_environment(environment)
    }

    fn spec(params: &[(&str, &str)], environment: Environment) -> Result<ConnectSpec> {
        ConnectSpec::from_config(
            &redis_metadata(),
            &config(params, environment),
            &Credentials::new().with_password("hunter2"),
        )
    }

    #[test]
    fn the_defaults_are_a_verified_local_port_on_database_zero() {
        let spec = spec(&[], Environment::Production).expect("valid");
        assert_eq!(
            spec.address(),
            &Address::Tcp {
                host: "localhost".to_owned(),
                port: 6379
            }
        );
        assert_eq!(spec.database(), 0);
        assert_eq!(spec.tls(), TlsMode::VerifyFull);
    }

    #[test]
    fn the_password_never_reaches_debug() {
        let spec = spec(
            &[("host", "cache.internal"), ("user", "app")],
            Environment::Production,
        )
        .expect("valid");
        let shown = format!("{spec:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        let [user, password] = spec.auth_arguments().expect("a password was given");
        assert_eq!(user, b"app");
        assert_eq!(password, b"hunter2");
    }

    #[test]
    fn without_a_password_no_auth_is_sent() {
        let spec = ConnectSpec::from_config(
            &redis_metadata(),
            &config(&[], Environment::Local),
            &Credentials::new(),
        )
        .expect("valid");
        assert!(spec.auth_arguments().is_none());
    }

    #[rstest]
    #[case("require")]
    #[case("disable")]
    fn a_weaker_tls_mode_needs_a_local_connection(#[case] mode: &str) {
        for environment in [
            Environment::Production,
            Environment::Staging,
            Environment::Development,
        ] {
            let error = spec(&[("tls", mode)], environment).expect_err("refused outside Local");
            assert!(error.to_string().contains("Local"), "{error}");
        }
        assert!(spec(&[("tls", mode)], Environment::Local).is_ok());
    }

    #[rstest]
    #[case(&[("port", "0")])]
    #[case(&[("port", "70000")])]
    #[case(&[("database", "-1")])]
    #[case(&[("database", "zero")])]
    #[case(&[("database", "99999999999")])]
    #[case(&[("tls", "prefer")])]
    #[case(&[("sentinel", "mymaster")])]
    #[case(&[("socket", "/tmp/redis.sock"), ("host", "cache.internal")])]
    #[case(&[("ca_file", "/etc/ca.pem"), ("tls", "require")])]
    fn an_unusable_configuration_is_refused(#[case] params: &[(&str, &str)]) {
        assert!(spec(params, Environment::Local).is_err(), "{params:?}");
    }

    #[test]
    fn a_socket_is_local_and_clear_text() {
        let spec = spec(
            &[("socket", "/tmp/redis.sock"), ("database", "3")],
            Environment::Production,
        )
        .expect("valid");
        assert_eq!(
            spec.address(),
            &Address::Unix(PathBuf::from("/tmp/redis.sock"))
        );
        assert_eq!(spec.tls(), TlsMode::Disable);
        assert_eq!(spec.database(), 3);
        assert_eq!(spec.server_name(), None);
    }

    #[test]
    fn a_password_in_the_parameters_is_refused() {
        let error = spec(&[("password", "hunter2")], Environment::Local).expect_err("a secret");
        assert!(!error.to_string().contains("hunter2"), "{error}");
    }
}
