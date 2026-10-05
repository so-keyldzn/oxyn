//! Listing a provider's models before it is saved: what the form sends, and
//! what it gets back.
//!
//! A failure of the provider is **data** ([`ModelListing::Failed`]), never an
//! [`IpcError`](crate::ipc::IpcError): the form shows it next to the field it
//! concerns, and an `Err` is kept for a request the front should not have
//! sent.

use oxyn_core::AiProviderKind;
use serde::{Deserialize, Serialize};

use super::ModelChoice;

/// The endpoint a form describes, to list its models.
///
/// **No `Debug`, derived or written**: `key` is an API key in clear, and
/// `base_url` has not been validated yet — it may carry the `user:password` the
/// domain refuses ([I-03](../../../../../CLAUDE.md#i-03)). Nothing logs it.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProbe {
    /// The declaration being edited, `None` for a new one.
    #[serde(default)]
    pub id: Option<String>,
    pub kind: AiProviderKind,
    /// As typed. Empty with an `id` means the stored endpoint.
    #[serde(default, deserialize_with = "opaque_url")]
    pub base_url: String,
    /// The key typed in the form. Absent on an edit, and the endpoint
    /// unchanged, the stored key is used — never across a changed endpoint.
    #[serde(default, deserialize_with = "opaque_key")]
    pub key: Option<String>,
}

/// Reads the key without ever quoting it: serde's own type error copies a
/// scalar it rejects (`invalid type: integer `…``), and Tauri hands that
/// text back to the webview ([I-03](../../../../../CLAUDE.md#i-03)).
fn opaque_key<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    match Option::<serde_json::Value>::deserialize(deserializer)? {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(key)) => Ok(Some(key)),
        Some(_) => Err(serde::de::Error::custom("`key` must be a string or null")),
    }
}

/// Same for the address, which may carry a password before it is checked.
fn opaque_url<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    match Option::<serde_json::Value>::deserialize(deserializer)? {
        None | Some(serde_json::Value::Null) => Ok(String::new()),
        Some(serde_json::Value::String(url)) => Ok(url),
        Some(_) => Err(serde::de::Error::custom("`baseUrl` must be a string")),
    }
}

/// What listing the models gave.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ModelListing {
    Ok {
        models: Vec<ModelChoice>,
        /// Served from the backend's cache rather than asked again.
        cached: bool,
        /// When the provider answered, in milliseconds since the epoch.
        fetched_at_ms: u64,
    },
    Failed {
        reason: ListingFailure,
        /// Short, in English, for the user. Never the key, never the URL's
        /// credentials or query.
        message: String,
    },
}

/// Why no list came back — what the form can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ListingFailure {
    /// `401`: the key was refused.
    Unauthorized,
    /// `403`: the key is valid but may not list models.
    Forbidden,
    /// The endpoint has no model list (`404`, `405`, another refusal).
    Unsupported,
    /// `429`.
    RateLimited,
    /// No answer in time.
    Timeout,
    /// No server answered: refused connection, unknown host, offline.
    Unreachable,
    /// An answer that is not a model list.
    Malformed,
    /// The address cannot be an endpoint.
    InvalidEndpoint,
    /// A key is needed and none was typed or stored.
    MissingKey,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_speaks_the_contract() {
        let ok = serde_json::to_value(ModelListing::Ok {
            models: Vec::new(),
            cached: true,
            fetched_at_ms: 7,
        })
        .expect("serialized");
        assert_eq!(
            ok,
            serde_json::json!({"status": "ok", "models": [], "cached": true, "fetchedAtMs": 7})
        );
        let failed = serde_json::to_value(ModelListing::Failed {
            reason: ListingFailure::RateLimited,
            message: "m".to_owned(),
        })
        .expect("serialized");
        assert_eq!(
            failed,
            serde_json::json!({"status": "failed", "reason": "rateLimited", "message": "m"})
        );
        for (reason, name) in [
            (ListingFailure::InvalidEndpoint, "invalidEndpoint"),
            (ListingFailure::MissingKey, "missingKey"),
        ] {
            assert_eq!(
                serde_json::to_value(reason).expect("serialized"),
                serde_json::json!(name)
            );
        }
    }

    #[test]
    fn a_malformed_probe_never_quotes_its_key_or_address() {
        // What Tauri returns to the webview for unreadable arguments is this
        // error's text.
        for value in [
            serde_json::json!(4_242_424_242_u64),
            serde_json::json!(-4_242_424_242_i64),
            serde_json::json!(4242.4242),
            serde_json::json!(true),
            serde_json::json!(["4242424242"]),
            serde_json::json!({"secret": "4242424242"}),
        ] {
            for field in ["key", "baseUrl"] {
                let mut input = serde_json::json!({
                    "id": null,
                    "kind": "openai",
                    "baseUrl": "https://api.example/v1",
                    "key": null,
                });
                input[field] = value.clone();
                let error = match serde_json::from_value::<ModelProbe>(input) {
                    Ok(_) => panic!("`{field}` = {value} must be refused"),
                    Err(error) => error.to_string(),
                };
                assert!(!error.contains("4242"), "{field}: {error}");
                assert!(!error.contains("true"), "{field}: {error}");
            }
        }
    }

    #[test]
    fn a_probe_reads_the_contract() {
        let probe: ModelProbe = serde_json::from_value(serde_json::json!({
            "id": null,
            "kind": "openai_compatible",
            "baseUrl": "http://localhost:11434/v1",
            "key": null,
        }))
        .expect("valid probe");
        assert!(probe.id.is_none() && probe.key.is_none());
        assert_eq!(probe.kind, AiProviderKind::OpenAiCompatible);
    }
}
