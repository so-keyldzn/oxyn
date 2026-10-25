//! The credentials a search DSL request carries, redacted before its text is
//! persisted ([ADR-0053](../../../../docs/adr/0053-redact-sql-passwords-before-persistence.md)).
//!
//! # Decided on the decoded body
//!
//! The body is read by the lexical JSON reading the MongoDB redaction holds
//! ([`crate::mongo::redact_credentials`]): keys and strings decoded as JSON
//! decodes them, a level of quoting read through, every other byte kept. What
//! differs is the keys Elasticsearch and OpenSearch take a credential under
//! (their API references, consulted on 2026-10-24, see RESEARCH-NOTES), at any
//! depth:
//!
//! - `password` and `password_hash` (the native user APIs, a token grant, a
//!   reindex from a remote cluster), `current_password` (OpenSearch's
//!   account API);
//! - `access_token`, `refresh_token`, `kerberos_ticket` and
//!   `service_account_token` (the token and API key grant APIs);
//! - `api_key` when it is a string (a reindex from a remote cluster, an
//!   inference endpoint): under `_security/api_key/grant`, the object of the
//!   same name is the key's definition, kept;
//! - `client_secret` and `secret_key` (inference service settings) and
//!   `secure_settings_password` (the keystore of
//!   `_nodes/reload_secure_settings`);
//! - `Authorization`, in any case: a header a reindex from remote sends;
//! - `hash` when it holds a bcrypt hash (`$2…`): an OpenSearch internal user.
//!   A document's own `hash` field — a digest, a commit — is kept.
//!
//! And the password of a connection string inside a string.
//!
//! # A request pasted from a shell
//!
//! The console's format holds no header, but what was typed is kept as typed,
//! a refused `curl` line included. Two shell forms are read on the text
//! itself: the credentials of an `Authorization:` header, and the password of
//! curl's `-u`/`--user user:password`. A SQL text that writes ` -u a:b` loses
//! `b` in the kept copy of [`crate::redact_any_language`], never in what runs.

use std::borrow::Cow;
use std::ops::Range;

use crate::mongo::{Credentials, redact_credentials, redact_uri_passwords};

/// The keys whose string value is a credential, wherever they are.
const CREDENTIAL_KEYS: [&str; 11] = [
    "password",
    "password_hash",
    "current_password",
    "access_token",
    "refresh_token",
    "kerberos_ticket",
    "service_account_token",
    "api_key",
    "client_secret",
    "secret_key",
    "secure_settings_password",
];

/// The header whose value is a credential, matched in any case.
const HEADER: &str = "authorization";

/// The key of an OpenSearch internal user's password hash.
const HASH: &str = "hash";

/// What replaces a credential read on the text itself.
const REDACTED: &str = "<redacted>";

const SEARCH_DSL: Credentials = Credentials {
    // Each key of `CREDENTIAL_KEYS` holds one of these, as do the header and
    // `hash`.
    markers: &[
        "password",
        "_token",
        "ticket",
        "api_key",
        "secret",
        "uthorization",
        HASH,
    ],
    key: |key, _, _| CREDENTIAL_KEYS.contains(&key) || key.eq_ignore_ascii_case(HEADER),
    string: |key, decoded| key == HASH && decoded.starts_with("$2"),
    strings_only: true,
};

/// Replaces the credentials of the search DSL requests in `text`, preserving
/// every other byte: the values of the keys this module lists with
/// `"<redacted>"`, the password of a connection string, an `Authorization:`
/// header's credentials and a curl `user:password`'s password with
/// `<redacted>`.
#[must_use]
pub(crate) fn redact_secrets(text: &str) -> String {
    let body = redact_credentials(text, &SEARCH_DSL);
    // A full URL is refused by the console, and kept as typed.
    let urls = redact_uri_passwords(&body);
    redact_shell_credentials(&urls).into_owned()
}

fn redact_shell_credentials(text: &str) -> Cow<'_, str> {
    let mut spans = header_credentials(text);
    spans.extend(curl_passwords(text));
    if spans.is_empty() {
        return Cow::Borrowed(text);
    }
    spans.sort_unstable_by_key(|span| span.start);
    let mut redacted = String::with_capacity(text.len());
    let mut copied = 0;
    for span in spans {
        if span.start < copied {
            continue;
        }
        redacted.push_str(text.get(copied..span.start).unwrap_or_default());
        redacted.push_str(REDACTED);
        copied = span.end;
    }
    redacted.push_str(text.get(copied..).unwrap_or_default());
    Cow::Owned(redacted)
}

/// Where a shell word ends: a blank, a quote, or the backslash of an escaped
/// quote inside a JSON string.
fn ends_word(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'"' | b'\'' | b'\\')
}

fn word_end(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(|&byte| !ends_word(byte)) {
        index = index.saturating_add(1);
    }
    index
}

fn skip_blanks(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|&byte| matches!(byte, b' ' | b'\t'))
    {
        index = index.saturating_add(1);
    }
    index
}

/// The credentials of each `Authorization: <scheme> <credentials>` — or of
/// `Authorization: <credentials>` when no scheme is written.
fn header_credentials(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    // ASCII case folding keeps every byte offset.
    let folded = text.to_ascii_lowercase();
    let needle = "authorization:";
    let mut spans = Vec::new();
    let mut search = 0;
    while let Some(found) = folded.get(search..).and_then(|rest| rest.find(needle)) {
        let after = search.saturating_add(found).saturating_add(needle.len());
        search = after;
        let first = skip_blanks(bytes, after);
        let first_end = word_end(bytes, first);
        if first_end == first {
            continue;
        }
        let second = skip_blanks(bytes, first_end);
        let second_end = word_end(bytes, second);
        let span = if second_end > second && second > first_end {
            second..second_end
        } else {
            first..first_end
        };
        if text.get(span.clone()) != Some(REDACTED) {
            spans.push(span);
        }
    }
    spans
}

/// The password of each curl `-u user:password`, `--user user:password` or
/// `--user=user:password`, quoted or not.
fn curl_passwords(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut words = words(bytes);
    while let Some(word) = words.next() {
        let flag = text.get(word.clone()).unwrap_or_default();
        let value = if flag == "-u" || flag == "--user" {
            words.next()
        } else if flag.starts_with("--user=") {
            Some(word.start.saturating_add("--user=".len())..word.end)
        } else {
            None
        };
        let Some(value) = value else { continue };
        let quoted = matches!(bytes.get(value.start), Some(b'"' | b'\''));
        let start = value.start.saturating_add(usize::from(quoted));
        let Some(colon) = text.get(start..value.end).and_then(|user| user.find(':')) else {
            continue;
        };
        let password = start.saturating_add(colon).saturating_add(1);
        let end = if quoted {
            value.end.saturating_sub(1).max(password)
        } else {
            value.end
        };
        if end > password && text.get(password..end) != Some(REDACTED) {
            spans.push(password..end);
        }
    }
    spans
}

/// The blank-separated words of `bytes`, as byte ranges.
fn words(bytes: &[u8]) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut index = 0;
    std::iter::from_fn(move || {
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index = index.saturating_add(1);
        }
        let start = index;
        while bytes
            .get(index)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            index = index.saturating_add(1);
        }
        (index > start).then_some(start..index)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "witness-secret";

    #[test]
    fn every_credential_key_of_a_body_is_redacted() {
        for key in CREDENTIAL_KEYS {
            let text = format!("POST /_security/user/app\n{{\"{key}\": \"{SECRET}\"}}");
            let redacted = redact_secrets(&text);
            assert!(!redacted.contains(SECRET), "{key}: {redacted}");
            assert!(
                redacted.starts_with("POST /_security/user/app\n"),
                "{redacted}"
            );
        }
    }

    #[test]
    fn a_change_of_password_keeps_everything_but_the_passwords() {
        let text = "PUT /_plugins/_security/api/account\n\
                    {\"current_password\": \"witness-secret\", \"password\": \"witness-secret\"}";
        assert_eq!(
            redact_secrets(text),
            "PUT /_plugins/_security/api/account\n\
             {\"current_password\": \"<redacted>\", \"password\": \"<redacted>\"}"
        );
    }

    #[test]
    fn a_reindex_from_remote_loses_its_credentials_not_its_host() {
        let text = r#"POST /_reindex
{"source": {"remote": {"host": "https://old:9200", "username": "u",
  "password": "witness-secret", "headers": {"authorization": "ApiKey witness-secret"},
  "api_key": "witness-secret"}, "index": "logs"}, "dest": {"index": "logs"}}"#;
        let redacted = redact_secrets(text);
        assert!(!redacted.contains(SECRET), "{redacted}");
        assert!(
            redacted.contains(r#""host": "https://old:9200""#),
            "{redacted}"
        );
        assert!(redacted.contains(r#""username": "u""#), "{redacted}");
    }

    #[test]
    fn a_key_definition_and_a_document_hash_are_kept() {
        let grant = r#"POST /_security/api_key/grant
{"grant_type": "password", "username": "u", "password": "witness-secret",
 "api_key": {"name": "reporting", "expiration": "1d"}}"#;
        let redacted = redact_secrets(grant);
        assert!(!redacted.contains(SECRET), "{redacted}");
        assert!(redacted.contains(r#""name": "reporting""#), "{redacted}");

        let document = r#"PUT /commits/_doc/1
{"hash": "9fceb02d0ae598e95dc970b74767f19372d61af8"}"#;
        assert_eq!(redact_secrets(document), document);
        let user = r#"PUT /_plugins/_security/api/internalusers/app
{"hash": "$2y$12$witness-secret", "backend_roles": []}"#;
        let redacted = redact_secrets(user);
        assert!(!redacted.contains(SECRET), "{redacted}");
        assert!(redacted.contains(r#""backend_roles": []"#), "{redacted}");
    }

    #[test]
    fn an_escaped_key_is_read_as_the_server_reads_it() {
        let text = r#"POST /_security/oauth2/token
{"grant_type": "refresh_token", "refresh_token": "witness-secret"}"#;
        assert!(!redact_secrets(text).contains(SECRET));
    }

    #[test]
    fn a_pasted_curl_line_loses_its_header_and_user_password() {
        for text in [
            "curl -H 'Authorization: ApiKey witness-secret' https://h:9200/_search",
            "curl -H \"authorization: Bearer witness-secret\" https://h:9200",
            "curl -H 'Authorization: witness-secret' https://h:9200",
            "curl -u elastic:witness-secret https://h:9200",
            "curl --user 'elastic:witness-secret' https://h:9200",
            "curl --user=elastic:witness-secret https://h:9200",
            "curl https://elastic:witness-secret@h:9200/_search",
        ] {
            let redacted = redact_secrets(text);
            assert!(!redacted.contains(SECRET), "{text} -> {redacted}");
            assert!(redacted.contains("h:9200"), "{redacted}");
        }
        assert_eq!(
            redact_secrets("curl -u elastic:witness-secret https://h:9200"),
            "curl -u elastic:<redacted> https://h:9200"
        );
    }

    #[test]
    fn a_query_on_a_field_named_password_loses_its_value_in_the_copy() {
        // The reading is lexical: a field named `password` in a query is a
        // key too. The kept copy loses that value, never the request that runs.
        let text = "POST /logs/_search\n{\"query\": {\"match\": {\"password\": \"reset\"}}}";
        assert!(!redact_secrets(text).contains("\"reset\""));
    }

    #[test]
    fn a_request_without_credentials_is_kept_byte_for_byte() {
        let text = "GET /_cluster/health\n\nPOST /logs/_search\n\
                    {\"size\": 10, \"query\": {\"match\": {\"msg\": \"-u is a flag\"}}}";
        assert_eq!(redact_secrets(text), text);
    }
}
