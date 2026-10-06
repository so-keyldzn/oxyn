//! What a prompt is written for: the connection's dialect, and who reads it.

use std::str::FromStr;

use oxyn_core::{AiProviderKind, DriverId, Environment, SqlDialect};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::external::presets::{CLAUDE_CODE, CODEX};

/// Every dialect, in the order of [`SqlDialect`]'s declaration.
///
/// `SqlDialect` is `#[non_exhaustive]` and lives in `oxyn-core`, so no `match`
/// here can be exhaustive: this list is how a name read from a file finds its
/// dialect. A dialect missing from it can only be named by no file — it still
/// gets a prompt, through the `ansi` fallback.
pub const DIALECTS: [SqlDialect; 11] = [
    SqlDialect::Ansi,
    SqlDialect::Postgres,
    SqlDialect::MySql,
    SqlDialect::Sqlite,
    SqlDialect::SqlServer,
    SqlDialect::Oracle,
    SqlDialect::ClickHouse,
    SqlDialect::DuckDb,
    SqlDialect::Snowflake,
    SqlDialect::BigQuery,
    SqlDialect::Redshift,
];

/// The dialect whose [`SqlDialect::as_str`] is `name`.
///
/// Not `SqlDialect`'s serde form: that one writes `my_sql` and `duck_db`,
/// where a file author writes what the rest of Oxyn shows — `mysql`, `duckdb`.
pub(crate) fn dialect_named(name: &str) -> Option<SqlDialect> {
    DIALECTS
        .into_iter()
        .find(|dialect| dialect.as_str() == name)
}

/// An external agent, as far as its prompt is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExternalAgentKind {
    /// Claude Code, through its ACP adapter, confined at launch (ADR-0032).
    ClaudeCode,
    /// Codex, through its ACP adapter, confined at launch (ADR-0032).
    Codex,
    /// An agent the user declared by hand: Oxyn has no preset for it and
    /// cannot confine it, so its prompt must say so.
    Other,
}

impl ExternalAgentKind {
    /// Every kind: the presets in their order, then [`Other`](Self::Other).
    pub const ALL: [Self; 3] = [Self::ClaudeCode, Self::Codex, Self::Other];

    /// The kind of the preset with this identifier; [`Other`](Self::Other)
    /// for any identifier that is not a preset's — a declaration without a
    /// preset is one Oxyn did not confine.
    #[must_use]
    pub fn from_preset_id(id: &str) -> Self {
        [Self::ClaudeCode, Self::Codex]
            .into_iter()
            .find(|kind| kind.as_str() == id)
            .unwrap_or(Self::Other)
    }

    /// The name of its fragment: the preset's identifier, or `external`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => CLAUDE_CODE.id,
            Self::Codex => CODEX.id,
            Self::Other => "external",
        }
    }
}

/// Who receives the prompt: a provider's protocol, or an external agent.
///
/// The protocol and not the model: what changes how a prompt must be written is
/// how tools reach the model and what else it was told, and a small local
/// model behind an OpenAI-compatible endpoint needs a plainer prompt than any
/// hosted one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Recipient {
    /// A provider Oxyn calls itself.
    Provider(AiProviderKind),
    /// An external agent process (ADR-0026).
    External(ExternalAgentKind),
}

impl Recipient {
    /// The stable name, written in files: `anthropic`, `openai`, `gemini`,
    /// `openai_compatible`, `claude-code`, `codex`, `external`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider(kind) => kind.as_str(),
            Self::External(kind) => kind.as_str(),
        }
    }

    /// The recipient whose [`as_str`](Self::as_str) is exactly `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        if let Some(kind) = ExternalAgentKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == name)
        {
            return Some(Self::External(kind));
        }
        AiProviderKind::from_str(name)
            .ok()
            .filter(|kind| kind.as_str() == name)
            .map(Self::Provider)
    }
}

impl Serialize for Recipient {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Recipient {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::named(&name).ok_or_else(|| serde::de::Error::custom("unknown recipient"))
    }
}

/// The serde form of a list of dialects: their [`SqlDialect::as_str`] names,
/// the ones a file carries.
pub(crate) mod dialect_names {
    use oxyn_core::SqlDialect;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(
        dialects: &[SqlDialect],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(dialects.len()))?;
        for dialect in dialects {
            seq.serialize_element(dialect.as_str())?;
        }
        seq.end()
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<SqlDialect>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .iter()
            .map(|name| {
                super::dialect_named(name)
                    .ok_or_else(|| serde::de::Error::custom("unknown dialect"))
            })
            .collect()
    }
}

/// Everything a prompt's variables can be filled from.
///
/// Four values Oxyn holds, from closed sets. None of them is written by the
/// database or its server: database content reaches the model through
/// `ContextBuilder` only (I-04, ADR-0049 § 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTarget {
    /// The connection's dialect: `{{dialect}}`, and the fragment chosen.
    pub dialect: SqlDialect,
    /// The connection's driver: `{{driver}}`.
    pub driver: DriverId,
    /// The connection's marking: `{{environment}}`.
    pub environment: Environment,
    /// Who reads the prompt: `{{recipient}}`, and the fragment chosen.
    pub recipient: Recipient,
}
