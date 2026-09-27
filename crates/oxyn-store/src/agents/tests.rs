//! What the external agents table must guarantee.

use super::*;
use oxyn_core::ExternalAgentConfig;

fn agent(id: &str, label: &str) -> ExternalAgentConfig {
    ExternalAgentConfig::new(
        ProviderId::new(id).expect("a valid identifier"),
        label,
        "claude",
    )
    .with_args(["--acp"])
}

#[test]
fn a_declaration_reads_back_identically() {
    let store = Store::open_in_memory().expect("open");
    let ecrit = agent("claude-code", "Claude Code");
    store.external_agents().save(&ecrit).expect("write");

    let relus = store.external_agents().list().expect("read");
    assert_eq!(relus.len(), 1);
    assert_eq!(relus[0].id, ecrit.id);
    assert_eq!(relus[0].command, "claude");
    assert_eq!(relus[0].args, ["--acp"], "the arguments survive the disk");
}

/// The table has **no** column where a secret could be stored.
///
/// This is the guarantee this mode rests on: an agent carries its own
/// authentication. The test reads the schema rather than the documentation,
/// because a column added later "just for a token" would turn no other test
/// red.
#[test]
fn the_table_has_no_secret_column() {
    let store = Store::open_in_memory().expect("open");
    let colonnes: Vec<String> = store
        .with_connection(|conn| {
            let mut requete =
                conn.prepare("SELECT name FROM pragma_table_info('external_agents')")?;
            let noms = requete.query_map([], |row| row.get(0))?;
            Ok(noms.collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .expect("read the schema");

    for interdite in ["secret_ref", "secret", "api_key", "token", "password"] {
        assert!(
            !colonnes.iter().any(|nom| nom == interdite),
            "`{interdite}` has no business here: an external agent entrusts no key"
        );
    }
}

/// An invalid declaration does not reach the disk.
#[test]
fn a_hostile_command_is_refused_before_the_disk() {
    let store = Store::open_in_memory().expect("open");
    let mut hostile = agent("hostile", "Hostile");
    hostile.command = "claude\n--evil".to_owned();

    assert!(
        store.external_agents().save(&hostile).is_err(),
        "a line break in the command has no legitimate use"
    );
    assert!(
        store.external_agents().list().expect("read").is_empty(),
        "nothing must have been written"
    );
}

/// A row that became unreadable is skipped, not propagated as an error.
///
/// An `args` corrupted by an SQLite editor must not make the configuration
/// screen unusable: the agent disappears from the list, with a trace.
#[test]
fn an_unreadable_row_is_skipped_without_breaking_the_list() {
    let store = Store::open_in_memory().expect("open");
    store
        .external_agents()
        .save(&agent("bon", "Bon agent"))
        .expect("write");
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO external_agents (id, label, command, args, env, created_at, updated_at)
                 VALUES ('casse', 'Cassé', 'claude', 'pas du json', '[]', ?1, ?1)",
                params![Utc::now()],
            )?;
            Ok(())
        })
        .expect("direct insert");

    let relus = store.external_agents().list().expect("read");
    assert_eq!(relus.len(), 1, "the healthy row is still served");
    assert_eq!(relus[0].label, "Bon agent");
}

#[test]
fn removing_a_declaration_leaves_the_others_alone() {
    let store = Store::open_in_memory().expect("open");
    store
        .external_agents()
        .save(&agent("un", "Un"))
        .expect("write");
    store
        .external_agents()
        .save(&agent("deux", "Deux"))
        .expect("write");

    let cible = ProviderId::new("un").expect("identifier");
    assert!(store.external_agents().remove(&cible).expect("removal"));
    assert!(
        !store.external_agents().remove(&cible).expect("removal"),
        "removing twice does not lie on the second pass"
    );

    let restants = store.external_agents().list().expect("read");
    assert_eq!(restants.len(), 1);
    assert_eq!(restants[0].label, "Deux");
}
