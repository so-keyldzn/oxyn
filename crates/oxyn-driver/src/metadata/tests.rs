use oxyn_core::ConnectionConfig;

use super::*;

fn postgres_metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
        .with_default_port(5432)
        .with_fields([
            ConnectionField::new("host", "Host", FieldKind::Text)
                .required()
                .with_default("localhost"),
            ConnectionField::new("port", "Port", FieldKind::Number).with_default("5432"),
            ConnectionField::new("user", "User", FieldKind::Text).required(),
            ConnectionField::new("password", "Password", FieldKind::Password),
            ConnectionField::new("database", "Database", FieldKind::Text).required(),
            ConnectionField::new(
                "sslmode",
                "TLS mode",
                FieldKind::Choice(vec!["disable".into(), "require".into()]),
            )
            .with_default("require"),
        ])
}

#[test]
fn a_consistent_declaration_passes_the_check() {
    postgres_metadata()
        .check()
        .expect("the reference declaration is consistent");
}

#[test]
fn a_secret_field_cannot_carry_a_default_value() {
    // A default value on a secret field ends up in clear text in the
    // binary and in the interface.
    let metadata =
        DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
            .with_field(
                ConnectionField::new("password", "Password", FieldKind::Password)
                    .with_default("postgres"),
            );
    let err = metadata.check().expect_err("refusal expected");
    assert!(err.to_string().contains("password"), "{err}");
}

#[test]
fn a_key_declared_twice_is_refused() {
    let metadata = DriverMetadata::new(DriverId::mysql(), "MySQL", DriverFamily::Relational)
        .with_field(ConnectionField::new("host", "Host", FieldKind::Text))
        .with_field(ConnectionField::new("host", "Server", FieldKind::Text));
    assert!(metadata.check().is_err());
}

#[test]
fn a_password_in_the_parameters_is_refused() {
    // I-03 made checkable: this is exactly the workspace file committed
    // into the team's repository.
    let metadata = postgres_metadata();
    let config = ConnectionConfig::new("prod-eu", DriverId::postgres())
        .with_param("host", "db.internal.example")
        .with_param("user", "app")
        .with_param("database", "till")
        .with_param("password", "hunter2");

    let err = metadata
        .validate(&config)
        .expect_err("a persisted password must be refused");
    let rendered = err.to_string();
    assert!(
        rendered.contains("password"),
        "the key is named: {rendered}"
    );
    assert!(
        !rendered.contains("hunter2"),
        "the value must never leave: {rendered}"
    );
}

#[test]
fn a_key_that_looks_like_a_secret_is_refused_even_undeclared() {
    let metadata = postgres_metadata();
    for key in ["sslpassword", "auth_token", "API_KEY", "pwd"] {
        let config = ConnectionConfig::new("x", DriverId::postgres())
            .with_param("host", "h")
            .with_param("user", "u")
            .with_param("database", "d")
            .with_param(key, "sensitive-value");
        assert!(
            metadata.validate(&config).is_err(),
            "`{key}` looks like a secret and should have been refused"
        );
    }
}

#[test]
fn legitimate_parameters_are_not_taken_for_secrets() {
    // False positives that would break real drivers: `sslkey` is a file
    // path, `authSource` a MongoDB database.
    for key in ["sslkey", "sslcert", "authSource", "keyspace", "sslmode"] {
        assert!(
            !looks_like_secret(key),
            "`{key}` is not a secret and must not be refused"
        );
    }
    for key in ["password", "sslpassword", "PWD", "api_key", "token"] {
        assert!(looks_like_secret(key), "`{key}` carries a secret");
    }
}

#[test]
fn a_missing_required_field_is_reported() {
    let metadata = postgres_metadata();
    let config = ConnectionConfig::new("x", DriverId::postgres())
        .with_param("host", "h")
        .with_param("user", "u");
    let err = metadata
        .validate(&config)
        .expect_err("`database` is required");
    assert!(err.to_string().contains("database"), "{err}");
}

#[test]
fn a_required_field_with_a_default_asks_for_nothing() {
    // `host` is required but carries `localhost`: not typing it is
    // legitimate.
    let metadata = postgres_metadata();
    let config = ConnectionConfig::new("x", DriverId::postgres())
        .with_param("user", "u")
        .with_param("database", "d");
    metadata
        .validate(&config)
        .expect("the default covers the missing input");
}

#[test]
fn a_configuration_for_another_driver_is_refused() {
    let metadata = postgres_metadata();
    let config = ConnectionConfig::new("x", DriverId::sqlite());
    assert!(metadata.validate(&config).is_err());
}

#[test]
fn a_hostile_key_does_not_come_out_in_the_message() {
    // A key may come from a file written by a third party: it may carry
    // control sequences, or the secret itself.
    let hostile = "password\u{1b}[2Jhunter2";
    assert_eq!(sanitize_key(hostile), "<unrepresentable key>");
    assert_eq!(sanitize_key("sslmode"), "sslmode");
    assert_eq!(sanitize_key(&"x".repeat(65)), "<unrepresentable key>");
}

#[test]
fn sorting_goes_by_family_then_by_name() {
    let postgres = postgres_metadata();
    let clickhouse = DriverMetadata::new(
        DriverId::new("clickhouse").expect("valid identifier"),
        "ClickHouse",
        DriverFamily::Analytical,
    );
    assert!(postgres.sort_key() < clickhouse.sort_key());
}

#[test]
fn secret_fields_are_found() {
    let metadata = postgres_metadata();
    let secrets: Vec<&str> = metadata.secret_fields().map(|f| f.key.as_str()).collect();
    assert_eq!(secrets, ["password"]);
    assert!(metadata.field("sslmode").is_some_and(|f| !f.is_secret()));
}
