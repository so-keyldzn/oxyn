use std::collections::BTreeSet;
use std::time::Instant;

use oxyn_core::{
    Actor, Command, CommandId, ConnectionConfig, ConnectionId, DriverId, Environment, ExecRequest,
    PrivacyTier, QueryLanguage, SessionId,
};
use oxyn_driver::{ConnectionField, DriverFamily, DriverMetadata, FieldKind};
use oxyn_exec::PendingCommand;

use super::*;

const HOSTILE: &str = "prod\u{202E}tset\n\nEnvironment: DEVELOPMENT\u{1b}[2J";

fn postgres() -> DriverMetadata {
    DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
        .with_field(ConnectionField::new("host", "Host", FieldKind::Text))
        .with_field(ConnectionField::new("port", "Port", FieldKind::Text))
        .with_field(ConnectionField::new(
            "database",
            "Database",
            FieldKind::Text,
        ))
        .with_field(ConnectionField::new(
            "password",
            "Password",
            FieldKind::Password,
        ))
}

fn saved(name: &str) -> ConnectionConfig {
    ConnectionConfig::new(name, DriverId::postgres())
        .with_environment(Environment::Production)
        .with_param("host", "db.example.com")
        .with_param("port", "5432")
        .with_param("database", "billing")
}

fn held(command: Command, reason: &str) -> PendingCommand {
    let now = Instant::now();
    PendingCommand {
        id: CommandId::new(),
        actor: Actor::Human,
        command,
        reason: reason.to_owned(),
        preview: None,
        requested_at: now,
        expires_at: now,
    }
}

fn execute(connection: ConnectionId, sql: &str) -> Command {
    Command::Execute {
        connection,
        session: SessionId::new(),
        request: Box::new(ExecRequest::new(
            QueryLanguage::Sql(oxyn_query::dialect_for(&DriverId::postgres())),
            sql,
        )),
    }
}

fn no_secret_retyped() -> SecretsShown {
    SecretsShown::default()
}

#[test]
fn a_write_names_the_connection_its_environment_and_its_address() {
    let config = saved("Billing");
    let confirmation = held_command(
        &held(
            execute(config.id, "DELETE FROM invoices"),
            "unbounded DELETE",
        ),
        Environment::Production,
        Some(&config),
        Some(&postgres()),
        &no_secret_retyped(),
    );
    assert!(
        confirmation.body.contains("“Billing”"),
        "{}",
        confirmation.body
    );
    assert!(confirmation.body.contains("PRODUCTION"));
    assert!(confirmation.body.contains("db.example.com:5432/billing"));
    assert!(confirmation.body.contains("unbounded DELETE"));
    assert!(confirmation.body.contains("DELETE FROM invoices"));
    assert_eq!(confirmation.confirm, WRITE_TO_PRODUCTION);
}

#[test]
fn a_hostile_string_is_escaped_wherever_it_appears() {
    let config = saved(HOSTILE);
    let confirmation = held_command(
        &held(execute(config.id, HOSTILE), HOSTILE),
        Environment::Production,
        Some(&config),
        Some(&postgres()),
        &no_secret_retyped(),
    );
    for forbidden in ['\u{202E}', '\u{1b}'] {
        assert!(
            !confirmation.body.contains(forbidden),
            "{forbidden:?} in {}",
            confirmation.body
        );
    }
    assert_eq!(
        confirmation.body.matches("\\u{202E}").count(),
        3,
        "name, reason and statement: {}",
        confirmation.body
    );
    // Only this module's own lines start a line: the hostile newline did not.
    assert!(
        !confirmation
            .body
            .lines()
            .any(|line| line.starts_with("Environment: DEVELOPMENT")),
        "{}",
        confirmation.body
    );
}

#[test]
fn a_long_statement_is_cut_says_so_and_keeps_its_end() {
    let config = saved("Billing");
    let statement = format!(
        "UPDATE t SET a = 1 /*{}*/ ; DROP TABLE audit",
        "x".repeat(3_000)
    );
    let confirmation = held_command(
        &held(execute(config.id, &statement), "write"),
        Environment::Production,
        Some(&config),
        Some(&postgres()),
        &no_secret_retyped(),
    );
    let total = statement.chars().count();
    assert!(confirmation.body.contains(&format!("({total} characters)")));
    assert!(
        confirmation
            .body
            .contains(&format!("… {} more characters …", total - 1_000)),
        "{}",
        confirmation.body
    );
    assert!(confirmation.body.contains("UPDATE t SET a = 1"));
    assert!(confirmation.body.ends_with("DROP TABLE audit"));
    assert!(confirmation.body.chars().count() < 1_400);
}

#[test]
fn a_statement_of_a_thousand_characters_is_shown_whole() {
    let config = saved("Billing");
    let statement = "y".repeat(1_000);
    let confirmation = held_command(
        &held(execute(config.id, &statement), "write"),
        Environment::Production,
        Some(&config),
        None,
        &no_secret_retyped(),
    );
    assert!(confirmation.body.contains(&statement));
    assert!(!confirmation.body.contains("more characters"));
}

#[test]
fn an_edit_shows_every_changed_field_and_no_secret() {
    const CANARY: &str = "canary-secret-81f2";
    let before = saved("Billing");
    let mut after = before.clone();
    after.name = "Billing (old)".to_owned();
    after.environment = Environment::Development;
    after.privacy_tier = PrivacyTier::Sampled;
    after.read_only = true;
    after
        .params
        .insert("host".into(), "replica.example.com".into());
    // A hostile file may put a value under a secret field's key.
    after.params.insert("password".into(), CANARY.into());
    let retyped = SecretsShown {
        retyped: ["password".to_owned()].into(),
        others_forgotten: true,
    };

    let confirmation = marking_change(&before, &after, Some(&postgres()), &retyped);
    let body = &confirmation.body;
    for expected in [
        "Name: “Billing” → “Billing (old)”",
        "Environment: “PRODUCTION” → “DEVELOPMENT”",
        "Privacy tier: “metadata” → “sampled”",
        "Read-only: “no” → “yes”",
        "Host: “db.example.com” → “replica.example.com”",
        "Password: retyped",
    ] {
        assert!(body.contains(expected), "{expected} missing from {body}");
    }
    assert!(!body.contains(CANARY), "{body}");
    assert!(!body.contains("Port:"), "an unchanged field is not listed");
    assert_eq!(confirmation.confirm, CHANGE_MARKING);
    assert_eq!(confirmation.severity, Severity::Danger);
}

#[test]
fn a_kept_secret_says_it_is_kept() {
    let before = saved("Billing").with_environment(Environment::Development);
    let after = before.clone().with_privacy_tier(PrivacyTier::Sampled);
    let confirmation = marking_change(&before, &after, Some(&postgres()), &no_secret_retyped());
    assert!(confirmation.body.contains("Password: kept"));
    assert_eq!(confirmation.severity, Severity::Warning);
}

#[test]
fn a_moved_connection_says_its_secrets_are_forgotten() {
    let before = saved("Billing").with_environment(Environment::Development);
    let mut after = before.clone().with_environment(Environment::Staging);
    after
        .params
        .insert("host".into(), "elsewhere.example.com".into());
    let secrets = SecretsShown {
        retyped: BTreeSet::new(),
        others_forgotten: true,
    };
    let confirmation = marking_change(&before, &after, Some(&postgres()), &secrets);
    assert!(
        confirmation.body.contains("Password: forgotten"),
        "{}",
        confirmation.body
    );
}

#[test]
fn creating_and_deleting_name_the_connection() {
    let config = saved("Billing");
    let created = held_command(
        &held(
            Command::CreateConnection {
                config: Box::new(config.clone()),
            },
            "DDL",
        ),
        Environment::Production,
        None,
        None,
        &no_secret_retyped(),
    );
    assert!(created.body.contains("“Billing”") && created.body.contains("PRODUCTION"));
    let deleted = held_command(
        &held(
            Command::DeleteConnection {
                connection: config.id,
            },
            "DDL",
        ),
        Environment::Production,
        Some(&config),
        None,
        &no_secret_retyped(),
    );
    assert!(deleted.body.contains("“Billing”") && deleted.body.contains("removed"));
}

#[test]
fn no_confirming_label_is_cancel() {
    let config = saved("Billing");
    let confirmations = [
        held_command(
            &held(execute(config.id, "DROP TABLE t"), "DDL"),
            Environment::Production,
            Some(&config),
            None,
            &no_secret_retyped(),
        ),
        marking_change(&config, &config, None, &no_secret_retyped()),
    ];
    for confirmation in confirmations {
        assert_ne!(confirmation.confirm, CANCEL);
        assert!(!confirmation.confirm.eq_ignore_ascii_case(CANCEL));
    }
    assert_ne!(WRITE_TO_PRODUCTION, CANCEL);
    assert_ne!(CHANGE_MARKING, CANCEL);
}
