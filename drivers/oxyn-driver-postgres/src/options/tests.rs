use oxyn_core::DriverId;

use super::*;
use crate::driver::postgres_metadata;

const PASSWORD: &str = "hunter2";

fn config() -> ConnectionConfig {
    ConnectionConfig::new("shop", DriverId::postgres())
        .with_param("host", "db.internal")
        .with_param("port", "6543")
        .with_param("database", "shop")
        .with_param("user", "read_request")
}

#[test]
fn debug_never_shows_the_password() {
    // I-03, checkable corollary. This test is the module's reason to exist.
    let spec = ConnectSpec::from_config(
        &postgres_metadata(),
        &config(),
        &Credentials::new().with_password(PASSWORD),
    )
    .expect("complete configuration");

    let rendered = format!("{spec:?}");
    assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");

    // What remains must stay useful for diagnostics.
    assert!(rendered.contains("db.internal"), "{rendered}");
    assert!(rendered.contains("6543"), "{rendered}");
    assert!(rendered.contains("read_request"), "{rendered}");
}

#[test]
fn recognized_parameters_reach_sqlx() {
    let spec = ConnectSpec::from_config(&postgres_metadata(), &config(), &Credentials::new())
        .expect("complete configuration");
    assert_eq!(spec.host(), "db.internal");
    assert_eq!(spec.port(), 6543);
    assert_eq!(spec.database(), "shop");
    assert_eq!(spec.user(), "read_request");
    assert_eq!(spec.ssl_mode(), "verify-full");
}

#[test]
fn omitted_or_empty_tls_mode_verifies_the_server_in_every_environment() {
    for config in [
        config(),
        config().with_environment(Environment::Production),
        config().with_environment(Environment::Staging),
        config().with_environment(Environment::Development),
        config().with_environment(Environment::Local),
    ] {
        for config in [
            config.clone(),
            config.clone().with_param("sslmode", ""),
            config.with_param("ssl_mode", "  "),
        ] {
            let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &Credentials::new())
                .expect("the default TLS mode must authenticate the server");
            assert_eq!(spec.ssl_mode(), "verify-full");
            assert!(matches!(
                spec.options().get_ssl_mode(),
                PgSslMode::VerifyFull
            ));
        }
    }
}

#[test]
fn weaker_tls_modes_are_refused_outside_local() {
    for config in [
        config(),
        config().with_environment(Environment::Production),
        config().with_environment(Environment::Staging),
        config().with_environment(Environment::Development),
    ] {
        for key in ["sslmode", "ssl_mode"] {
            for mode in [
                "disable",
                "allow",
                "prefer",
                "require",
                "verify-ca",
                " VERIFY_CA ",
            ] {
                let error = ConnectSpec::from_config(
                    &postgres_metadata(),
                    &config.clone().with_param(key, mode),
                    &Credentials::new(),
                )
                .expect_err("only Local may weaken server authentication");
                let OxynError::Config(message) = error else {
                    panic!("a weak TLS mode must be a configuration error");
                };
                assert!(message.contains("verify-full"), "{message}");
                assert!(message.contains("Local"), "{message}");
            }
        }
    }
}

#[test]
fn the_default_port_is_the_protocol_s() {
    let without_port = ConnectionConfig::new("shop", DriverId::postgres())
        .with_param("host", "localhost")
        .with_param("database", "shop")
        .with_param("user", "read_request");
    let spec = ConnectSpec::from_config(&postgres_metadata(), &without_port, &Credentials::new())
        .expect("complete configuration");
    assert_eq!(spec.port(), DEFAULT_PORT);
}

#[test]
fn an_unreadable_port_is_refused_with_a_message_that_says_what_to_do() {
    let bad = config().with_param("port", "five-thousand");
    let error = ConnectSpec::from_config(&postgres_metadata(), &bad, &Credentials::new())
        .expect_err("refusal expected");
    assert!(matches!(error, OxynError::Config(_)), "{error:?}");
    assert!(error.to_string().contains("65535"), "{error}");
}

#[test]
fn an_unknown_sslmode_is_refused_rather_than_silently_downgraded() {
    // Downgrading it to `prefer` would open a connection in clear where the
    // user asked for encryption.
    let bad = config().with_param("sslmode", "maybe");
    let error = ConnectSpec::from_config(&postgres_metadata(), &bad, &Credentials::new())
        .expect_err("refusal expected");
    assert!(matches!(error, OxynError::Config(_)), "{error:?}");
    assert!(error.to_string().contains("verify-full"), "{error}");
}

#[test]
fn the_six_libpq_tls_modes_are_accepted_locally() {
    for name in [
        "disable",
        "allow",
        "prefer",
        "require",
        "verify-ca",
        "verify-full",
    ] {
        let with_mode = config()
            .with_environment(Environment::Local)
            .with_param("sslmode", name);
        let spec = ConnectSpec::from_config(&postgres_metadata(), &with_mode, &Credentials::new())
            .expect("known mode");
        assert_eq!(spec.ssl_mode(), name);
    }
}

#[test]
fn a_secret_stored_in_the_parameters_is_refused() {
    // The configuration is persisted: a secret has no place in it (I-03).
    // `DriverMetadata::validate` is what refuses, and this module calls it.
    let leak = config().with_param("password", PASSWORD);
    let error = ConnectSpec::from_config(&postgres_metadata(), &leak, &Credentials::new())
        .expect_err("refusal expected");
    assert!(matches!(error, OxynError::Config(_)), "{error:?}");
    assert!(!error.to_string().contains(PASSWORD), "{error}");
}

#[test]
fn a_missing_required_field_is_named() {
    let without_database = ConnectionConfig::new("shop", DriverId::postgres())
        .with_param("host", "localhost")
        .with_param("user", "read_request");
    let error =
        ConnectSpec::from_config(&postgres_metadata(), &without_database, &Credentials::new())
            .expect_err("refusal expected");
    assert!(error.to_string().contains("database"), "{error}");
}

#[test]
fn a_configuration_targeting_another_driver_is_refused() {
    let elsewhere = ConnectionConfig::new("fichier", DriverId::sqlite())
        .with_param("host", "localhost")
        .with_param("database", "shop")
        .with_param("user", "read_request");
    let error = ConnectSpec::from_config(&postgres_metadata(), &elsewhere, &Credentials::new())
        .expect_err("refusal expected");
    assert!(matches!(error, OxynError::Config(_)), "{error:?}");
}
