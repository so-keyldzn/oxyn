//! Which agent a conversation runs, and what the picker says of the others.

use oxyn_ai::external::presets::{self, CLAUDE_CODE};
use oxyn_ai::{UserAgentFile, parse_agent_file, schema_agent};
use oxyn_core::{AiProviderKind, DriverId, Environment, ProviderId};

use super::*;

fn connection(driver: &str) -> ConnectionConfig {
    serde_json::from_value(serde_json::json!({
        "id": ConnectionId::new(),
        "name": "test",
        "driver": driver,
    }))
    .expect("a minimal connection")
}

fn user(file: &str, front: &str) -> UserAgentFile {
    UserAgentFile {
        file_name: file.to_owned(),
        result: parse_agent_file(file, &format!("---\n{front}---\nYou help.\n")),
    }
}

fn declared(kind: &'static str, id: &str, recipient: Recipient) -> Declared {
    Declared {
        reference: DestinationRef {
            kind,
            id: id.to_owned(),
        },
        recipient,
    }
}

const ANTHROPIC: Recipient = Recipient::Provider(AiProviderKind::Anthropic);
const LOCAL: Recipient = Recipient::Provider(AiProviderKind::OpenAiCompatible);

#[test]
fn an_unset_environment_is_production_in_the_target() {
    let target = target_of(&connection(DriverId::POSTGRES), ANTHROPIC);
    assert_eq!(target.environment, Environment::Production);
    assert_eq!(target.dialect.as_str(), "postgres");
    assert_eq!(target.driver.as_str(), DriverId::POSTGRES);
}

#[test]
fn a_recorded_agent_runs_or_is_replaced_by_sql_and_named() {
    let schema = schema_agent().id;
    let gone = AgentId::new();
    let catalog = AgentCatalog::shipped_only();

    assert_eq!(recorded(&catalog, None), (sql_agent_id(), None));
    assert_eq!(recorded(&catalog, Some(schema)), (schema, None));
    assert_eq!(
        recorded(&catalog, Some(gone)),
        (
            sql_agent_id(),
            Some(MissingAgent {
                name: gone.to_string()
            })
        )
    );
}

#[test]
fn only_a_pinned_preset_reads_its_presets_fragment() {
    let pinned = presets::PresetDraft::blank(CLAUDE_CODE);
    let mut config: ExternalAgentConfig = serde_json::from_value(serde_json::json!({
        "id": ProviderId::new("agent").expect("a valid id"),
        "label": "Claude Code",
        "command": pinned.command,
        "args": pinned.args,
        "created_at": "2026-10-06T00:00:00Z",
        "updated_at": "2026-10-06T00:00:00Z",
    }))
    .expect("a declaration");
    assert_eq!(
        agent_recipient(&config),
        Recipient::External(ExternalAgentKind::ClaudeCode)
    );

    // An argument more and Oxyn did not confine it: the fragment must not
    // say it has no shell.
    config.args.push("--yolo".to_owned());
    assert_eq!(
        agent_recipient(&config),
        Recipient::External(ExternalAgentKind::Other)
    );
}

#[test]
fn the_list_is_filtered_by_dialect_and_recipient_and_names_disabled_destinations() {
    let catalog = AgentCatalog::new(
        oxyn_ai::shipped_agents(),
        vec![
            user(
                "pg.md",
                "id: 0199a3c0-0000-7000-8000-0000000000d1\nname: Postgres tuner\napplies_to: [postgres]\n",
            ),
            user(
                "local.md",
                "id: 0199a3c0-0000-7000-8000-0000000000d2\nname: Small model\nrecipients: [openai_compatible]\n",
            ),
            user("broken.md", "name: no id\n"),
        ],
    );
    let destinations = [
        declared("provider", "hosted", ANTHROPIC),
        declared("provider", "ollama", LOCAL),
    ];
    let names = |options: &[AgentRoleOption]| -> Vec<String> {
        options.iter().map(|option| option.name.clone()).collect()
    };

    let sqlite = connection(DriverId::SQLITE);
    let on_sqlite = options(&catalog, &sqlite, &[ANTHROPIC, LOCAL], &destinations);
    // Byte order within a group, as the catalog sorts.
    assert_eq!(
        names(&on_sqlite),
        ["SQL", "Schema", "Small model", "broken.md"]
    );
    let small = on_sqlite
        .iter()
        .find(|option| option.name == "Small model")
        .expect("listed");
    assert_eq!(
        small.disabled_destinations,
        [DestinationRef {
            kind: "provider",
            id: "hosted".to_owned()
        }]
    );
    let broken = on_sqlite
        .iter()
        .find(|option| option.name == "broken.md")
        .expect("listed with its error");
    assert_eq!(broken.id, "broken.md");
    assert_eq!(broken.origin, "user");
    assert!(
        broken
            .error
            .as_deref()
            .is_some_and(|error| error.contains("broken.md"))
    );

    let postgres = connection(DriverId::POSTGRES);
    let hosted_on_postgres = options(&catalog, &postgres, &[ANTHROPIC], &destinations);
    assert_eq!(
        names(&hosted_on_postgres),
        ["SQL", "Schema", "Postgres tuner", "broken.md"]
    );
    let sql = hosted_on_postgres.first().expect("SQL first");
    assert_eq!(sql.id, sql_agent_id().to_string());
    assert_eq!(sql.origin, "shipped");
    assert!(sql.error.is_none() && sql.disabled_destinations.is_empty());
}

#[test]
fn the_rendered_spec_is_the_one_render_system_prompt_makes() {
    let sql = sql_agent();
    let target = target_of(&connection(DriverId::SQLITE), LOCAL);
    let rendered = rendered(&sql, &target).expect("renders");
    assert_eq!(
        rendered.system_prompt,
        render_system_prompt(&sql, &target).expect("renders")
    );
    assert_eq!(rendered.allowed_tools, sql.allowed_tools);
    assert_eq!(rendered.id, sql.id);
}
