use oxyn_core::{MutationRisk, StatementIntent};
use serde_json::json;

use super::*;

fn scope() -> ToolScope {
    ToolScope::new(ConnectionId::new(), SessionId::new(), QueryLanguage::SQL)
}

fn make_call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall::new("call_1", name, args)
}

fn all_command_names() -> Vec<String> {
    ToolRegistry::builtin()
        .names()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_tool_produces_a_core_command() {
    // I-01: there is no second API for the AI. Each tool names the
    // `Command` variant it produces, and that variant exists.
    let command_names = [
        "Connect",
        "Disconnect",
        "Execute",
        "PreviewRelation",
        "Cancel",
        "RefreshCatalog",
        "DescribeCatalog",
        "Export",
        "OpenDocument",
        "WriteDocument",
        "CreateConnection",
        "UpdateConnection",
        "DeleteConnection",
    ];
    for tool in &ToolRegistry::builtin().tools {
        assert!(
            command_names.contains(&tool.command()),
            "the tool \"{}\" claims to produce \"{}\", which is not a Command",
            tool.name(),
            tool.command()
        );
    }
}

#[test]
fn connection_management_is_not_a_tool() {
    // An agent that could create a connection to the host of its choice
    // would have an exfiltration channel. These commands have no tool:
    // that is stronger than a PolicyGate refusal.
    let registry = ToolRegistry::builtin();
    for forbidden in [
        "create_connection",
        "update_connection",
        "delete_connection",
        "export",
        "cancel",
    ] {
        assert!(!registry.contains(forbidden), "{forbidden}");
    }
    for tool in &registry.tools {
        assert!(
            !tool.command().contains("Connection"),
            "{} produces {}",
            tool.name(),
            tool.command()
        );
        assert_ne!(tool.command(), "Export");
    }
}

/// Schema change is not a tool, and must not become one.
///
/// [ADR-0025](../../../docs/adr/0025-proposition-de-changement-de-schema.md)
/// writes that `Propose change…` is **unavailable** to an `Actor::Agent`,
/// and not "confirmable" — that is [I-02](../../../CLAUDE.md#i-02) to the
/// word, which names the stronger confirmation as insufficient.
///
/// This guarantee held by **absence of path**: no tool reaches the act.
/// That is stronger than a refusal, but nothing would have kept it true — a
/// divergence survey reported it on 2026-09-14. This test holds it: the day
/// someone exposes a structure tool, it fails and forces reopening the ADR
/// rather than contradicting it silently.
#[test]
fn schema_change_is_not_a_tool() {
    let registry = ToolRegistry::builtin();
    for forbidden in [
        "propose_change",
        "alter_table",
        "create_table",
        "drop_table",
        "apply_ddl",
    ] {
        assert!(!registry.contains(forbidden), "{forbidden}");
    }
    // And by the produced command, so that renaming the tool is not enough
    // to slip through.
    for tool in &registry.tools {
        for marker in ["Ddl", "Alter", "Schema", "Propose"] {
            assert!(
                !tool.command().contains(marker),
                "{} produces {}, which touches the schema",
                tool.name(),
                tool.command()
            );
        }
    }
}

/// ADR-0042: `Drop…`, `Truncate…` and `Rename…` from the catalog are human
/// actions. An agent that wants a table gone writes SQL like anyone, and
/// the gate refuses it on production; no tool reaches the review.
#[test]
fn object_operations_are_not_tools() {
    let registry = ToolRegistry::builtin();
    for forbidden in [
        "review_object_operation",
        "run_object_operation",
        "drop_object",
        "truncate_table",
        "rename_object",
        "rename_table",
    ] {
        assert!(!registry.contains(forbidden), "{forbidden}");
    }
    for tool in &registry.tools {
        for marker in ["Drop", "Truncate", "Rename", "ObjectOperation"] {
            assert!(
                !tool.name().to_lowercase().contains(&marker.to_lowercase())
                    && !tool.command().contains(marker),
                "{} reaches {}, an object operation",
                tool.name(),
                tool.command()
            );
        }
    }
}

#[test]
fn a_tool_outside_the_allowlist_is_refused() {
    let registry = ToolRegistry::builtin();
    let call = make_call(REFRESH_CATALOG, json!({}));
    let refusal = registry
        .translate(&call, &[EXECUTE_QUERY.to_owned()], &scope())
        .expect_err("the tool is not granted");
    assert!(
        matches!(refusal, AiError::ToolNotAllowed { .. }),
        "{refusal:?}"
    );
}

#[test]
fn an_unknown_tool_is_refused_before_the_allowlist() {
    let registry = ToolRegistry::builtin();
    let call = make_call("drop_everything", json!({"target": "*"}));
    let refusal = registry
        .translate(&call, &all_command_names(), &scope())
        .expect_err("the tool does not exist");
    assert!(
        matches!(refusal, AiError::UnknownTool { .. }),
        "{refusal:?}"
    );
}

#[test]
fn the_model_chooses_neither_the_connection_nor_the_session() {
    // The failure aimed at: a model that targets a connection other than
    // the one opened by the user, and exfiltrates from one database to
    // another.
    let registry = ToolRegistry::builtin();
    let granted_scope = scope();
    let call = make_call(
        EXECUTE_QUERY,
        json!({
            "statement": "SELECT 1",
            "connection": "00000000-0000-0000-0000-000000000000",
        }),
    );
    let refusal = registry
        .translate(&call, &all_command_names(), &granted_scope)
        .expect_err("`connection` is not in the schema");
    assert!(
        matches!(refusal, AiError::InvalidArguments { .. }),
        "{refusal:?}"
    );

    // And the legitimate call does target the imposed scope.
    let call = make_call(EXECUTE_QUERY, json!({"statement": "SELECT 1"}));
    let command = registry
        .translate(&call, &all_command_names(), &granted_scope)
        .expect("valid call");
    assert_eq!(command.target_connection(), Some(granted_scope.connection));
}

#[test]
fn the_model_cannot_declare_itself_read_only() {
    // The failure aimed at: a model that attaches `read_only: true` to a
    // DELETE to pass itself off as a read before the PolicyGate.
    let registry = ToolRegistry::builtin();
    let call = make_call(
        EXECUTE_QUERY,
        json!({"statement": "DELETE FROM purchases", "read_only": true}),
    );
    assert!(
        registry
            .translate(&call, &all_command_names(), &scope())
            .is_err()
    );

    // Without the field, the classification decides alone — and it sees a
    // DELETE without WHERE.
    let call = make_call(EXECUTE_QUERY, json!({"statement": "DELETE FROM purchases"}));
    let command = registry
        .translate(&call, &all_command_names(), &scope())
        .expect("valid call");
    assert_eq!(command.intent(), StatementIntent::Write);
    assert_eq!(command.mutation_risk(), MutationRisk::UnboundedDelete);
    assert!(command.is_mutating());
}

#[test]
fn a_read_stays_bounded_as_read_only() {
    let registry = ToolRegistry::builtin();
    let call = make_call(EXECUTE_QUERY, json!({"statement": "SELECT * FROM clients"}));
    let command = registry
        .translate(&call, &all_command_names(), &scope())
        .expect("valid call");
    let Command::Execute { request, .. } = command else {
        panic!("execute_query must produce Command::Execute");
    };
    assert!(request.limits.read_only);
    assert_eq!(request.intent, StatementIntent::Read);
    assert!(request.params.is_empty(), "the model binds no values");
}

#[test]
fn an_unreadable_text_is_treated_as_mutating() {
    // "When in doubt, protect": what cannot be analyzed is `Unknown`, which
    // counts as mutating, hence submitted to approval.
    let registry = ToolRegistry::builtin();
    let call = make_call(EXECUTE_QUERY, json!({"statement": "SELEKT * FORM t"}));
    let command = registry
        .translate(&call, &all_command_names(), &scope())
        .expect("valid call");
    assert_eq!(command.intent(), StatementIntent::Unknown);
    assert!(command.is_mutating());
}

#[test]
fn an_empty_statement_is_refused() {
    let registry = ToolRegistry::builtin();
    let call = make_call(EXECUTE_QUERY, json!({"statement": "   \n  "}));
    let refusal = registry
        .translate(&call, &all_command_names(), &scope())
        .expect_err("empty statement");
    assert!(
        matches!(refusal, AiError::InvalidArguments { .. }),
        "{refusal:?}"
    );
}

#[test]
fn a_tool_without_arguments_accepts_null() {
    // Several providers pass `null` instead of `{}`.
    let registry = ToolRegistry::builtin();
    let call = make_call(REFRESH_CATALOG, serde_json::Value::Null);
    let command = registry
        .translate(&call, &all_command_names(), &scope())
        .expect("valid call");
    assert_eq!(command.name(), "RefreshCatalog");
}

#[test]
fn schemas_can_be_passed_to_a_provider() {
    let registry = ToolRegistry::builtin();
    let specs = registry
        .specs_for(&all_command_names())
        .expect("known tools");
    assert_eq!(specs.len(), 4);
    for spec in &specs {
        let params = &spec.parameters;
        assert_eq!(params.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(
            params.get("$schema").is_none(),
            "`$schema` makes the strict validation of some providers fail"
        );
        assert!(params.get("$defs").is_none(), "subschemas must be inlined");
        assert!(
            params.get("title").is_none(),
            "the Rust type's name has no business in a prompt"
        );
        assert!(
            params.get("description").is_none(),
            "the root description comes from the type's `///`: it has no business \
                 in a prompt"
        );
        assert_eq!(
            params.get("additionalProperties"),
            Some(&serde_json::Value::Bool(false)),
            "an invented argument must make the translation fail, not be ignored"
        );
        assert!(!spec.description.is_empty());
    }
}

#[test]
fn the_search_bound_is_announced_to_the_model() {
    // The failure aimed at: an agent that does not see the bound exceeds
    // it, gets refused, and does not know by how much to shorten.
    let registry = ToolRegistry::builtin();
    let specs = registry
        .specs_for(&[DESCRIBE_SCHEMA.to_owned()])
        .expect("known tool");
    let description = specs
        .first()
        .and_then(|spec| spec.parameters.pointer("/properties/search/description"))
        .and_then(serde_json::Value::as_str)
        .expect("`search` is described");
    assert!(
        description.contains(&format!("At most {MAX_CATALOG_FOCUS_BYTES} bytes")),
        "{description}"
    );

    // And the announced bound is the one that refuses.
    let exact_fit = "a".repeat(MAX_CATALOG_FOCUS_BYTES);
    let call = make_call(DESCRIBE_SCHEMA, json!({ "search": exact_fit }));
    assert!(
        registry
            .translate(&call, &all_command_names(), &scope())
            .is_ok()
    );
    let too_long = "é".repeat(MAX_CATALOG_FOCUS_BYTES / 2 + 1);
    let call = make_call(DESCRIBE_SCHEMA, json!({ "search": too_long }));
    assert!(
        registry
            .translate(&call, &all_command_names(), &scope())
            .is_err()
    );
}

#[test]
fn a_samples_bounds_are_announced_to_the_model() {
    // Same failure as for the search: a bound the schema keeps quiet is a
    // bound the agent exceeds. The attributes' figures are literals; this
    // test keeps them aligned with the constants that refuse.
    let specs = ToolRegistry::builtin()
        .specs_for(&[REQUEST_SAMPLE.to_owned()])
        .expect("known tool");
    let schema = &specs.first().expect("a tool").parameters;
    let at_pointer = |pointer: &str| schema.pointer(pointer).cloned();
    assert_eq!(
        at_pointer("/properties/rows/minimum"),
        Some(json!(1)),
        "{schema}"
    );
    assert_eq!(
        at_pointer("/properties/rows/maximum"),
        Some(json!(MAX_SAMPLE_ROWS)),
        "{schema}"
    );
    assert_eq!(
        at_pointer("/properties/columns/maxItems"),
        Some(json!(MAX_SAMPLE_COLUMNS)),
        "{schema}"
    );
    let description_of = |field: &str| {
        schema
            .pointer(&format!("/properties/{field}/description"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    for field in ["relation", "namespace", "columns"] {
        assert!(
            description_of(field)
                .contains(&format!("at most {MAX_SAMPLE_NAME_BYTES} bytes of UTF-8"))
                || description_of(field)
                    .contains(&format!("At most {MAX_SAMPLE_NAME_BYTES} bytes of UTF-8")),
            "{field}: {}",
            description_of(field)
        );
    }
    assert!(
        description_of("columns").contains(&format!("At most {MAX_SAMPLE_COLUMNS} columns")),
        "{}",
        description_of("columns")
    );
    assert!(
        description_of("rows").contains(&format!(
            "{DEFAULT_SAMPLE_ROWS} when omitted, from 1 to {MAX_SAMPLE_ROWS}"
        )),
        "{}",
        description_of("rows")
    );

    // And the announced bounds are the ones that refuse.
    assert!(sample_request(json!({ "relation": "t", "rows": MAX_SAMPLE_ROWS })).is_ok());
    assert!(sample_request(json!({ "relation": "t", "rows": MAX_SAMPLE_ROWS + 1 })).is_err());
    assert!(sample_request(json!({ "relation": "t", "rows": 0 })).is_err());
    let columns = |n: usize| (0..n).map(|i| format!("c{i}")).collect::<Vec<_>>();
    assert!(
        sample_request(json!({ "relation": "t", "columns": columns(MAX_SAMPLE_COLUMNS) })).is_ok()
    );
    assert!(
        sample_request(json!({ "relation": "t", "columns": columns(MAX_SAMPLE_COLUMNS + 1) }))
            .is_err()
    );
    assert!(sample_request(json!({ "relation": "a".repeat(MAX_SAMPLE_NAME_BYTES) })).is_ok());
    assert!(
        sample_request(json!({ "relation": "é".repeat(MAX_SAMPLE_NAME_BYTES / 2 + 1) })).is_err()
    );
}

#[test]
fn translate_never_returns_a_samples_read() {
    // The public API that returns a bare `Command` must not return a
    // sample's read: that would be a read of values without the approval
    // screen, within reach of any caller of the crate.
    let call = make_call(REQUEST_SAMPLE, json!({ "relation": "customers" }));
    let refusal = ToolRegistry::builtin()
        .translate(&call, &all_command_names(), &scope())
        .expect_err("no command for a sample");
    assert!(
        matches!(&refusal, AiError::ToolNotAllowed { name } if name == REQUEST_SAMPLE),
        "{refusal:?}"
    );
    // `request`, on the other hand, returns the request to have approved.
    assert!(sample_request(json!({ "relation": "customers" })).is_ok());
}

fn sample_request(args: serde_json::Value) -> Result<SampleAsk, AiError> {
    let call = make_call(REQUEST_SAMPLE, args);
    match ToolRegistry::builtin().request(&call, &all_command_names(), &scope())? {
        ToolRequest::Sample(ask) => Ok(ask),
        ToolRequest::Dispatch(command) => {
            panic!("request_sample must ask for an approval, not {command:?}")
        }
    }
}

#[test]
fn a_requested_sample_is_a_bounded_read_to_approve() {
    let ask = sample_request(json!({"relation": "clients"})).expect("valid call");
    assert_eq!(ask.rows(), DEFAULT_SAMPLE_ROWS, "the default value");
    assert!(ask.columns.is_empty(), "all, at the user's choice");
    assert!(!ask.command.is_mutating());
    let Command::PreviewRelation { shape, .. } = &ask.command else {
        panic!("the read is a preview: {:?}", ask.command);
    };
    assert_eq!(
        *shape,
        PreviewShape::unordered(),
        "neither filter nor order: the agent writes no predicate"
    );

    let ask = sample_request(json!({"relation": "clients", "rows": MAX_SAMPLE_ROWS}))
        .expect("the ceiling is allowed");
    assert_eq!(ask.rows(), MAX_SAMPLE_ROWS);
    for too_long in [0, MAX_SAMPLE_ROWS + 1] {
        let refusal = sample_request(json!({"relation": "clients", "rows": too_long}))
            .expect_err("out of bounds");
        assert!(
            matches!(refusal, AiError::InvalidArguments { .. }),
            "{too_long}"
        );
    }
}

#[test]
fn the_model_chooses_neither_the_connection_nor_a_filter_for_a_sample() {
    for invented in [
        json!({"relation": "clients", "connection": "00000000-0000-0000-0000-000000000000"}),
        json!({"relation": "clients", "predicate": "1=1"}),
        json!({"relation": "clients", "approved": true}),
    ] {
        let refusal = sample_request(invented.clone()).expect_err("field outside the schema");
        assert!(
            matches!(refusal, AiError::InvalidArguments { .. }),
            "{invented}"
        );
    }
    let granted_scope = scope();
    let call = make_call(REQUEST_SAMPLE, json!({"relation": "clients"}));
    let Ok(ToolRequest::Sample(ask)) =
        ToolRegistry::builtin().request(&call, &all_command_names(), &granted_scope)
    else {
        panic!("valid call");
    };
    assert_eq!(
        ask.command.target_connection(),
        Some(granted_scope.connection)
    );
}

/// I-10: a hostile name stays **data**. It travels in the command's
/// `relation` field, as is, and no statement text is composed here — it is
/// the driver that will quote it.
#[test]
fn a_hostile_name_stays_a_field_and_enters_no_statement() {
    let hostile = r#"users"; DROP TABLE audit; --"#;
    let ask = sample_request(json!({
        "relation": hostile,
        "namespace": hostile,
        "columns": [hostile, "email", "email"],
    }))
    .expect("a name legal for the server remains requestable");
    assert_eq!(ask.columns, [hostile, "email"], "deduplicated, in order");
    assert!(
        ask.command.statement_text().is_none(),
        "no statement is composed at translation"
    );
    let Command::PreviewRelation {
        relation,
        namespace,
        ..
    } = &ask.command
    else {
        panic!("the read is a preview");
    };
    assert_eq!(relation, hostile);
    assert_eq!(namespace.as_deref(), Some(hostile));
}

#[test]
fn a_samples_names_are_bounded() {
    let long = "n".repeat(MAX_SAMPLE_NAME_BYTES + 1);
    for args in [
        json!({"relation": ""}),
        json!({"relation": "   "}),
        json!({"relation": long}),
        json!({"relation": "clients", "columns": [long]}),
        json!({"relation": "clients", "columns": [""]}),
        json!({"relation": "clients", "columns": vec!["c"; MAX_SAMPLE_COLUMNS + 1]}),
    ] {
        assert!(sample_request(args.clone()).is_err(), "{args}");
    }
}

#[test]
fn a_tool_unknown_to_the_allowlist_fails_at_declaration() {
    let registry = ToolRegistry::builtin();
    let refusal = registry
        .specs_for(&["drop_everything".to_owned()])
        .expect_err("tool absent from the registry");
    assert!(
        matches!(refusal, AiError::UnknownTool { .. }),
        "{refusal:?}"
    );
}

#[test]
fn an_unanalyzed_language_is_mutating() {
    // Mongo is not analyzed by oxyn-query: the classification returns
    // `Unknown`, hence mutating, hence approval. It is the intended
    // behavior as long as no analyzer exists for that language.
    let registry = ToolRegistry::builtin();
    let granted_scope = ToolScope::new(
        ConnectionId::new(),
        SessionId::new(),
        QueryLanguage::MongoQuery,
    );
    let call = make_call(EXECUTE_QUERY, json!({"statement": "db.clients.find({})"}));
    let command = registry
        .translate(&call, &all_command_names(), &granted_scope)
        .expect("valid call");
    assert!(command.is_mutating());
}
