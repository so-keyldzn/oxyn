//! Model transports kept from one question to the next.
//!
//! Each question built its transport anew — an HTTP client, its TLS setup, a
//! keyring read —, so no connection to the endpoint was ever reused: every
//! question paid a handshake, and the keyring was read for each one.
//!
//! A transport is kept per declaration, and only for as long as what built it
//! holds: the same protocol, the same endpoint, the same key **reference**,
//! and the same reach measured for this question. A new key always gets a new
//! reference (`Credentials::store_provider_key`), a cleared key none, so a key
//! replaced or removed never answers through a transport built with the old
//! one. The reach is still measured at every question, before this cache is
//! consulted: a name that resolved locally yesterday is not trusted today
//! ([ADR-0023](../../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use oxyn_core::{AiProviderConfig, AiProviderKind, ProviderId};
use oxyn_llm::{LlmProvider, Reach};
use parking_lot::Mutex;

/// The transports kept, one per declared provider.
#[derive(Default)]
pub(crate) struct Transports {
    held: Mutex<HashMap<ProviderId, Held>>,
}

/// Counted, never printed: a transport holds its key (I-03).
impl fmt::Debug for Transports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transports")
            .field("held", &self.held.lock().len())
            .finish()
    }
}

struct Held {
    built: Built,
    transport: Arc<dyn LlmProvider>,
}

/// What a transport was built from. The key itself is not here: its
/// reference changes whenever it does.
#[derive(PartialEq, Eq)]
struct Built {
    kind: AiProviderKind,
    base_url: String,
    secret_ref: Option<String>,
    reach: Reach,
}

impl Built {
    fn of(config: &AiProviderConfig, reach: Reach) -> Self {
        Self {
            kind: config.kind,
            base_url: config.base_url.clone(),
            secret_ref: config.secret_ref.clone(),
            reach,
        }
    }
}

impl Transports {
    /// The transport kept for `config`, if it was built from what `config`
    /// declares now, for `reach`.
    pub(crate) fn get(
        &self,
        config: &AiProviderConfig,
        reach: Reach,
    ) -> Option<Arc<dyn LlmProvider>> {
        let wanted = Built::of(config, reach);
        self.held
            .lock()
            .get(&config.id)
            .filter(|held| held.built == wanted)
            .map(|held| Arc::clone(&held.transport))
    }

    /// Keeps `transport`, built for `config` and `reach`, replacing the one
    /// kept for that declaration.
    pub(crate) fn keep(
        &self,
        config: &AiProviderConfig,
        reach: Reach,
        transport: Arc<dyn LlmProvider>,
    ) {
        self.held.lock().insert(
            config.id.clone(),
            Held {
                built: Built::of(config, reach),
                transport,
            },
        );
    }

    /// Drops the transport of a declaration edited or removed: the key it
    /// holds goes with it.
    pub(crate) fn forget(&self, id: &ProviderId) {
        self.held.lock().remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(secret: Option<&str>) -> AiProviderConfig {
        let mut config = AiProviderConfig::new(
            "anthropic-work".parse().expect("a provider id"),
            AiProviderKind::Anthropic,
            "Work",
            "https://api.anthropic.com",
            "claude",
        );
        config.secret_ref = secret.map(str::to_owned);
        config
    }

    fn transport(config: &AiProviderConfig) -> Arc<dyn LlmProvider> {
        oxyn_llm::build_provider(
            config.kind,
            &config.base_url,
            Some(oxyn_llm::ApiKey::new("sk-test")),
            Reach::Remote,
        )
        .expect("a transport")
    }

    #[test]
    fn a_transport_is_reused_until_what_built_it_changes() {
        let transports = Transports::default();
        let config = declared(Some("oxyn:provider:anthropic-work.a"));
        assert!(transports.get(&config, Reach::Remote).is_none());

        let built = transport(&config);
        transports.keep(&config, Reach::Remote, Arc::clone(&built));
        let reused = transports.get(&config, Reach::Remote).expect("kept");
        assert!(Arc::ptr_eq(&reused, &built));

        // A new key is a new reference.
        let rekeyed = declared(Some("oxyn:provider:anthropic-work.b"));
        assert!(transports.get(&rekeyed, Reach::Remote).is_none());
        // A cleared key, another endpoint, another reach: built again.
        assert!(transports.get(&declared(None), Reach::Remote).is_none());
        let mut moved = config.clone();
        moved.base_url = "https://proxy.example.com".to_owned();
        assert!(transports.get(&moved, Reach::Remote).is_none());
        assert!(transports.get(&config, Reach::Unresolved).is_none());

        transports.forget(&config.id);
        assert!(transports.get(&config, Reach::Remote).is_none());
    }
}
