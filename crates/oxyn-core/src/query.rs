//! What is asked of a database: language, intent, risk, limits.
//!
//! SQL is **one case among others**, not the default the others reduce to
//! (ADR-0003). A query therefore always carries an explicit
//! [`QueryLanguage`]; a driver that receives a language it does not declare in
//! its capabilities refuses, it does not translate.
//!
//! [`StatementIntent`] and [`MutationRisk`] are the two inputs of the
//! `PolicyGate`. They are **declared** by whoever builds the query — the
//! `oxyn-query` analyzer, or the caller — and the gate trusts them. Hence the
//! rule that governs this module: when in doubt, declare the most restrictive.
//! [`StatementIntent::Unknown`] counts as mutating.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::value::ScalarValue;

/// Language a query is written in.
///
/// The enumeration is closed: these values are domain vocabulary, and a
/// language is also declared as a capability
/// ([`Capabilities`](crate::capabilities::Capabilities)).
///
// TODO(phase 4): WASM plugins may bring a language absent from this list
// (PLUGIN-CONTRACT). That will be an ADR decision, not an `Other(String)`
// added along the way: a language without a matching capability has no way of
// being refused cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QueryLanguage {
    /// SQL, in a given dialect.
    Sql(SqlDialect),
    /// Cypher (Neo4j, Memgraph).
    Cypher,
    /// Gremlin (TinkerPop).
    Gremlin,
    /// Document query language (MongoDB).
    MongoQuery,
    /// Redis commands.
    RedisCommand,
    /// Query DSL (Elasticsearch, OpenSearch).
    SearchDsl,
    /// CQL (Cassandra).
    Cql,
    /// PartiQL (DynamoDB).
    PartiQl,
    /// InfluxQL (InfluxDB 1.x).
    InfluxQl,
    /// Flux (InfluxDB 2.x).
    Flux,
}

impl QueryLanguage {
    /// ANSI SQL, without a particular dialect.
    pub const SQL: Self = Self::Sql(SqlDialect::Ansi);

    /// Stable name, usable in a log or an interface.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Sql(_) => "sql",
            Self::Cypher => "cypher",
            Self::Gremlin => "gremlin",
            Self::MongoQuery => "mongo",
            Self::RedisCommand => "redis",
            Self::SearchDsl => "search-dsl",
            Self::Cql => "cql",
            Self::PartiQl => "partiql",
            Self::InfluxQl => "influxql",
            Self::Flux => "flux",
        }
    }

    /// The SQL dialect, if it is SQL.
    #[must_use]
    pub const fn sql_dialect(&self) -> Option<SqlDialect> {
        match self {
            Self::Sql(d) => Some(*d),
            _ => None,
        }
    }
}

impl std::fmt::Display for QueryLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(d) => write!(f, "sql/{d}"),
            other => f.write_str(other.as_str()),
        }
    }
}

/// SQL dialect.
///
/// A dialect is not a product: Redshift speaks the PostgreSQL protocol but does
/// not accept the same grammar, hence a distinct value here although there is
/// **no** distinct driver crate (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SqlDialect {
    /// Standard SQL, without proprietary extension.
    #[default]
    Ansi,
    /// PostgreSQL.
    Postgres,
    /// MySQL and MariaDB.
    MySql,
    /// SQLite.
    Sqlite,
    /// Microsoft SQL Server (T-SQL).
    SqlServer,
    /// Oracle (PL/SQL).
    Oracle,
    /// ClickHouse.
    ClickHouse,
    /// DuckDB.
    DuckDb,
    /// Snowflake.
    Snowflake,
    /// Google BigQuery.
    BigQuery,
    /// Amazon Redshift.
    Redshift,
}

impl SqlDialect {
    /// Stable name of the dialect.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ansi => "ansi",
            Self::Postgres => "postgres",
            Self::MySql => "mysql",
            Self::Sqlite => "sqlite",
            Self::SqlServer => "sqlserver",
            Self::Oracle => "oracle",
            Self::ClickHouse => "clickhouse",
            Self::DuckDb => "duckdb",
            Self::Snowflake => "snowflake",
            Self::BigQuery => "bigquery",
            Self::Redshift => "redshift",
        }
    }
}

impl std::fmt::Display for SqlDialect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a statement does, from the database's point of view.
///
/// The enumeration is **closed**, on purpose, whereas the repository's
/// convention is to mark public enumerations `#[non_exhaustive]`. The
/// `PolicyGate` matrix reads cell by cell over these five values: adding an
/// intent must force every `match` to be revisited, hence be a visible break
/// rather than a `_ =>` that decides silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementIntent {
    /// Read-only: `SELECT`, `EXPLAIN`, introspection.
    Read,
    /// Data write: `INSERT`, `UPDATE`, `DELETE`, `MERGE`.
    Write,
    /// Structure change: `CREATE`, `ALTER`, `DROP`, `TRUNCATE`.
    Ddl,
    /// Privilege change: `GRANT`, `REVOKE`, role management.
    Grant,
    /// Undetermined intent. It is the default value, and it counts as
    /// **mutating**: a statement no analyzer could classify may write, and the
    /// only safe choice is to treat it as such.
    #[default]
    Unknown,
}

impl StatementIntent {
    /// Can the statement modify something?
    ///
    /// [`Unknown`](Self::Unknown) answers `true`. It is not a convenient
    /// approximation: it is the only answer that does not let an unrecognized
    /// write through (I-02).
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        !matches!(self, Self::Read)
    }

    /// Is the statement certainly read-only?
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        matches!(self, Self::Read)
    }

    /// Stable name, for the audit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Ddl => "ddl",
            Self::Grant => "grant",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for StatementIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Mutation form whose scope is not bounded by the statement.
///
/// It is not a rough severity measure: each variant matches a recognizable
/// syntactic form whose effect is not limited to the rows the user believes
/// they target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MutationRisk {
    /// Nothing in particular: bounded mutation, or read.
    #[default]
    None,
    /// `UPDATE` without `WHERE`: every row of the table.
    UnboundedUpdate,
    /// `DELETE` without `WHERE`: every row of the table.
    UnboundedDelete,
    /// `TRUNCATE`: emptying, often unlogged and not undoable.
    Truncate,
    /// `DROP` of an object: the structure goes with the data.
    DropObject,
}

impl MutationRisk {
    /// Is there a risk to report?
    #[must_use]
    pub const fn is_some(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Approval reason, showable as is to the user.
    #[must_use]
    pub const fn reason(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::UnboundedUpdate => Some("UPDATE without a WHERE clause: every row"),
            Self::UnboundedDelete => Some("DELETE without a WHERE clause: every row"),
            Self::Truncate => Some("TRUNCATE: the table is emptied"),
            Self::DropObject => Some("DROP: the object and its data are removed"),
        }
    }
}

impl std::fmt::Display for MutationRisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason().unwrap_or("no risk reported"))
    }
}

/// Bounds imposed on an execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecLimits {
    /// Number of rows beyond which the result is truncated. `None` = no bound,
    /// which is only reasonable for an export.
    pub max_rows: Option<usize>,
    /// Delay after which the execution is abandoned **and cancelled on the
    /// server side**. A delay that only drops the future leaves the query
    /// running (DRIVER-CONTRACT §2).
    pub timeout: Option<Duration>,
    /// Forbids any write for this execution.
    pub read_only: bool,
}

impl ExecLimits {
    /// Default timeout.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
    /// Default number of rows.
    pub const DEFAULT_MAX_ROWS: usize = 10_000;

    /// Limits with no bound or protection: to be kept for exports and batch
    /// processing, never for an execution triggered by a click.
    #[must_use]
    pub const fn unbounded() -> Self {
        Self {
            max_rows: None,
            timeout: None,
            read_only: false,
        }
    }

    /// Allows writing.
    #[must_use]
    pub fn writable(mut self) -> Self {
        self.read_only = false;
        self
    }

    /// Replaces the maximum number of rows.
    #[must_use]
    pub fn with_max_rows(mut self, max_rows: impl Into<Option<usize>>) -> Self {
        self.max_rows = max_rows.into();
        self
    }

    /// Replaces the timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.timeout = timeout.into();
        self
    }
}

impl Default for ExecLimits {
    /// The default is **cautious**, not convenient: bounded in rows, bounded
    /// in time, and read-only.
    ///
    /// As for connection marking (SECURITY), the default is the most
    /// restrictive value. A write is always something the caller asked for
    /// explicitly.
    fn default() -> Self {
        Self {
            max_rows: Some(Self::DEFAULT_MAX_ROWS),
            timeout: Some(Self::DEFAULT_TIMEOUT),
            read_only: true,
        }
    }
}

/// A complete execution request.
///
/// The `Debug` **and** the serialization mask bound parameters. The query text
/// is kept — it is its *shape*, and observability explicitly allows it — but
/// bound values are exactly what I-03 forbids logging **and** writing to a
/// workspace file.
///
/// The protection covers both because the failure would happen with the first
/// one to persist a `Command` — session resumption, durable queue, plugin
/// bridge — without the asymmetry between a protected `Debug` and a derived
/// `Serialize` showing up at compile time or in review. A request read back
/// from a file therefore comes back **without its values**: the server will
/// refuse it, which is the right failure — much safer than finding a password
/// pasted in the wrong field written in clear on disk.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecRequest {
    /// Language of the query.
    pub language: QueryLanguage,
    /// Text of the query, as the user or the caller wrote it.
    pub text: String,
    /// Bound parameters, in placeholder order.
    ///
    /// Never serialized: see the type's documentation.
    #[serde(skip)]
    pub params: Vec<ScalarValue>,
    /// Declared intent.
    pub intent: StatementIntent,
    /// Declared risk.
    pub risk: MutationRisk,
    /// Execution bounds.
    pub limits: ExecLimits,
    /// Whether the text begins, commits, rolls back or marks a transaction
    /// (`BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `RELEASE`…).
    ///
    /// Such a statement reads nothing and writes nothing of its own, so its
    /// intent is `Read`; yet its effect lands on whatever the session already
    /// holds — a transaction the user opened. Like the intent, it is replaced
    /// by the executor's reclassification, never taken from the caller.
    #[serde(default)]
    pub transaction_control: bool,
}

impl ExecRequest {
    /// Builds a request with the cautious defaults: intent
    /// [`Unknown`](StatementIntent::Unknown), no risk reported, default limits.
    ///
    /// Since the default intent is mutating, an unqualified request goes
    /// through approval rather than executing silently.
    #[must_use]
    pub fn new(language: QueryLanguage, text: impl Into<String>) -> Self {
        Self {
            language,
            text: text.into(),
            params: Vec::new(),
            intent: StatementIntent::Unknown,
            risk: MutationRisk::None,
            limits: ExecLimits::default(),
            transaction_control: false,
        }
    }

    /// Declares the intent.
    #[must_use]
    pub fn with_intent(mut self, intent: StatementIntent) -> Self {
        self.intent = intent;
        self
    }

    /// Declares the risk.
    #[must_use]
    pub fn with_risk(mut self, risk: MutationRisk) -> Self {
        self.risk = risk;
        self
    }

    /// Sets the bound parameters.
    #[must_use]
    pub fn with_params(mut self, params: Vec<ScalarValue>) -> Self {
        self.params = params;
        self
    }

    /// Sets the limits.
    #[must_use]
    pub fn with_limits(mut self, limits: ExecLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Can the request modify something?
    ///
    /// True as soon as the intent is mutating **or** a risk is reported: an
    /// inconsistent declaration (intent `Read` and risk `Truncate`) is settled
    /// on the cautious side.
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        self.intent.is_mutating() || self.risk.is_some()
    }
}

/// Display wrapper: renders a count, never the values.
struct Masque(usize);

impl std::fmt::Debug for Masque {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} redacted bound value(s)>", self.0)
    }
}

impl std::fmt::Debug for ExecRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecRequest")
            .field("language", &self.language)
            .field("text", &self.text)
            .field("params", &Masque(self.params.len()))
            .field("intent", &self.intent)
            .field("risk", &self.risk)
            .field("limits", &self.limits)
            .field("transaction_control", &self.transaction_control)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_in_doubt_it_is_mutating() {
        assert!(StatementIntent::Unknown.is_mutating());
        assert!(!StatementIntent::Unknown.is_read_only());
        assert!(StatementIntent::default().is_mutating());
    }

    #[test]
    fn only_read_is_not_mutating() {
        assert!(!StatementIntent::Read.is_mutating());
        for intent in [
            StatementIntent::Write,
            StatementIntent::Ddl,
            StatementIntent::Grant,
            StatementIntent::Unknown,
        ] {
            assert!(intent.is_mutating(), "{intent} should be mutating");
        }
    }

    #[test]
    fn default_limits_are_cautious() {
        let limites = ExecLimits::default();
        assert!(limites.read_only, "the default must forbid writing");
        assert!(
            limites.max_rows.is_some(),
            "the default must bound the rows"
        );
        assert!(limites.timeout.is_some(), "the default must bound the time");
    }

    #[test]
    fn unbounded_limits_are_explicit() {
        let limites = ExecLimits::unbounded();
        assert_eq!(limites.max_rows, None);
        assert_eq!(limites.timeout, None);
        assert!(!limites.read_only);
    }

    #[test]
    fn an_unqualified_request_is_mutating() {
        let demande = ExecRequest::new(QueryLanguage::SQL, "SELECT 1");
        assert!(
            demande.is_mutating(),
            "without a declared intent, we protect"
        );
    }

    #[test]
    fn an_inconsistent_declaration_is_settled_on_the_cautious_side() {
        let demande = ExecRequest::new(QueryLanguage::SQL, "TRUNCATE t")
            .with_intent(StatementIntent::Read)
            .with_risk(MutationRisk::Truncate);
        assert!(
            demande.is_mutating(),
            "a reported risk prevails over a read intent"
        );
    }

    #[test]
    fn the_debug_masks_bound_values() {
        let demande = ExecRequest::new(QueryLanguage::SQL, "SELECT * FROM users WHERE ssn = $1")
            .with_params(vec![ScalarValue::Text("123-45-6789".into())]);
        let rendu = format!("{demande:?}");
        assert!(
            !rendu.contains("123-45-6789"),
            "a bound value leaked into the Debug: {rendu}"
        );
        assert!(rendu.contains("1 redacted bound value(s)"));
        assert!(
            rendu.contains("SELECT * FROM users"),
            "the shape of the query stays loggable"
        );
    }

    #[test]
    fn the_language_renders_with_its_dialect() {
        assert_eq!(
            QueryLanguage::Sql(SqlDialect::Postgres).to_string(),
            "sql/postgres"
        );
        assert_eq!(QueryLanguage::Cypher.to_string(), "cypher");
        assert_eq!(
            QueryLanguage::Sql(SqlDialect::Redshift).sql_dialect(),
            Some(SqlDialect::Redshift)
        );
        assert_eq!(QueryLanguage::Flux.sql_dialect(), None);
    }

    #[test]
    fn every_risk_carries_a_showable_reason() {
        for risque in [
            MutationRisk::UnboundedUpdate,
            MutationRisk::UnboundedDelete,
            MutationRisk::Truncate,
            MutationRisk::DropObject,
        ] {
            assert!(risque.is_some());
            assert!(risque.reason().is_some(), "{risque:?} without a reason");
        }
        assert!(!MutationRisk::None.is_some());
        assert!(MutationRisk::None.reason().is_none());
    }
}
