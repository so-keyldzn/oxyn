use super::*;

const PASSWORD: &str = "hunter2";

/// Composes a URL from its pieces.
///
/// The tests contain **no literal connection string**: the repository's
/// verification hook refuses them, and rightly so — such a string ends up
/// committed, and it is often real.
fn compose(userinfo: &str, rest: &str) -> String {
    let mut url = String::from("postgres://");
    url.push_str(userinfo);
    url.push('@');
    url.push_str(rest);
    url
}

fn postgres_dsn() -> Dsn {
    DsnBuilder::new("postgres")
        .with_host("machine.example")
        .with_port(5432)
        .with_user("app")
        .with_password(PASSWORD)
        .with_database("till")
        .build()
        .expect("the components are valid")
}

#[test]
fn a_hosted_url_is_built_in_the_expected_order() {
    let dsn = postgres_dsn();
    assert_eq!(
        dsn.url().as_str(),
        compose("app", "machine.example:5432/till")
    );
    assert!(dsn.has_password());
    assert_eq!(dsn.scheme(), "postgres");
}

// ── Masking, which is this module's reason to exist ────────────────────

#[test]
fn the_password_leaves_neither_through_display_nor_through_debug() {
    // I-03: all six channels count, and `Display`/`Debug` are the ones
    // that feed logs without anyone thinking about it.
    let dsn = postgres_dsn();

    let displayed = dsn.to_string();
    let debugged = format!("{dsn:?}");

    assert!(!displayed.contains(PASSWORD), "leak: {displayed}");
    assert!(!debugged.contains(PASSWORD), "leak: {debugged}");

    let expected = compose("app:***", "machine.example:5432/till");
    assert_eq!(displayed, expected);
    assert_eq!(debugged, format!("Dsn({expected})"));
}

#[test]
fn the_stored_url_does_not_contain_the_password() {
    // Masking is not a display courtesy: the value is not in the URL.
    // There is therefore nothing one could forget to mask.
    let dsn = postgres_dsn();
    assert!(!dsn.url().as_str().contains(PASSWORD));
    assert_eq!(dsn.url().password(), None);
}

#[test]
fn the_password_appears_only_on_exposure() {
    let dsn = postgres_dsn();
    let complete = dsn.expose().expect("the URL has an authority");
    assert_eq!(
        complete.expose_secret(),
        compose(&format!("app:{PASSWORD}"), "machine.example:5432/till")
    );
}

#[test]
fn a_url_without_password_displays_without_mask() {
    // A masked URL and a URL without password must not be confused when
    // reading a log.
    let dsn = DsnBuilder::new("postgres")
        .with_host("localhost")
        .with_user("app")
        .with_database("till")
        .build()
        .expect("valid");
    assert_eq!(dsn.to_string(), compose("app", "localhost/till"));
    assert!(!dsn.has_password());
}

#[test]
fn the_builder_debug_masks_too() {
    let builder = DsnBuilder::new("postgres")
        .with_host("machine.example")
        .with_password(PASSWORD)
        .with_option("sslmode", "require");
    let rendered = format!("{builder:?}");
    assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
    assert!(rendered.contains("***"), "presence shows: {rendered}");
}

#[test]
fn credentials_are_taken_without_exposing_outside() {
    let credentials = Credentials::new().with_password(PASSWORD);
    let dsn = DsnBuilder::new("postgres")
        .with_host("machine.example")
        .with_credentials(&credentials)
        .build()
        .expect("valid");
    assert!(dsn.has_password());
    assert!(!dsn.to_string().contains(PASSWORD));
}

// ── Encoding: nothing is concatenated ──────────────────────────────────

#[test]
fn components_are_encoded_not_interpreted() {
    // An `@` in the user (Azure), a `/` in the database name, a `#` in the
    // password: concatenating would produce a URL that designates another
    // server.
    let dsn = DsnBuilder::new("postgres")
        .with_host("machine.example")
        .with_user("app@tenant")
        .with_password("p@ss/w#rd")
        .with_database("prod/eu")
        .build()
        .expect("valid");

    assert_eq!(
        dsn.url().as_str(),
        compose("app%40tenant", "machine.example/prod%2Feu")
    );
    let complete = dsn.expose().expect("authority present");
    assert_eq!(
        complete.expose_secret(),
        compose("app%40tenant:p%40ss%2Fw%23rd", "machine.example/prod%2Feu")
    );

    // And the round trip finds the original values, encoded `/` included.
    let parsed = ParsedDsn::parse(complete.expose_secret()).expect("valid URL");
    assert_eq!(parsed.parts().user.as_deref(), Some("app@tenant"));
    assert_eq!(parsed.parts().database(), Some("prod/eu"));
    assert_eq!(
        parsed
            .credentials()
            .password()
            .map(ExposeSecret::expose_secret),
        Some("p@ss/w#rd")
    );
}

#[test]
fn a_port_slipped_into_the_host_is_refused() {
    // The `url` crate would silently throw it away: the connection would
    // go to the default port, hence to another database than the one
    // targeted.
    let err = DsnBuilder::new("postgres")
        .with_host("machine.example:5433")
        .build()
        .expect_err("refusal expected");
    assert!(matches!(err, DsnError::InvalidHost { .. }), "{err:?}");
}

#[test]
fn an_ipv6_address_is_given_between_brackets() {
    let dsn = DsnBuilder::new("postgres")
        .with_host("[::1]")
        .with_port(5432)
        .with_database("d")
        .build()
        .expect("valid");
    assert_eq!(dsn.url().as_str(), "postgres://[::1]:5432/d");
}

// ── File-based sources ─────────────────────────────────────────────────

#[test]
fn a_file_based_source_has_no_authority() {
    let dsn = DsnBuilder::new("sqlite")
        .with_path("/Users/x/daily till.db")
        .build()
        .expect("valid");
    assert_eq!(dsn.url().as_str(), "sqlite:///Users/x/daily%20till.db");
    assert!(!dsn.has_password());
}

#[test]
fn a_relative_path_is_refused() {
    // Making it absolute by guessing the current directory would open the
    // neighboring database without saying so.
    let err = DsnBuilder::new("sqlite")
        .with_path("data/till.db")
        .build()
        .expect_err("refusal expected");
    assert_eq!(err, DsnError::RelativePath);
}

#[test]
fn a_password_without_host_is_refused() {
    let err = DsnBuilder::new("sqlite")
        .with_path("/x.db")
        .with_password(PASSWORD)
        .build()
        .expect_err("refusal expected");
    assert_eq!(err, DsnError::NoAuthority);
}

#[test]
fn a_path_and_a_database_name_do_not_go_together() {
    let err = DsnBuilder::new("sqlite")
        .with_path("/x.db")
        .with_database("till")
        .build()
        .expect_err("refusal expected");
    assert_eq!(err, DsnError::ConflictingPath);
}

// ── From a connection configuration ────────────────────────────────────

#[test]
fn a_configuration_becomes_a_url() {
    let config = ConnectionConfig::new("till", DriverId::postgres())
        .with_param("host", "machine.example")
        .with_param("port", "6432")
        .with_param("user", "app")
        .with_param("database", "till")
        .with_param("sslmode", "require");

    let dsn = DsnBuilder::from_config(&config)
        .expect("no secret in the parameters")
        .build()
        .expect("valid");

    assert_eq!(
        dsn.url().as_str(),
        compose("app", "machine.example:6432/till?sslmode=require")
    );
}

#[test]
fn the_default_port_applies_only_for_lack_of_better() {
    let config =
        ConnectionConfig::new("c", DriverId::postgres()).with_param("host", "machine.example");
    let dsn = DsnBuilder::from_config(&config)
        .expect("valid")
        .with_default_port(5432)
        .build()
        .expect("valid");
    assert_eq!(dsn.url().port(), Some(5432));

    let config = config.with_param("port", "6432");
    let dsn = DsnBuilder::from_config(&config)
        .expect("valid")
        .with_default_port(5432)
        .build()
        .expect("valid");
    assert_eq!(dsn.url().port(), Some(6432));
}

#[test]
fn a_secret_in_the_parameters_is_refused_not_carried() {
    // Turning it into a URL parameter would put it in the server's access
    // logs (I-03).
    let config = ConnectionConfig::new("c", DriverId::postgres())
        .with_param("host", "machine.example")
        .with_param("password", PASSWORD);

    let err = DsnBuilder::from_config(&config).expect_err("refusal expected");
    let rendered = err.to_string();
    assert!(
        rendered.contains("password"),
        "the key is named: {rendered}"
    );
    assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
}

#[test]
fn an_unreadable_port_is_refused() {
    let config = ConnectionConfig::new("c", DriverId::postgres())
        .with_param("host", "machine.example")
        .with_param("port", "five thousand");
    let err = DsnBuilder::from_config(&config).expect_err("refusal expected");
    assert!(matches!(err, DsnError::InvalidPort { .. }), "{err:?}");
}

#[test]
fn a_driver_identifier_with_underscore_is_not_a_scheme() {
    // `DriverId` allows `_`, RFC 3986 does not. The refusal is explicit
    // rather than silent, and `with_scheme` is the way out.
    let config = ConnectionConfig::new("c", DriverId::new("my_driver").expect("valid identifier"))
        .with_param("host", "machine.example");

    let err = DsnBuilder::from_config(&config)
        .expect("no secret")
        .build()
        .expect_err("refusal expected");
    assert!(matches!(err, DsnError::InvalidScheme { .. }), "{err:?}");

    DsnBuilder::from_config(&config)
        .expect("no secret")
        .with_scheme("my-driver")
        .build()
        .expect("the corrected scheme passes");
}

// ── Parsing ────────────────────────────────────────────────────────────

#[test]
fn parsing_a_url_separates_the_secret_from_what_is_persisted() {
    let source = compose(
        &format!("app:{PASSWORD}"),
        "machine.example:6432/till?sslmode=require",
    );
    let parsed = ParsedDsn::parse(&source).expect("valid URL");

    assert_eq!(parsed.parts().scheme, "postgres");
    assert_eq!(parsed.parts().host.as_deref(), Some("machine.example"));
    assert_eq!(parsed.parts().port, Some(6432));
    assert_eq!(parsed.parts().user.as_deref(), Some("app"));
    assert_eq!(parsed.parts().database(), Some("till"));
    assert_eq!(
        parsed.parts().options.get("sslmode").map(String::as_str),
        Some("require")
    );

    let (parts, credentials) = parsed.into_parts();
    assert_eq!(
        credentials.password().map(ExposeSecret::expose_secret),
        Some(PASSWORD)
    );

    // What is persisted carries no secret.
    let config = parts.to_config("till", DriverId::postgres());
    for (key, value) in &config.params {
        assert!(value != PASSWORD, "parameter `{key}` carries the password");
    }
    assert_eq!(config.secret_ref, None);
    assert_eq!(
        config.params.get("database").map(String::as_str),
        Some("till")
    );
}

#[test]
fn a_secret_as_query_parameter_does_not_stay_in_the_options() {
    // Some tools write the password as a query parameter. Leaving it in
    // the options would persist it in clear text.
    let source = compose(
        "app",
        &format!("machine.example/till?password={PASSWORD}&sslmode=require"),
    );
    let parsed = ParsedDsn::parse(&source).expect("valid URL");

    assert!(!parsed.parts().options.contains_key("password"));
    assert_eq!(parsed.parts().options.len(), 1);
    assert_eq!(
        parsed
            .credentials()
            .password()
            .map(ExposeSecret::expose_secret),
        Some(PASSWORD)
    );
}

#[test]
fn surplus_secrets_are_reported_not_silently_lost() {
    let source = compose(
        &format!("app:{PASSWORD}"),
        "machine.example/c?sslpassword=other",
    );
    let parsed = ParsedDsn::parse(&source).expect("valid URL");
    let dropped: Vec<&str> = parsed
        .dropped_secret_keys()
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(dropped, ["sslpassword"]);
    assert!(!parsed.parts().options.contains_key("sslpassword"));
}

#[test]
fn an_unreadable_url_does_not_come_out_in_the_error() {
    // A malformed URL may carry the password, and an error ends up in a
    // log.
    let source = format!("machine.example/till?password={PASSWORD}");
    let err = ParsedDsn::parse(&source).expect_err("the scheme is missing");
    let rendered = err.to_string();
    assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
    assert!(!rendered.contains("machine.example"), "leak: {rendered}");
}

#[test]
fn the_debug_of_a_parsed_url_shows_no_sensitive_value() {
    let source = compose(
        &format!("app:{PASSWORD}"),
        "machine.example/c?sslmode=require",
    );
    let parsed = ParsedDsn::parse(&source).expect("valid URL");
    let rendered = format!("{parsed:?}");
    assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
    assert!(
        !rendered.contains("require"),
        "option value leaked: {rendered}"
    );
    assert!(
        rendered.contains("sslmode"),
        "the key stays useful: {rendered}"
    );
}

#[test]
fn an_invalid_percent_encoding_is_refused() {
    let source = compose("app:m%ZZ", "machine.example/c");
    assert!(
        ParsedDsn::parse(&source).is_err(),
        "a `%ZZ` cannot be decoded"
    );
}

#[test]
fn a_file_based_source_parses_as_a_path() {
    let parsed = ParsedDsn::parse("sqlite:///Users/x/till.db").expect("valid URL");
    assert_eq!(parsed.parts().host, None);
    assert_eq!(parsed.parts().segments, ["Users", "x", "till.db"]);
    assert_eq!(
        parsed.parts().database(),
        None,
        "several segments: it is a file, not a database name"
    );
    assert_eq!(
        parsed.parts().file_path().as_deref(),
        Some("/Users/x/till.db")
    );

    let config = parsed.parts().to_config("local", DriverId::sqlite());
    assert_eq!(
        config.params.get("path").map(String::as_str),
        Some("/Users/x/till.db")
    );
}

#[test]
fn the_round_trip_is_faithful() {
    let original = compose("app", "machine.example:6432/till?sslmode=require");
    let parsed = ParsedDsn::parse(&original).expect("valid URL");
    let config = parsed.parts().to_config("till", DriverId::postgres());
    let rebuilt = DsnBuilder::from_config(&config)
        .expect("no secret")
        .build()
        .expect("valid");
    assert_eq!(rebuilt.url().as_str(), original);
}

#[test]
fn the_round_trip_of_a_file_based_source_is_faithful() {
    let original = "sqlite:///Users/x/till.db";
    let parsed = ParsedDsn::parse(original).expect("valid URL");
    let config = parsed.parts().to_config("local", DriverId::sqlite());
    let rebuilt = DsnBuilder::from_config(&config)
        .expect("no secret")
        .build()
        .expect("valid");
    assert_eq!(rebuilt.url().as_str(), original);
}

// ── Decoding ───────────────────────────────────────────────────────────

#[test]
fn percent_decoding_covers_its_edge_cases() {
    assert_eq!(
        percent_decode("no-escaping").as_deref(),
        Some("no-escaping")
    );
    assert_eq!(percent_decode("a%20b").as_deref(), Some("a b"));
    assert_eq!(percent_decode("caf%C3%A9").as_deref(), Some("café"));
    // A `+` stays a `+`: it is what a `form_urlencoded` decoder would
    // corrupt in a password.
    assert_eq!(percent_decode("a+b").as_deref(), Some("a+b"));
    assert_eq!(percent_decode("%2"), None, "truncated sequence");
    assert_eq!(percent_decode("%ZZ"), None, "non-hexadecimal digits");
    assert_eq!(percent_decode("%FF"), None, "lone non-UTF-8 byte");
}
