use super::{REDACTED_VALUE, redact_secrets};

const SECRET: &str = "witness-secret";

/// `witness-secret`, base64: what a SASL payload of the console would carry.
const ENCODED: &str = "d2l0bmVzcy1zZWNyZXQ=";

fn assert_redacted(text: &str) {
    let redacted = redact_secrets(text);
    assert!(
        !redacted.contains(SECRET) && !redacted.contains(ENCODED),
        "{text} → {redacted}"
    );
    assert!(redacted.contains("<redacted>"), "{text} → {redacted}");
    assert_eq!(redact_secrets(&redacted), redacted, "idempotent: {text}");
}

/// A connection string with a password, its scheme passed apart so that no
/// line of this file reads as one.
fn uri(scheme: &str, password: &str, hosts: &str) -> String {
    format!("{scheme}://admin:{password}@{hosts}")
}

#[test]
fn the_password_of_create_and_update_user_is_redacted_and_nothing_else() {
    for (text, expected) in [
        (
            r#"{"createUser": "app", "pwd": "witness-secret", "roles": ["read"]}"#,
            r#"{"createUser": "app", "pwd": "<redacted>", "roles": ["read"]}"#,
        ),
        (
            "{\n  \"updateUser\": \"app\",\n  \"pwd\" :\t\"witness-secret\"\n}",
            "{\n  \"updateUser\": \"app\",\n  \"pwd\" :\t\"<redacted>\"\n}",
        ),
    ] {
        assert_eq!(redact_secrets(text), expected);
    }
}

#[test]
fn a_password_is_found_at_any_depth_spelled_any_way_and_of_any_type() {
    for text in [
        r#"{"bulkWrite": 1, "x": [{"y": {"pwd": "witness-secret"}}]}"#,
        r#"{"createUser":"u","pwd":"witness-secret"}"#,
        r#"{"createUser": "app", "pwd": "witness-secret"}"#,
        r#"{"createUser": "app", "pwd": "witness-\"secret\\"}"#,
        r#"{"createUser": "app", "pwd": "witness-secret"}"#,
        r#"{"createUser": "app", "pwd": {"$secret": ["witness-secret", {"a": "}"}]}}"#,
        r#"{"createUser": "app", "pwd": 0witness-secret}"#,
        r#"{"createUser": "a"}{"updateUser": "b", "pwd": "witness-secret"}"#,
    ] {
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("witness"), "{text} → {redacted}");
        assert!(redacted.contains(REDACTED_VALUE), "{redacted}");
    }
}

#[test]
fn the_payload_of_a_sasl_conversation_is_redacted_in_every_form() {
    for text in [
        r#"{"saslStart": 1, "mechanism": "PLAIN", "payload": {"$binary": {"base64": "d2l0bmVzcy1zZWNyZXQ=", "subType": "00"}}}"#,
        r#"{"saslStart": 1, "mechanism": "PLAIN", "payload": {"$binary": "d2l0bmVzcy1zZWNyZXQ=", "$type": "00"}}"#,
        r#"{"saslContinue": 1, "conversationId": 1, "payload": "d2l0bmVzcy1zZWNyZXQ="}"#,
        // The command name and the key, escaped.
        r#"{"saslStart": 1, "mechanism": "SCRAM-SHA-256", "payload": "d2l0bmVzcy1zZWNyZXQ="}"#,
        // Shell syntax, which the console refuses but a history keeps.
        r#"db.runCommand({saslStart: 1, mechanism: "PLAIN", payload: BinData(0, "d2l0bmVzcy1zZWNyZXQ=")})"#,
    ] {
        assert_redacted(text);
    }
}

#[test]
fn a_payload_outside_a_sasl_command_is_data() {
    for text in [
        r#"{"insert": "events", "documents": [{"payload": "kept"}]}"#,
        r#"{"find": "events", "filter": {"payload": {"$exists": true}}}"#,
        r#"{"aggregate": "c", "payload": "kept", "cursor": {}}"#,
    ] {
        assert_eq!(redact_secrets(text), text);
    }
}

#[test]
fn the_password_of_a_connection_string_in_a_command_is_redacted() {
    for (scheme, password, hosts) in [
        ("mongodb", SECRET, "h1:27017,h2/?replicaSet=rs"),
        ("mongodb+srv", SECRET, "cluster.example"),
        // A raw `@` in the password stays inside the redaction.
        ("mongodb", "witness@secret", "h1/db"),
    ] {
        let text = format!(r#"{{"addShard": "{}"}}"#, uri(scheme, password, hosts));
        let expected = format!(r#"{{"addShard": "{}"}}"#, uri(scheme, "<redacted>", hosts));
        assert_eq!(redact_secrets(&text), expected);
    }
    // Escaped, it is the same string to the server.
    assert_eq!(
        redact_secrets(r#"{"addShard": "mongodb:\/\/admin:witness-secret@h1"}"#),
        format!(
            r#"{{"addShard": "{}"}}"#,
            uri("mongodb", "<redacted>", "h1")
        )
    );
    for kept in [
        r#"{"addShard": "mongodb://h1:27017/db"}"#,
        r#"{"addShard": "mongodb://reader@h1/?authMechanism=MONGODB-X509"}"#,
        r#"{"find": "links", "filter": {"url": "https://example.com/a:b"}}"#,
    ] {
        assert_eq!(redact_secrets(kept), kept);
    }
}

#[test]
fn a_command_quoted_inside_a_string_is_read_again() {
    let command = r#"{"createUser": "app", "pwd": "witness-secret"}"#;
    let once = serde_json::to_string(command).expect("a string encodes");
    let twice = serde_json::to_string(&once).expect("a string encodes");
    for text in [
        format!("Executed: {once}"),
        format!(r#"{{"summary": {twice}}}"#),
        format!(r#"[{once}]"#),
        format!(r#"{{"insert": "notes", "documents": [{{"text": {once}}}]}}"#),
    ] {
        assert_redacted(&text);
    }
}

#[test]
fn the_shell_syntax_a_history_may_keep_is_read_too() {
    for text in [
        r#"db.createUser({user: "app", pwd: "witness-secret", roles: []})"#,
        "db.createUser({user: 'app', pwd: 'witness-secret two', roles: []})",
        r#"db.getSiblingDB("admin").updateUser("app", {pwd: "witness-secret"})"#,
        r#"{"createUser": "app", "pwd": BinData(0, "witness-secret")}"#,
    ] {
        assert_redacted(text);
    }
}

#[test]
fn an_unterminated_password_is_redacted_through_the_end() {
    for text in [
        r#"{"createUser": "app", "pwd": "witness-secret"#,
        r#"{"createUser": "app", "pwd": {"a": "witness-secret"#,
        r#"{"createUser": "app", "pwd": "witness-secret\"#,
        r#"{"saslStart": 1, "payload": {"$binary": "witness-secret"#,
    ] {
        assert!(!redact_secrets(text).contains(SECRET), "{text}");
    }
}

#[test]
fn text_that_only_mentions_the_key_is_kept() {
    for text in [
        r#"{"find": "users", "filter": {"note": "pwd: hunter"}}"#,
        r#"{"find": "users", "filter": {"x": "\"pwd\": 1"}}"#,
        r#"{"find": "pwd", "projection": {"password": 0}}"#,
        r#"{"insert": "c", "documents": [{"pwds": "kept"}]}"#,
        r#"{"find": "users", "filter": {"name": "a\\b"}}"#,
        "SELECT pwd FROM users",
        "",
        "\"",
        "'",
        "pwd",
        "{pwd",
    ] {
        assert_eq!(redact_secrets(text), text);
    }
}
