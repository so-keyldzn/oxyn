use super::*;

fn ollama() -> AiProviderConfig {
    AiProviderConfig::new(
        ProviderId::ollama(),
        AiProviderKind::OpenAiCompatible,
        "Laptop Ollama",
        "http://localhost:11434/v1",
        "llama3.2",
    )
}

#[test]
fn a_url_carrying_credentials_is_refused() {
    // ADR-0023: the `base_url` is stored stripped of its credentials. The
    // refusal is the chosen form — a silent cleanup would give the user
    // back a configuration they did not enter.
    let mut config = ollama();
    config.base_url = "https://alice:p4ssw0rd@api.example.com/v1".to_owned();

    let error = config
        .validate()
        .expect_err("a URL with credentials is not written");
    let message = error.to_string();
    assert!(!message.contains("p4ssw0rd"), "{message}");
    assert!(!message.contains("alice"), "{message}");

    // A user name alone is enough to refuse: it is already half of a
    // pair, and the password field will follow.
    config.base_url = "https://alice@api.example.com/v1".to_owned();
    assert!(config.validate().is_err());
}

#[test]
fn invisible_characters_are_refused_in_every_agent_field() {
    for character in [
        '\u{2028}',
        '\u{2029}',
        '\u{202E}',
        '\u{2066}',
        '\u{200B}',
        '\u{0600}',
        '\u{13430}',
    ] {
        for field in 0..5 {
            let mut agent = ExternalAgentConfig::new(ProviderId::for_new_agent(), "Agent", "agent");
            let hostile = format!("before{character}after");
            match field {
                0 => agent.label = hostile,
                1 => agent.command = hostile,
                2 => agent.args = vec![hostile],
                3 => agent.env = vec![(hostile, "value".to_owned())],
                _ => agent.env = vec![("MODE".to_owned(), hostile)],
            }
            assert!(
                agent.validate().is_err(),
                "{character:?} accepted in field {field}"
            );
        }
    }
}

#[test]
fn agent_command_and_each_argument_are_bounded_in_utf8_bytes() {
    let mut agent =
        ExternalAgentConfig::new(ProviderId::for_new_agent(), "Agent", "é".repeat(2048));
    agent.args = vec!["é".repeat(2048)];
    assert!(agent.validate().is_ok());
    agent.command.push('x');
    assert!(agent.validate().is_err(), "command above 4096 bytes");
    agent.command = "agent".to_owned();
    agent.args[0].push('x');
    assert!(agent.validate().is_err(), "argument above 4096 bytes");
}

/// Environment **values** are checked, not only the names.
///
/// The value half was not checked at all. The case that hurts is not
/// cosmetic: a value containing a NUL is silently truncated by the system
/// call at launch, so that the agent receives something other than what
/// the screen shows and what the local state persisted.
#[test]
fn a_hostile_environment_value_is_refused() {
    let base = ExternalAgentConfig::new(
        ProviderId::new("claude-code").expect("valid identifier"),
        "Claude Code",
        "claude",
    );
    let with_value = |value: String| {
        let mut agent = base.clone();
        agent.env = vec![("MODE".to_owned(), value)];
        agent
    };
    assert!(
        with_value("acp".to_owned()).validate().is_ok(),
        "an ordinary value stays accepted"
    );

    for (case, value) in [
        ("NUL", "acp\0rest".to_owned()),
        ("line break", "acp\nrest".to_owned()),
        ("too long", "v".repeat(MAX_AGENT_ENV_VALUE_BYTES + 1)),
    ] {
        assert!(
            with_value(value).validate().is_err(),
            "{case} must be refused"
        );
    }
}

/// The label and the variable names head the confirmation dialog: a
/// newline there pushes the command the user must read out of view.
#[test]
fn a_control_character_in_the_label_or_a_variable_name_is_refused() {
    let base = ExternalAgentConfig::new(
        ProviderId::new("claude-code").expect("valid identifier"),
        "Claude Code",
        "npx",
    );
    assert!(base.validate().is_ok());

    let mut agent = base.clone();
    agent.label = format!("Claude Code{}", "\n".repeat(40));
    assert!(agent.validate().is_err(), "newlines in the label");

    let mut agent = base.clone();
    agent.label = "Claude\u{1b}[2J".to_owned();
    assert!(agent.validate().is_err(), "escape in the label");

    let mut agent = base;
    agent.env = vec![("MODE\n\n\n".to_owned(), "acp".to_owned())];
    assert!(agent.validate().is_err(), "newlines in a variable name");
}

#[test]
fn a_url_without_credentials_is_accepted() {
    assert!(ollama().validate().is_ok());
    let mut config = ollama();
    config.base_url = "https://api.anthropic.com".to_owned();
    config.kind = AiProviderKind::Anthropic;
    assert!(config.validate().is_ok());
}

/// Stands for a key pasted after `?key=`. URL-safe on purpose (letters,
/// digits, dashes): percent-encoding by the parser could not hide it from
/// an assertion. And not shaped like any vendor's key (I-03, what does not
/// go into a test).
const QUERY_WITNESS: &str = "oxyn-witness-query-3b9d";

#[test]
fn a_url_carrying_a_query_or_fragment_is_refused_without_quoting_it() {
    // Presence is what is refused, not content: the two bare delimiters
    // parse to `Some("")` and slip through a test on emptiness.
    for suffix in [
        format!("?key={QUERY_WITNESS}"),
        format!("#{QUERY_WITNESS}"),
        format!("?key={QUERY_WITNESS}#{QUERY_WITNESS}"),
        "?".to_owned(),
        "#".to_owned(),
        "?region=eu".to_owned(),
    ] {
        let mut config = ollama();
        config.base_url = format!("https://api.example.com/v1{suffix}");

        let error = config
            .validate()
            .expect_err("a query or a fragment is not written");
        assert!(
            matches!(error, OxynError::Config(_)),
            "a refusal of the input, not another family: {error:?}"
        );
        for rendered in [error.to_string(), format!("{error:?}")] {
            assert!(
                !rendered.contains(QUERY_WITNESS),
                "witness leaked for `{suffix}`: {rendered}"
            );
            assert!(!rendered.contains("key="), "{rendered}");
            assert!(!rendered.contains("region"), "{rendered}");
            assert!(
                rendered.contains("its own field"),
                "the refusal points to the key field: {rendered}"
            );
        }
    }
}

/// The `Debug` protects a configuration **not yet validated**, for a query
/// as it does for credentials: the key a user pastes after `?key=` lives in
/// the `Command::SaveAiProvider` until the executor refuses it.
#[test]
fn a_url_carrying_a_query_is_not_rendered_before_validation() {
    for suffix in [format!("?key={QUERY_WITNESS}"), format!("#{QUERY_WITNESS}")] {
        let mut config = ollama();
        config.base_url = format!("https://api.example.com/v1{suffix}");

        // Without this line the test would prove the easy case.
        assert!(
            config.validate().is_err(),
            "validation does refuse a query or a fragment"
        );

        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains(QUERY_WITNESS),
            "secret leaked for `{suffix}`: {rendered}"
        );
        assert!(
            rendered.contains("api.example.com"),
            "the host stays diagnosable: {rendered}"
        );
    }
}

#[test]
fn a_query_less_url_is_still_accepted() {
    for base_url in [
        "http://localhost:11434/v1",
        "https://api.example.com",
        "https://api.example.com/v1/",
        "https://api.example.com:8443/openai/v1",
        "http://127.0.0.1:8080/",
    ] {
        let mut config = ollama();
        config.base_url = base_url.to_owned();
        assert!(config.validate().is_ok(), "wrongly refused: `{base_url}`");
    }
}

/// The justification of the refusal, proved rather than remembered
/// ([I-12](../../../CLAUDE.md#i-12)): every transport appends its path with
/// a relative `Url::join`, and the version of `url` in `Cargo.lock` drops
/// the base's query and fragment when it does. A query on a base URL was
/// therefore never sent — refusing it loses no working configuration. The
/// test fails if a `url` upgrade changes that.
#[test]
fn a_relative_join_drops_the_base_query() {
    for base in [
        format!("https://h/v1/?key={QUERY_WITNESS}"),
        format!("https://h/v1/#{QUERY_WITNESS}"),
        format!("https://h/v1/?key={QUERY_WITNESS}#{QUERY_WITNESS}"),
    ] {
        let base = Url::parse(&base).expect("the fixture is an absolute URL");
        for path in ["chat/completions", "models"] {
            let joined = base.join(path).expect("a relative path joins");
            assert_eq!(joined.query(), None, "{path}");
            assert_eq!(joined.fragment(), None, "{path}");
            assert!(!joined.as_str().contains(QUERY_WITNESS), "{path}");
            assert_eq!(joined.path(), format!("/v1/{path}"));
        }
    }
}

#[test]
fn same_endpoint_as_still_holds_for_query_less_urls() {
    let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
    for written in [
        "https://api.example.com/v1",
        "https://api.example.com/v1/",
        "https://api.example.com:443/v1",
        "HTTPS://API.Example.com:443/v1/",
    ] {
        let theirs = config_with(AiProviderKind::OpenAi, written);
        // The premise: both are declarations `validate` lets through, the
        // only kind that is stored now.
        assert!(mine.validate().is_ok() && theirs.validate().is_ok());
        assert!(mine.same_endpoint_as(&theirs), "`{written}`");
        assert!(theirs.same_endpoint_as(&mine), "`{written}`");
    }
    // And the line is still drawn where it was: another path is another
    // endpoint.
    let elsewhere = config_with(AiProviderKind::OpenAi, "https://api.example.com/v2");
    assert!(!mine.same_endpoint_as(&elsewhere));
}

#[test]
fn a_url_without_host_or_unreadable_is_refused() {
    for brut in [
        "",
        "not a url",
        "/v1/chat",
        "mailto:someone@example.com",
        "data:text/plain,hello",
    ] {
        let mut config = ollama();
        config.base_url = brut.to_owned();
        assert!(config.validate().is_err(), "wrongly accepted: `{brut}`");
    }
}

#[test]
fn the_bounds_of_showable_fields_hold() {
    let mut config = ollama();
    config.label = "   ".to_owned();
    assert!(
        config.validate().is_err(),
        "an empty name cannot be selected"
    );

    config = ollama();
    config.label = "a".repeat(MAX_PROVIDER_LABEL_BYTES + 1);
    assert!(config.validate().is_err());

    config = ollama();
    config.model = String::new();
    assert!(config.validate().is_err());

    config = ollama();
    // The model bound exists so that the provenance fits in its budget:
    // exceeding it would make a valid declaration unusable at the moment of
    // writing a document.
    config.model = "m".repeat(MAX_PROVIDER_MODEL_BYTES + 1);
    assert!(config.validate().is_err());

    config = ollama();
    config.base_url = format!("http://h/{}", "p".repeat(MAX_PROVIDER_BASE_URL_BYTES));
    assert!(config.validate().is_err());
}

#[test]
fn no_key_goes_through_this_configuration() {
    // I-03: the only path is a reference, and the `Debug` does not render
    // it. The test fails if someone replaces the hand-written `Debug` with
    // a `#[derive(Debug)]`.
    let config = ollama().with_secret_ref("keychain://oxyn/ollama");

    let rendered = format!("{config:?}");
    assert!(
        !rendered.contains("keychain://oxyn/ollama"),
        "reference leaked: {rendered}"
    );
    assert!(rendered.contains("Laptop Ollama"), "{rendered}");
    assert!(rendered.contains("11434"), "the host stays diagnosable");

    // And nothing in the structure can carry the key itself: the only
    // field meant for the keychain is a reference.
    let json = serde_json::to_string(&config).expect("serialization");
    assert!(json.contains("keychain://oxyn/ollama"));
    assert!(!json.contains("api_key"), "{json}");
    assert!(!json.contains("password"), "{json}");
}

/// The `Debug` protects a configuration **not yet validated**.
///
/// It is the real case: `Command::SaveAiProvider` carries the
/// configuration, and `validate` is only called at the other end, in the
/// executor. In between, a key pasted into the "endpoint" field — the most
/// common typing mistake — must not be able to reach a log
/// ([I-03](../../../CLAUDE.md#i-03)).
#[test]
fn a_url_carrying_credentials_is_not_rendered_before_validation() {
    let mut config = ollama();
    config.base_url = "https://login:p4ssw0rd@api.example.com/v1".to_owned();

    // The test's premise: this configuration is not validated, and would
    // not be. Without this line, the test would prove the easy case.
    assert!(
        config.validate().is_err(),
        "validation does refuse a URL carrying credentials"
    );

    let rendered = format!("{config:?}");
    assert!(!rendered.contains("p4ssw0rd"), "secret leaked: {rendered}");
    assert!(
        !rendered.contains("login:"),
        "identifier leaked: {rendered}"
    );
    assert!(
        rendered.contains("api.example.com"),
        "the host stays diagnosable: {rendered}"
    );
}

#[test]
fn an_empty_secret_reference_is_refused() {
    // A blank reference is a field left half filled: it designates
    // nothing in the keychain and the failure would occur when the runtime
    // opens, far from the input.
    let config = ollama().with_secret_ref("  ");
    assert!(config.validate().is_err());
}

#[test]
fn the_declaration_round_trips_faithfully() {
    let config = ollama().with_secret_ref("keychain://oxyn/ollama");
    let json = serde_json::to_string(&config).expect("serialization");
    let read_back: AiProviderConfig = serde_json::from_str(&json).expect("deserialization");
    assert_eq!(read_back, config);
}

#[test]
fn protocol_families_have_a_stable_name() {
    // This name is written to the database and into a provenance:
    // changing it would make what was already written unreadable.
    for (kind, name) in [
        (AiProviderKind::Anthropic, "anthropic"),
        (AiProviderKind::OpenAi, "openai"),
        (AiProviderKind::Gemini, "gemini"),
        (AiProviderKind::OpenAiCompatible, "openai_compatible"),
    ] {
        assert_eq!(kind.as_str(), name);
        assert_eq!(name.parse::<AiProviderKind>(), Ok(kind));
        assert_eq!(
            serde_json::to_string(&kind).expect("serialization"),
            format!("\"{name}\"")
        );
    }
    assert!("mistral".parse::<AiProviderKind>().is_err());
    assert!(serde_json::from_str::<AiProviderKind>("\"OpenAI\"").is_err());
}

#[test]
fn the_family_is_not_deduced_from_the_identifier() {
    // Three OpenAI-compatible declarations, three endpoints: it is the
    // family that says the transport, not the name.
    for id in [
        ProviderId::ollama(),
        ProviderId::openrouter(),
        ProviderId::azure_openai(),
    ] {
        let config = AiProviderConfig::new(
            id,
            AiProviderKind::OpenAiCompatible,
            "endpoint",
            "http://127.0.0.1:8080/v1",
            "model",
        );
        assert_eq!(config.kind, AiProviderKind::OpenAiCompatible);
        assert!(config.validate().is_ok());
    }
}

#[test]
fn invalid_identifiers_are_refused() {
    assert!(ProviderId::new("").is_err());
    assert!(ProviderId::new("OpenAI").is_err(), "uppercase");
    assert!(ProviderId::new("1ollama").is_err(), "leading digit");
    assert!(ProviderId::new("open ai").is_err(), "space");
    assert!(ProviderId::new("open.ai").is_err(), "dot");
    assert!(ProviderId::new("a".repeat(33)).is_err(), "too long");
    assert!(ProviderId::new("a").is_ok());
    assert!(ProviderId::new("lm-studio").is_ok());
    assert!(ProviderId::new("openai_v2").is_ok());
}

#[test]
fn the_error_does_not_copy_the_faulty_value() {
    // A malformed provider identifier may be a key pasted into the wrong
    // field (I-03).
    let err = ProviderId::new("sk-proj-THISMUSTNOTLEAK").expect_err("invalid");
    let rendered = err.to_string();
    assert!(!rendered.contains("THISMUSTNOTLEAK"), "{rendered}");
}

#[test]
fn the_constants_are_valid_identifiers() {
    for name in [
        ProviderId::OLLAMA,
        ProviderId::LM_STUDIO,
        ProviderId::LLAMA_CPP,
        ProviderId::OPENAI,
        ProviderId::AZURE_OPENAI,
        ProviderId::OPENROUTER,
        ProviderId::ANTHROPIC,
        ProviderId::GEMINI,
        ProviderId::OPENAI_COMPATIBLE,
    ] {
        assert!(ProviderId::new(name).is_ok(), "{name}");
    }
}

#[test]
fn an_identifier_serializes_as_a_bare_string() {
    let json = serde_json::to_string(&ProviderId::openai()).expect("serialization");
    assert_eq!(json, "\"openai\"");
    let read_back: ProviderId = serde_json::from_str(&json).expect("deserialization");
    assert_eq!(read_back, ProviderId::openai());
    assert!(serde_json::from_str::<ProviderId>("\"OPENAI\"").is_err());
}

#[test]
fn a_new_declaration_gets_a_valid_and_distinct_identity() {
    // The longest family is the one that would overflow if the shape
    // changed: it is the one tested, not the shortest.
    for family in [
        AiProviderKind::OpenAiCompatible,
        AiProviderKind::Anthropic,
        AiProviderKind::OpenAi,
        AiProviderKind::Gemini,
    ] {
        let minted = ProviderId::for_new_declaration(family);
        ProviderId::new(minted.as_str())
            .unwrap_or_else(|error| panic!("identity refused by its own validation: {error}"));
        assert!(
            minted.as_str().starts_with(family.as_str()),
            "the family stays readable in the keychain: {minted:?}"
        );
    }

    // Two declarations of the same family and label remain two
    // declarations. The trap this test closes: an identifier derived from
    // the label would turn the second into a silent replacement of the
    // first, keychain key included.
    let first = ProviderId::for_new_declaration(AiProviderKind::Anthropic);
    let second = ProviderId::for_new_declaration(AiProviderKind::Anthropic);
    assert_ne!(first, second);
}

fn config_with(kind: AiProviderKind, base_url: &str) -> AiProviderConfig {
    let mut config = ollama();
    config.kind = kind;
    config.base_url = base_url.to_owned();
    config
}

#[test]
fn same_endpoint_as_is_true_for_a_host_written_in_a_different_case() {
    let mine = config_with(
        AiProviderKind::OpenAiCompatible,
        "HTTPS://API.Example.com/v1",
    );
    let theirs = config_with(
        AiProviderKind::OpenAiCompatible,
        "https://api.example.com/v1",
    );
    assert!(mine.same_endpoint_as(&theirs));
}

#[test]
fn same_endpoint_as_is_true_for_an_explicit_default_port() {
    let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com:443/v1");
    let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
    assert!(mine.same_endpoint_as(&theirs));
}

#[test]
fn same_endpoint_as_is_true_for_a_trailing_slash() {
    let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1/");
    let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
    assert!(mine.same_endpoint_as(&theirs));
}

#[test]
fn same_endpoint_as_is_false_for_a_different_host() {
    let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
    let theirs = config_with(AiProviderKind::OpenAi, "https://api.evil.example/v1");
    assert!(!mine.same_endpoint_as(&theirs));
}

#[test]
fn same_endpoint_as_is_false_for_a_different_port() {
    let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com:8443/v1");
    let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
    assert!(!mine.same_endpoint_as(&theirs));
}

#[test]
fn same_endpoint_as_is_false_for_http_against_https() {
    let mine = config_with(AiProviderKind::OpenAi, "http://api.example.com/v1");
    let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
    assert!(!mine.same_endpoint_as(&theirs));
}

#[test]
fn same_endpoint_as_is_false_for_a_different_path() {
    let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
    let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v2");
    assert!(!mine.same_endpoint_as(&theirs));
}

#[test]
fn same_endpoint_as_is_false_for_a_different_query() {
    // `validate` refuses a query now, so no stored declaration carries one:
    // this covers the values it has not seen — an unvalidated draft, a row
    // written by another binary — where a different query must not inherit
    // a stored key.
    let mine = config_with(
        AiProviderKind::OpenAi,
        "https://api.example.com/v1?region=eu",
    );
    let theirs = config_with(
        AiProviderKind::OpenAi,
        "https://api.example.com/v1?region=us",
    );
    assert!(!mine.same_endpoint_as(&theirs));
}

#[test]
fn same_endpoint_as_is_false_for_a_different_kind_with_the_same_url() {
    let mine = config_with(AiProviderKind::Anthropic, "https://api.example.com/v1");
    let theirs = config_with(
        AiProviderKind::OpenAiCompatible,
        "https://api.example.com/v1",
    );
    assert!(!mine.same_endpoint_as(&theirs));
}

#[test]
fn same_endpoint_as_is_false_for_an_unreadable_url_on_either_side() {
    let readable = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
    let unreadable = config_with(AiProviderKind::OpenAi, "not a url");
    assert!(!readable.same_endpoint_as(&unreadable));
    assert!(!unreadable.same_endpoint_as(&readable));
    assert!(!unreadable.same_endpoint_as(&unreadable));
}
