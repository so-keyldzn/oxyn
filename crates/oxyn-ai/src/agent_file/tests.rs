//! Agent files: what ships is valid, every combination renders, and every
//! hostile file is refused with its line.

use oxyn_core::{AgentId, AiProviderKind, DriverId, Environment, SqlDialect};

use super::files::{AGENTS, DIALECT_FRAGMENTS, RECIPIENT_FRAGMENTS};
use super::render::check_variables;
use super::*;
use crate::external::presets::PRESETS;
use crate::spec::AgentSpec;
use crate::tools::{ERD_HINT, ToolRegistry};

/// Every recipient, by name: `AiProviderKind` is `#[non_exhaustive]`, so the
/// list is written out, and checked against the fragments below.
const RECIPIENTS: [&str; 7] = [
    "anthropic",
    "openai",
    "gemini",
    "openai_compatible",
    "claude-code",
    "codex",
    "external",
];

fn recipient(name: &str) -> Recipient {
    Recipient::named(name).expect("a recipient of the list")
}

fn target(dialect: SqlDialect, recipient: Recipient) -> PromptTarget {
    PromptTarget {
        dialect,
        driver: DriverId::new(DriverId::POSTGRES).expect("a valid driver name"),
        environment: Environment::Staging,
        recipient,
    }
}

/// A minimal valid file, with `extra` appended to its front matter.
fn file_with(extra: &str, body: &str) -> String {
    format!("---\nid: 0199a3c0-0000-7000-8000-0000000000ff\nname: Test\n{extra}---\n{body}\n")
}

fn refusal(text: &str) -> AgentFileError {
    parse_agent_file("test.md", text).expect_err("the file must be refused")
}

#[test]
fn shipped_agents_are_valid() {
    let registry = ToolRegistry::builtin();
    for (file, text) in AGENTS {
        let agent = parse_agent_file(file, text).unwrap_or_else(|err| panic!("{err}"));
        agent
            .validate(&registry)
            .unwrap_or_else(|err| panic!("{file}: {err}"));
        assert!(
            agent.system_prompt.contains(ERD_HINT),
            "{file}: the `erd` sentence drifted from the one tools state"
        );
    }
    let agents = shipped_agents();
    assert_eq!(agents.len(), AGENTS.len());
    assert_ne!(agents.first().map(|a| a.id), agents.get(1).map(|a| a.id));
}

#[test]
fn every_fragment_is_a_valid_text_without_front_matter() {
    let fragments = DIALECT_FRAGMENTS.iter().chain(RECIPIENT_FRAGMENTS.iter());
    for (_, file, text) in fragments {
        assert!(text.len() <= MAX_FILE_BYTES, "{file}");
        assert!(
            !text.starts_with("---"),
            "{file}: a fragment has no front matter"
        );
        check_variables(file, text, 1).unwrap_or_else(|err| panic!("{err}"));
    }
}

#[test]
fn the_fragment_tables_name_what_exists() {
    for (name, file, _) in DIALECT_FRAGMENTS {
        let dialect = DIALECTS
            .into_iter()
            .find(|dialect| dialect.as_str() == name)
            .unwrap_or_else(|| panic!("{file} is written for no dialect"));
        assert!(file.ends_with(&format!("/{}.md", dialect.as_str())));
    }
    for (name, file, _) in RECIPIENT_FRAGMENTS {
        assert!(
            RECIPIENTS.contains(&name),
            "{file} is written for no recipient"
        );
        assert!(file.ends_with(&format!("/{name}.md")));
    }
    // The dialects Oxyn has or plans a driver for carry their own fragment.
    for dialect in ["ansi", "postgres", "redshift", "mysql", "sqlite", "duckdb"] {
        assert!(
            DIALECT_FRAGMENTS
                .iter()
                .any(|(name, _, _)| *name == dialect),
            "{dialect}"
        );
    }
}

#[test]
fn every_role_dialect_and_recipient_renders() {
    for agent in shipped_agents() {
        for dialect in DIALECTS {
            for name in RECIPIENTS {
                let target = target(dialect, recipient(name));
                let prompt = render_system_prompt(&agent, &target)
                    .unwrap_or_else(|err| panic!("{} × {dialect} × {name}: {err}", agent.name));
                assert!(
                    !prompt.contains("{{"),
                    "{} × {dialect} × {name}",
                    agent.name
                );
                assert!(
                    !prompt.contains("}}"),
                    "{} × {dialect} × {name}",
                    agent.name
                );
                assert!(prompt.starts_with(agent.system_prompt.trim()));
                assert!(prompt.contains(&format!("`{}`", dialect.as_str())));
                assert!(
                    prompt.contains("`staging`"),
                    "{{{{environment}}}} is filled"
                );
            }
        }
    }
}

#[test]
fn the_prompt_is_role_then_dialect_then_recipient_and_nothing_else() {
    let agent = sql_agent_for_tests();
    let target = target(SqlDialect::Sqlite, recipient("anthropic"));
    let prompt = render_system_prompt(&agent, &target).expect("renders");
    let dialect = include_str!("../../prompts/dialects/sqlite.md")
        .trim()
        .replace("{{dialect}}", "sqlite")
        .replace("{{driver}}", "postgres");
    let recipient = include_str!("../../prompts/recipients/anthropic.md")
        .trim()
        .replace("{{environment}}", "staging");
    assert_eq!(
        prompt,
        format!("{}\n\n{dialect}\n\n{recipient}", agent.system_prompt.trim())
    );
}

fn sql_agent_for_tests() -> AgentSpec {
    crate::sql_agent()
}

#[test]
fn a_dialect_without_a_fragment_falls_back_to_ansi() {
    let ansi = include_str!("../../prompts/dialects/ansi.md");
    let heading = ansi.lines().next().expect("ansi.md has a heading");
    for dialect in [
        SqlDialect::Oracle,
        SqlDialect::SqlServer,
        SqlDialect::BigQuery,
    ] {
        let prompt = render_system_prompt(
            &sql_agent_for_tests(),
            &target(dialect, recipient("openai")),
        )
        .expect("renders");
        assert!(
            prompt.contains(&heading.replace("{{dialect}}", dialect.as_str())),
            "{dialect}"
        );
    }
}

#[test]
fn every_variable_is_filled_from_the_target() {
    let agent = AgentSpec::new(
        AgentId::new(),
        "Vars",
        "{{dialect}} {{driver}} {{environment}} {{recipient}}",
    );
    let target = target(SqlDialect::MySql, recipient("claude-code"));
    let prompt = render_system_prompt(&agent, &target).expect("renders");
    assert!(prompt.starts_with("mysql postgres staging claude-code\n\n"));
    assert_eq!(VARIABLES.len(), 4);
}

#[test]
fn an_unknown_variable_is_refused_never_emptied() {
    // `{{tables}}` would be database content in the system prompt (I-04).
    let text = file_with("", "First line.\nSecond {{tables}} line.");
    assert_eq!(
        refusal(&text),
        AgentFileError::UnknownVariable {
            file: "test.md".to_owned(),
            line: 6,
        }
    );
    assert!(matches!(
        refusal(&file_with("", "Unclosed {{dialect")),
        AgentFileError::UnknownVariable { .. }
    ));

    let in_code = AgentSpec::new(AgentId::new(), "Coded", "{{schema}}");
    let target = target(SqlDialect::Postgres, recipient("gemini"));
    assert!(matches!(
        render_system_prompt(&in_code, &target),
        Err(AgentFileError::UnknownVariable { .. })
    ));
}

#[test]
fn an_unknown_key_is_refused_without_repeating_it() {
    let error = refusal(&file_with("SECRET_MARKER: 1\n", "Prompt."));
    assert_eq!(
        error,
        AgentFileError::FrontMatter {
            file: "test.md".to_owned(),
            line: 4,
            problem: YamlProblem::UnknownKey,
        }
    );
    assert!(!error.to_string().contains("SECRET_MARKER"), "{error}");

    let nested = refusal(&file_with("context:\n  max_relation: 3\n", "Prompt."));
    assert!(
        matches!(
            nested,
            AgentFileError::FrontMatter {
                problem: YamlProblem::UnknownKey,
                ..
            }
        ),
        "{nested}"
    );
}

#[test]
fn a_duplicate_key_is_refused() {
    let error = refusal(&file_with("name: Again\n", "Prompt."));
    assert!(
        matches!(
            error,
            AgentFileError::FrontMatter {
                problem: YamlProblem::DuplicateKey,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn anchors_aliases_and_merge_keys_are_refused() {
    for (extra, expected) in [
        ("description: &d text\n", YamlProblem::AnchorOrAlias),
        ("description: *d\n", YamlProblem::AnchorOrAlias),
        (
            "context:\n  <<: {max_relations: 3}\n",
            YamlProblem::MergeKey,
        ),
    ] {
        let error = refusal(&file_with(extra, "Prompt."));
        assert!(
            matches!(
                error,
                AgentFileError::FrontMatter { problem, .. } if problem == expected
            ),
            "{extra:?}: {error}"
        );
    }
}

#[test]
fn only_true_and_false_are_booleans() {
    let error = refusal(&file_with("context:\n  include_comments: no\n", "Prompt."));
    assert!(
        matches!(
            error,
            AgentFileError::FrontMatter {
                problem: YamlProblem::NotStrictBoolean,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn an_oversized_file_is_refused_before_parsing() {
    let text = file_with("", &"x".repeat(MAX_FILE_BYTES));
    assert!(matches!(refusal(&text), AgentFileError::TooLarge { .. }));
}

#[test]
fn the_front_matter_comes_first_and_is_closed() {
    for text in [
        "You are an agent.\n",
        "\n---\nid: x\n---\nPrompt.\n",
        "\u{feff}---\nid: x\n---\nPrompt.\n",
        "",
    ] {
        assert!(
            matches!(refusal(text), AgentFileError::MissingFrontMatter { .. }),
            "{text:?}"
        );
    }
    assert_eq!(
        refusal("---\nid: x\nname: y\n"),
        AgentFileError::UnclosedFrontMatter {
            file: "test.md".to_owned(),
            line: 3,
        }
    );
}

#[test]
fn an_unknown_dialect_or_recipient_is_refused_at_its_line() {
    assert_eq!(
        refusal(&file_with(
            "applies_to:\n  - postgres\n  - my_sql\n",
            "Prompt."
        )),
        AgentFileError::UnknownDialect {
            file: "test.md".to_owned(),
            line: 6,
        }
    );
    assert_eq!(
        refusal(&file_with("recipients: [anthropic, cursor]\n", "Prompt.")),
        AgentFileError::UnknownRecipient {
            file: "test.md".to_owned(),
            line: 4,
        }
    );
}

#[test]
fn a_declaration_validate_refuses_is_refused() {
    let error = refusal(&file_with("tools: [drop_all_tables]\n", "Prompt."));
    assert!(
        matches!(error, AgentFileError::Declaration { .. }),
        "{error}"
    );
    assert!(!error.to_string().contains("drop_all_tables"), "{error}");

    let error = refusal(&file_with("max_turns: 65\n", "Prompt."));
    assert!(
        matches!(error, AgentFileError::Declaration { .. }),
        "{error}"
    );

    let error = refusal(&file_with("", "   "));
    assert!(
        matches!(error, AgentFileError::Declaration { .. }),
        "{error}"
    );
}

#[test]
fn a_hostile_file_never_panics() {
    // I-09: these files can come from a user. Truncations of a valid file and
    // a few degenerate shapes must all end in `Ok` or `Err`.
    let valid = file_with("tools: [execute_query]\n", "Prompt {{dialect}}.");
    for end in 0..valid.len() {
        if let Some(prefix) = valid.get(..end) {
            let _ = parse_agent_file("cut.md", prefix);
        }
    }
    for text in [
        "---\n---\n",
        "---\n- a\n---\n",
        "---\n[\n---\n",
        "---\nid: [\n---\n{{",
    ] {
        let _ = parse_agent_file("odd.md", text);
    }
}

#[test]
fn offered_for_honors_applies_to_and_recipients() {
    let text = file_with(
        "applies_to: [postgres, redshift]\nrecipients: [anthropic, claude-code]\n",
        "Prompt.",
    );
    let agent = parse_agent_file("pg.md", &text).expect("valid");
    assert_eq!(
        agent.applies_to,
        [SqlDialect::Postgres, SqlDialect::Redshift]
    );
    assert!(agent.offered_for(&target(SqlDialect::Postgres, recipient("anthropic"))));
    assert!(agent.offered_for(&target(SqlDialect::Redshift, recipient("claude-code"))));
    assert!(!agent.offered_for(&target(SqlDialect::Sqlite, recipient("anthropic"))));
    assert!(!agent.offered_for(&target(SqlDialect::Postgres, recipient("codex"))));

    // Empty lists offer the agent everywhere.
    for shipped in shipped_agents() {
        for dialect in DIALECTS {
            for name in RECIPIENTS {
                assert!(shipped.offered_for(&target(dialect, recipient(name))));
            }
        }
    }
}

#[test]
fn recipients_read_back_by_their_names() {
    for name in RECIPIENTS {
        assert_eq!(recipient(name).as_str(), name);
    }
    assert_eq!(
        Recipient::named("openai_compatible"),
        Some(Recipient::Provider(AiProviderKind::OpenAiCompatible))
    );
    assert_eq!(Recipient::named("OpenAI"), None);
    for preset in PRESETS {
        assert_eq!(
            ExternalAgentKind::from_preset_id(preset.id).as_str(),
            preset.id
        );
    }
    // A declaration without a preset is one Oxyn did not confine.
    assert_eq!(
        ExternalAgentKind::from_preset_id("aider"),
        ExternalAgentKind::Other
    );
    assert_eq!(
        Recipient::named("external"),
        Some(Recipient::External(ExternalAgentKind::Other))
    );
    // Only the key names it: an arbitrary word is not a recipient.
    assert_eq!(Recipient::named("aider"), None);
    let agent = parse_agent_file("ext.md", &file_with("recipients: [external]\n", "Prompt."))
        .expect("`external` is accepted in `recipients`");
    assert_eq!(
        agent.recipients,
        [Recipient::External(ExternalAgentKind::Other)]
    );
}

#[test]
fn dialects_are_read_by_the_names_oxyn_shows() {
    for (index, dialect) in DIALECTS.iter().enumerate() {
        assert!(
            !DIALECTS
                .iter()
                .take(index)
                .any(|seen| seen.as_str() == dialect.as_str()),
            "{dialect} listed twice"
        );
    }
    // JSON carries the same names as a file: `mysql`, not serde's `my_sql`.
    let mut spec = AgentSpec::new(AgentId::new(), "Json", "Prompt.");
    spec.applies_to = vec![SqlDialect::MySql, SqlDialect::DuckDb];
    spec.recipients = vec![recipient("codex")];
    let json = serde_json::to_value(&spec).expect("serializes");
    assert_eq!(json["applies_to"], serde_json::json!(["mysql", "duckdb"]));
    assert_eq!(json["recipients"], serde_json::json!(["codex"]));
    let back: AgentSpec = serde_json::from_value(json).expect("reads back");
    assert_eq!(back, spec);
}

/// A file whose name names nothing in particular, with `front` as its whole
/// front matter.
fn front_only(front: &str) -> String {
    format!("---\nid: 0199a3c0-0000-7000-8000-0000000000fe\n{front}---\nYou help.\n")
}

#[test]
fn a_name_or_description_with_a_control_or_bidi_character_is_refused() {
    for (front, field, line) in [
        ("name: \"SQL\\u202E\"\n", "name", 3),
        ("name: \"S\\u0007QL\"\n", "name", 3),
        (
            "name: Fine\ndescription: \"looks \\u2066shipped\"\n",
            "description",
            4,
        ),
        (
            "name: Fine\ndescription: \"two\\nlines\"\n",
            "description",
            4,
        ),
    ] {
        let err = parse_agent_file("user.md", &front_only(front)).expect_err(front);
        assert_eq!(
            err,
            AgentFileError::MisleadingCharacter {
                file: "user.md".to_owned(),
                line,
                field,
            },
            "{front}"
        );
        let message = err.to_string();
        assert!(
            !message
                .chars()
                .any(|c| c.is_control() || c == '\u{202E}' || c == '\u{2066}'),
            "{message:?}"
        );
    }
}

#[test]
fn a_name_or_description_over_its_cap_is_refused() {
    let long_name = format!("name: {}\n", "n".repeat(parse::MAX_NAME_CHARS + 1));
    assert!(matches!(
        parse_agent_file("user.md", &front_only(&long_name)),
        Err(AgentFileError::TextTooLong {
            field: "name",
            line: 3,
            ..
        })
    ));
    let at_cap = format!("name: {}\n", "n".repeat(parse::MAX_NAME_CHARS));
    parse_agent_file("user.md", &front_only(&at_cap)).expect("a name at the cap");

    let long_description = format!(
        "name: Fine\ndescription: {}\n",
        "d".repeat(parse::MAX_DESCRIPTION_CHARS + 1)
    );
    assert!(matches!(
        parse_agent_file("user.md", &front_only(&long_description)),
        Err(AgentFileError::TextTooLong {
            field: "description",
            line: 4,
            ..
        })
    ));
}

#[test]
fn a_duplicated_tool_is_reported_without_its_name() {
    for tools in [
        "tools: [execute_query, execute_query]\n",
        "tools: [\"x\\u202Ey\", \"x\\u202Ey\"]\n",
    ] {
        let err = parse_agent_file("user.md", &front_only(&format!("name: Fine\n{tools}")))
            .expect_err(tools);
        let message = err.to_string();
        assert!(matches!(err, AgentFileError::Declaration { .. }), "{err:?}");
        assert!(!message.contains("execute_query"), "{message}");
        assert!(!message.contains('\u{202E}'), "{message:?}");
    }
}

#[test]
fn a_context_over_its_ceiling_is_refused_in_a_file() {
    let err = parse_agent_file(
        "user.md",
        &front_only("name: Fine\ncontext:\n  max_sample_rows: 100000\n"),
    )
    .expect_err("over the ceiling");
    assert!(err.to_string().contains("max_sample_rows"), "{err}");
}
