//! What a driver says about itself, and the connection form that follows from
//! it.
//!
//! **No driver codes its own connection screen.** It describes its fields
//! here, and the interface renders them. It is not a saving of code: it is
//! what guarantees that a password field stays a password field in all
//! fourteen drivers, that it never ends up in a workspace file, and that adding
//! a driver does not require touching the interface.
//!
//! # The rule this module enforces
//!
//! A field of kind [`FieldKind::Password`] designates a value that lives in the
//! system keychain, never in
//! [`ConnectionConfig::params`](oxyn_core::ConnectionConfig::params).
//! [`DriverMetadata::validate`] **refuses** a configuration that would carry
//! one: it is the checkable corollary of I-03, and the workspace file committed
//! by mistake into the team's repository is the failure it prevents.
//!
//! The refusal goes further than the declared fields: [`looks_like_secret`]
//! recognizes keys that *look like* a secret, including those no driver has
//! declared. A driver that forgets to mark its field `Password` therefore does
//! not create a silent leak.

use std::fmt;

use oxyn_core::{ConnectionConfig, DriverId, OxynError, Result};
use serde::{Deserialize, Serialize};

/// Family of a data source.
///
/// Serves two purposes, and two only: grouping the list of drivers in the
/// interface (see
/// [`DriverRegistry::sorted`](crate::registry::DriverRegistry::sorted)), and
/// giving a reading landmark. **A family decides nothing**: what a session can
/// do is read in [`Capabilities`](oxyn_core::Capabilities), never in its
/// family. Two sources of the same family do not have the same capabilities,
/// and that is the very point of ADR-0003.
///
/// The order of the variants is the display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DriverFamily {
    /// Transactional relational: PostgreSQL, MySQL, SQLite, SQL Server.
    Relational,
    /// Columnar analytical: ClickHouse, DuckDB, BigQuery, Snowflake.
    Analytical,
    /// Document: MongoDB, Couchbase.
    Document,
    /// Key-value: Redis, DynamoDB.
    KeyValue,
    /// Vector: Qdrant, pgvector as such.
    Vector,
    /// Graph: Neo4j, Memgraph.
    Graph,
    /// Time series: InfluxDB, TimescaleDB.
    TimeSeries,
    /// Search engine: Elasticsearch, OpenSearch.
    Search,
}

impl DriverFamily {
    /// Every family, in display order.
    pub const ALL: [Self; 8] = [
        Self::Relational,
        Self::Analytical,
        Self::Document,
        Self::KeyValue,
        Self::Vector,
        Self::Graph,
        Self::TimeSeries,
        Self::Search,
    ];

    /// Stable name, the one written to a file or a log.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Relational => "relational",
            Self::Analytical => "analytical",
            Self::Document => "document",
            Self::KeyValue => "key-value",
            Self::Vector => "vector",
            Self::Graph => "graph",
            Self::TimeSeries => "time-series",
            Self::Search => "search",
        }
    }
}

impl fmt::Display for DriverFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Kind of a connection form field.
///
/// The kind governs the rendering (a `Password` is typed masked) **and** the
/// storage: [`Password`](Self::Password) is the only kind whose value does not
/// join [`ConnectionConfig::params`](oxyn_core::ConnectionConfig::params).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FieldKind {
    /// Free text on one line: host, database name, default schema.
    Text,
    /// Secret. Typed masked, stored in the keychain, **never** persisted with
    /// the configuration.
    Password,
    /// Integer. The rendering may offer a step; validation stays with the
    /// driver.
    Number,
    /// Checkbox.
    Bool,
    /// Closed choice. The values are those the driver accepts, in the order it
    /// wants to offer them — `sslmode` is the typical example.
    Choice(Vec<String>),
    /// Path of a local file or directory. The interface opens a picker; it is
    /// what SQLite and DuckDB need.
    Path,
}

impl FieldKind {
    /// Is this field's value a secret?
    #[must_use]
    pub const fn is_secret(&self) -> bool {
        matches!(self, Self::Password)
    }

    /// Stable name of the kind.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Password => "password",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Choice(_) => "choice",
            Self::Path => "path",
        }
    }
}

impl fmt::Display for FieldKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A connection form field.
///
/// The `key` is the one under which the value is stored in
/// [`ConnectionConfig::params`](oxyn_core::ConnectionConfig::params) — except
/// for a secret field, which is never stored there. It is also the one that
/// [`DsnBuilder::from_config`](crate::dsn::DsnBuilder::from_config) recognizes:
/// a driver that names its host `server` rather than `host` will build a URL
/// option instead of a host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionField {
    /// Technical key, stable from one version to the next.
    pub key: String,
    /// Displayed label.
    pub label: String,
    /// Kind, which governs rendering and storage.
    pub kind: FieldKind,
    /// Must the field be filled for the connection to be attemptable?
    pub required: bool,
    /// Proposed value. **Forbidden on a secret field**: a default value is
    /// written in clear text in the binary and in the interface.
    pub default: Option<String>,
    /// Contextual help, in one sentence.
    pub help: Option<String>,
}

impl ConnectionField {
    /// Optional field, without default value or help.
    #[must_use]
    pub fn new(key: impl Into<String>, label: impl Into<String>, kind: FieldKind) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
            required: false,
            default: None,
            help: None,
        }
    }

    /// Marks the field required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Proposes a default value.
    ///
    /// Of no useful effect on a secret field: [`DriverMetadata::check`] refuses
    /// such a declaration rather than silently ignoring it.
    #[must_use]
    pub fn with_default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Attaches contextual help.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Is this field's value a secret?
    #[must_use]
    pub const fn is_secret(&self) -> bool {
        self.kind.is_secret()
    }
}

/// What a driver says about itself.
///
/// Obtained through [`Driver::metadata`](crate::traits::Driver::metadata), and
/// built once and for all: the value is borrowed, never recomputed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverMetadata {
    /// Driver identifier, per protocol (ADR-0003).
    pub id: DriverId,
    /// Displayed name: "PostgreSQL", not "postgres".
    pub display_name: String,
    /// Family, for grouping in the interface.
    pub family: DriverFamily,
    /// Default port of the protocol, when it has one. `None` for an embedded
    /// source such as SQLite.
    pub default_port: Option<u16>,
    /// The connection form fields, in input order.
    pub connection_fields: Vec<ConnectionField>,
    /// The other product names this driver serves because they speak its
    /// protocol (ADR-0003): "MariaDB" for MySQL. For search only — a user
    /// looking for MariaDB must find the driver that opens it, and the
    /// driver's identity stays its protocol.
    #[serde(default)]
    pub aliases: Vec<String>,
}

impl DriverMetadata {
    /// Minimal metadata: neither default port nor connection field.
    #[must_use]
    pub fn new(id: DriverId, display_name: impl Into<String>, family: DriverFamily) -> Self {
        Self {
            id,
            display_name: display_name.into(),
            family,
            default_port: None,
            connection_fields: Vec::new(),
            aliases: Vec::new(),
        }
    }

    /// Adds a product name the driver serves, for search.
    #[must_use]
    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    /// Sets the protocol's default port.
    #[must_use]
    pub fn with_default_port(mut self, port: u16) -> Self {
        self.default_port = Some(port);
        self
    }

    /// Adds a field to the form.
    #[must_use]
    pub fn with_field(mut self, field: ConnectionField) -> Self {
        self.connection_fields.push(field);
        self
    }

    /// Adds several fields, in order.
    #[must_use]
    pub fn with_fields(mut self, fields: impl IntoIterator<Item = ConnectionField>) -> Self {
        self.connection_fields.extend(fields);
        self
    }

    /// The field carrying this key.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&ConnectionField> {
        self.connection_fields.iter().find(|f| f.key == key)
    }

    /// The fields whose value is a secret.
    pub fn secret_fields(&self) -> impl Iterator<Item = &ConnectionField> {
        self.connection_fields.iter().filter(|f| f.is_secret())
    }

    /// Interface sort key: family, then displayed name.
    #[must_use]
    pub fn sort_key(&self) -> (DriverFamily, &str) {
        (self.family, self.display_name.as_str())
    }

    /// Checks the internal consistency of the declaration.
    ///
    /// Called by [`DriverRegistry::register`](crate::registry::DriverRegistry::register):
    /// an inconsistent declaration is a driver bug, and it is better found at
    /// registration than on the first form displayed.
    ///
    /// # Errors
    /// [`OxynError::Config`] if a key is empty, declared twice, or if a secret
    /// field carries a default value.
    pub fn check(&self) -> Result<()> {
        let mut seen: Vec<&str> = Vec::with_capacity(self.connection_fields.len());
        for field in &self.connection_fields {
            let key = field.key.trim();
            if key.is_empty() {
                return Err(OxynError::Config(format!(
                    "driver `{}`: a connection field has an empty key",
                    self.id
                )));
            }
            if seen.contains(&field.key.as_str()) {
                return Err(OxynError::Config(format!(
                    "driver `{}`: connection field `{}` is declared twice",
                    self.id, field.key
                )));
            }
            if field.is_secret() && field.default.is_some() {
                return Err(OxynError::Config(format!(
                    "driver `{}`: secret field `{}` carries a default value, \
                     which would be written in clear text",
                    self.id, field.key
                )));
            }
            seen.push(&field.key);
        }
        Ok(())
    }

    /// Checks that a connection configuration is usable by this driver.
    ///
    /// Three checks, in this order:
    ///
    /// 1. the configuration does designate this driver;
    /// 2. **no persisted parameter carries a secret** — neither a field
    ///    declared [`FieldKind::Password`], nor a key that looks like one
    ///    ([`looks_like_secret`]). It is I-03 made checkable;
    /// 3. every required field without a default value is filled.
    ///
    /// Error messages cite only **keys**, never values.
    ///
    /// # Errors
    /// [`OxynError::Config`] naming what is missing or what should not have
    /// been there.
    pub fn validate(&self, config: &ConnectionConfig) -> Result<()> {
        if config.driver != self.id {
            return Err(OxynError::Config(format!(
                "the connection names driver `{}`, not `{}`",
                config.driver, self.id
            )));
        }

        for key in config.params.keys() {
            if looks_like_secret(key) {
                return Err(OxynError::Config(format!(
                    "parameter `{}` carries a secret: a secret lives in the system keychain \
                     and is not persisted with the connection (I-03)",
                    sanitize_key(key)
                )));
            }
        }

        for field in &self.connection_fields {
            if field.is_secret() {
                // The loop above already refused keys that *look like* a
                // secret; this one catches a secret field named in an
                // unexpected way — `dsn`, `file_passphrase`…
                if config.params.contains_key(&field.key) {
                    return Err(OxynError::Config(format!(
                        "parameter `{}` is declared secret by driver `{}`: \
                         it is not persisted with the connection (I-03)",
                        sanitize_key(&field.key),
                        self.id
                    )));
                }
                continue;
            }
            if !field.required || field.default.is_some() {
                continue;
            }
            let filled = config
                .params
                .get(&field.key)
                .is_some_and(|v| !v.trim().is_empty());
            if !filled {
                return Err(OxynError::Config(format!(
                    "driver `{}`: parameter `{}` is required",
                    self.id, field.key
                )));
            }
        }
        Ok(())
    }
}

/// Does this parameter key likely designate a secret?
///
/// Used in the two places where a value could leak: the validation of a
/// configuration ([`DriverMetadata::validate`]) and the construction of a
/// connection URL ([`DsnBuilder::from_config`](crate::dsn::DsnBuilder::from_config)).
///
/// The recognition is **deliberately broad and fallible on the cautious
/// side**: it refuses `sslpassword`, which is indeed a secret, and lets through
/// `sslkey` and `authSource`, which are not. Being wrong by refusing costs an
/// error message; being wrong by accepting writes a password into a workspace
/// file.
#[must_use]
pub fn looks_like_secret(key: &str) -> bool {
    /// Fragments whose presence is enough.
    const FRAGMENTS: [&str; 10] = [
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "credential",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
    ];
    /// Keys where only the exact form counts: taking them as fragments would
    /// refuse `sslkey` and `keyspace`, which are not secrets.
    const EXACT: [&str; 2] = ["pass", "key"];

    let normalized = key.trim().to_ascii_lowercase();
    FRAGMENTS.iter().any(|f| normalized.contains(f)) || EXACT.iter().any(|e| normalized == *e)
}

/// Makes a key fit to show in an error message.
///
/// A key comes from a workspace file, that is from untrusted input (SECURITY,
/// input surface no. 3): nothing prevents a third party from having stored the
/// secret **in the key**. What leaves here is therefore bounded in length and
/// restricted to a harmless alphabet; the rest is replaced.
pub(crate) fn sanitize_key(key: &str) -> String {
    /// Beyond this, a key is no longer a key.
    const MAX: usize = 64;

    let acceptable = key.len() <= MAX
        && !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if acceptable {
        key.to_owned()
    } else {
        "<unrepresentable key>".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::ConnectionConfig;

    use super::*;

    fn postgres_metadata() -> DriverMetadata {
        DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
            .with_default_port(5432)
            .with_fields([
                ConnectionField::new("host", "Host", FieldKind::Text)
                    .required()
                    .with_default("localhost"),
                ConnectionField::new("port", "Port", FieldKind::Number).with_default("5432"),
                ConnectionField::new("user", "User", FieldKind::Text).required(),
                ConnectionField::new("password", "Password", FieldKind::Password),
                ConnectionField::new("database", "Database", FieldKind::Text).required(),
                ConnectionField::new(
                    "sslmode",
                    "TLS mode",
                    FieldKind::Choice(vec!["disable".into(), "require".into()]),
                )
                .with_default("require"),
            ])
    }

    #[test]
    fn a_consistent_declaration_passes_the_check() {
        postgres_metadata()
            .check()
            .expect("the reference declaration is consistent");
    }

    #[test]
    fn a_secret_field_cannot_carry_a_default_value() {
        // A default value on a secret field ends up in clear text in the
        // binary and in the interface.
        let metadata =
            DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
                .with_field(
                    ConnectionField::new("password", "Password", FieldKind::Password)
                        .with_default("postgres"),
                );
        let err = metadata.check().expect_err("refusal expected");
        assert!(err.to_string().contains("password"), "{err}");
    }

    #[test]
    fn a_key_declared_twice_is_refused() {
        let metadata = DriverMetadata::new(DriverId::mysql(), "MySQL", DriverFamily::Relational)
            .with_field(ConnectionField::new("host", "Host", FieldKind::Text))
            .with_field(ConnectionField::new("host", "Server", FieldKind::Text));
        assert!(metadata.check().is_err());
    }

    #[test]
    fn a_password_in_the_parameters_is_refused() {
        // I-03 made checkable: this is exactly the workspace file committed
        // into the team's repository.
        let metadata = postgres_metadata();
        let config = ConnectionConfig::new("prod-eu", DriverId::postgres())
            .with_param("host", "db.internal.example")
            .with_param("user", "app")
            .with_param("database", "till")
            .with_param("password", "hunter2");

        let err = metadata
            .validate(&config)
            .expect_err("a persisted password must be refused");
        let rendered = err.to_string();
        assert!(
            rendered.contains("password"),
            "the key is named: {rendered}"
        );
        assert!(
            !rendered.contains("hunter2"),
            "the value must never leave: {rendered}"
        );
    }

    #[test]
    fn a_key_that_looks_like_a_secret_is_refused_even_undeclared() {
        let metadata = postgres_metadata();
        for key in ["sslpassword", "auth_token", "API_KEY", "pwd"] {
            let config = ConnectionConfig::new("x", DriverId::postgres())
                .with_param("host", "h")
                .with_param("user", "u")
                .with_param("database", "d")
                .with_param(key, "sensitive-value");
            assert!(
                metadata.validate(&config).is_err(),
                "`{key}` looks like a secret and should have been refused"
            );
        }
    }

    #[test]
    fn legitimate_parameters_are_not_taken_for_secrets() {
        // False positives that would break real drivers: `sslkey` is a file
        // path, `authSource` a MongoDB database.
        for key in ["sslkey", "sslcert", "authSource", "keyspace", "sslmode"] {
            assert!(
                !looks_like_secret(key),
                "`{key}` is not a secret and must not be refused"
            );
        }
        for key in ["password", "sslpassword", "PWD", "api_key", "token"] {
            assert!(looks_like_secret(key), "`{key}` carries a secret");
        }
    }

    #[test]
    fn a_missing_required_field_is_reported() {
        let metadata = postgres_metadata();
        let config = ConnectionConfig::new("x", DriverId::postgres())
            .with_param("host", "h")
            .with_param("user", "u");
        let err = metadata
            .validate(&config)
            .expect_err("`database` is required");
        assert!(err.to_string().contains("database"), "{err}");
    }

    #[test]
    fn a_required_field_with_a_default_asks_for_nothing() {
        // `host` is required but carries `localhost`: not typing it is
        // legitimate.
        let metadata = postgres_metadata();
        let config = ConnectionConfig::new("x", DriverId::postgres())
            .with_param("user", "u")
            .with_param("database", "d");
        metadata
            .validate(&config)
            .expect("the default covers the missing input");
    }

    #[test]
    fn a_configuration_for_another_driver_is_refused() {
        let metadata = postgres_metadata();
        let config = ConnectionConfig::new("x", DriverId::sqlite());
        assert!(metadata.validate(&config).is_err());
    }

    #[test]
    fn a_hostile_key_does_not_come_out_in_the_message() {
        // A key may come from a file written by a third party: it may carry
        // control sequences, or the secret itself.
        let hostile = "password\u{1b}[2Jhunter2";
        assert_eq!(sanitize_key(hostile), "<unrepresentable key>");
        assert_eq!(sanitize_key("sslmode"), "sslmode");
        assert_eq!(sanitize_key(&"x".repeat(65)), "<unrepresentable key>");
    }

    #[test]
    fn sorting_goes_by_family_then_by_name() {
        let postgres = postgres_metadata();
        let clickhouse = DriverMetadata::new(
            DriverId::new("clickhouse").expect("valid identifier"),
            "ClickHouse",
            DriverFamily::Analytical,
        );
        assert!(postgres.sort_key() < clickhouse.sort_key());
    }

    #[test]
    fn secret_fields_are_found() {
        let metadata = postgres_metadata();
        let secrets: Vec<&str> = metadata.secret_fields().map(|f| f.key.as_str()).collect();
        assert_eq!(secrets, ["password"]);
        assert!(metadata.field("sslmode").is_some_and(|f| !f.is_secret()));
    }
}
