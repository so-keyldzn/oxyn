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

use oxyn_core::AgentId;

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

/// What defines an agent.
///
/// Serializable end to end: an agent fits in a file, and that file is readable
/// without Oxyn (I-11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}

/// Default value of [`AgentSpec::max_turns`] on deserialization.
const fn default_max_turns() -> usize {
    DEFAULT_MAX_TURNS
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
        }
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
    /// that is zero or beyond [`MAX_TURNS_CEILING`], a tool declared twice, or
    /// an output schema that is not a JSON object.
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
        if let Some(schema) = &self.output_schema
            && !schema.is_object()
        {
            return Err(AiError::InvalidSpec(
                "`output_schema` is not a JSON object".to_owned(),
            ));
        }
        for (index, tool) in self.allowed_tools.iter().enumerate() {
            if self
                .allowed_tools
                .iter()
                .take(index)
                .any(|seen| seen == tool)
            {
                return Err(AiError::InvalidSpec(format!(
                    "tool `{tool}` is declared twice"
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
mod tests {
    use crate::tools::{EXECUTE_QUERY, REFRESH_CATALOG};

    use super::*;

    fn spec() -> AgentSpec {
        AgentSpec::new(AgentId::new(), "SQL", "You write SQL.").with_tools([EXECUTE_QUERY])
    }

    #[test]
    fn a_minimal_agent_is_valid() {
        let registre = ToolRegistry::builtin();
        spec().validate(&registre).expect("valid declaration");
    }

    #[test]
    fn an_agent_without_tools_is_legal() {
        let registre = ToolRegistry::builtin();
        let sans_outil = AgentSpec::new(AgentId::new(), "Doc", "You describe schemas.");
        sans_outil.validate(&registre).expect("no tool granted");
        assert!(!sans_outil.allows(EXECUTE_QUERY));
    }

    #[test]
    fn a_declaration_cannot_invent_a_tool() {
        // A specification sometimes comes from a plugin: it restricts the
        // registry's list, it never extends it (PLUGIN-CONTRACT).
        let registre = ToolRegistry::builtin();
        let hostile = spec().with_tools(["drop_all_tables"]);
        let refus = hostile
            .validate(&registre)
            .expect_err("tool absent from the registry");
        assert!(matches!(refus, AiError::UnknownTool { .. }), "{refus:?}");
    }

    #[test]
    fn an_endless_loop_is_refused() {
        let registre = ToolRegistry::builtin();
        let refus = spec()
            .with_max_turns(MAX_TURNS_CEILING + 1)
            .validate(&registre)
            .expect_err("beyond the ceiling");
        assert!(matches!(refus, AiError::InvalidSpec(_)), "{refus:?}");

        let refus = spec()
            .with_max_turns(0)
            .validate(&registre)
            .expect_err("zero turns");
        assert!(matches!(refus, AiError::InvalidSpec(_)), "{refus:?}");
    }

    #[test]
    fn a_tool_declared_twice_is_refused() {
        let registre = ToolRegistry::builtin();
        let refus = spec()
            .with_tools([EXECUTE_QUERY, EXECUTE_QUERY])
            .validate(&registre)
            .expect_err("duplicate");
        assert!(matches!(refus, AiError::InvalidSpec(_)), "{refus:?}");
    }

    #[test]
    fn an_empty_prompt_is_refused() {
        let registre = ToolRegistry::builtin();
        let mut creux = spec();
        creux.system_prompt = "   ".to_owned();
        assert!(creux.validate(&registre).is_err());
    }

    #[test]
    fn an_agent_comes_from_a_file_without_a_line_of_rust() {
        // The property of ARCHITECTURE §7.3: an agent is a configuration.
        let json = r#"{
            "id": "0199a3c0-0000-7000-8000-0000000000ff",
            "name": "Reviewer",
            "description": "Reads schemas and comments on them.",
            "system_prompt": "You review database schemas.",
            "allowed_tools": ["execute_query", "refresh_catalog"],
            "max_turns": 4
        }"#;
        let lu = AgentSpec::from_json(json).expect("readable declaration");
        assert_eq!(lu.name, "Reviewer");
        assert_eq!(lu.max_turns, 4);
        assert!(lu.allows(REFRESH_CATALOG));
        assert_eq!(
            lu.context,
            ContextPolicy::default(),
            "missing fields take the cautious default"
        );
        lu.validate(&ToolRegistry::builtin()).expect("known tools");
    }

    #[test]
    fn a_declaration_reads_back_after_serialization() {
        let origine = spec()
            .with_description("writes SQL")
            .with_output_schema(serde_json::json!({"type": "object"}));
        let json = serde_json::to_string(&origine).expect("serialization");
        let relue = AgentSpec::from_json(&json).expect("deserialization");
        assert_eq!(relue, origine);
    }

    #[test]
    fn an_output_schema_that_is_not_an_object_is_refused() {
        let registre = ToolRegistry::builtin();
        let refus = spec()
            .with_output_schema(serde_json::json!("string"))
            .validate(&registre)
            .expect_err("a JSON schema is an object");
        assert!(matches!(refus, AiError::InvalidSpec(_)), "{refus:?}");
    }
}
