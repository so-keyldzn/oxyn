use oxyn_core::DriverId;
use oxyn_driver::{ConnectionField, DriverFamily, FieldKind};

use super::*;

#[test]
fn details_send_declared_values_and_no_secret() {
    let metadata =
        DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
            .with_field(ConnectionField::new("host", "Host", FieldKind::Text))
            .with_field(ConnectionField::new(
                "password",
                "Password",
                FieldKind::Password,
            ));
    let mut config = ConnectionConfig::new("prod", DriverId::postgres())
        .with_param("host", "db.internal")
        .with_param("undeclared", "kept-out")
        .with_secret_ref("connection:0123");
    // A secret that reached the parameters by a bug elsewhere must still
    // not come back through the edit form.
    config
        .params
        .insert("password".to_owned(), "hunter2".to_owned());

    let json = serde_json::to_string(&ConnectionDetails::of(&config, Some(&metadata), true))
        .expect("serializable");
    assert!(json.contains("db.internal"), "{json}");
    assert!(!json.contains("hunter2"), "secret sent: {json}");
    assert!(!json.contains("connection:0123"), "reference sent: {json}");
    assert!(!json.contains("kept-out"), "undeclared value sent: {json}");
    assert!(json.contains(r#""hasStoredSecrets":true"#), "{json}");

    let unknown =
        serde_json::to_string(&ConnectionDetails::of(&config, None, true)).expect("serializable");
    assert!(!unknown.contains("db.internal"), "{unknown}");
}

#[test]
fn a_summary_tells_homonyms_apart_without_a_secret() {
    let metadata =
        DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
            .with_field(ConnectionField::new("host", "Host", FieldKind::Text))
            .with_field(ConnectionField::new("port", "Port", FieldKind::Number))
            .with_field(ConnectionField::new(
                "database",
                "Database",
                FieldKind::Text,
            ))
            .with_field(ConnectionField::new(
                "password",
                "Password",
                FieldKind::Password,
            ));
    let mut config = ConnectionConfig::new("billing", DriverId::postgres())
        .with_param("host", "db\u{202e}.internal")
        .with_param("port", "5433")
        .with_param("database", "billing")
        .with_param("undeclared", "kept-out")
        .with_secret_ref("connection:0123");
    config
        .params
        .insert("password".to_owned(), "hunter2".to_owned());
    let summary = ConnectionSummary::of(&config, Some(&metadata));
    assert_eq!(summary.driver_name, "PostgreSQL");
    assert_eq!(
        summary.location.as_deref(),
        Some("db.internal:5433 / billing")
    );
    let json = serde_json::to_string(&summary).expect("serializable");
    for leaked in ["hunter2", "kept-out", "connection:0123"] {
        assert!(!json.contains(leaked), "{leaked} sent: {json}");
    }

    let sqlite = DriverMetadata::new(
        DriverId::new("sqlite").expect("id"),
        "SQLite",
        DriverFamily::Relational,
    )
    .with_field(ConnectionField::new(
        "path",
        "Database file",
        FieldKind::Path,
    ));
    let file = ConnectionConfig::new("scratch", DriverId::new("sqlite").expect("id"))
        .with_param("path", "/Users/me/private/scratch.sqlite");
    assert_eq!(
        ConnectionSummary::of(&file, Some(&sqlite))
            .location
            .as_deref(),
        Some("scratch.sqlite"),
        "a file name, never the directories above it"
    );

    let seeds = ConnectionConfig::new("orders", DriverId::mongodb())
        .with_param("host", "a.internal:27018,b.internal")
        .with_param("port", "27017");
    assert_eq!(
        ConnectionSummary::of(&seeds, Some(&oxyn_driver_mongodb::mongodb_metadata()))
            .location
            .as_deref(),
        Some("a.internal:27018,b.internal"),
        "a seed list keeps its own ports"
    );
}

#[test]
fn an_edit_debug_shows_keys_not_values_and_needs_its_marking() {
    let edit: ConnectionEdit = serde_json::from_str(
        r#"{"name":"prod","environment":"production","privacyTier":"local","readOnly":true,
                "values":{"host":"db.internal"},"secrets":{"password":"hunter2"}}"#,
    )
    .expect("valid edit");
    let rendered = format!("{edit:?}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(!rendered.contains("db.internal"), "{rendered}");

    for missing in [
        r#"{"name":"x","privacyTier":"local","readOnly":false}"#,
        r#"{"name":"x","environment":"local","readOnly":false}"#,
        r#"{"name":"x","environment":"local","privacyTier":"local"}"#,
    ] {
        assert!(
            serde_json::from_str::<ConnectionEdit>(missing).is_err(),
            "an edit without its marking is accepted: {missing}"
        );
    }
}

#[test]
fn a_patch_changes_only_what_it_names() {
    let mut preferences = WorkspacePreferences {
        appearance: Appearance::Light,
        sidebar_collapsed: true,
        ..Default::default()
    };
    let change: PreferencesChange =
        serde_json::from_str(r#"{"theme":"system","groupThousands":true}"#).expect("patch");
    change.apply(&mut preferences);
    assert!(preferences.follow_system_appearance);
    assert_eq!(
        preferences.appearance,
        Appearance::Light,
        "the explicit palette survives"
    );
    assert!(preferences.group_thousands);
    assert!(preferences.sidebar_collapsed, "an unnamed field is kept");
    assert_eq!(
        DisplayPreferences::of(&preferences).theme,
        ThemeChoice::System
    );
    assert!(
        serde_json::from_str::<PreferencesChange>(r#"{"objectLocation":"x"}"#).is_err(),
        "a field this surface does not offer is refused"
    );
}
