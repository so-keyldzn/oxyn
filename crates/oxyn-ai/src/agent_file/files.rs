//! The files shipped with Oxyn, embedded at build time.
//!
//! A table per axis rather than a directory read: the binary carries them, a
//! test parses each one (`shipped_agents_are_valid`), and a broken shipped file
//! therefore never reaches a user.

use oxyn_core::SqlDialect;

use super::parse::parse_agent_file;
use super::target::Recipient;
use crate::spec::AgentSpec;

/// The SQL agent's file, `(file, text)`.
pub(crate) const SQL_AGENT: (&str, &str) = ("agents/sql.md", include_str!("../../agents/sql.md"));

/// The Schema agent's file, `(file, text)`.
pub(crate) const SCHEMA_AGENT: (&str, &str) =
    ("agents/schema.md", include_str!("../../agents/schema.md"));

/// The shipped agents, in the order the picker offers them.
pub(super) const AGENTS: [(&str, &str); 2] = [SQL_AGENT, SCHEMA_AGENT];

/// The dialect fragments, `(dialect, file, text)`.
///
/// Written for the dialects Oxyn has or plans a driver for; every other one
/// falls back to [`ANSI`].
pub(super) const DIALECT_FRAGMENTS: [(&str, &str, &str); 6] = [
    ANSI,
    (
        "postgres",
        "prompts/dialects/postgres.md",
        include_str!("../../prompts/dialects/postgres.md"),
    ),
    (
        "redshift",
        "prompts/dialects/redshift.md",
        include_str!("../../prompts/dialects/redshift.md"),
    ),
    (
        "mysql",
        "prompts/dialects/mysql.md",
        include_str!("../../prompts/dialects/mysql.md"),
    ),
    (
        "sqlite",
        "prompts/dialects/sqlite.md",
        include_str!("../../prompts/dialects/sqlite.md"),
    ),
    (
        "duckdb",
        "prompts/dialects/duckdb.md",
        include_str!("../../prompts/dialects/duckdb.md"),
    ),
];

/// The fragment of a dialect Oxyn has no notes for: standard SQL, and the
/// rules of the `PolicyGate` that hold whatever the dialect.
const ANSI: (&str, &str, &str) = (
    "ansi",
    "prompts/dialects/ansi.md",
    include_str!("../../prompts/dialects/ansi.md"),
);

/// The recipient fragments, `(recipient, file, text)`: one per provider
/// protocol, one per external agent preset, and one for an external agent
/// declared by hand.
pub(super) const RECIPIENT_FRAGMENTS: [(&str, &str, &str); 7] = [
    (
        "anthropic",
        "prompts/recipients/anthropic.md",
        include_str!("../../prompts/recipients/anthropic.md"),
    ),
    (
        "openai",
        "prompts/recipients/openai.md",
        include_str!("../../prompts/recipients/openai.md"),
    ),
    (
        "gemini",
        "prompts/recipients/gemini.md",
        include_str!("../../prompts/recipients/gemini.md"),
    ),
    (
        "openai_compatible",
        "prompts/recipients/openai_compatible.md",
        include_str!("../../prompts/recipients/openai_compatible.md"),
    ),
    (
        "claude-code",
        "prompts/recipients/claude-code.md",
        include_str!("../../prompts/recipients/claude-code.md"),
    ),
    (
        "codex",
        "prompts/recipients/codex.md",
        include_str!("../../prompts/recipients/codex.md"),
    ),
    (
        "external",
        "prompts/recipients/external.md",
        include_str!("../../prompts/recipients/external.md"),
    ),
];

/// The agents shipped with Oxyn, in the order the picker offers them.
///
/// # Panics
/// Never in a build whose tests pass: the files are constants of the binary,
/// and `shipped_agents_are_valid` parses each of them.
#[must_use]
pub fn shipped_agents() -> Vec<AgentSpec> {
    AGENTS.into_iter().map(shipped_agent).collect()
}

/// One shipped agent, read from its embedded file.
///
/// The panic bears on a constant of the binary: its failure is a broken
/// shipped file, hence a programming bug that the tests catch, never input.
pub(crate) fn shipped_agent((file, text): (&str, &str)) -> AgentSpec {
    parse_agent_file(file, text)
        .unwrap_or_else(|err| panic!("the shipped agent file is valid: {err}"))
}

/// The fragment of `dialect`, `(file, text)`; `ansi.md` when none is written.
pub(super) fn dialect_fragment(dialect: SqlDialect) -> (&'static str, &'static str) {
    let (_, file, text) = DIALECT_FRAGMENTS
        .into_iter()
        .find(|(name, _, _)| *name == dialect.as_str())
        .unwrap_or(ANSI);
    (file, text)
}

/// The fragment of `recipient`, `(file, text)`.
pub(super) fn recipient_fragment(recipient: Recipient) -> Option<(&'static str, &'static str)> {
    RECIPIENT_FRAGMENTS
        .into_iter()
        .find(|(name, _, _)| *name == recipient.as_str())
        .map(|(_, file, text)| (file, text))
}
