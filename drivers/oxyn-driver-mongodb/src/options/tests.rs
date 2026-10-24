use mongodb::options::Tls;
use oxyn_core::{ConnectionConfig, DriverId, Environment};
use oxyn_driver::Credentials;
use secrecy::SecretString;

use super::*;
use crate::driver::mongodb_metadata;

fn config(environment: Environment) -> ConnectionConfig {
    ConnectionConfig::new("t", DriverId::mongodb()).with_environment(environment)
}

fn spec(config: &ConnectionConfig) -> Result<ConnectSpec> {
    ConnectSpec::from_config(&mongodb_metadata(), config, &Credentials::new())
}

#[test]
fn a_bare_configuration_targets_localhost_with_verified_tls() {
    let spec = spec(&config(Environment::Production)).expect("defaults are complete");
    assert_eq!(spec.tls(), TlsMode::VerifyFull);
    assert_eq!(spec.database(), None);
    assert_eq!(
        spec.overview.seeds,
        Seeds::Hosts(vec![("localhost".to_owned(), DEFAULT_PORT)])
    );
}

#[test]
fn a_weak_tls_mode_needs_a_local_connection() {
    for mode in ["require", "disable"] {
        let production = config(Environment::Production).with_param("tls", mode);
        let error = spec(&production).expect_err("refused outside Local");
        assert!(error.to_string().contains("Local"), "{error}");
        let local = config(Environment::Local).with_param("tls", mode);
        assert!(spec(&local).is_ok(), "{mode} on Local");
    }
}

#[test]
fn hosts_lists_ports_and_ipv6_are_read() {
    let parsed = spec(
        &config(Environment::Local)
            .with_param("host", "a.example:27018, b.example ,[::1]:27019,[fe80::1]")
            .with_param("port", "27100"),
    )
    .expect("a seed list");
    assert_eq!(
        parsed.overview.seeds,
        Seeds::Hosts(vec![
            ("a.example".to_owned(), 27018),
            ("b.example".to_owned(), 27100),
            ("::1".to_owned(), 27019),
            ("fe80::1".to_owned(), 27100),
        ])
    );
}

#[test]
fn a_host_cannot_smuggle_connection_string_options() {
    for host in ["h/?authSource=x", "user@h", "h#x", "h, ", "a b"] {
        let error =
            spec(&config(Environment::Local).with_param("host", host)).expect_err("not a host");
        assert!(
            !error.to_string().contains(host),
            "the value is not echoed: {error}"
        );
    }
}

#[test]
fn a_dns_seed_list_names_one_dns_name() {
    let ok = config(Environment::Production)
        .with_param("srv", "true")
        .with_param("host", "cluster0.example.net");
    assert!(matches!(
        spec(&ok).expect("srv").overview.seeds,
        Seeds::Srv(_)
    ));
    for host in [
        "a.example,b.example",
        "a.example:27017",
        "a/b",
        "a..b",
        "a_b.example",
    ] {
        let refused = config(Environment::Production)
            .with_param("srv", "true")
            .with_param("host", host);
        assert!(spec(&refused).is_err(), "{host}");
    }
    let direct = config(Environment::Production)
        .with_param("srv", "true")
        .with_param("host", "cluster0.example.net")
        .with_param("directConnection", "true");
    assert!(spec(&direct).is_err());
}

#[test]
fn an_unknown_parameter_is_refused_rather_than_ignored() {
    let error = spec(&config(Environment::Local).with_param("readPreference", "secondary"))
        .expect_err("not applied by the driver");
    assert!(error.to_string().contains("readPreference"), "{error}");
}

#[test]
fn the_debug_form_never_shows_the_password() {
    let credentials = Credentials::new().with_password(SecretString::from("hunter2-very-secret"));
    let spec = ConnectSpec::from_config(
        &mongodb_metadata(),
        &config(Environment::Local).with_param("user", "app"),
        &credentials,
    )
    .expect("complete");
    let shown = format!("{spec:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(shown.contains("<redacted>"), "{shown}");
}

#[test]
fn a_password_without_a_user_is_refused() {
    let credentials = Credentials::new().with_password(SecretString::from("x"));
    assert!(
        ConnectSpec::from_config(
            &mongodb_metadata(),
            &config(Environment::Local),
            &credentials
        )
        .is_err()
    );
}

#[test]
fn tls_files_must_be_absolute_and_useful() {
    let relative = config(Environment::Production).with_param("tlsCAFile", "ca.pem");
    assert!(spec(&relative).is_err());
    let disabled = config(Environment::Local)
        .with_param("tls", "disable")
        .with_param("tlsCAFile", "/etc/ca.pem");
    assert!(spec(&disabled).is_err());
}

#[tokio::test]
async fn the_options_never_retry_and_carry_the_credential() {
    let credentials = Credentials::new().with_password(SecretString::from("pw"));
    let spec = ConnectSpec::from_config(
        &mongodb_metadata(),
        &config(Environment::Local)
            .with_param("user", "app")
            .with_param("database", "shop")
            .with_param("authMechanism", "SCRAM-SHA-256")
            .with_param("tls", "require"),
        &credentials,
    )
    .expect("complete");
    let options = spec.client_options().await.expect("no DNS needed");
    assert_eq!(options.retry_reads, Some(false));
    assert_eq!(options.retry_writes, Some(false));
    assert_eq!(options.app_name.as_deref(), Some("oxyn"));
    let credential = options.credential.expect("a user is named");
    assert_eq!(credential.username.as_deref(), Some("app"));
    assert_eq!(
        credential.source.as_deref(),
        Some("shop"),
        "the database, then admin"
    );
    assert_eq!(credential.mechanism, Some(AuthMechanism::ScramSha256));
    assert!(matches!(
        options.tls,
        Some(Tls::Enabled(tls)) if tls.allow_invalid_certificates == Some(true)
    ));
}
