use super::*;

#[test]
fn the_default_is_production() {
    // SECURITY: the default is the most restrictive value, not the most
    // permissive.
    assert_eq!(Environment::default(), Environment::Production);
    assert!(Environment::default().is_production());
    assert!(ConnectionConfig::new("no environment", DriverId::postgres()).is_production());
}

#[test]
fn an_environment_missing_from_the_file_means_production() {
    let json = r#"{
            "id": "018f0000-0000-7000-8000-000000000000",
            "name": "customer database",
            "driver": "postgres"
        }"#;
    let cfg: ConnectionConfig = serde_json::from_str(json).expect("deserialization");
    assert_eq!(
        cfg.environment,
        Environment::Production,
        "a missing field must never lower the protection"
    );
    assert!(!cfg.read_only);
    assert!(cfg.params.is_empty());
}

#[test]
fn a_connection_without_explicit_tier_means_metadata() {
    // ADR-0006: the default is safe. A connection whose tier is not filled
    // in is **never** `Sampled` — it is I-02's direction of caution applied
    // to the AI boundary.
    let cfg = ConnectionConfig::new("customer database", DriverId::postgres());
    assert_eq!(cfg.privacy_tier, PrivacyTier::Metadata);
    assert!(!cfg.privacy_tier.allows_row_values());
}

#[test]
fn a_tier_missing_from_the_file_means_metadata() {
    // The missing field is the real case: a workspace written before the
    // field existed. It must not be read back as `Sampled`.
    let json = r#"{
            "id": "018f0000-0000-7000-8000-000000000000",
            "name": "customer database",
            "driver": "postgres"
        }"#;
    let cfg: ConnectionConfig = serde_json::from_str(json).expect("deserialization");
    assert_eq!(cfg.privacy_tier, PrivacyTier::Metadata);
}

#[test]
fn the_tier_is_persisted_with_the_connection() {
    // The tier belongs to the connection: the round trip must be exact,
    // otherwise a workspace read back would lower — or widen — the protection.
    for tier in [
        PrivacyTier::Local,
        PrivacyTier::Metadata,
        PrivacyTier::Sampled,
    ] {
        let cfg = ConnectionConfig::new("sandbox", DriverId::sqlite()).with_privacy_tier(tier);
        let json = serde_json::to_string(&cfg).expect("serialization");
        let read_back: ConnectionConfig = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(read_back.privacy_tier, tier, "{json}");
        assert_eq!(tier.as_str().parse::<PrivacyTier>(), Ok(tier));
    }
    assert!("confidential".parse::<PrivacyTier>().is_err());
}

#[test]
fn tiers_are_ordered_from_least_to_most_disclosing() {
    assert!(PrivacyTier::Local < PrivacyTier::Metadata);
    assert!(PrivacyTier::Metadata < PrivacyTier::Sampled);
    assert_eq!(
        PrivacyTier::Sampled.most_restrictive(PrivacyTier::Metadata),
        PrivacyTier::Metadata
    );
    assert_eq!(
        PrivacyTier::Metadata.most_restrictive(PrivacyTier::Local),
        PrivacyTier::Local
    );

    assert!(!PrivacyTier::Local.allows_row_values());
    assert!(!PrivacyTier::Metadata.allows_row_values());
    assert!(PrivacyTier::Sampled.allows_row_values());

    assert!(!PrivacyTier::Local.allows_remote_provider());
    assert!(PrivacyTier::Metadata.allows_remote_provider());
    assert!(PrivacyTier::Sampled.allows_remote_provider());
}

#[test]
fn environments_are_ordered_from_least_to_most_restrictive() {
    assert!(Environment::Local < Environment::Development);
    assert!(Environment::Development < Environment::Staging);
    assert!(Environment::Staging < Environment::Production);
    assert_eq!(
        Environment::Local.max(Environment::Production),
        Environment::Production
    );
}

#[test]
fn environment_parsing_and_rendering() {
    for env in [
        Environment::Local,
        Environment::Development,
        Environment::Staging,
        Environment::Production,
    ] {
        let read_back: Environment = env.as_str().parse().expect("round trip");
        assert_eq!(env, read_back);
    }
    assert_eq!(
        "prod"
            .parse::<Environment>()
            .expect("abbreviation accepted"),
        Environment::Production
    );
    assert!("uat".parse::<Environment>().is_err());
}

#[test]
fn the_debug_leaks_no_parameter_value() {
    let cfg = ConnectionConfig::new("prod-eu", DriverId::postgres())
        .with_param("host", "db.internal.example")
        .with_param("sslmode", "require")
        .with_secret_ref("oxyn/connection/prod-eu");

    let rendered = format!("{cfg:?}");

    assert!(
        !rendered.contains("db.internal.example"),
        "host leaked: {rendered}"
    );
    assert!(!rendered.contains("require"), "value leaked: {rendered}");
    assert!(
        !rendered.contains("oxyn/connection/prod-eu"),
        "secret reference leaked: {rendered}"
    );
    assert!(
        !rendered.contains(&cfg.id.to_string()),
        "identifier leaked: {rendered}"
    );

    // What remains must remain useful for diagnosis.
    assert!(
        rendered.contains("prod-eu"),
        "the name is showable: {rendered}"
    );
    assert!(
        rendered.contains("host"),
        "the keys are showable: {rendered}"
    );
    assert!(rendered.contains("postgres"));
    // `Production` and not `production`: the derived `Debug` renders the
    // Rust variant name. Lowercase is the serde form
    // (`rename_all = "lowercase"`), which only applies to what is persisted.
    assert!(
        rendered.contains("Production"),
        "the environment must stay readable: {rendered}"
    );
}

#[test]
fn what_is_persisted_only_contains_a_secret_reference() {
    // The only path for a secret is `secret_ref`, and it designates a
    // keychain entry — never the value. That is what keeps a workspace file
    // committed to a team repository from carrying away a production
    // password.
    let cfg = ConnectionConfig::new("prod-eu", DriverId::postgres())
        .with_param("host", "db.internal.example")
        .with_secret_ref("oxyn/connection/prod-eu");

    let json = serde_json::to_string(&cfg).expect("serialization");

    assert!(!json.contains("password"), "suspicious field: {json}");
    assert!(
        json.contains("oxyn/connection/prod-eu"),
        "the reference is persisted"
    );

    let read_back: ConnectionConfig = serde_json::from_str(&json).expect("deserialization");
    assert_eq!(read_back, cfg, "the round trip must be faithful");
    assert_eq!(
        read_back.secret_ref.as_deref(),
        Some("oxyn/connection/prod-eu")
    );
}

#[test]
fn parameter_order_is_kept() {
    let cfg = ConnectionConfig::new("x", DriverId::postgres())
        .with_param("host", "h")
        .with_param("port", "5432")
        .with_param("dbname", "d");
    let keys: Vec<_> = cfg.params.keys().map(String::as_str).collect();
    assert_eq!(keys, ["host", "port", "dbname"]);
}

#[test]
fn a_read_only_connection_is_declared() {
    let cfg = ConnectionConfig::new("replica", DriverId::postgres()).read_only();
    assert!(cfg.read_only);
}
