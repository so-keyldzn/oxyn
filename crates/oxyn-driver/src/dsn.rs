//! The connection URL: build it, parse it, and never log it.
//!
//! A connection string is the only place in the product where a secret and an
//! address travel together. It is therefore the exact place where I-03 gets
//! lost: `tracing::info!("connecting to {dsn}")` is enough to write a
//! production password into a log file.
//!
//! # The four rules this module holds through types
//!
//! **The password enters the URL only at the last moment.** [`Dsn`] stores a
//! [`Url`] **without** a password and a [`SecretString`] beside it. The full
//! string exists only for the duration of the call to [`Dsn::expose`], which
//! returns a [`SecretString`] — a type without `Display`, with a masked
//! `Debug`, and zeroed when dropped.
//!
//! **`Display` and `Debug` of [`Dsn`] return a redacted URL.** They cannot do
//! otherwise: the value is not there. That is what sets this masking apart
//! from courtesy — there is nothing one could forget to mask.
//!
//! **No error message cites the URL.** The variants of [`DsnError`] carry only
//! `&'static str`s and sanitized keys: a malformed URL may contain the
//! password, and an error ends up in a log.
//!
//! **A secret stored in the parameters is refused, not carried.**
//! [`DsnBuilder::from_config`] rejects a key that looks like a secret rather
//! than turning it into a URL parameter
//! ([`crate::metadata::looks_like_secret`]): a query parameter ends up in the
//! server's access logs.
//!
//! # What this module does not do
//!
//! It knows no proprietary connection-string dialect: no SQL Server
//! `Server=…;Database=…;`, no `libpq` `key=value`. Those forms belong to the
//! drivers that speak them. Here, a URL.

use std::fmt;

use indexmap::IndexMap;
use oxyn_core::{ConnectionConfig, DriverId, OxynError};
use secrecy::{ExposeSecret, SecretString};
use url::Url;

use crate::credentials::Credentials;
use crate::metadata::{DriverMetadata, looks_like_secret, sanitize_key};

/// Working host, replaced before the URL leaves [`DsnBuilder::build`].
///
/// The `.invalid` TLD is reserved by RFC 2606: even if an intermediate URL
/// leaked, it would designate no reachable machine.
const PLACEHOLDER_HOST: &str = "oxyn.invalid";

/// What replaces the password in a redacted URL.
///
/// None of these characters is encoded by the `USERINFO` set of the `url`
/// crate: the rendering stays readable.
const REDACTED: &str = "***";

/// What can go wrong when building or parsing a connection URL.
///
/// **No variant carries a value coming from the URL.** Details are
/// `&'static str`s, into which no runtime data can enter; parameter keys are
/// sanitized before appearing there. That is what makes it possible to log one
/// of these errors without re-reading this module.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DsnError {
    /// The URL scheme is not usable.
    #[error("invalid URL scheme: {detail}")]
    InvalidScheme {
        /// What was wrong, without repeating the value.
        detail: &'static str,
    },

    /// The host is not usable as is.
    #[error("invalid host: {detail}")]
    InvalidHost {
        /// What was wrong, without repeating the value.
        detail: &'static str,
    },

    /// The port is not a 16-bit integer.
    #[error("invalid port: {detail}")]
    InvalidPort {
        /// What was wrong, without repeating the value.
        detail: &'static str,
    },

    /// The URL could not be parsed.
    ///
    /// The faulty text is **never** repeated: a malformed connection URL is
    /// still a connection URL, and it may carry a password.
    #[error("the connection URL cannot be parsed: {detail}")]
    Malformed {
        /// What was wrong, without repeating the value.
        detail: &'static str,
    },

    /// A persisted parameter carries a secret.
    #[error(
        "parameter `{key}` carries a secret: a secret lives in the system keychain \
         and is not persisted with the connection (I-03)"
    )]
    SecretInParams {
        /// The faulty key, sanitized. Never the value.
        key: String,
    },

    /// The URL has no authority: neither user, nor password, nor port can
    /// appear in it. That is the case of file-based sources (SQLite).
    #[error("this source has no host: it accepts neither user, nor password, nor port")]
    NoAuthority,

    /// The path of a file-based source is not absolute.
    #[error(
        "the path must be absolute: a driver does not resolve a relative path, \
         it does not know the application's current directory"
    )]
    RelativePath,

    /// The path and the database name are both filled.
    #[error("`path` and `database` both name the URL path: give only one of them")]
    ConflictingPath,
}

impl From<DsnError> for OxynError {
    fn from(err: DsnError) -> Self {
        Self::Config(err.to_string())
    }
}

/// A connection URL ready to be handed to a database client.
///
/// Does **not** carry the password in its URL: it is stored beside it and
/// injected by [`Dsn::expose`]. Direct consequence, and that is the whole
/// point: `Display` and `Debug` have no secret to mask.
///
/// Not `Clone`: [`SecretString`] is not either, because a copy of a secret is
/// one more copy to zero.
pub struct Dsn {
    /// The URL, without password.
    url: Url,
    /// The password, injected only by [`Dsn::expose`].
    password: Option<SecretString>,
}

impl Dsn {
    /// The URL without password. Safe to display, log and compare.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The URL scheme.
    #[must_use]
    pub fn scheme(&self) -> &str {
        self.url.scheme()
    }

    /// Will a password be injected by [`Dsn::expose`]?
    #[must_use]
    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// The redacted URL, as `Display` and `Debug` render it.
    ///
    /// The password is replaced by `***` **when there is one**: a URL without
    /// password and a URL whose password is masked must not be confused when
    /// reading a log.
    #[must_use]
    pub fn redacted(&self) -> String {
        if self.password.is_none() {
            return self.url.as_str().to_owned();
        }
        let mut redacted = self.url.clone();
        if redacted.set_password(Some(REDACTED)).is_err() {
            // Without authority, there is no room for a password — and
            // `build` already refused that combination.
            return self.url.as_str().to_owned();
        }
        String::from(redacted)
    }

    /// The full URL, password included.
    ///
    /// The name is unpleasant on purpose: every call is a place to review. The
    /// result has neither `Display` nor `Serialize`, its `Debug` is masked,
    /// and it is zeroed when dropped — handing it to a database client is its
    /// only legitimate use.
    ///
    /// ```ignore
    /// let dsn = builder.build()?;
    /// let url = dsn.expose()?;
    /// let pool = PgPool::connect(url.expose_secret()).await?;
    /// ```
    ///
    /// # Zeroing, and its limits
    ///
    /// The string built here is **moved** into the [`SecretString`], without a
    /// copy. The intermediate buffer of the `url` crate, however, is freed
    /// without being overwritten: zeroing is *best effort*, and the protection
    /// that counts remains the absence of any display path.
    ///
    /// # Errors
    /// [`DsnError::NoAuthority`] if a password comes with a URL without host —
    /// a combination [`DsnBuilder::build`] already refuses.
    pub fn expose(&self) -> Result<SecretString, DsnError> {
        let Some(password) = self.password.as_ref() else {
            return Ok(SecretString::from(self.url.as_str()));
        };
        let mut complete = self.url.clone();
        complete
            .set_password(Some(password.expose_secret()))
            .map_err(|()| DsnError::NoAuthority)?;
        Ok(SecretString::from(String::from(complete)))
    }
}

impl fmt::Display for Dsn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.redacted())
    }
}

impl fmt::Debug for Dsn {
    /// Written by hand, like [`Display`](fmt::Display): this type carries a
    /// secret (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Dsn({})", self.redacted())
    }
}

/// Builds a [`Dsn`] piece by piece.
///
/// Each component is set through an accessor of the `url` crate, which encodes
/// it according to the character set that matches it. **Nothing is
/// concatenated**: a database name containing `/`, a user containing `@` or a
/// password containing `#` are encoded, not interpreted. It is the rule of SQL
/// composed by Oxyn (I-10), applied to URLs.
pub struct DsnBuilder {
    scheme: String,
    host: Option<String>,
    port: Option<u16>,
    default_port: Option<u16>,
    user: Option<String>,
    password: Option<SecretString>,
    database: Option<String>,
    path: Option<String>,
    options: IndexMap<String, String>,
}

impl DsnBuilder {
    /// Parameter key of the host.
    pub const HOST: &'static str = "host";
    /// Parameter key of the port.
    pub const PORT: &'static str = "port";
    /// Parameter key of the user.
    pub const USER: &'static str = "user";
    /// Parameter key of the database name.
    pub const DATABASE: &'static str = "database";
    /// Parameter key of the file path, for an embedded source.
    pub const PATH: &'static str = "path";

    /// An empty builder for this URL scheme.
    ///
    /// The scheme is only validated at [`build`](Self::build), so that the
    /// chaining stays readable.
    #[must_use]
    pub fn new(scheme: impl Into<String>) -> Self {
        Self {
            scheme: scheme.into(),
            host: None,
            port: None,
            default_port: None,
            user: None,
            password: None,
            database: None,
            path: None,
            options: IndexMap::new(),
        }
    }

    /// Takes the parameters of a connection configuration.
    ///
    /// The scheme is the driver identifier. The recognized keys — and their
    /// usual aliases — become the host, the port, the user, the database or
    /// the path; **everything else becomes a URL option**, in input order.
    ///
    /// # Errors
    ///
    /// * [`DsnError::SecretInParams`] if a key looks like a secret. Turning it
    ///   into a URL parameter would put it in the server's access logs: refusal
    ///   is the only right answer (I-03);
    /// * [`DsnError::InvalidPort`] if `port` is not a 16-bit integer;
    /// * [`DsnError::ConflictingPath`] if a path and a database name are both
    ///   given.
    pub fn from_config(config: &ConnectionConfig) -> Result<Self, DsnError> {
        let mut builder = Self::new(config.driver.as_str());
        for (key, value) in &config.params {
            if looks_like_secret(key) {
                return Err(DsnError::SecretInParams {
                    key: sanitize_key(key),
                });
            }
            match key.trim().to_ascii_lowercase().as_str() {
                "host" | "hostname" | "server" => builder.host = Some(value.clone()),
                "port" => {
                    let port = value
                        .trim()
                        .parse::<u16>()
                        .map_err(|_| DsnError::InvalidPort {
                            detail: "expected: an integer between 1 and 65535",
                        })?;
                    builder.port = Some(port);
                }
                "user" | "username" => builder.user = Some(value.clone()),
                "database" | "dbname" | "db" => builder.database = Some(value.clone()),
                "path" | "file" | "filename" => builder.path = Some(value.clone()),
                _ => {
                    builder.options.insert(key.clone(), value.clone());
                }
            }
        }
        if builder.path.is_some() && builder.database.is_some() {
            return Err(DsnError::ConflictingPath);
        }
        Ok(builder)
    }

    /// Like [`from_config`](Self::from_config), also taking the default port
    /// declared by the driver.
    ///
    /// # Errors
    /// Those of [`from_config`](Self::from_config).
    pub fn for_driver(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
    ) -> Result<Self, DsnError> {
        let mut builder = Self::from_config(config)?;
        builder.default_port = metadata.default_port;
        Ok(builder)
    }

    /// Replaces the URL scheme.
    ///
    /// Useful when the driver identifier is not a valid scheme — a `_` is
    /// allowed in a [`DriverId`] and forbidden in a URL scheme — or when the
    /// client expects a different scheme (`postgresql` rather than
    /// `postgres`).
    #[must_use]
    pub fn with_scheme(mut self, scheme: impl Into<String>) -> Self {
        self.scheme = scheme.into();
        self
    }

    /// Sets the host. An IPv6 address is written between brackets: `[::1]`.
    #[must_use]
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Sets the port.
    #[must_use]
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Sets the port to use when the configuration gives none.
    #[must_use]
    pub fn with_default_port(mut self, port: u16) -> Self {
        self.default_port = Some(port);
        self
    }

    /// Sets the user.
    #[must_use]
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Sets the password.
    ///
    /// It will enter the URL only at [`Dsn::expose`].
    #[must_use]
    pub fn with_password(mut self, password: impl Into<SecretString>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Takes the password carried by resolved credentials.
    ///
    /// Copies the secret into a second [`SecretString`]: the type is not
    /// `Clone`, and exposing then re-wrapping is the only path. Both copies are
    /// zeroed when dropped.
    #[must_use]
    pub fn with_credentials(mut self, credentials: &Credentials) -> Self {
        if let Some(password) = credentials.password() {
            self.password = Some(SecretString::from(password.expose_secret()));
        }
        self
    }

    /// Sets the database name, which becomes the path's only segment.
    ///
    /// A name containing `/` is encoded, never split.
    #[must_use]
    pub fn with_database(mut self, database: impl Into<String>) -> Self {
        self.database = Some(database.into());
        self
    }

    /// Sets the full path of a file-based source.
    ///
    /// The path must be **absolute**: a driver does not know the application's
    /// current directory and has no business guessing it. The `.` and `..`
    /// segments are removed by the `url` crate — a path that depends on them
    /// must be canonicalized by the caller before arriving here.
    ///
    /// Forms that are not paths — SQLite's `:memory:` — do not go through a
    /// URL: the driver recognizes them before building a [`Dsn`].
    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Adds an option, which becomes a query parameter of the URL.
    #[must_use]
    pub fn with_option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    /// Assembles the URL.
    ///
    /// # Errors
    ///
    /// * [`DsnError::InvalidScheme`] if the scheme is not a URL scheme;
    /// * [`DsnError::InvalidHost`] if the host carries a `:` — a port slipped
    ///   into the host would be **silently ignored** by the `url` crate, and
    ///   the connection would go to the default port, that is to another
    ///   database than the one intended;
    /// * [`DsnError::NoAuthority`] if a user, a password or a port comes with a
    ///   source without host;
    /// * [`DsnError::RelativePath`] if the path of a file-based source is not
    ///   absolute;
    /// * [`DsnError::ConflictingPath`] if a path and a database name are both
    ///   given.
    pub fn build(self) -> Result<Dsn, DsnError> {
        validate_scheme(&self.scheme)?;
        if self.path.is_some() && self.database.is_some() {
            return Err(DsnError::ConflictingPath);
        }

        let port = self.port.or(self.default_port);
        let host = self.host.as_deref().filter(|h| !h.is_empty());
        if host.is_none() && (port.is_some() || self.user.is_some() || self.password.is_some()) {
            return Err(DsnError::NoAuthority);
        }

        let segments = self.path_segments()?;

        // Start from a working host: the `url` crate's accessors require an
        // authority, and all the encoding goes through them.
        let mut url =
            Url::parse(&format!("{}://{PLACEHOLDER_HOST}", self.scheme)).map_err(|_| {
                DsnError::InvalidScheme {
                    detail: "expected: a letter, then letters, digits, `+`, `-` or `.`",
                }
            })?;

        if !segments.is_empty() {
            // The block bounds the borrow: `PathSegmentsMut` has a `Drop` that
            // recomposes the URL, so it must end before any other access.
            let mut path = url.path_segments_mut().map_err(|()| DsnError::Malformed {
                detail: "this URL form does not accept a path",
            })?;
            path.extend(segments.iter());
        }

        if !self.options.is_empty() {
            let mut query = url.query_pairs_mut();
            for (key, value) in &self.options {
                query.append_pair(key, value);
            }
        }

        match host {
            Some(host) => {
                check_host(host)?;
                url.set_host(Some(host))
                    .map_err(|_| DsnError::InvalidHost {
                        detail: "forbidden character in a host name",
                    })?;
                if let Some(port) = port {
                    url.set_port(Some(port))
                        .map_err(|()| DsnError::InvalidHost {
                            detail: "this URL form does not accept a port",
                        })?;
                }
                if let Some(user) = self.user.as_deref() {
                    url.set_username(user).map_err(|()| DsnError::InvalidHost {
                        detail: "this URL form does not accept a user",
                    })?;
                }
            }
            None => {
                // Empty authority: the `sqlite:` + `///` + absolute path form.
                url.set_host(Some("")).map_err(|_| DsnError::InvalidHost {
                    detail: "this URL form does not accept an empty authority",
                })?;
            }
        }

        Ok(Dsn {
            url,
            password: self.password,
        })
    }

    /// The path segments, each meant to be encoded separately.
    fn path_segments(&self) -> Result<Vec<String>, DsnError> {
        if let Some(path) = self.path.as_deref() {
            if !path.starts_with('/') {
                return Err(DsnError::RelativePath);
            }
            return Ok(path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect());
        }
        match self.database.as_deref() {
            Some(database) if !database.is_empty() => Ok(vec![database.to_owned()]),
            _ => Ok(Vec::new()),
        }
    }
}

impl fmt::Debug for DsnBuilder {
    /// Written by hand: this type carries a password (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DsnBuilder")
            .field("scheme", &self.scheme)
            .field("host", &self.host)
            .field("port", &self.port.or(self.default_port))
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| REDACTED))
            .field("database", &self.database)
            .field("path", &self.path)
            .field("options", &self.options.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// The components of a parsed connection URL, **without the password**.
///
/// Everything is already decoded: `user%40domain` reads `user@domain`. The
/// path is kept **as segments** and not as one string: otherwise, a database
/// name containing an encoded `/` would become indistinguishable from a
/// two-level path, and the round trip would no longer be faithful.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct DsnParts {
    /// URL scheme, as is.
    pub scheme: String,
    /// Host, absent for a file-based source.
    pub host: Option<String>,
    /// Explicit port. `None` means "the driver's".
    pub port: Option<u16>,
    /// User, decoded.
    pub user: Option<String>,
    /// Path segments, decoded, without the `/`.
    pub segments: Vec<String>,
    /// The other query parameters, decoded, in URL order.
    /// **None of them carries a secret**: see [`ParsedDsn::parse`].
    pub options: IndexMap<String, String>,
}

impl DsnParts {
    /// The database name, when the path fits in one segment.
    ///
    /// Returns `None` for an empty path and for a path with several segments,
    /// which then designates a file.
    #[must_use]
    pub fn database(&self) -> Option<&str> {
        match self.segments.as_slice() {
            [single] if !single.is_empty() => Some(single),
            _ => None,
        }
    }

    /// The absolute path, rebuilt from the segments.
    ///
    /// It is what a file-based source expects. Returns `None` when the path is
    /// empty.
    #[must_use]
    pub fn file_path(&self) -> Option<String> {
        if self.segments.is_empty() {
            None
        } else {
            Some(format!("/{}", self.segments.join("/")))
        }
    }

    /// An equivalent connection configuration, **without any secret**.
    ///
    /// It is the "the user pastes a URL" path: what comes out of it can be
    /// persisted as is, and the password that came with it goes to the
    /// keychain by another path (see [`ParsedDsn::into_parts`]).
    ///
    /// The environment of the created connection is
    /// [`Production`](oxyn_core::Environment::Production): it is the default of
    /// [`ConnectionConfig`], and a URL says nothing about the environment.
    #[must_use]
    pub fn to_config(&self, name: impl Into<String>, driver: DriverId) -> ConnectionConfig {
        let mut config = ConnectionConfig::new(name, driver);
        if let Some(host) = self.host.as_deref() {
            config = config.with_param(DsnBuilder::HOST, host);
        }
        if let Some(port) = self.port {
            config = config.with_param(DsnBuilder::PORT, port.to_string());
        }
        if let Some(user) = self.user.as_deref() {
            config = config.with_param(DsnBuilder::USER, user);
        }
        match (self.host.is_some(), self.database(), self.file_path()) {
            (true, Some(database), _) => {
                config = config.with_param(DsnBuilder::DATABASE, database);
            }
            (_, _, Some(path)) => {
                config = config.with_param(DsnBuilder::PATH, path);
            }
            (_, _, None) => {}
        }
        for (key, value) in &self.options {
            // `options` cannot contain a secret: `parse` removed them. The
            // guard stays, because it is the invariant.
            if !looks_like_secret(key) {
                config = config.with_param(key, value);
            }
        }
        config
    }
}

impl fmt::Debug for DsnParts {
    /// Written by hand, like that of [`oxyn_core::ConnectionConfig`]: the
    /// **values** of the options are not printed.
    ///
    /// This type carries no password — [`ParsedDsn::parse`] removes it — but
    /// its options come from a URL, that is from untrusted input. Knowing that
    /// an `sslmode` option exists helps diagnosis; knowing which one does not
    /// help in a log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DsnParts")
            .field("scheme", &self.scheme)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("segments", &self.segments)
            .field("options", &self.options.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// The result of parsing a connection URL.
///
/// Separates what can be persisted ([`DsnParts`]) from what cannot
/// ([`Credentials`]). It is the very shape of the rule: a secret never joins a
/// connection configuration.
pub struct ParsedDsn {
    parts: DsnParts,
    credentials: Credentials,
    dropped_secrets: Vec<String>,
}

impl ParsedDsn {
    /// Parses a connection URL.
    ///
    /// The URL's password — and, failing that, the first query parameter that
    /// looks like a secret — is set aside in [`Credentials`]. The other
    /// parameters that look like a secret are **not** kept: their keys are
    /// returned by [`dropped_secret_keys`](Self::dropped_secret_keys), so that
    /// the interface can ask for them again rather than persist them in clear
    /// text or silently lose them.
    ///
    /// # Errors
    /// [`DsnError::Malformed`] if the URL cannot be parsed, or if a `%`
    /// encoding of the user, the password or the path is invalid. The faulty
    /// text is never repeated in the error.
    pub fn parse(text: &str) -> Result<Self, DsnError> {
        let url = Url::parse(text.trim()).map_err(|_| DsnError::Malformed {
            detail: "expected: a scheme, an optional authority, a path",
        })?;

        let user = match url.username() {
            "" => None,
            raw => Some(decode(raw, "the user carries an invalid `%` encoding")?),
        };

        let mut credentials = Credentials::new();
        match url.password() {
            None | Some("") => {}
            Some(raw) => {
                let clear = decode(raw, "the password carries an invalid `%` encoding")?;
                credentials = credentials.with_password(clear);
            }
        }

        let mut segments = Vec::new();
        for raw in url.path().split('/').filter(|s| !s.is_empty()) {
            segments.push(decode(raw, "the path carries an invalid `%` encoding")?);
        }

        let mut options = IndexMap::new();
        let mut dropped_secrets = Vec::new();
        for (key, value) in url.query_pairs() {
            if !looks_like_secret(&key) {
                options.insert(key.into_owned(), value.into_owned());
                continue;
            }
            if credentials.password().is_none() {
                credentials = credentials.with_password(value.into_owned());
            } else {
                dropped_secrets.push(sanitize_key(&key));
            }
        }

        Ok(Self {
            parts: DsnParts {
                scheme: url.scheme().to_owned(),
                host: url.host_str().filter(|h| !h.is_empty()).map(str::to_owned),
                port: url.port(),
                user,
                segments,
                options,
            },
            credentials,
            dropped_secrets,
        })
    }

    /// The components that can be persisted.
    #[must_use]
    pub fn parts(&self) -> &DsnParts {
        &self.parts
    }

    /// The credentials extracted from the URL.
    #[must_use]
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// The keys of the secrets the URL carried in addition to the password,
    /// and which were **not** kept.
    ///
    /// The keys are sanitized: a URL is untrusted input.
    #[must_use]
    pub fn dropped_secret_keys(&self) -> &[String] {
        &self.dropped_secrets
    }

    /// Separates for good what can be persisted from the secret.
    #[must_use]
    pub fn into_parts(self) -> (DsnParts, Credentials) {
        (self.parts, self.credentials)
    }
}

impl fmt::Debug for ParsedDsn {
    /// Written by hand: this type carries credentials (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedDsn")
            .field("parts", &self.parts)
            .field("credentials", &self.credentials)
            .field("dropped_secrets", &self.dropped_secrets)
            .finish()
    }
}

/// Checks that a string is a URL scheme.
///
/// RFC 3986 grammar: a letter, then letters, digits, `+`, `-`, `.`. The `_`,
/// allowed in a [`DriverId`], is not part of it.
fn validate_scheme(scheme: &str) -> Result<(), DsnError> {
    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return Err(DsnError::InvalidScheme {
            detail: "the scheme is empty",
        });
    };
    if !first.is_ascii_alphabetic() {
        return Err(DsnError::InvalidScheme {
            detail: "the scheme must start with an ASCII letter",
        });
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return Err(DsnError::InvalidScheme {
            detail: "allowed characters: letters, digits, `+`, `-`, `.` — not `_`",
        });
    }
    Ok(())
}

/// Refuses a host that the `url` crate would accept by truncating it.
///
/// `set_host` on `machine.example:5433` keeps `machine.example` and **throws
/// the port away** without a word. The connection would then go to the default
/// port, that is potentially to another database than the one targeted.
fn check_host(host: &str) -> Result<(), DsnError> {
    if host.starts_with('[') {
        return if host.ends_with(']') {
            Ok(())
        } else {
            Err(DsnError::InvalidHost {
                detail: "IPv6 address without a closing bracket",
            })
        };
    }
    if host.contains(':') {
        return Err(DsnError::InvalidHost {
            detail: "the port does not belong in the host: it would be ignored silently",
        });
    }
    Ok(())
}

/// Decodes the `%XX` sequences of a URL component.
///
/// # Errors
/// [`DsnError::Malformed`] carrying `detail` if the sequence is truncated, if a
/// digit is not hexadecimal, or if the result is not UTF-8.
fn decode(input: &str, detail: &'static str) -> Result<String, DsnError> {
    percent_decode(input).ok_or(DsnError::Malformed { detail })
}

/// Decodes the `%XX` sequences, or returns `None` if the input is invalid.
///
/// Written here because the `url` crate exposes no decoder, and
/// `form_urlencoded` is not one: it also translates `+` into a space, which
/// would corrupt a password.
fn percent_decode(input: &str) -> Option<String> {
    if !input.contains('%') {
        return Some(input.to_owned());
    }
    let mut bytes = Vec::with_capacity(input.len());
    let mut remaining = input.bytes();
    while let Some(byte) = remaining.next() {
        if byte != b'%' {
            bytes.push(byte);
            continue;
        }
        let high = hex_value(remaining.next()?)?;
        let low = hex_value(remaining.next()?)?;
        // `high` is at most 15: the product fits in a `u8`.
        bytes.push(high * 16 + low);
    }
    String::from_utf8(bytes).ok()
}

/// The value of an ASCII hexadecimal digit.
const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "hunter2";

    /// Composes a URL from its pieces.
    ///
    /// The tests contain **no literal connection string**: the repository's
    /// verification hook refuses them, and rightly so — such a string ends up
    /// committed, and it is often real.
    fn compose(userinfo: &str, rest: &str) -> String {
        let mut url = String::from("postgres://");
        url.push_str(userinfo);
        url.push('@');
        url.push_str(rest);
        url
    }

    fn postgres_dsn() -> Dsn {
        DsnBuilder::new("postgres")
            .with_host("machine.example")
            .with_port(5432)
            .with_user("app")
            .with_password(PASSWORD)
            .with_database("till")
            .build()
            .expect("the components are valid")
    }

    #[test]
    fn a_hosted_url_is_built_in_the_expected_order() {
        let dsn = postgres_dsn();
        assert_eq!(
            dsn.url().as_str(),
            compose("app", "machine.example:5432/till")
        );
        assert!(dsn.has_password());
        assert_eq!(dsn.scheme(), "postgres");
    }

    // ── Masking, which is this module's reason to exist ────────────────────

    #[test]
    fn the_password_leaves_neither_through_display_nor_through_debug() {
        // I-03: all six channels count, and `Display`/`Debug` are the ones
        // that feed logs without anyone thinking about it.
        let dsn = postgres_dsn();

        let displayed = dsn.to_string();
        let debugged = format!("{dsn:?}");

        assert!(!displayed.contains(PASSWORD), "leak: {displayed}");
        assert!(!debugged.contains(PASSWORD), "leak: {debugged}");

        let expected = compose("app:***", "machine.example:5432/till");
        assert_eq!(displayed, expected);
        assert_eq!(debugged, format!("Dsn({expected})"));
    }

    #[test]
    fn the_stored_url_does_not_contain_the_password() {
        // Masking is not a display courtesy: the value is not in the URL.
        // There is therefore nothing one could forget to mask.
        let dsn = postgres_dsn();
        assert!(!dsn.url().as_str().contains(PASSWORD));
        assert_eq!(dsn.url().password(), None);
    }

    #[test]
    fn the_password_appears_only_on_exposure() {
        let dsn = postgres_dsn();
        let complete = dsn.expose().expect("the URL has an authority");
        assert_eq!(
            complete.expose_secret(),
            compose(&format!("app:{PASSWORD}"), "machine.example:5432/till")
        );
    }

    #[test]
    fn a_url_without_password_displays_without_mask() {
        // A masked URL and a URL without password must not be confused when
        // reading a log.
        let dsn = DsnBuilder::new("postgres")
            .with_host("localhost")
            .with_user("app")
            .with_database("till")
            .build()
            .expect("valid");
        assert_eq!(dsn.to_string(), compose("app", "localhost/till"));
        assert!(!dsn.has_password());
    }

    #[test]
    fn the_builder_debug_masks_too() {
        let builder = DsnBuilder::new("postgres")
            .with_host("machine.example")
            .with_password(PASSWORD)
            .with_option("sslmode", "require");
        let rendered = format!("{builder:?}");
        assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
        assert!(rendered.contains("***"), "presence shows: {rendered}");
    }

    #[test]
    fn credentials_are_taken_without_exposing_outside() {
        let credentials = Credentials::new().with_password(PASSWORD);
        let dsn = DsnBuilder::new("postgres")
            .with_host("machine.example")
            .with_credentials(&credentials)
            .build()
            .expect("valid");
        assert!(dsn.has_password());
        assert!(!dsn.to_string().contains(PASSWORD));
    }

    // ── Encoding: nothing is concatenated ──────────────────────────────────

    #[test]
    fn components_are_encoded_not_interpreted() {
        // An `@` in the user (Azure), a `/` in the database name, a `#` in the
        // password: concatenating would produce a URL that designates another
        // server.
        let dsn = DsnBuilder::new("postgres")
            .with_host("machine.example")
            .with_user("app@tenant")
            .with_password("p@ss/w#rd")
            .with_database("prod/eu")
            .build()
            .expect("valid");

        assert_eq!(
            dsn.url().as_str(),
            compose("app%40tenant", "machine.example/prod%2Feu")
        );
        let complete = dsn.expose().expect("authority present");
        assert_eq!(
            complete.expose_secret(),
            compose("app%40tenant:p%40ss%2Fw%23rd", "machine.example/prod%2Feu")
        );

        // And the round trip finds the original values, encoded `/` included.
        let parsed = ParsedDsn::parse(complete.expose_secret()).expect("valid URL");
        assert_eq!(parsed.parts().user.as_deref(), Some("app@tenant"));
        assert_eq!(parsed.parts().database(), Some("prod/eu"));
        assert_eq!(
            parsed
                .credentials()
                .password()
                .map(ExposeSecret::expose_secret),
            Some("p@ss/w#rd")
        );
    }

    #[test]
    fn a_port_slipped_into_the_host_is_refused() {
        // The `url` crate would silently throw it away: the connection would
        // go to the default port, hence to another database than the one
        // targeted.
        let err = DsnBuilder::new("postgres")
            .with_host("machine.example:5433")
            .build()
            .expect_err("refusal expected");
        assert!(matches!(err, DsnError::InvalidHost { .. }), "{err:?}");
    }

    #[test]
    fn an_ipv6_address_is_given_between_brackets() {
        let dsn = DsnBuilder::new("postgres")
            .with_host("[::1]")
            .with_port(5432)
            .with_database("d")
            .build()
            .expect("valid");
        assert_eq!(dsn.url().as_str(), "postgres://[::1]:5432/d");
    }

    // ── File-based sources ─────────────────────────────────────────────────

    #[test]
    fn a_file_based_source_has_no_authority() {
        let dsn = DsnBuilder::new("sqlite")
            .with_path("/Users/x/daily till.db")
            .build()
            .expect("valid");
        assert_eq!(dsn.url().as_str(), "sqlite:///Users/x/daily%20till.db");
        assert!(!dsn.has_password());
    }

    #[test]
    fn a_relative_path_is_refused() {
        // Making it absolute by guessing the current directory would open the
        // neighboring database without saying so.
        let err = DsnBuilder::new("sqlite")
            .with_path("data/till.db")
            .build()
            .expect_err("refusal expected");
        assert_eq!(err, DsnError::RelativePath);
    }

    #[test]
    fn a_password_without_host_is_refused() {
        let err = DsnBuilder::new("sqlite")
            .with_path("/x.db")
            .with_password(PASSWORD)
            .build()
            .expect_err("refusal expected");
        assert_eq!(err, DsnError::NoAuthority);
    }

    #[test]
    fn a_path_and_a_database_name_do_not_go_together() {
        let err = DsnBuilder::new("sqlite")
            .with_path("/x.db")
            .with_database("till")
            .build()
            .expect_err("refusal expected");
        assert_eq!(err, DsnError::ConflictingPath);
    }

    // ── From a connection configuration ────────────────────────────────────

    #[test]
    fn a_configuration_becomes_a_url() {
        let config = ConnectionConfig::new("till", DriverId::postgres())
            .with_param("host", "machine.example")
            .with_param("port", "6432")
            .with_param("user", "app")
            .with_param("database", "till")
            .with_param("sslmode", "require");

        let dsn = DsnBuilder::from_config(&config)
            .expect("no secret in the parameters")
            .build()
            .expect("valid");

        assert_eq!(
            dsn.url().as_str(),
            compose("app", "machine.example:6432/till?sslmode=require")
        );
    }

    #[test]
    fn the_default_port_applies_only_for_lack_of_better() {
        let config =
            ConnectionConfig::new("c", DriverId::postgres()).with_param("host", "machine.example");
        let dsn = DsnBuilder::from_config(&config)
            .expect("valid")
            .with_default_port(5432)
            .build()
            .expect("valid");
        assert_eq!(dsn.url().port(), Some(5432));

        let config = config.with_param("port", "6432");
        let dsn = DsnBuilder::from_config(&config)
            .expect("valid")
            .with_default_port(5432)
            .build()
            .expect("valid");
        assert_eq!(dsn.url().port(), Some(6432));
    }

    #[test]
    fn a_secret_in_the_parameters_is_refused_not_carried() {
        // Turning it into a URL parameter would put it in the server's access
        // logs (I-03).
        let config = ConnectionConfig::new("c", DriverId::postgres())
            .with_param("host", "machine.example")
            .with_param("password", PASSWORD);

        let err = DsnBuilder::from_config(&config).expect_err("refusal expected");
        let rendered = err.to_string();
        assert!(
            rendered.contains("password"),
            "the key is named: {rendered}"
        );
        assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
    }

    #[test]
    fn an_unreadable_port_is_refused() {
        let config = ConnectionConfig::new("c", DriverId::postgres())
            .with_param("host", "machine.example")
            .with_param("port", "five thousand");
        let err = DsnBuilder::from_config(&config).expect_err("refusal expected");
        assert!(matches!(err, DsnError::InvalidPort { .. }), "{err:?}");
    }

    #[test]
    fn a_driver_identifier_with_underscore_is_not_a_scheme() {
        // `DriverId` allows `_`, RFC 3986 does not. The refusal is explicit
        // rather than silent, and `with_scheme` is the way out.
        let config =
            ConnectionConfig::new("c", DriverId::new("my_driver").expect("valid identifier"))
                .with_param("host", "machine.example");

        let err = DsnBuilder::from_config(&config)
            .expect("no secret")
            .build()
            .expect_err("refusal expected");
        assert!(matches!(err, DsnError::InvalidScheme { .. }), "{err:?}");

        DsnBuilder::from_config(&config)
            .expect("no secret")
            .with_scheme("my-driver")
            .build()
            .expect("the corrected scheme passes");
    }

    // ── Parsing ────────────────────────────────────────────────────────────

    #[test]
    fn parsing_a_url_separates_the_secret_from_what_is_persisted() {
        let source = compose(
            &format!("app:{PASSWORD}"),
            "machine.example:6432/till?sslmode=require",
        );
        let parsed = ParsedDsn::parse(&source).expect("valid URL");

        assert_eq!(parsed.parts().scheme, "postgres");
        assert_eq!(parsed.parts().host.as_deref(), Some("machine.example"));
        assert_eq!(parsed.parts().port, Some(6432));
        assert_eq!(parsed.parts().user.as_deref(), Some("app"));
        assert_eq!(parsed.parts().database(), Some("till"));
        assert_eq!(
            parsed.parts().options.get("sslmode").map(String::as_str),
            Some("require")
        );

        let (parts, credentials) = parsed.into_parts();
        assert_eq!(
            credentials.password().map(ExposeSecret::expose_secret),
            Some(PASSWORD)
        );

        // What is persisted carries no secret.
        let config = parts.to_config("till", DriverId::postgres());
        for (key, value) in &config.params {
            assert!(value != PASSWORD, "parameter `{key}` carries the password");
        }
        assert_eq!(config.secret_ref, None);
        assert_eq!(
            config.params.get("database").map(String::as_str),
            Some("till")
        );
    }

    #[test]
    fn a_secret_as_query_parameter_does_not_stay_in_the_options() {
        // Some tools write the password as a query parameter. Leaving it in
        // the options would persist it in clear text.
        let source = compose(
            "app",
            &format!("machine.example/till?password={PASSWORD}&sslmode=require"),
        );
        let parsed = ParsedDsn::parse(&source).expect("valid URL");

        assert!(!parsed.parts().options.contains_key("password"));
        assert_eq!(parsed.parts().options.len(), 1);
        assert_eq!(
            parsed
                .credentials()
                .password()
                .map(ExposeSecret::expose_secret),
            Some(PASSWORD)
        );
    }

    #[test]
    fn surplus_secrets_are_reported_not_silently_lost() {
        let source = compose(
            &format!("app:{PASSWORD}"),
            "machine.example/c?sslpassword=other",
        );
        let parsed = ParsedDsn::parse(&source).expect("valid URL");
        let dropped: Vec<&str> = parsed
            .dropped_secret_keys()
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(dropped, ["sslpassword"]);
        assert!(!parsed.parts().options.contains_key("sslpassword"));
    }

    #[test]
    fn an_unreadable_url_does_not_come_out_in_the_error() {
        // A malformed URL may carry the password, and an error ends up in a
        // log.
        let source = format!("machine.example/till?password={PASSWORD}");
        let err = ParsedDsn::parse(&source).expect_err("the scheme is missing");
        let rendered = err.to_string();
        assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
        assert!(!rendered.contains("machine.example"), "leak: {rendered}");
    }

    #[test]
    fn the_debug_of_a_parsed_url_shows_no_sensitive_value() {
        let source = compose(
            &format!("app:{PASSWORD}"),
            "machine.example/c?sslmode=require",
        );
        let parsed = ParsedDsn::parse(&source).expect("valid URL");
        let rendered = format!("{parsed:?}");
        assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
        assert!(
            !rendered.contains("require"),
            "option value leaked: {rendered}"
        );
        assert!(
            rendered.contains("sslmode"),
            "the key stays useful: {rendered}"
        );
    }

    #[test]
    fn an_invalid_percent_encoding_is_refused() {
        let source = compose("app:m%ZZ", "machine.example/c");
        assert!(
            ParsedDsn::parse(&source).is_err(),
            "a `%ZZ` cannot be decoded"
        );
    }

    #[test]
    fn a_file_based_source_parses_as_a_path() {
        let parsed = ParsedDsn::parse("sqlite:///Users/x/till.db").expect("valid URL");
        assert_eq!(parsed.parts().host, None);
        assert_eq!(parsed.parts().segments, ["Users", "x", "till.db"]);
        assert_eq!(
            parsed.parts().database(),
            None,
            "several segments: it is a file, not a database name"
        );
        assert_eq!(
            parsed.parts().file_path().as_deref(),
            Some("/Users/x/till.db")
        );

        let config = parsed.parts().to_config("local", DriverId::sqlite());
        assert_eq!(
            config.params.get("path").map(String::as_str),
            Some("/Users/x/till.db")
        );
    }

    #[test]
    fn the_round_trip_is_faithful() {
        let original = compose("app", "machine.example:6432/till?sslmode=require");
        let parsed = ParsedDsn::parse(&original).expect("valid URL");
        let config = parsed.parts().to_config("till", DriverId::postgres());
        let rebuilt = DsnBuilder::from_config(&config)
            .expect("no secret")
            .build()
            .expect("valid");
        assert_eq!(rebuilt.url().as_str(), original);
    }

    #[test]
    fn the_round_trip_of_a_file_based_source_is_faithful() {
        let original = "sqlite:///Users/x/till.db";
        let parsed = ParsedDsn::parse(original).expect("valid URL");
        let config = parsed.parts().to_config("local", DriverId::sqlite());
        let rebuilt = DsnBuilder::from_config(&config)
            .expect("no secret")
            .build()
            .expect("valid");
        assert_eq!(rebuilt.url().as_str(), original);
    }

    // ── Decoding ───────────────────────────────────────────────────────────

    #[test]
    fn percent_decoding_covers_its_edge_cases() {
        assert_eq!(
            percent_decode("no-escaping").as_deref(),
            Some("no-escaping")
        );
        assert_eq!(percent_decode("a%20b").as_deref(), Some("a b"));
        assert_eq!(percent_decode("caf%C3%A9").as_deref(), Some("café"));
        // A `+` stays a `+`: it is what a `form_urlencoded` decoder would
        // corrupt in a password.
        assert_eq!(percent_decode("a+b").as_deref(), Some("a+b"));
        assert_eq!(percent_decode("%2"), None, "truncated sequence");
        assert_eq!(percent_decode("%ZZ"), None, "non-hexadecimal digits");
        assert_eq!(percent_decode("%FF"), None, "lone non-UTF-8 byte");
    }
}
