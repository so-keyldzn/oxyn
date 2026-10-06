//! The catalog: what is offered where, and a user file never displaces a
//! shipped agent.

use oxyn_core::{AiProviderKind, DriverId, Environment, SqlDialect};

use super::*;
use crate::agent_file::{ExternalAgentKind, Recipient, parse_agent_file};
use crate::builtin::{schema_agent, sql_agent};

fn target(dialect: SqlDialect, recipient: Recipient) -> PromptTarget {
    PromptTarget {
        dialect,
        driver: DriverId::new(DriverId::POSTGRES).expect("a valid driver name"),
        environment: Environment::Production,
        recipient,
    }
}

fn anthropic_on(dialect: SqlDialect) -> PromptTarget {
    target(dialect, Recipient::Provider(AiProviderKind::Anthropic))
}

fn user_file(file_name: &str, front: &str) -> UserAgentFile {
    let text = format!("---\n{front}---\nYou help.\n");
    UserAgentFile {
        file_name: file_name.to_owned(),
        result: parse_agent_file(file_name, &text),
    }
}

const PG_ONLY: &str = "id: 0199a3c0-0000-7000-8000-0000000000a1\nname: Postgres tuner\n\
                       applies_to: [postgres]\n";
const LOCAL_ONLY: &str = "id: 0199a3c0-0000-7000-8000-0000000000a2\nname: Small model\n\
                          recipients: [openai_compatible]\n";

fn names(entries: &[CatalogEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.name.as_str()).collect()
}

#[test]
fn shipped_only_offers_the_shipped_agents_sql_first() {
    let catalog = AgentCatalog::shipped_only();
    let offered = catalog.offered(&anthropic_on(SqlDialect::Postgres));

    assert_eq!(names(&offered), ["SQL", "Schema"]);
    assert!(
        offered
            .iter()
            .all(|entry| entry.origin == AgentOrigin::Shipped
                && entry.error.is_none()
                && entry.file_name.is_none())
    );
    assert_eq!(
        offered.first().and_then(|entry| entry.id),
        Some(sql_agent().id)
    );
}

#[test]
fn the_list_is_filtered_by_dialect() {
    let catalog = AgentCatalog::new(Vec::new(), vec![user_file("pg.md", PG_ONLY)]);

    assert_eq!(
        names(&catalog.offered(&anthropic_on(SqlDialect::Postgres))),
        ["Postgres tuner"]
    );
    assert!(
        catalog
            .offered(&anthropic_on(SqlDialect::Sqlite))
            .is_empty()
    );
}

#[test]
fn the_list_is_filtered_by_recipient() {
    let catalog = AgentCatalog::new(Vec::new(), vec![user_file("local.md", LOCAL_ONLY)]);
    let local = target(
        SqlDialect::Postgres,
        Recipient::Provider(AiProviderKind::OpenAiCompatible),
    );
    let codex = target(
        SqlDialect::Postgres,
        Recipient::External(ExternalAgentKind::Codex),
    );

    assert_eq!(names(&catalog.offered(&local)), ["Small model"]);
    assert!(catalog.offered(&codex).is_empty());
}

#[test]
fn shipped_come_first_then_user_each_by_name() {
    let catalog = AgentCatalog::new(
        vec![schema_agent(), sql_agent()],
        vec![
            user_file(
                "z.md",
                "id: 0199a3c0-0000-7000-8000-0000000000b1\nname: Zeta\n",
            ),
            user_file(
                "a.md",
                "id: 0199a3c0-0000-7000-8000-0000000000b2\nname: Alpha\n",
            ),
        ],
    );

    assert_eq!(
        names(&catalog.offered(&anthropic_on(SqlDialect::Postgres))),
        ["SQL", "Schema", "Alpha", "Zeta"]
    );
}

#[test]
fn a_user_file_cannot_take_a_shipped_id() {
    let sql = sql_agent();
    let front = format!("id: {}\nname: Impostor\n", sql.id);
    let catalog = AgentCatalog::new(vec![sql.clone()], vec![user_file("sql.md", &front)]);

    // The shipped agent keeps its id, its name and its prompt.
    assert_eq!(catalog.get(&sql.id), Some(&sql));

    let offered = catalog.offered(&anthropic_on(SqlDialect::Postgres));
    let refused = offered
        .iter()
        .find(|entry| entry.origin == AgentOrigin::User)
        .expect("the colliding file is listed");
    assert_eq!(refused.id, None, "an entry in error cannot be picked");
    assert_eq!(refused.file_name.as_deref(), Some("sql.md"));
    assert!(matches!(refused.error, Some(CatalogError::IdTaken { .. })));
}

#[test]
fn a_user_file_cannot_take_an_earlier_user_id() {
    let catalog = AgentCatalog::new(
        Vec::new(),
        vec![
            user_file(
                "first.md",
                "id: 0199a3c0-0000-7000-8000-0000000000c1\nname: First\n",
            ),
            user_file(
                "second.md",
                "id: 0199a3c0-0000-7000-8000-0000000000c1\nname: Second\n",
            ),
        ],
    );
    let offered = catalog.offered(&anthropic_on(SqlDialect::Postgres));

    let first = offered
        .iter()
        .find(|entry| entry.name == "First")
        .expect("listed");
    assert!(first.error.is_none());
    let second = offered
        .iter()
        .find(|entry| entry.file_name.as_deref() == Some("second.md"))
        .expect("listed");
    assert!(matches!(second.error, Some(CatalogError::IdTaken { .. })));
    assert_eq!(
        first
            .id
            .and_then(|id| catalog.get(&id))
            .map(|spec| spec.name.as_str()),
        Some("First")
    );
}

#[test]
fn an_invalid_file_is_listed_whatever_the_target_and_never_returned() {
    let broken = UserAgentFile {
        file_name: "broken.md".to_owned(),
        result: parse_agent_file("broken.md", "no front matter"),
    };
    let catalog = AgentCatalog::new(Vec::new(), vec![broken]);

    for dialect in [SqlDialect::Postgres, SqlDialect::Sqlite] {
        let offered = catalog.offered(&anthropic_on(dialect));
        let [entry] = offered.as_slice() else {
            panic!("exactly the broken file is listed, got {offered:?}");
        };
        assert_eq!(entry.name, "broken.md");
        assert_eq!(entry.id, None);
        assert!(matches!(entry.error, Some(CatalogError::File(_))));
    }
}

#[test]
fn get_ignores_unknown_ids() {
    let catalog = AgentCatalog::shipped_only();
    assert!(catalog.get(&oxyn_core::AgentId::new()).is_none());
    assert_eq!(catalog.get(&schema_agent().id), Some(&schema_agent()));
}

/// What a `{:?}` added to a log one day would print: names, ids and counts,
/// never what a user's file says.
#[test]
fn debug_never_prints_a_prompt_or_a_description() {
    let text = "---\nid: 0199a3c0-0000-7000-8000-0000000000a9\nname: Tuner\n\
                description: DESCRIPTION-MARKER\n---\nPROMPT-MARKER\n";
    let file = UserAgentFile {
        file_name: "tuner.md".to_owned(),
        result: parse_agent_file("tuner.md", text),
    };
    let catalog = AgentCatalog::new(shipped_agents(), vec![file.clone()]);
    let offered = catalog.offered(&anthropic_on(SqlDialect::Postgres));
    let spec = catalog
        .get(
            &"0199a3c0-0000-7000-8000-0000000000a9"
                .parse()
                .expect("an id"),
        )
        .expect("the agent is valid");

    for printed in [
        format!("{file:?}"),
        format!("{catalog:?}"),
        format!("{offered:?}"),
        format!("{spec:?}"),
        format!("{:?}", sql_agent()),
    ] {
        assert!(!printed.contains("MARKER"), "{printed}");
        assert!(!printed.contains("You help"), "{printed}");
    }
    assert!(format!("{file:?}").contains("tuner.md"));
}
