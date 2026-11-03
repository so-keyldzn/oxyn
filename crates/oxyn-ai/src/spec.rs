//! An agent's declaration — a file, not code.
//!
//! The agents of the vision (SQL, Schema, Performance, Migration, Security,
//! Documentation, Data Quality, Analytics, Visualization) are **configurations,
//! not separate implementations** (ARCHITECTURE §7.3): a system prompt, a
//! subset of tools, a context policy, an output schema. Adding an agent takes
//! no Rust code — that is what keeps the list manageable and what opens the
//! door to agents provided by plugins (PLUGIN-CONTRACT, phase 4).
//!
//! # A declaration is input, not trusted data
//!
//! An [`AgentSpec`] can come from a manifest written by a third party. It is
//! therefore validated before use ([`AgentSpec::validate`]) and it can, by
//! construction, grant nothing the tool registry does not already know:
//! `allowed_tools` **restricts**, it never extends. A declaration that names a
//! nonexistent tool is refused, not ignored.
//!
//! What the declaration cannot contain, and why:
//!
//! * **no connection or session** — they come from the
//!   [`ToolScope`](crate::tools::ToolScope), which the user determines by
//!   opening the conversation;
//! * **no privacy tier** — it is attached to the connection and never to
//!   anything else (ADR-0006, I-04). An agent that could declare its own tier
//!   would make the connection's setting inoperative;
//! * **no endpoint or key** — a plugin does not get a network channel by way of
//!   an agent (I-03).

use serde::{Deserialize, Serialize};

use oxyn_core::{AgentId, QueryLanguage, SqlDialect};

use crate::agent_file::{PromptTarget, Recipient, same_language};
use crate::context::ContextPolicy;
use crate::error::AiError;
use crate::tools::ToolRegistry;

/// Default number of turns.
///
/// Enough to read a schema, write a query, read its result and correct itself
/// once. Beyond that, a conversation that does not succeed costs tokens
/// without producing anything.
pub const DEFAULT_MAX_TURNS: usize = 8;

/// Absolute ceiling on the number of turns.
///
/// A declaration coming from a plugin must not be able to ask for a quasi
/// infinite loop: it is a bill, and on a remote provider, a bill the user
/// discovers afterwards.
pub const MAX_TURNS_CEILING: usize = 64;

/// Ceiling of [`ContextPolicy::max_context_tokens`].
///
/// The context is sent with every question, and a declaration read from a
/// file must not be able to make that a bill (as [`MAX_TURNS_CEILING`]).
/// Derived from the shipped agents: the Schema agent's 12,000 tokens is the
/// largest, and this leaves a user agent a little more than twice that —
/// about 128 KiB of schema text at `CHARS_PER_TOKEN` = 4.
pub const MAX_CONTEXT_TOKENS_CEILING: usize = 32_000;

/// Ceiling of [`ContextPolicy::max_relations`].
///
/// Derived from the shipped agents: the Schema agent's 60 is the largest.
/// Four times that is more relations than [`MAX_CONTEXT_TOKENS_CEILING`]
/// can describe, so a higher value would only make the gate select and
/// estimate relations it then drops.
pub const MAX_RELATIONS_CEILING: usize = 240;

/// Ceiling of [`ContextPolicy::max_fields_per_relation`].
///
/// Four times the default of 64, which no shipped agent changes: a wider
/// relation is described in part, and said so.
pub const MAX_FIELDS_PER_RELATION_CEILING: usize = 256;

/// Ceiling of [`ContextPolicy::max_sample_rows`]: the product bound of a
/// requested sample, [`MAX_SAMPLE_ROWS`](crate::tools::MAX_SAMPLE_ROWS).
/// Every row is a row that leaves the machine; a declaration does not get
/// more than the `request_sample` tool does.
pub const MAX_SAMPLE_ROWS_CEILING: usize = crate::tools::MAX_SAMPLE_ROWS as usize;

/// What defines an agent.
///
/// Serializable end to end: an agent fits in a file, and that file is readable
/// without Oxyn (I-11).
///
/// `Debug` is written by hand: a declaration can come from a user's file, and
/// its prompt and description are third-party text a log must not copy
/// (I-03). It prints the id and counts.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    /// The role, stable from one session to the next. It is what the audit log
    /// records next to every command this agent emits.
    pub id: AgentId,

    /// Displayable name.
    pub name: String,

    /// What the agent does, for the user who chooses it. **Not sent to the
    /// model**: it is [`system_prompt`](Self::system_prompt) that addresses
    /// it.
    #[serde(default)]
    pub description: String,

    /// The system prompt. In English: it is code text.
    pub system_prompt: String,

    /// The granted tools, by their name in the
    /// [`crate::tools::ToolRegistry`].
    ///
    /// An empty list is legal and means **no tool**: an agent that only
    /// comments on a schema has nothing to execute, and granting it a tool
    /// "just in case" widens the surface for nothing.
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// How much schema this agent needs to see, and in what form.
    #[serde(default)]
    pub context: ContextPolicy,

    /// JSON schema of the expected answer, when the agent must produce a
    /// structure and not prose.
    ///
    /// Purely declarative at this stage: it is the caller who decides how to
    /// enforce it, because not all providers can constrain an output.
    /// `// TODO(phase 4)`: pass it to the provider when `oxyn-llm` exposes a
    /// response format field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,

    /// Maximum number of model → tools → model round trips.
    #[serde(default = "default_max_turns")]
    pub max_turns: usize,

    /// The query languages this agent writes; `[sql]` when a declaration
    /// says nothing, so that every agent written before the field behaves
    /// as it did (ADR-0065).
    ///
    /// Narrows where the agent is offered, as
    /// [`applies_to`](Self::applies_to) does, and grants nothing: the tools,
    /// the `PolicyGate` and the tier are the same whatever the language.
    /// Written as [`QueryLanguage::as_str`] names; the dialect of a `Sql`
    /// entry is never read. Never empty once [`validate`](Self::validate)d.
    #[serde(
        default = "sql_only",
        with = "crate::agent_file::language_names",
        skip_serializing_if = "is_sql_only"
    )]
    pub languages: Vec<QueryLanguage>,

    /// The SQL dialects this agent is written for; empty means every one.
    ///
    /// Narrows where the agent is **offered** ([`offered_for`](Self::offered_for));
    /// it grants nothing (ADR-0049 § 2). Written as [`SqlDialect::as_str`]
    /// names, as in a file. Only read on a SQL connection: a list here
    /// requires `sql` among [`languages`](Self::languages).
    #[serde(default, with = "crate::agent_file::dialect_names")]
    pub applies_to: Vec<SqlDialect>,

    /// The recipients this agent is written for; empty means every one.
    #[serde(default)]
    pub recipients: Vec<Recipient>,
}

/// Default value of [`AgentSpec::max_turns`] on deserialization.
const fn default_max_turns() -> usize {
    DEFAULT_MAX_TURNS
}

/// Default value of [`AgentSpec::languages`]: what every agent wrote before
/// the field existed.
pub(crate) fn sql_only() -> Vec<QueryLanguage> {
    vec![QueryLanguage::SQL]
}

/// Is `languages` the default? Then it is not written out, and a declaration
/// serializes as it did before the field (ADR-0065).
fn is_sql_only(languages: &[QueryLanguage]) -> bool {
    matches!(languages, [only] if same_language(*only, QueryLanguage::SQL))
}

impl std::fmt::Debug for AgentSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSpec")
            .field("id", &self.id)
            .field("prompt_chars", &self.system_prompt.chars().count())
            .field("tools", &self.allowed_tools.len())
            .field("max_turns", &self.max_turns)
            .field("languages", &self.languages.len())
            .field("applies_to", &self.applies_to.len())
            .field("recipients", &self.recipients.len())
            .finish_non_exhaustive()
    }
}

impl AgentSpec {
    /// Declares a minimal agent: an identifier, a name, a prompt.
    ///
    /// Without tools: granting them is an explicit act.
    #[must_use]
    pub fn new(id: AgentId, name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: String::new(),
            system_prompt: system_prompt.into(),
            allowed_tools: Vec::new(),
            context: ContextPolicy::default(),
            output_schema: None,
            max_turns: DEFAULT_MAX_TURNS,
            languages: sql_only(),
            applies_to: Vec::new(),
            recipients: Vec::new(),
        }
    }

    /// Is this agent offered for `target`? Only if
    /// [`languages`](Self::languages) names the target's language, and each
    /// of [`applies_to`](Self::applies_to) — read on a SQL target only — and
    /// [`recipients`](Self::recipients) is empty or names the target's value.
    #[must_use]
    pub fn offered_for(&self, target: &PromptTarget) -> bool {
        self.writes(target.language)
            && target.dialect().is_none_or(|dialect| {
                self.applies_to.is_empty() || self.applies_to.contains(&dialect)
            })
            && (self.recipients.is_empty() || self.recipients.contains(&target.recipient))
    }

    /// Does this agent write `language`, whatever its SQL dialect?
    #[must_use]
    pub fn writes(&self, language: QueryLanguage) -> bool {
        self.languages
            .iter()
            .any(|written| same_language(*written, language))
    }

    /// Sets the query languages this agent writes.
    #[must_use]
    pub fn with_languages(mut self, languages: impl IntoIterator<Item = QueryLanguage>) -> Self {
        self.languages = languages.into_iter().collect();
        self
    }

    /// Sets the description shown to the user.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Grants tools, by their name.
    #[must_use]
    pub fn with_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_tools = tools.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the context policy.
    #[must_use]
    pub fn with_context(mut self, context: ContextPolicy) -> Self {
        self.context = context;
        self
    }

    /// Sets the expected output schema.
    #[must_use]
    pub fn with_output_schema(mut self, schema: serde_json::Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    /// Sets the maximum number of turns.
    #[must_use]
    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// Reads a declaration written outside Oxyn.
    ///
    /// Only validates the **shape**: then call [`validate`](Self::validate)
    /// with the real tool registry.
    ///
    /// # Errors
    /// [`AiError::InvalidSpec`] if the text is not a JSON `AgentSpec`.
    pub fn from_json(text: &str) -> Result<Self, AiError> {
        serde_json::from_str(text).map_err(|err| AiError::InvalidSpec(err.to_string()))
    }

    /// Checks that the declaration is consistent and that its tools exist.
    ///
    /// Called by [`AgentRuntime::new`](crate::runtime::AgentRuntime::new): an
    /// invalid agent must not be able to start a conversation, because the
    /// failure would then show at the first tool call, several paid requests
    /// later.
    ///
    /// # Errors
    /// [`AiError::InvalidSpec`] for an empty name or prompt, a number of turns
    /// that is zero or beyond [`MAX_TURNS_CEILING`], a context policy beyond
    /// one of its ceilings ([`MAX_CONTEXT_TOKENS_CEILING`] and the others), a
    /// tool declared twice — named by its position, never by its text —, an
    /// output schema that is not a JSON object, an empty `languages`, or
    /// `applies_to` dialects on an agent that does not write SQL.
    /// [`AiError::UnknownTool`] for a tool the registry does not know.
    pub fn validate(&self, registry: &ToolRegistry) -> Result<(), AiError> {
        if self.name.trim().is_empty() {
            return Err(AiError::InvalidSpec("`name` is empty".to_owned()));
        }
        if self.system_prompt.trim().is_empty() {
            return Err(AiError::InvalidSpec("`system_prompt` is empty".to_owned()));
        }
        if self.max_turns == 0 {
            return Err(AiError::InvalidSpec(
                "`max_turns` is zero: the agent could never answer".to_owned(),
            ));
        }
        if self.max_turns > MAX_TURNS_CEILING {
            return Err(AiError::InvalidSpec(format!(
                "`max_turns` exceeds the ceiling of {MAX_TURNS_CEILING}"
            )));
        }
        if self.languages.is_empty() {
            return Err(AiError::InvalidSpec(
                "`languages` is empty: the agent would be offered on no connection".to_owned(),
            ));
        }
        if !self.applies_to.is_empty() && !self.writes(QueryLanguage::SQL) {
            return Err(AiError::InvalidSpec(
                "`applies_to` names SQL dialects, and `languages` does not list `sql`".to_owned(),
            ));
        }
        if let Some(schema) = &self.output_schema
            && !schema.is_object()
        {
            return Err(AiError::InvalidSpec(
                "`output_schema` is not a JSON object".to_owned(),
            ));
        }
        let context = &self.context;
        for (field, value, ceiling) in [
            (
                "max_context_tokens",
                context.max_context_tokens,
                MAX_CONTEXT_TOKENS_CEILING,
            ),
            (
                "max_relations",
                context.max_relations,
                MAX_RELATIONS_CEILING,
            ),
            (
                "max_fields_per_relation",
                context.max_fields_per_relation,
                MAX_FIELDS_PER_RELATION_CEILING,
            ),
            (
                "max_sample_rows",
                context.max_sample_rows,
                MAX_SAMPLE_ROWS_CEILING,
            ),
        ] {
            if value > ceiling {
                return Err(AiError::InvalidSpec(format!(
                    "`context.{field}` exceeds the ceiling of {ceiling}"
                )));
            }
        }
        for (index, tool) in self.allowed_tools.iter().enumerate() {
            if self
                .allowed_tools
                .iter()
                .take(index)
                .any(|seen| seen == tool)
            {
                // Not the name: a declaration can come from a user's file,
                // and its text never reaches a message (ADR-0049 § 1).
                return Err(AiError::InvalidSpec(format!(
                    "`tools` lists the same tool twice (entry {})",
                    index.saturating_add(1)
                )));
            }
            if !registry.contains(tool) {
                return Err(AiError::UnknownTool { name: tool.clone() });
            }
        }
        Ok(())
    }

    /// Is this tool granted to this agent?
    #[must_use]
    pub fn allows(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|name| name == tool)
    }
}

#[cfg(test)]
mod tests;
