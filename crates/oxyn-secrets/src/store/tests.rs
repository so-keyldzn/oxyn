use super::*;
use crate::memory_store::MemorySecretStore;
use oxyn_core::DriverId;

#[test]
fn a_connection_reference_is_stable() {
    let id = ConnectionId::new();
    assert_eq!(SecretRef::for_connection(id), SecretRef::for_connection(id));
    assert_ne!(
        SecretRef::for_connection(id),
        SecretRef::for_connection(ConnectionId::new())
    );

    let reference = SecretRef::for_connection(id);
    assert!(reference.as_str().starts_with("oxyn:conn:"));
    assert_eq!(reference.kind(), SecretRef::KIND_CONNECTION);
    assert_eq!(reference.name(), id.to_string());
    assert!(reference.is_connection());
}

#[test]
fn a_connection_without_written_reference_derives_its_own() {
    let cfg = ConnectionConfig::new("customer database", DriverId::postgres());
    let reference = SecretRef::for_connection_config(&cfg).expect("derivation");
    assert_eq!(reference, SecretRef::for_connection(cfg.id));
}

#[test]
fn a_written_reference_is_authoritative() {
    let cfg = ConnectionConfig::new("customer database", DriverId::postgres())
        .with_secret_ref("oxyn:conn:inherited-from-v0");
    let reference = SecretRef::for_connection_config(&cfg).expect("parsing");
    assert_eq!(reference.name(), "inherited-from-v0");
    assert_ne!(reference, SecretRef::for_connection(cfg.id));
}

#[test]
fn a_malformed_written_reference_is_a_refusal_not_a_fallback() {
    // Falling back to the derived reference would read *another* secret than
    // the one the file designates. Reading nothing is less serious.
    let cfg = ConnectionConfig::new("customer database", DriverId::postgres())
        .with_secret_ref("oxyn/connection/prod-eu");
    assert!(SecretRef::for_connection_config(&cfg).is_err());
}

#[test]
fn the_canonical_format_round_trips() {
    for text in [
        "oxyn:conn:018f0000-0000-7000-8000-000000000000",
        "oxyn:llm:anthropic",
        "oxyn:llm:azure.openai",
        "oxyn:tunnel_ssh:bastion-eu",
    ] {
        let reference = SecretRef::parse(text).expect(text);
        assert_eq!(reference.as_str(), text);
        assert_eq!(reference.to_string(), text);
        let read_back: SecretRef = text.parse().expect("FromStr");
        assert_eq!(reference, read_back);
    }
}

#[test]
fn a_reference_from_a_file_cannot_designate_what_it_wants() {
    // Each of these values is plausible in a workspace file written by a third
    // party. All must be refused before reaching the system keychain.
    for text in [
        "",
        "oxyn",
        "oxyn:conn",
        "oxyn:conn:",
        "oxyn::name",
        "other:conn:name",
        "oxyn:conn:name:extra",
        "oxyn:conn:name with space",
        "oxyn:conn:name\nligne2",
        "oxyn:conn:name\u{0}",
        "oxyn:conn:../../other",
        "oxyn:conn:..",
        "oxyn:conn:.",
        "oxyn:conn:-name",
        "oxyn:CONN:name",
        "oxyn:1conn:name",
        "oxyn:conn:name/path",
        "oxyn:conn:name;rm -rf",
    ] {
        assert!(
            SecretRef::parse(text).is_err(),
            "{text:?} should have been refused"
        );
    }
    assert!(SecretRef::parse(&format!("oxyn:conn:{}", "a".repeat(129))).is_err());
}

#[test]
fn a_refusal_does_not_copy_the_offending_value() {
    let err = SecretRef::parse("oxyn:conn:central-bank database").expect_err("space forbidden");
    let message = err.to_string();
    assert!(!message.contains("bank"), "value leaked: {message}");
}

#[test]
fn the_provider_name_is_validated_at_construction() {
    assert!(SecretRef::for_provider("anthropic").is_ok());
    assert!(SecretRef::for_provider("").is_err());
    assert!(SecretRef::for_provider("open ai").is_err());
    assert!(SecretRef::for_provider("openai:prod").is_err());
}

#[test]
fn the_reference_crosses_json_with_its_validation() {
    let reference = SecretRef::for_provider("ollama").expect("valid");
    let json = serde_json::to_string(&reference).expect("serialization");
    assert_eq!(json, "\"oxyn:llm:ollama\"");
    let read_back: SecretRef = serde_json::from_str(&json).expect("deserialization");
    assert_eq!(read_back, reference);
    assert!(
        serde_json::from_str::<SecretRef>("\"oxyn:llm:oll ama\"").is_err(),
        "validation must apply to deserialization too"
    );
}

#[test]
fn the_debug_of_a_reference_is_complete() {
    // A reference is public: it is written in clear text in workspace files.
    // Masking it would bring nothing and would blur the meaning of the masks
    // that matter.
    let reference = SecretRef::for_provider("anthropic").expect("valid");
    assert!(format!("{reference:?}").contains("oxyn:llm:anthropic"));
}

#[test]
fn bundle_methods_go_through_the_json_envelope() {
    let store = MemorySecretStore::new();
    let reference = SecretRef::for_connection(ConnectionId::new());

    assert!(store.get_bundle(&reference).expect("read").is_none());

    let bundle = CredentialBundle::new()
        .with_password("hunter2")
        .with_token("sk-witness");
    store.put_bundle(&reference, &bundle).expect("write");

    let read_back = store
        .get_bundle(&reference)
        .expect("read")
        .expect("the bundle was just written");
    assert_eq!(read_back.password(), Some("hunter2"));
    assert_eq!(read_back.token(), Some("sk-witness"));
}

#[test]
fn a_bare_password_is_not_read_back_as_a_bundle() {
    // Real case: an entry written by an earlier version, or by another tool,
    // under the same reference.
    let store = MemorySecretStore::new();
    let reference = SecretRef::for_connection(ConnectionId::new());
    store
        .put(&reference, SecretString::from("hunter2"))
        .expect("write");

    let err = store.get_bundle(&reference).expect_err("this is not JSON");
    assert!(matches!(err, SecretError::Malformed { .. }));
    assert!(
        !err.to_string().contains("hunter2"),
        "the content leaked into the error: {err}"
    );
}
