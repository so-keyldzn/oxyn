//! Configuration of a connection: marking of its environment, and privacy
//! tier of what may reach an AI prompt.
//!
//! Three rules govern this module. The first two come from
//! [`SECURITY`](../../../docs/SECURITY.md):
//!
//! 1. **No secret here.** Password, full connection string, SSH key, client
//!    certificate: none of it enters [`ConnectionConfig`]. What is persisted
//!    is a *reference* to the secret
//!    ([`secret_ref`](ConnectionConfig::secret_ref)), resolved by
//!    `oxyn-secrets` against the system keychain. The failure avoided is
//!    concrete: a workspace file containing a production password, committed
//!    by the user to their team's repository.
//! 2. **The default is [`Environment::Production`]**, the most restrictive
//!    value. The opposite default is what lets an `UPDATE` without `WHERE`
//!    through on the customer database because the user hastily added the
//!    connection without filling in the field.
//!
//! The third comes from [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md):
//!
//! 3. **The [`PrivacyTier`] is carried by the connection**, not by the
//!    session, the provider or the application, and its default is
//!    [`Metadata`](PrivacyTier::Metadata). The type lives here — and not in
//!    `oxyn-ai` — precisely because this structure is what carries it: a tier
//!    stored elsewhere ends up being a global setting, which ADR-0006 refuses.
//!
//! The `Debug` of [`ConnectionConfig`] is written by hand: parameter values
//! are masked. A derived `Debug` is the most frequent leak, because it is
//! invisible in review — it is the `tracing::debug!("{cfg:?}")` added six
//! months later that leaks (I-03).

use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ids::{ConnectionId, DriverId, IdParseError};

/// Environment of a connection.
///
/// The four values are exactly those of
/// [`SECURITY`](../../../docs/SECURITY.md); the enumeration is closed for the
/// same reason as [`StatementIntent`](crate::query::StatementIntent): one more
/// environment must force a re-read of every decision that depends on it.
///
/// The order of the variants is that of increasing restriction, and it is
/// **significant**: `Ord` serves to take the more restrictive of two
/// environments (see `DefaultPolicy`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    /// Local, disposable database.
    Local,
    /// Shared development environment.
    Development,
    /// Staging: resembles production, but the data does not count the same
    /// way.
    Staging,
    /// Production. **It is the default** when the environment is not filled
    /// in.
    #[default]
    Production,
}

impl Environment {
    /// Is it production?
    #[must_use]
    pub const fn is_production(&self) -> bool {
        matches!(self, Self::Production)
    }

    /// Stable name, the one written in workspace files.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Development => "development",
            Self::Staging => "staging",
            Self::Production => "production",
        }
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Environment {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "development" | "dev" => Ok(Self::Development),
            "staging" | "stage" => Ok(Self::Staging),
            "production" | "prod" => Ok(Self::Production),
            _ => Err(IdParseError::new(
                "Environment",
                "expected: local, development, staging or production",
            )),
        }
    }
}

/// What is allowed to leave the machine for a given connection.
///
/// Authority: [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md). The
/// table of tiers lives there and is not copied here.
///
/// # Why per connection, and never globally
///
/// The failure targeted by [AI-PROVIDERS](../../../docs/AI-PROVIDERS.md): the
/// user sets the tier to `Sampled` for their sandbox database, forgets it,
/// then three days later opens their employer's customer database. If the
/// tier were global, real rows would go to a third-party provider. Technically
/// nothing failed; contractually, it is irreversible.
///
/// Design consequence: this type carries **no** constructor that derives it
/// from a provider, a session or an application setting. It comes from the
/// [`ConnectionConfig::privacy_tier`] field and nothing else (I-04).
///
/// # `Metadata` by default is not "nothing leaves"
///
/// DDL, names, types, indexes and cardinalities **leave** as soon as a remote
/// provider is configured. A `patients` table with a `hiv_status` column
/// reveals the essentials without a single row leaving. It is a deliberate
/// trade-off, and the interface must show it permanently.
///
/// # The enumeration is closed
///
/// Unlike the repository's convention on public enumerations: ADR-0006's triad
/// is a contract, and a fourth tier would be an ADR decision, not a variant
/// added along the way. A `_ =>` in the interface that swallowed an unknown
/// tier would silently choose the wrong behavior.
///
/// The order is that of **increasing disclosure**: `Local < Metadata <
/// Sampled`. That is what makes [`most_restrictive`](Self::most_restrictive)
/// writable, and hence composable when two tiers apply to the same send.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyTier {
    /// Nothing leaves the machine. Local model only.
    Local,
    /// DDL, names, types, indexes, cardinalities, execution plans.
    /// **No row value.** It is the default.
    #[default]
    Metadata,
    /// Same, plus a sample of rows explicitly approved, column by column.
    Sampled,
}

impl PrivacyTier {
    /// Can row values reach a prompt?
    ///
    /// Only [`Sampled`](Self::Sampled) answers `true`, and even then the values
    /// must have been approved column by column upstream: this predicate is a
    /// necessary condition, not a sufficient one.
    ///
    /// It is **the** predicate that governs any content likely to quote a row
    /// — a sample, but also a server error message, which copies the value
    /// that violates a constraint.
    #[must_use]
    pub const fn allows_row_values(&self) -> bool {
        matches!(self, Self::Sampled)
    }

    /// Is a provider whose data leaves the machine usable?
    ///
    /// `false` for [`Local`](Self::Local). It is not a default a setting
    /// overturns: it is the tier's promise.
    ///
    /// The local/remote classification of an endpoint is **never** made on the
    /// shape of its URL — an OpenAI-compatible endpoint listening on the
    /// loopback can be a proxy to the cloud. It is made on the real host after
    /// resolution, which is a blocking operation: it lives in `oxyn-llm`, with
    /// the rest of what talks to the network.
    #[must_use]
    pub const fn allows_remote_provider(&self) -> bool {
        !matches!(self, Self::Local)
    }

    /// The more restrictive of the two tiers.
    ///
    /// Used wherever two tiers meet — a conversation that touches two
    /// connections, a context assembled before the user switches connection.
    /// The result never discloses more than the more cautious of the two.
    #[must_use]
    pub fn most_restrictive(self, other: Self) -> Self {
        self.min(other)
    }

    /// Stable name, for display, persistence and audit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Metadata => "metadata",
            Self::Sampled => "sampled",
        }
    }

    /// What leaves the machine under this tier, in one showable sentence.
    ///
    /// In English: this sentence reaches a prompt, so it is code text. The
    /// interface must show the effective tier **permanently** and not in a
    /// settings panel: a user who cannot tell at a glance where their query
    /// goes does not give informed consent (AI-PROVIDERS).
    #[must_use]
    pub const fn describe(&self) -> &'static str {
        match self {
            Self::Local => "nothing leaves this machine; local model only",
            Self::Metadata => "schema only: names, types, indexes, cardinalities — no row values",
            Self::Sampled => "schema, plus row samples you approved column by column",
        }
    }
}

impl fmt::Display for PrivacyTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PrivacyTier {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "metadata" => Ok(Self::Metadata),
            "sampled" => Ok(Self::Sampled),
            _ => Err(IdParseError::new(
                "PrivacyTier",
                "expected: local, metadata or sampled",
            )),
        }
    }
}

/// Configuration of a connection, as persisted in the workspace.
///
/// The parameters are an [`IndexMap`] and not a `HashMap`: their order is the
/// one the user entered, and it is kept when the file is rewritten. A
/// configuration file that reorders itself produces unreadable diffs in a
/// repository.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// Internal identifier. Does not appear in the interface.
    pub id: ConnectionId,
    /// Name given by the user. It is **this** that is shown, including in
    /// confirmation requests.
    pub name: String,
    /// Driver, per protocol (ADR-0003).
    pub driver: DriverId,
    /// Environment. Absent from the file, it is
    /// [`Environment::Production`].
    #[serde(default)]
    pub environment: Environment,
    /// What is allowed to reach an AI prompt for this connection.
    ///
    /// Absent from the file, it is [`PrivacyTier::Metadata`]: ADR-0006's
    /// default, and never [`Sampled`](PrivacyTier::Sampled). The direction of
    /// caution is that of [`environment`](Self::environment) — a missing field
    /// does not lower the protection.
    #[serde(default)]
    pub privacy_tier: PrivacyTier,
    /// Non-secret parameters: host, port, database, schema, TLS mode…
    ///
    /// A password has no business here. See
    /// [`secret_ref`](Self::secret_ref).
    #[serde(default)]
    pub params: IndexMap<String, String>,
    /// Reference to the secret in the system keychain. Never the secret
    /// itself.
    #[serde(default)]
    pub secret_ref: Option<String>,
    /// The connection is declared read-only by the user.
    ///
    /// The `PolicyGate` turns it into a **refusal** of any mutating command,
    /// human actor included: marking a connection read-only is a declaration
    /// of intent, not a display preference.
    #[serde(default)]
    pub read_only: bool,
}

impl ConnectionConfig {
    /// Creates a configuration with the cautious defaults: fresh identifier,
    /// [`Production`](Environment::Production) environment,
    /// [`Metadata`](PrivacyTier::Metadata) tier, no parameter, no secret.
    #[must_use]
    pub fn new(name: impl Into<String>, driver: DriverId) -> Self {
        Self {
            id: ConnectionId::new(),
            name: name.into(),
            driver,
            environment: Environment::default(),
            privacy_tier: PrivacyTier::default(),
            params: IndexMap::new(),
            secret_ref: None,
            read_only: false,
        }
    }

    /// Sets the environment.
    #[must_use]
    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    /// Sets this connection's privacy tier.
    ///
    /// It is a user act on **one** connection: there is deliberately no path
    /// that applies it to several at once (ADR-0006).
    #[must_use]
    pub fn with_privacy_tier(mut self, tier: PrivacyTier) -> Self {
        self.privacy_tier = tier;
        self
    }

    /// Adds a non-secret parameter.
    #[must_use]
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    /// Attaches a secret reference.
    #[must_use]
    pub fn with_secret_ref(mut self, secret_ref: impl Into<String>) -> Self {
        self.secret_ref = Some(secret_ref.into());
        self
    }

    /// Marks the connection read-only.
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Does the connection target production?
    #[must_use]
    pub const fn is_production(&self) -> bool {
        self.environment.is_production()
    }
}

impl fmt::Debug for ConnectionConfig {
    /// Deliberately partial rendering: parameter **values** and the secret
    /// reference are not printed. Only the keys are — knowing that a `host`
    /// parameter exists is useful; knowing which one is not, in a log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct ClesSeules<'a>(&'a IndexMap<String, String>);
        impl fmt::Debug for ClesSeules<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_map()
                    .entries(self.0.keys().map(|k| (k, "<redacted>")))
                    .finish()
            }
        }

        f.debug_struct("ConnectionConfig")
            .field("name", &self.name)
            .field("driver", &self.driver)
            .field("environment", &self.environment)
            // The tier is showable, and it must be: an incident is diagnosed
            // by knowing under which tier the connection was running.
            .field("privacy_tier", &self.privacy_tier)
            .field("params", &ClesSeules(&self.params))
            .field(
                "secret_ref",
                &self.secret_ref.as_ref().map(|_| "<redacted reference>"),
            )
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_production() {
        // SECURITY: the default is the most restrictive value, not the most
        // permissive.
        assert_eq!(Environment::default(), Environment::Production);
        assert!(Environment::default().is_production());
        assert!(ConnectionConfig::new("no environment", DriverId::postgres()).is_production());
    }

    #[test]
    fn an_environment_missing_from_the_file_means_production() {
        let json = r#"{
            "id": "018f0000-0000-7000-8000-000000000000",
            "name": "base client",
            "driver": "postgres"
        }"#;
        let cfg: ConnectionConfig = serde_json::from_str(json).expect("deserialization");
        assert_eq!(
            cfg.environment,
            Environment::Production,
            "a missing field must never lower the protection"
        );
        assert!(!cfg.read_only);
        assert!(cfg.params.is_empty());
    }

    #[test]
    fn a_connection_without_explicit_tier_means_metadata() {
        // ADR-0006: the default is safe. A connection whose tier is not filled
        // in is **never** `Sampled` — it is I-02's direction of caution applied
        // to the AI boundary.
        let cfg = ConnectionConfig::new("base client", DriverId::postgres());
        assert_eq!(cfg.privacy_tier, PrivacyTier::Metadata);
        assert!(!cfg.privacy_tier.allows_row_values());
    }

    #[test]
    fn a_tier_missing_from_the_file_means_metadata() {
        // The missing field is the real case: a workspace written before the
        // field existed. It must not be read back as `Sampled`.
        let json = r#"{
            "id": "018f0000-0000-7000-8000-000000000000",
            "name": "base client",
            "driver": "postgres"
        }"#;
        let cfg: ConnectionConfig = serde_json::from_str(json).expect("deserialization");
        assert_eq!(cfg.privacy_tier, PrivacyTier::Metadata);
    }

    #[test]
    fn the_tier_is_persisted_with_the_connection() {
        // The tier belongs to the connection: the round trip must be exact,
        // otherwise a workspace read back would lower — or widen — the protection.
        for niveau in [
            PrivacyTier::Local,
            PrivacyTier::Metadata,
            PrivacyTier::Sampled,
        ] {
            let cfg =
                ConnectionConfig::new("sandbox", DriverId::sqlite()).with_privacy_tier(niveau);
            let json = serde_json::to_string(&cfg).expect("serialization");
            let relu: ConnectionConfig = serde_json::from_str(&json).expect("deserialization");
            assert_eq!(relu.privacy_tier, niveau, "{json}");
            assert_eq!(niveau.as_str().parse::<PrivacyTier>(), Ok(niveau));
        }
        assert!("confidentiel".parse::<PrivacyTier>().is_err());
    }

    #[test]
    fn tiers_are_ordered_from_least_to_most_disclosing() {
        assert!(PrivacyTier::Local < PrivacyTier::Metadata);
        assert!(PrivacyTier::Metadata < PrivacyTier::Sampled);
        assert_eq!(
            PrivacyTier::Sampled.most_restrictive(PrivacyTier::Metadata),
            PrivacyTier::Metadata
        );
        assert_eq!(
            PrivacyTier::Metadata.most_restrictive(PrivacyTier::Local),
            PrivacyTier::Local
        );

        assert!(!PrivacyTier::Local.allows_row_values());
        assert!(!PrivacyTier::Metadata.allows_row_values());
        assert!(PrivacyTier::Sampled.allows_row_values());

        assert!(!PrivacyTier::Local.allows_remote_provider());
        assert!(PrivacyTier::Metadata.allows_remote_provider());
        assert!(PrivacyTier::Sampled.allows_remote_provider());
    }

    #[test]
    fn environments_are_ordered_from_least_to_most_restrictive() {
        assert!(Environment::Local < Environment::Development);
        assert!(Environment::Development < Environment::Staging);
        assert!(Environment::Staging < Environment::Production);
        assert_eq!(
            Environment::Local.max(Environment::Production),
            Environment::Production
        );
    }

    #[test]
    fn environment_parsing_and_rendering() {
        for env in [
            Environment::Local,
            Environment::Development,
            Environment::Staging,
            Environment::Production,
        ] {
            let relu: Environment = env.as_str().parse().expect("aller-retour");
            assert_eq!(env, relu);
        }
        assert_eq!(
            "prod"
                .parse::<Environment>()
                .expect("abbreviation accepted"),
            Environment::Production
        );
        assert!("recette".parse::<Environment>().is_err());
    }

    #[test]
    fn the_debug_leaks_no_parameter_value() {
        let cfg = ConnectionConfig::new("prod-eu", DriverId::postgres())
            .with_param("host", "db.interne.example")
            .with_param("sslmode", "require")
            .with_secret_ref("oxyn/connexion/prod-eu");

        let rendu = format!("{cfg:?}");

        assert!(
            !rendu.contains("db.interne.example"),
            "host leaked: {rendu}"
        );
        assert!(!rendu.contains("require"), "value leaked: {rendu}");
        assert!(
            !rendu.contains("oxyn/connexion/prod-eu"),
            "secret reference leaked: {rendu}"
        );
        assert!(
            !rendu.contains(&cfg.id.to_string()),
            "identifier leaked: {rendu}"
        );

        // What remains must remain useful for diagnosis.
        assert!(rendu.contains("prod-eu"), "the name is showable: {rendu}");
        assert!(rendu.contains("host"), "the keys are showable: {rendu}");
        assert!(rendu.contains("postgres"));
        // `Production` and not `production`: the derived `Debug` renders the
        // Rust variant name. Lowercase is the serde form
        // (`rename_all = "lowercase"`), which only applies to what is persisted.
        assert!(
            rendu.contains("Production"),
            "the environment must stay readable: {rendu}"
        );
    }

    #[test]
    fn what_is_persisted_only_contains_a_secret_reference() {
        // The only path for a secret is `secret_ref`, and it designates a
        // keychain entry — never the value. That is what keeps a workspace file
        // committed to a team repository from carrying away a production
        // password.
        let cfg = ConnectionConfig::new("prod-eu", DriverId::postgres())
            .with_param("host", "db.interne.example")
            .with_secret_ref("oxyn/connexion/prod-eu");

        let json = serde_json::to_string(&cfg).expect("serialization");

        assert!(!json.contains("password"), "suspicious field: {json}");
        assert!(
            json.contains("oxyn/connexion/prod-eu"),
            "the reference is persisted"
        );

        let relu: ConnectionConfig = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(relu, cfg, "the round trip must be faithful");
        assert_eq!(relu.secret_ref.as_deref(), Some("oxyn/connexion/prod-eu"));
    }

    #[test]
    fn parameter_order_is_kept() {
        let cfg = ConnectionConfig::new("x", DriverId::postgres())
            .with_param("host", "h")
            .with_param("port", "5432")
            .with_param("dbname", "d");
        let cles: Vec<_> = cfg.params.keys().map(String::as_str).collect();
        assert_eq!(cles, ["host", "port", "dbname"]);
    }

    #[test]
    fn a_read_only_connection_is_declared() {
        let cfg = ConnectionConfig::new("replica", DriverId::postgres()).read_only();
        assert!(cfg.read_only);
    }
}
