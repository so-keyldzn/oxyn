use crate::tools::{EXECUTE_QUERY, REFRESH_CATALOG};

use super::*;

fn spec() -> AgentSpec {
    AgentSpec::new(AgentId::new(), "SQL", "You write SQL.").with_tools([EXECUTE_QUERY])
}

#[test]
fn a_minimal_agent_is_valid() {
    let registry = ToolRegistry::builtin();
    spec().validate(&registry).expect("valid declaration");
}

#[test]
fn an_agent_without_tools_is_legal() {
    let registry = ToolRegistry::builtin();
    let without_tool = AgentSpec::new(AgentId::new(), "Doc", "You describe schemas.");
    without_tool.validate(&registry).expect("no tool granted");
    assert!(!without_tool.allows(EXECUTE_QUERY));
}

#[test]
fn a_declaration_cannot_invent_a_tool() {
    // A specification sometimes comes from a plugin: it restricts the
    // registry's list, it never extends it (PLUGIN-CONTRACT).
    let registry = ToolRegistry::builtin();
    let hostile = spec().with_tools(["drop_all_tables"]);
    let refusal = hostile
        .validate(&registry)
        .expect_err("tool absent from the registry");
    assert!(
        matches!(refusal, AiError::UnknownTool { .. }),
        "{refusal:?}"
    );
}

#[test]
fn an_endless_loop_is_refused() {
    let registry = ToolRegistry::builtin();
    let refusal = spec()
        .with_max_turns(MAX_TURNS_CEILING + 1)
        .validate(&registry)
        .expect_err("beyond the ceiling");
    assert!(matches!(refusal, AiError::InvalidSpec(_)), "{refusal:?}");

    let refusal = spec()
        .with_max_turns(0)
        .validate(&registry)
        .expect_err("zero turns");
    assert!(matches!(refusal, AiError::InvalidSpec(_)), "{refusal:?}");
}

#[test]
fn a_tool_declared_twice_is_refused() {
    let registry = ToolRegistry::builtin();
    let refusal = spec()
        .with_tools([EXECUTE_QUERY, EXECUTE_QUERY])
        .validate(&registry)
        .expect_err("duplicate");
    assert!(matches!(refusal, AiError::InvalidSpec(_)), "{refusal:?}");
    // Its position, not its text: the list may come from a user's file.
    assert!(!refusal.to_string().contains(EXECUTE_QUERY), "{refusal}");
    assert!(refusal.to_string().contains("entry 2"), "{refusal}");
}

#[test]
fn each_context_bound_has_a_ceiling() {
    let registry = ToolRegistry::builtin();
    let base = ContextPolicy::default();
    let over = [
        (
            "max_context_tokens",
            ContextPolicy {
                max_context_tokens: MAX_CONTEXT_TOKENS_CEILING + 1,
                ..base.clone()
            },
        ),
        (
            "max_relations",
            ContextPolicy {
                max_relations: MAX_RELATIONS_CEILING + 1,
                ..base.clone()
            },
        ),
        (
            "max_fields_per_relation",
            ContextPolicy {
                max_fields_per_relation: MAX_FIELDS_PER_RELATION_CEILING + 1,
                ..base.clone()
            },
        ),
        (
            "max_sample_rows",
            ContextPolicy {
                max_sample_rows: MAX_SAMPLE_ROWS_CEILING + 1,
                ..base.clone()
            },
        ),
    ];
    for (field, context) in over {
        let refusal = spec()
            .with_context(context)
            .validate(&registry)
            .expect_err("over the ceiling");
        assert!(refusal.to_string().contains(field), "{refusal}");
    }

    let at_ceiling = ContextPolicy {
        max_context_tokens: MAX_CONTEXT_TOKENS_CEILING,
        max_relations: MAX_RELATIONS_CEILING,
        max_fields_per_relation: MAX_FIELDS_PER_RELATION_CEILING,
        max_sample_rows: MAX_SAMPLE_ROWS_CEILING,
        ..base
    };
    spec()
        .with_context(at_ceiling)
        .validate(&registry)
        .expect("each ceiling is allowed");
}

#[test]
fn the_shipped_agents_fit_under_the_ceilings() {
    let registry = ToolRegistry::builtin();
    for agent in crate::agent_file::shipped_agents() {
        agent
            .validate(&registry)
            .expect("a shipped agent validates");
    }
    assert_eq!(
        MAX_SAMPLE_ROWS_CEILING,
        usize::try_from(crate::tools::MAX_SAMPLE_ROWS).expect("a small bound")
    );
}

#[test]
fn an_empty_prompt_is_refused() {
    let registry = ToolRegistry::builtin();
    let mut blank = spec();
    blank.system_prompt = "   ".to_owned();
    assert!(blank.validate(&registry).is_err());
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
    let read_back = AgentSpec::from_json(json).expect("readable declaration");
    assert_eq!(read_back.name, "Reviewer");
    assert_eq!(read_back.max_turns, 4);
    assert!(read_back.allows(REFRESH_CATALOG));
    assert_eq!(
        read_back.context,
        ContextPolicy::default(),
        "missing fields take the cautious default"
    );
    read_back
        .validate(&ToolRegistry::builtin())
        .expect("known tools");
}

#[test]
fn a_declaration_reads_back_after_serialization() {
    let original = spec()
        .with_description("writes SQL")
        .with_output_schema(serde_json::json!({"type": "object"}));
    let json = serde_json::to_string(&original).expect("serialization");
    let reread = AgentSpec::from_json(&json).expect("deserialization");
    assert_eq!(reread, original);
}

#[test]
fn an_output_schema_that_is_not_an_object_is_refused() {
    let registry = ToolRegistry::builtin();
    let refusal = spec()
        .with_output_schema(serde_json::json!("string"))
        .validate(&registry)
        .expect_err("a JSON schema is an object");
    assert!(matches!(refusal, AiError::InvalidSpec(_)), "{refusal:?}");
}
