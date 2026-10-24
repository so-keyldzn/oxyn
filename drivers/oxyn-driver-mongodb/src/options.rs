//! The connection form to the client options, without ever making them
//! displayable.
//!
//! # Fields, not a connection string
//!
//! A `mongodb://user:password@host/` string carries the password. Stored as a
//! connection parameter, it would land in the workspace file; shown in an
//! error, in a log ([I-03](../../../CLAUDE.md#i-03)). The form therefore takes
//! the parts — hosts, database, user, authentication source, TLS files — and
//! the password goes to the keychain like every other driver's. The options
//! are built **programmatically**: no string holding the password is ever
//! composed, not even to hand it to the client library.
//!
//! The one string this module composes is `mongodb+srv://<host>/`, for the
//! library to resolve a DNS seed list. It holds no credential, and the host is
//! checked to be a DNS name first: a `/`, `?` or `@` in it would make the
//! library read options the form never showed.
//!
//! # TLS: verified, outside a local database
//!
//! The rule of [ADR-0052](../../../docs/adr/0052-verified-tls-outside-local.md)
//! applies: `verify-full` — chain and host name verified — is the default and
//! the only mode outside [`Environment::Local`]. A CA file and a client
//! certificate are accepted: they change whom the client trusts or proves to
//! be, never whether it verifies.
//!
//! # No talkative `Debug`
//!
//! `mongodb::options::ClientOptions` derives `Debug`; its `Credential` redacts
//! itself, but a `ClientOptions` printed whole still says too much about a
//! production topology for a log. [`ConnectSpec`] has a hand-written `Debug`
//! that shows the overview only.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use mongodb::options::{AuthMechanism, ClientOptions, Credential, ServerAddress, Tls, TlsOptions};
use oxyn_core::{ConnectionConfig, Environment, OxynError, Result};
use oxyn_driver::{Credentials, DriverMetadata};
use secrecy::ExposeSecret as _;

/// Default port of the protocol.
pub const DEFAULT_PORT: u16 = 27017;

/// The application name every connection announces: `currentOp` and the
/// server log show it, which is how a DBA tells Oxyn from a batch job.
const APP_NAME: &str = "oxyn";

/// How long to wait for a server to answer the handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to look for a suitable server before failing an operation.
const SERVER_SELECTION_TIMEOUT: Duration = Duration::from_secs(15);

/// The pool of one Oxyn session: the statement that runs, the `killSessions`
/// that stops it, and the catalog reads beside them.
const MAX_POOL_SIZE: u32 = 4;

/// The three TLS modes of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// TLS required, chain and host name verified. The default, and the only
    /// mode outside [`Environment::Local`].
    VerifyFull,
    /// TLS required, certificate not verified: encrypted, server not
    /// authenticated.
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
                    "parameter `tls` expects one of verify-full, require, disable; got `{}`",
                    value.escape_debug()
                ))
            })
    }
}

/// The authentication mechanisms the form offers: the two SCRAM variants,
/// which a test server proves, or the server's choice among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    /// Negotiated with the server (`saslSupportedMechs`).
    Default,
    /// `SCRAM-SHA-256`.
    ScramSha256,
    /// `SCRAM-SHA-1`.
    ScramSha1,
}

impl Mechanism {
    /// Every mechanism, in the order the form offers them.
    pub const ALL: [Self; 3] = [Self::Default, Self::ScramSha256, Self::ScramSha1];

    /// The name, as the form and the MongoDB documentation spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::ScramSha256 => "SCRAM-SHA-256",
            Self::ScramSha1 => "SCRAM-SHA-1",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|mechanism| mechanism.name().eq_ignore_ascii_case(value))
            .ok_or_else(|| {
                OxynError::Config(format!(
                    "parameter `authMechanism` expects one of default, SCRAM-SHA-256, \
                     SCRAM-SHA-1; got `{}`",
                    value.escape_debug()
                ))
            })
    }

    const fn library(self) -> Option<AuthMechanism> {
        match self {
            Self::Default => None,
            Self::ScramSha256 => Some(AuthMechanism::ScramSha256),
            Self::ScramSha1 => Some(AuthMechanism::ScramSha1),
        }
    }
}

/// Where the servers are: a seed list, or a DNS name that resolves to one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seeds {
    /// `host[:port]`, one or more.
    Hosts(Vec<(String, u16)>),
    /// `mongodb+srv`: one DNS name, no port.
    Srv(String),
}

/// The showable part of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Overview {
    seeds: Seeds,
    database: Option<String>,
    user: Option<String>,
    auth_source: Option<String>,
    mechanism: Mechanism,
    replica_set: Option<String>,
    direct: bool,
    tls: TlsMode,
    ca_file: Option<PathBuf>,
    certificate_key_file: Option<PathBuf>,
}

/// The parameters of a MongoDB connection, secret included, not displayable.
#[derive(Clone)]
pub struct ConnectSpec {
    overview: Overview,
    /// Absent without a user: the server then sees an unauthenticated client.
    password: Option<String>,
}

impl fmt::Debug for ConnectSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectSpec")
            .field("overview", &self.overview)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl ConnectSpec {
    /// Builds the parameters from a configuration and resolved credentials.
    ///
    /// The recognized keys are those of [`crate::mongodb_metadata`]. **Any
    /// other key is refused**: a client option the driver does not apply
    /// would otherwise be ignored while the user believes it holds.
    ///
    /// # Errors
    /// [`OxynError::Config`] for an incomplete or contradictory configuration,
    /// an unknown key, a host that is not a host, or a TLS mode other than
    /// `verify-full` on a connection not marked [`Environment::Local`].
    pub fn from_config(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
        credentials: &Credentials,
    ) -> Result<Self> {
        metadata.validate(config)?;

        let mut hosts = "localhost".to_owned();
        let mut port = DEFAULT_PORT;
        let mut srv = false;
        let mut database = None;
        let mut user = None;
        let mut auth_source = None;
        let mut mechanism = Mechanism::Default;
        let mut replica_set = None;
        let mut direct = false;
        let mut tls = TlsMode::VerifyFull;
        let mut ca_file = None;
        let mut certificate_key_file = None;

        let some = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        for (key, value) in &config.params {
            let value = value.trim();
            match key.trim() {
                "host" => value.clone_into(&mut hosts),
                "port" => port = parse_port(value)?,
                "srv" => srv = parse_bool("srv", value)?,
                "database" => database = some(value),
                "user" => user = some(value),
                "authSource" => auth_source = some(value),
                "authMechanism" => mechanism = Mechanism::parse(value)?,
                "replicaSet" => replica_set = some(value),
                "directConnection" => direct = parse_bool("directConnection", value)?,
                "tls" => tls = TlsMode::parse(value)?,
                "tlsCAFile" => ca_file = some(value).map(PathBuf::from),
                "tlsCertificateKeyFile" => certificate_key_file = some(value).map(PathBuf::from),
                other => {
                    return Err(OxynError::Config(format!(
                        "parameter `{}` is not a MongoDB connection option Oxyn applies",
                        other.escape_debug()
                    )));
                }
            }
        }

        let seeds = if srv {
            if direct {
                return Err(OxynError::Config(
                    "a DNS seed list (`srv`) and `directConnection` exclude each other: the \
                     list names several servers"
                        .to_owned(),
                ));
            }
            if hosts.contains(',') || hosts.contains(':') {
                return Err(OxynError::Config(
                    "a DNS seed list (`srv`) names one DNS name, without a port".to_owned(),
                ));
            }
            check_dns_name(&hosts)?;
            Seeds::Srv(hosts)
        } else {
            Seeds::Hosts(parse_hosts(&hosts, port)?)
        };
        if let Seeds::Hosts(list) = &seeds
            && direct
            && list.len() > 1
        {
            return Err(OxynError::Config(
                "`directConnection` takes exactly one host".to_owned(),
            ));
        }
        for (path, name) in [
            (&ca_file, "tlsCAFile"),
            (&certificate_key_file, "tlsCertificateKeyFile"),
        ] {
            if path.as_ref().is_some_and(|path| !path.is_absolute()) {
                return Err(OxynError::Config(format!(
                    "parameter `{name}` must be an absolute path"
                )));
            }
        }
        if tls != TlsMode::VerifyFull && config.environment != Environment::Local {
            return Err(OxynError::Config(format!(
                "`tls = {}` is only accepted on a connection marked Local: elsewhere the server \
                 must be authenticated (`verify-full`)",
                tls.name()
            )));
        }
        if tls == TlsMode::Disable && (ca_file.is_some() || certificate_key_file.is_some()) {
            return Err(OxynError::Config(
                "TLS files are given while `tls = disable`: they would be ignored".to_owned(),
            ));
        }
        let password = credentials
            .password()
            .map(|secret| secret.expose_secret().to_owned());
        if password.is_some() && user.is_none() {
            return Err(OxynError::Config(
                "a password is stored for this connection, but no user is named".to_owned(),
            ));
        }

        Ok(Self {
            overview: Overview {
                seeds,
                database,
                user,
                auth_source,
                mechanism,
                replica_set,
                direct,
                tls,
                ca_file,
                certificate_key_file,
            },
            password,
        })
    }

    /// The database the console sends its commands to when no context was
    /// chosen, if the form named one.
    #[must_use]
    pub fn database(&self) -> Option<&str> {
        self.overview.database.as_deref()
    }

    /// The requested TLS mode.
    #[must_use]
    pub const fn tls(&self) -> TlsMode {
        self.overview.tls
    }

    /// The client options, password included.
    ///
    /// Resolves the DNS seed list for `srv` — network I/O, on the caller's
    /// runtime, never on the interface thread ([I-05](../../../CLAUDE.md#i-05)).
    /// The only legitimate caller opens a client.
    pub(crate) async fn client_options(
        &self,
    ) -> std::result::Result<ClientOptions, mongodb::error::Error> {
        let overview = &self.overview;
        let mut options = match &overview.seeds {
            Seeds::Srv(name) => {
                // The name passed `check_dns_name`: nothing in it can start a
                // path, a query or a user part of the string.
                let mut composed = String::from("mongodb+srv://");
                composed.push_str(name);
                composed.push('/');
                ClientOptions::parse(composed).await?
            }
            Seeds::Hosts(list) => {
                let mut options = ClientOptions::default();
                options.hosts = list
                    .iter()
                    .map(|(host, port)| ServerAddress::Tcp {
                        host: host.clone(),
                        port: Some(*port),
                    })
                    .collect();
                options
            }
        };
        options.app_name = Some(APP_NAME.to_owned());
        // DRIVER-CONTRACT §4: the driver never retries on its own. A retried
        // write whose first attempt applied is a duplicate (I-13).
        options.retry_reads = Some(false);
        options.retry_writes = Some(false);
        options.connect_timeout = Some(CONNECT_TIMEOUT);
        options.server_selection_timeout = Some(SERVER_SELECTION_TIMEOUT);
        options.max_pool_size = Some(MAX_POOL_SIZE);
        options.min_pool_size = Some(0);
        options.direct_connection = Some(overview.direct);
        if let Some(name) = &overview.replica_set {
            options.repl_set_name = Some(name.clone());
        }
        options.tls = Some(match overview.tls {
            TlsMode::Disable => Tls::Disabled,
            mode => {
                let mut tls = TlsOptions::default();
                tls.allow_invalid_certificates = Some(mode == TlsMode::Require);
                tls.ca_file_path.clone_from(&overview.ca_file);
                tls.cert_key_file_path
                    .clone_from(&overview.certificate_key_file);
                Tls::Enabled(tls)
            }
        });
        if let Some(user) = &overview.user {
            // The MongoDB rule: the source is the one named, else the
            // connection's database, else `admin` — and a DNS seed list may
            // have named one in its TXT record.
            let source = overview
                .auth_source
                .clone()
                .or_else(|| options.credential.as_ref().and_then(|c| c.source.clone()))
                .or_else(|| overview.database.clone())
                .unwrap_or_else(|| "admin".to_owned());
            let mut credential = Credential::default();
            credential.username = Some(user.clone());
            credential.password.clone_from(&self.password);
            credential.source = Some(source);
            credential.mechanism = overview.mechanism.library();
            options.credential = Some(credential);
        }
        Ok(options)
    }
}

fn parse_port(value: &str) -> Result<u16> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| {
            OxynError::Config("parameter `port` expects an integer between 1 and 65535".to_owned())
        })
}

fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value {
        "" | "false" => Ok(false),
        "true" => Ok(true),
        _ => Err(OxynError::Config(format!(
            "parameter `{name}` expects true or false"
        ))),
    }
}

/// `host[:port]`, comma-separated; `[v6]:port` for an IPv6 address.
fn parse_hosts(text: &str, default_port: u16) -> Result<Vec<(String, u16)>> {
    let mut hosts = Vec::new();
    for entry in text.split(',').map(str::trim) {
        if entry.is_empty() {
            return Err(OxynError::Config(
                "parameter `host` has an empty entry in its list".to_owned(),
            ));
        }
        let (host, port) = if let Some(rest) = entry.strip_prefix('[') {
            let Some((address, after)) = rest.split_once(']') else {
                return Err(OxynError::Config(
                    "an IPv6 host is written `[address]` or `[address]:port`".to_owned(),
                ));
            };
            let port = match after.strip_prefix(':') {
                Some(port) => parse_port(port)?,
                None if after.is_empty() => default_port,
                None => {
                    return Err(OxynError::Config(
                        "an IPv6 host is written `[address]` or `[address]:port`".to_owned(),
                    ));
                }
            };
            (address.to_owned(), port)
        } else {
            match entry.rsplit_once(':') {
                Some((host, port)) => (host.to_owned(), parse_port(port)?),
                None => (entry.to_owned(), default_port),
            }
        };
        if host.is_empty()
            || host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '?' | '@' | '#'))
        {
            return Err(OxynError::Config(
                "parameter `host` holds a character a host name cannot contain".to_owned(),
            ));
        }
        hosts.push((host, port));
    }
    Ok(hosts)
}

/// A DNS name: labels of letters, digits and hyphens, separated by dots.
fn check_dns_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if valid {
        Ok(())
    } else {
        Err(OxynError::Config(
            "a DNS seed list (`srv`) needs a DNS name: letters, digits, hyphens and dots"
                .to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests;
