//! Listing a provider's models from the form, before anything is saved.
//!
//! The form describes an endpoint — kind, address, perhaps a key — and asks
//! what it serves. Nothing here persists: the typed key never reaches the
//! keyring, and a declaration's stored key is only read for the endpoint it was
//! typed for, by the same rule as a save
//! ([`AiProviderConfig::same_endpoint_as`]).
//!
//! The request goes through [`oxyn_llm::list_models`], which builds the
//! transport a conversation would use: a list never comes back from an address
//! the conversation would not reach. It carries the key to that endpoint and
//! nothing else — no prompt, no database content, no registry of Oxyn's
//! ([AI-PROVIDERS](../../../../../docs/AI-PROVIDERS.md#listing-a-providers-models)).
//!
//! A failure is **data**: the provider refusing the key is an answer the form
//! shows, not an error of the call.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxyn_core::{AiProviderConfig, AiProviderKind, ProviderId};
use oxyn_llm::{ApiKey, LlmError, Reach};
use parking_lot::Mutex;
use url::Url;

use super::{MAX_KEY_BYTES, MODELS_TIMEOUT, model_choice, now_ms, parse_provider_id};
use crate::backend::Backend;
use crate::ipc::IpcError;
use crate::ipc::ai::{ListingFailure, ModelChoice, ModelListing, ModelProbe};

/// How long a listed endpoint is answered from memory.
///
/// Product choice: a provider's catalog changes over days, and a form reopened
/// within minutes should not ask again. « Refresh models » and « Test
/// connection » bypass it.
const LIST_TTL: Duration = Duration::from_secs(10 * 60);

/// Endpoints remembered at most; the oldest goes first. A user declares a
/// handful of providers — the bound only keeps a script in the webview from
/// growing the map.
const MAX_CACHED_LISTS: usize = 32;

/// How long a loopback endpoint may take: a local server answers its list in
/// milliseconds, and one that does not within this time is not running
/// properly. Oxyn's choice.
const LOCAL_LIST_TIMEOUT: Duration = Duration::from_secs(5);

/// What a probe sends to a placeholder field the domain requires but listing
/// does not use.
const PROBE_PLACEHOLDER: &str = "probe";

impl Backend {
    /// Lists the models of the endpoint a form describes.
    ///
    /// Two blocking steps, both on the blocking pool: the keychain read and the
    /// DNS resolution that classifies the endpoint
    /// ([I-05](../../../../../CLAUDE.md#i-05)).
    ///
    /// # Errors
    /// Only a request the front should not have sent: an unparsable `id`, a
    /// key longer than any API key. Everything the provider does is in the
    /// [`ModelListing`].
    pub async fn list_draft_models(
        &self,
        probe: ModelProbe,
        refresh: bool,
    ) -> Result<ModelListing, IpcError> {
        self.list_draft_models_within(probe, refresh, None).await
    }

    /// [`list_draft_models`](Self::list_draft_models), under a chosen timeout:
    /// the tests exercise it without waiting twenty seconds.
    pub(super) async fn list_draft_models_within(
        &self,
        probe: ModelProbe,
        refresh: bool,
        timeout: Option<Duration>,
    ) -> Result<ModelListing, IpcError> {
        let id = probe.id.as_deref().map(parse_provider_id).transpose()?;
        let typed_key = probe
            .key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        if typed_key
            .as_ref()
            .is_some_and(|key| key.len() > MAX_KEY_BYTES)
        {
            return Err(IpcError::invalid("This key is longer than any API key"));
        }

        let existing = match &id {
            Some(id) => self
                .declared_providers()
                .await?
                .into_iter()
                .find(|config| &config.id == id),
            None => None,
        };
        let base_url = match (probe.base_url.trim(), &existing) {
            ("", Some(previous)) => previous.base_url.clone(),
            (typed, _) => typed.to_owned(),
        };
        let candidate = AiProviderConfig::new(
            id.unwrap_or_else(|| ProviderId::for_new_declaration(probe.kind)),
            probe.kind,
            PROBE_PLACEHOLDER,
            base_url,
            PROBE_PLACEHOLDER,
        );
        let url = match endpoint(&candidate) {
            Ok(url) => url,
            Err(message) => return Ok(failed(ListingFailure::InvalidEndpoint, message)),
        };
        let key = ListKey::of(probe.kind, &url);

        if !refresh && let Some(listing) = self.inner.ai.model_lists.get(&key, Instant::now()) {
            return Ok(listing);
        }

        let stored = existing.filter(|previous| previous.same_endpoint_as(&candidate));
        let sent_key =
            typed_key.is_some() || stored.as_ref().is_some_and(|c| c.secret_ref.is_some());
        let credentials = Arc::clone(&self.inner.credentials);
        let base_url = candidate.base_url.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let key = match typed_key {
                Some(key) => Some(ApiKey::new(key)),
                None => match &stored {
                    Some(config) => credentials.provider_key(config)?,
                    None => None,
                },
            };
            Ok::<_, oxyn_core::OxynError>((key, oxyn_llm::endpoint_reach(&base_url)))
        })
        .await
        .map_err(|error| IpcError::invalid(format!("preparing the listing: {error}")))?;
        let (api_key, reach) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::warn!(error = %error, "the stored provider key could not be read");
                return Ok(failed(
                    ListingFailure::MissingKey,
                    "The stored key could not be read from the system keychain.",
                ));
            }
        };

        let limit = timeout.unwrap_or(if reach == Reach::Local {
            LOCAL_LIST_TIMEOUT
        } else {
            MODELS_TIMEOUT
        });
        let listed = tokio::time::timeout(
            limit,
            oxyn_llm::list_models(probe.kind, &candidate.base_url, api_key, reach),
        )
        .await;
        let models = match listed {
            Ok(Ok(models)) => models,
            Ok(Err(error)) => {
                let (reason, message) = classify(&error, probe.kind, &url, reach, sent_key);
                // `LlmError`'s rendering is written to be shown: the key is
                // scrubbed from bodies, and no variant copies the URL (I-03).
                tracing::debug!(?reason, error = %error, "listing models failed");
                return Ok(failed(reason, message));
            }
            Err(_) => {
                return Ok(failed(
                    ListingFailure::Timeout,
                    format!(
                        "The endpoint did not answer within {} seconds.",
                        limit.as_secs()
                    ),
                ));
            }
        };

        let models: Vec<ModelChoice> = models.into_iter().map(model_choice).collect();
        let fetched_at_ms = now_ms();
        self.inner
            .ai
            .model_lists
            .put(key, models.clone(), fetched_at_ms, Instant::now());
        Ok(ModelListing::Ok {
            models,
            cached: false,
            fetched_at_ms,
        })
    }
}

fn failed(reason: ListingFailure, message: impl Into<String>) -> ModelListing {
    ModelListing::Failed {
        reason,
        message: message.into(),
    }
}

/// The endpoint a probe names, refused when it cannot be one.
///
/// The messages never quote the address: it may carry a password.
fn endpoint(candidate: &AiProviderConfig) -> Result<Url, &'static str> {
    if candidate.base_url.is_empty() {
        return Err("Enter the provider's address.");
    }
    let url = Url::parse(&candidate.base_url)
        .map_err(|_| "This address is not a valid URL. It must start with http:// or https://.")?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The address must not carry a user name or password. \
                    Enter the key in its own field.");
    }
    if !matches!(url.scheme(), "http" | "https") {
        return Err("The address must start with http:// or https://.");
    }
    candidate
        .validate()
        .map_err(|_| "This address is not a valid endpoint.")?;
    Ok(url)
}

/// What the cache knows an endpoint by: its family and the address the
/// request resolves from. Never the key, never the URL's credentials, query or
/// fragment — the request does not carry the query either.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ListKey {
    kind: AiProviderKind,
    endpoint: String,
}

impl ListKey {
    fn of(kind: AiProviderKind, url: &Url) -> Self {
        let port = url
            .port_or_known_default()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        Self {
            kind,
            endpoint: format!(
                "{}://{}{port}{}",
                url.scheme(),
                url.host_str().unwrap_or_default(),
                url.path().trim_end_matches('/'),
            ),
        }
    }
}

/// One remembered list.
struct Listed {
    models: Vec<ModelChoice>,
    fetched_at_ms: u64,
    at: Instant,
}

/// The lists endpoints gave, successes only, for [`LIST_TTL`].
#[derive(Default)]
pub(crate) struct ModelLists {
    entries: Mutex<HashMap<ListKey, Listed>>,
}

impl ModelLists {
    fn get(&self, key: &ListKey, now: Instant) -> Option<ModelListing> {
        let mut entries = self.entries.lock();
        let fresh = entries
            .get(key)
            .is_some_and(|listed| now.saturating_duration_since(listed.at) < LIST_TTL);
        if !fresh {
            entries.remove(key);
            return None;
        }
        entries.get(key).map(|listed| ModelListing::Ok {
            models: listed.models.clone(),
            cached: true,
            fetched_at_ms: listed.fetched_at_ms,
        })
    }

    fn put(&self, key: ListKey, models: Vec<ModelChoice>, fetched_at_ms: u64, now: Instant) {
        let mut entries = self.entries.lock();
        entries.retain(|_, listed| now.saturating_duration_since(listed.at) < LIST_TTL);
        if !entries.contains_key(&key) && entries.len() >= MAX_CACHED_LISTS {
            let oldest = entries
                .iter()
                .min_by_key(|(_, listed)| listed.at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert(
            key,
            Listed {
                models,
                fetched_at_ms,
                at: now,
            },
        );
    }
}

/// What the form is told of a failure: a reason it can act on, and a short
/// sentence. Read on the error's variant and status, never on its text.
fn classify(
    error: &LlmError,
    kind: AiProviderKind,
    url: &Url,
    reach: Reach,
    sent_key: bool,
) -> (ListingFailure, String) {
    use ListingFailure as F;
    let unreachable = || {
        if reach == Reach::Local {
            "No server answered at this address. Is it running?"
        } else {
            "Could not reach the provider. Check the address and your network connection."
        }
    };
    match error {
        LlmError::Http { status, .. } => match status {
            401 if !sent_key => (F::MissingKey, "This endpoint requires an API key.".into()),
            401 => (F::Unauthorized, "Invalid API key.".into()),
            403 => (
                F::Forbidden,
                "This key is not allowed to list the provider's models.".into(),
            ),
            404 | 405 => (
                F::Unsupported,
                format!(
                    "The provider does not expose a model list at this endpoint.{}",
                    address_hint(kind, url)
                ),
            ),
            429 => (
                F::RateLimited,
                "The provider is limiting requests. Try again in a moment.".into(),
            ),
            408 | 504 => (F::Timeout, "The provider did not answer in time.".into()),
            300..=399 => (
                F::InvalidEndpoint,
                "The address redirects elsewhere. Enter the final address.".into(),
            ),
            500..=599 => (
                F::Unreachable,
                format!("The provider failed to answer (HTTP {status}). Try again later."),
            ),
            _ => (
                F::Unsupported,
                format!("The provider refused to list its models (HTTP {status})."),
            ),
        },
        // Refused, unknown host, or a connection that never opened in time:
        // in every case nothing answered.
        LlmError::Transport { .. } => (F::Unreachable, unreachable().into()),
        LlmError::ResponseTimeout { .. } => {
            (F::Timeout, "The provider did not answer in time.".into())
        }
        LlmError::ConnectionLost { .. } => (
            F::Unreachable,
            "The connection closed before the list arrived.".into(),
        ),
        LlmError::Decode { .. } => (
            F::Malformed,
            format!(
                "The endpoint answered, but not with a model list.{}",
                address_hint(kind, url)
            ),
        ),
        LlmError::MissingApiKey { .. } => (
            F::MissingKey,
            "An API key is needed to list this provider's models.".into(),
        ),
        LlmError::Config { .. } => (
            F::InvalidEndpoint,
            "This address cannot be used for this provider.".into(),
        ),
        LlmError::Unsupported { .. } | LlmError::NotImplemented { .. } => (
            F::Unsupported,
            "Oxyn cannot list models for this kind of provider.".into(),
        ),
        _ => (F::Unreachable, unreachable().into()),
    }
}

/// A hint for an address that likely misses or repeats the API version.
///
/// Said, never applied: Oxyn does not rewrite an endpoint the user typed.
fn address_hint(kind: AiProviderKind, url: &Url) -> &'static str {
    let path = url.path().trim_end_matches('/');
    match kind {
        AiProviderKind::OpenAi | AiProviderKind::OpenAiCompatible if path.is_empty() => {
            " Most OpenAI-compatible servers expect the address to end with /v1."
        }
        AiProviderKind::Anthropic if path.ends_with("/v1") => {
            " For Anthropic, the address stops before /v1."
        }
        AiProviderKind::Gemini if path.ends_with("/v1beta") || path.ends_with("/v1") => {
            " For Gemini, the address stops before the API version."
        }
        _ => "",
    }
}

#[cfg(test)]
#[path = "draft_models_tests.rs"]
mod tests;
