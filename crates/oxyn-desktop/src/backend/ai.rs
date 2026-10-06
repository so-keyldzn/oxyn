//! The AI workspace, host side: declarations, models, conversations.
//!
//! Three things exist only here, because nowhere else can do them without
//! breaking an invariant:
//!
//! * **a key leaves the keyring** at the last moment, through
//!   [`KeyringCredentials`](crate::credentials::KeyringCredentials), and never
//!   crosses back to the webview ([I-03](../../../../CLAUDE.md#i-03));
//! * **an endpoint is classified** by a blocking DNS resolution, on the blocking
//!   pool, every time it matters — never read from storage
//!   ([ADR-0023](../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md));
//! * **the context is assembled** under the tier the connection has *now*, by the
//!   single gate of `oxyn-ai` ([I-04](../../../../CLAUDE.md#i-04)), in
//!   [`conversation`].
//!
//! Declarations are written through the command bus, like everything else
//! ([I-01](../../../../CLAUDE.md#i-01)). An agent cannot reach these commands:
//! the `PolicyGate` denies `SaveAiProvider` and its siblings to `Actor::Agent`.
//!
//! # Without a declaration there is no AI workspace
//!
//! An empty list is not a degraded state; it is Oxyn without AI
//! ([ADR-0006](../../../../docs/adr/0006-ai-privacy-tiers.md)). The front hides
//! every AI entry on it.

mod agents;
mod catalog_fill;
mod conversation;
mod draft_models;
#[cfg(test)]
mod environment_tests;
mod mentions;
mod persistence;
mod samples;
mod threads;
mod transports;
mod user_agents;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oxyn_ai::external::locate::SearchPath;
use oxyn_ai::external::presets::PresetDraft;
use oxyn_core::{
    Actor, AiProviderConfig, CancelToken, Command, ConnectionId, ExternalAgentConfig, ProviderId,
};
use oxyn_exec::{ExecutorSink, Outcome};
use oxyn_llm::Reach;

use self::catalog_fill::{CatalogFill, Want};
pub(crate) use self::draft_models::ModelLists;
pub(crate) use self::threads::AiState;
use super::Backend;
use crate::ipc::IpcError;
use crate::ipc::ai::{
    AgentDraft, AgentPresetDraft, DeclaredProvider, ExternalAgent, ListingFailure, ModelChoice,
    ModelCost, ModelListing, ModelProbe, ProviderDraft,
};

/// How long listing a provider's models may take before the screen says so.
///
/// Product choice: the call is a single HTTP request the user is watching, and
/// an endpoint that does not answer in this time is not one to wait for.
const MODELS_TIMEOUT: Duration = Duration::from_secs(20);

/// The largest API key accepted from the front.
///
/// Product bound, not a provider limit: real keys are a few hundred bytes, and
/// a megabyte pasted by mistake has no business reaching the keyring.
const MAX_KEY_BYTES: usize = 4096;

impl Backend {
    /// The declared providers, each classified local, remote or unresolved.
    ///
    /// Two blocking reads, neither on an IPC thread: the store, then one DNS
    /// resolution per declaration ([I-05](../../../../CLAUDE.md#i-05)).
    pub async fn ai_providers(&self) -> Result<Vec<DeclaredProvider>, IpcError> {
        let providers = self.declared_providers().await?;
        tokio::task::spawn_blocking(move || {
            providers
                .iter()
                .map(|config| {
                    let reach = oxyn_llm::endpoint_reach(&config.base_url);
                    DeclaredProvider::of(config, reach, now_ms())
                })
                .collect()
        })
        .await
        .map_err(|error| IpcError::invalid(format!("classifying the endpoints: {error}")))
    }

    /// The declared external agents. Nothing to classify: their reach is
    /// unknowable ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
    ///
    /// Whether each is confined reads Codex's managed configuration, off the
    /// IPC thread (I-05).
    pub async fn external_agents(&self) -> Result<Vec<ExternalAgent>, IpcError> {
        let agents = self.declared_agents().await?;
        tokio::task::spawn_blocking(move || agents.iter().map(agent_view).collect())
            .await
            .map_err(|error| IpcError::invalid(format!("reading the agents: {error}")))
    }

    /// Declares a provider, or edits one: key to the keyring first, then the
    /// declaration through the bus.
    ///
    /// The declaration is validated **before** the key is written, so a
    /// refused endpoint leaves no orphan entry. A typed key gets a fresh entry:
    /// a failed save forgets it without changing the previous declaration's
    /// key. A successful save forgets the old entry only once the declaration
    /// no longer references it
    /// ([I-03](../../../../CLAUDE.md#i-03)).
    pub async fn save_ai_provider(
        &self,
        draft: ProviderDraft,
    ) -> Result<DeclaredProvider, IpcError> {
        let _ordered = self.inner.ai.provider_declarations.lock().await;
        let existing = match &draft.id {
            Some(id) => {
                let id = parse_provider_id(id)?;
                Some(
                    self.declared_providers()
                        .await?
                        .into_iter()
                        .find(|config| config.id == id)
                        .ok_or_else(|| IpcError::invalid("This provider is no longer declared"))?,
                )
            }
            None => None,
        };
        let key = draft
            .key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        if key.is_some_and(|key| key.len() > MAX_KEY_BYTES) {
            return Err(IpcError::invalid("This key is longer than any API key"));
        }

        let mut config = compose_provider(&draft, existing.as_ref());
        config.validate()?;

        match key {
            Some(key) => {
                let credentials = Arc::clone(&self.inner.credentials);
                let id = config.id.clone();
                let key = key.to_owned();
                let reference =
                    tokio::task::spawn_blocking(move || credentials.store_provider_key(&id, &key))
                        .await
                        .map_err(|error| {
                            IpcError::invalid(format!("the keyring task failed: {error}"))
                        })??;
                config = config.with_secret_ref(reference.as_str());
            }
            None if draft.clear_key => config.secret_ref = None,
            None => {}
        }

        let saved = self
            .dispatch_ai(Command::SaveAiProvider {
                config: Box::new(config.clone()),
            })
            .await;
        // Whatever the save did: a transport built from the previous
        // declaration must not answer for this one.
        self.inner.ai.transports.forget(&config.id);
        if !matches!(saved, Ok(Outcome::AiProviderSaved { .. })) {
            if key.is_some()
                && let Some(reference) = config.secret_ref
            {
                self.forget_provider_key(reference).await;
            }
            return Err(saved
                .err()
                .unwrap_or_else(|| IpcError::invalid("The provider was not saved")));
        }

        // A fresh key must never overwrite the one the saved endpoint still
        // uses; only a successful save makes that previous entry unreachable.
        if let Some(reference) = existing.and_then(|previous| previous.secret_ref)
            && config.secret_ref.as_ref() != Some(&reference)
        {
            self.forget_provider_key(reference).await;
        }

        tokio::task::spawn_blocking(move || {
            let reach = oxyn_llm::endpoint_reach(&config.base_url);
            DeclaredProvider::of(&config, reach, now_ms())
        })
        .await
        .map_err(|error| IpcError::invalid(format!("classifying the endpoint: {error}")))
    }

    /// Removes a declaration, then forgets its key.
    ///
    /// In that order: a key forgotten first, followed by a failed removal,
    /// would leave a declaration that fails for no visible reason.
    pub async fn remove_ai_provider(&self, id: &str) -> Result<(), IpcError> {
        let _ordered = self.inner.ai.provider_declarations.lock().await;
        let id = parse_provider_id(id)?;
        let secret_ref = self
            .declared_providers()
            .await?
            .into_iter()
            .find(|config| config.id == id)
            .and_then(|config| config.secret_ref);
        let removed = self
            .dispatch_ai(Command::RemoveAiProvider { id: id.clone() })
            .await;
        // The transport holds the key: it goes with the declaration.
        self.inner.ai.transports.forget(&id);
        removed?;
        if let Some(reference) = secret_ref {
            self.forget_provider_key(reference).await;
        }
        Ok(())
    }

    /// The models a declared provider says it serves.
    ///
    /// One request to the endpoint, with the stored key: it doubles as the
    /// check that the endpoint answers. Nothing from a database goes with it,
    /// so no tier applies. The same path and cache as a form's listing
    /// ([`list_draft_models`](Self::list_draft_models)), on the stored
    /// endpoint and key.
    pub async fn provider_models(&self, id: &str) -> Result<Vec<ModelChoice>, IpcError> {
        let parsed = parse_provider_id(id)?;
        let config = self
            .declared_providers()
            .await?
            .into_iter()
            .find(|config| config.id == parsed)
            .ok_or_else(|| IpcError::invalid("This provider is no longer declared"))?;
        let probe = ModelProbe {
            id: Some(id.to_owned()),
            kind: config.kind,
            base_url: String::new(),
            key: None,
        };
        match self.list_draft_models(probe, false).await? {
            ModelListing::Ok { models, .. } => Ok(models),
            // Listing is a read: asking again cannot apply anything twice.
            ModelListing::Failed { reason, message } => Err(IpcError {
                message,
                retryable: matches!(
                    reason,
                    ListingFailure::RateLimited
                        | ListingFailure::Timeout
                        | ListingFailure::Unreachable
                ),
            }),
        }
    }

    /// Declares an external program only after the host confirms it.
    /// A refusal, premature approval or expiry leaves the declaration untouched.
    pub async fn save_external_agent(
        &self,
        draft: AgentDraft,
    ) -> Result<Option<ExternalAgent>, IpcError> {
        let mut agent = self.review_external_agent(&draft).await?;
        let names: std::collections::BTreeSet<String> = draft
            .env
            .iter()
            .filter(|var| var.secret)
            .map(|var| var.name.trim().to_owned())
            .collect();
        // Confirmed before the keychain is touched, so a refusal leaves no
        // entry behind; a secret value is named in the dialog, never shown.
        if !self
            .confirm_external_agent(&masked_secrets(&agent, &names), draft.id.is_some())
            .await?
        {
            return Ok(None);
        }
        // A replacement must not revive an agent removed while the dialog was
        // open. Persist the reviewed configuration, not a new draft or id.
        if draft.id.is_some() {
            self.review_external_agent(&draft).await?;
        }
        let previous = self
            .declared_agents()
            .await?
            .into_iter()
            .find(|old| old.id == agent.id);
        let credentials = Arc::clone(&self.inner.credentials);
        agent = tokio::task::spawn_blocking(move || {
            credentials.protect_agent_environment(&mut agent, &names)?;
            agent.validate_stored_environment()?;
            Ok::<_, oxyn_core::OxynError>(agent)
        })
        .await
        .map_err(|_| IpcError::invalid("the agent keychain task failed"))??;
        self.dispatch_ai(Command::SaveExternalAgent {
            agent: Box::new(agent.clone()),
        })
        .await?;
        if let Some(previous) = previous {
            self.forget_agent_secrets(previous).await;
        }
        tokio::task::spawn_blocking(move || agent_view(&agent))
            .await
            .map(Some)
            .map_err(|error| IpcError::invalid(format!("reading the agent: {error}")))
    }

    /// The declaration a draft becomes, validated — what the confirmation
    /// dialog must show, and must show only once it is known to be valid: a
    /// dialog confirmed for a declaration then refused is a question asked for
    /// nothing, and a label full of newlines hides the command below it.
    ///
    /// A draft naming an agent replaces it, which must still be declared.
    async fn review_external_agent(
        &self,
        draft: &AgentDraft,
    ) -> Result<ExternalAgentConfig, IpcError> {
        let id = match draft.id.as_deref() {
            None => ProviderId::for_new_agent(),
            Some(id) => {
                let id = parse_provider_id(id)?;
                if !self
                    .declared_agents()
                    .await?
                    .iter()
                    .any(|agent| agent.id == id)
                {
                    return Err(IpcError::invalid(
                        "the agent to replace is no longer declared",
                    ));
                }
                id
            }
        };
        let agent = agent_of(id, draft);
        agent.validate()?;
        Ok(agent)
    }

    /// The ready-made agent declarations, as they are before looking at the
    /// machine.
    #[must_use]
    pub fn ai_agent_presets(&self) -> Vec<AgentPresetDraft> {
        oxyn_ai::external::presets::PRESETS
            .into_iter()
            .map(|preset| AgentPresetDraft::of(PresetDraft::blank(preset), false))
            .collect()
    }

    /// A preset completed with what the machine has, for the user to review.
    ///
    /// Asked when the AI settings screen opens, and by « Detect again ».
    /// Nothing is run and nothing is saved here. Reads `PATH` and `HOME` of this
    /// process, then the file system, off the IPC thread (I-05).
    pub async fn ai_detect_agent(&self, id: &str) -> Result<AgentPresetDraft, IpcError> {
        let preset = oxyn_ai::external::presets::preset(id)
            .ok_or_else(|| IpcError::invalid("Oxyn knows no such agent"))?;
        tokio::task::spawn_blocking(move || {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(std::path::PathBuf::from);
            let search = SearchPath::usual(
                std::env::var_os("PATH").as_deref(),
                home.as_deref(),
                preset.node_major,
            );
            AgentPresetDraft::of(PresetDraft::detect(preset, &search), true)
        })
        .await
        .map_err(|error| IpcError::invalid(format!("looking for the agent: {error}")))
    }

    /// Removes an external agent, then forgets its environment secrets.
    pub async fn remove_external_agent(&self, id: &str) -> Result<(), IpcError> {
        let id = parse_provider_id(id)?;
        let previous = self
            .declared_agents()
            .await?
            .into_iter()
            .find(|agent| agent.id == id);
        self.dispatch_ai(Command::RemoveExternalAgent { id })
            .await?;
        if let Some(previous) = previous {
            self.forget_agent_secrets(previous).await;
        }
        Ok(())
    }

    /// Lists the relations of the schemas never listed, so that `@` offers
    /// them before the tree is expanded: the listings a question would read,
    /// within the same bounds, and no description
    /// ([ADR-0036](../../../../docs/adr/0036-l-assistant-complete-le-catalogue.md)).
    ///
    /// As [`Actor::Human`]: typing `@` is the user's gesture, and no model is
    /// involved. The reads are the tree's expansions, through the bus
    /// ([I-01](../../../../CLAUDE.md#i-01)). Nothing is shown on a thread: the
    /// menu says it is completing while this runs.
    pub async fn list_mentionable(&self, connection: ConnectionId) -> Result<(), IpcError> {
        let catalog = self
            .inner
            .executor
            .catalog(connection)
            .ok_or_else(|| IpcError::invalid("This connection has no catalog cache"))?;
        let sink = ExecutorSink::new(Arc::clone(&self.inner.executor));
        let fill = CatalogFill {
            sink: &sink,
            actor: Actor::Human,
            connection,
            catalog,
        };
        fill.run(Want::Names, &CancelToken::new(), &|_| {}).await;
        Ok(())
    }

    pub(crate) async fn declared_providers(&self) -> Result<Vec<AiProviderConfig>, IpcError> {
        match self.dispatch_ai(Command::ListAiProviders).await? {
            Outcome::AiProvidersListed { providers } => Ok(providers),
            _ => Err(IpcError::invalid(
                "Listing the AI providers gave an unexpected answer",
            )),
        }
    }

    pub(crate) async fn declared_agents(&self) -> Result<Vec<ExternalAgentConfig>, IpcError> {
        let _migration = self.inner.ai.environment_migration.lock().await;
        let Outcome::ExternalAgentsListed { mut agents } =
            self.dispatch_ai(Command::ListExternalAgents).await?
        else {
            return Err(IpcError::invalid(
                "Listing the external agents gave an unexpected answer",
            ));
        };
        for agent in &mut agents {
            if agent
                .env
                .iter()
                .any(|(name, _)| oxyn_core::agent_env_is_secret(name))
            {
                let credentials = Arc::clone(&self.inner.credentials);
                let mut upgraded = agent.clone();
                upgraded = tokio::task::spawn_blocking(move || {
                    credentials.protect_agent_environment(&mut upgraded, &Default::default())?;
                    Ok::<_, oxyn_core::OxynError>(upgraded)
                })
                .await
                .map_err(|_| IpcError::invalid("the agent keychain migration task failed"))??;
                self.dispatch_ai(Command::SaveExternalAgent {
                    agent: Box::new(upgraded.clone()),
                })
                .await?;
                *agent = upgraded;
            }
        }
        Ok(agents)
    }

    /// A human dispatch whose outcome carries data the generic
    /// [`describe`](super::describe) drops.
    async fn dispatch_ai(&self, command: Command) -> Result<Outcome, IpcError> {
        match self
            .inner
            .executor
            .dispatch(Actor::Human, command, &CancelToken::new())
            .await?
        {
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            outcome => Ok(outcome),
        }
    }

    async fn forget_agent_secrets(&self, agent: ExternalAgentConfig) {
        let credentials = Arc::clone(&self.inner.credentials);
        let result = tokio::task::spawn_blocking(move || {
            for (_, reference) in agent.env_secret_refs {
                if credentials.forget_agent_secret(&reference).is_err() {
                    tracing::warn!("orphan agent secret left in the keychain");
                }
            }
        })
        .await;
        if result.is_err() {
            tracing::warn!("agent keychain cleanup task failed");
        }
    }

    async fn forget_provider_key(&self, reference: String) {
        let credentials = Arc::clone(&self.inner.credentials);
        match tokio::task::spawn_blocking(move || credentials.forget_provider_key(&reference)).await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(error = %error, "orphan provider key left in the keyring");
            }
            Err(error) => tracing::warn!(error = %error, "the keyring task did not finish"),
        }
    }
}

/// The declaration as the confirmation dialog shows it: a value bound for the
/// keychain appears as `<keychain>`, so a typed token is never drawn on screen
/// ([I-03](../../../../CLAUDE.md#i-03)), while its name still tells the user
/// what the program receives.
fn masked_secrets(
    agent: &ExternalAgentConfig,
    secret_names: &std::collections::BTreeSet<String>,
) -> ExternalAgentConfig {
    let mut shown = agent.clone();
    for (name, value) in &mut shown.env {
        if oxyn_core::agent_env_is_secret(name) || secret_names.contains(name.as_str()) {
            *value = KEYCHAIN_PLACEHOLDER.to_owned();
        }
    }
    let stored = std::mem::take(&mut shown.env_secret_refs);
    shown.env.extend(
        stored
            .into_iter()
            .map(|(name, _)| (name, KEYCHAIN_PLACEHOLDER.to_owned())),
    );
    shown
}

/// What the dialog shows in place of a secret environment value.
const KEYCHAIN_PLACEHOLDER: &str = "<keychain>";

/// The declaration a draft describes.
///
/// A new declaration gets an opaque identity, never one derived from the
/// label: two providers both called « Prod » must stay two declarations, keys
/// included. An edit keeps its identity and its creation date; it keeps its
/// key reference only while the kind and the endpoint (scheme, host, port,
/// path, query) stay the same — a changed endpoint takes a key typed for it
/// ([I-03](../../../../CLAUDE.md#i-03)).
fn compose_provider(
    draft: &ProviderDraft,
    existing: Option<&AiProviderConfig>,
) -> AiProviderConfig {
    let id = existing.map_or_else(
        || ProviderId::for_new_declaration(draft.kind),
        |previous| previous.id.clone(),
    );
    let base_url = match (draft.base_url.trim(), existing) {
        ("", Some(previous)) => previous.base_url.clone(),
        (typed, _) => typed.to_owned(),
    };
    let mut config = AiProviderConfig::new(
        id,
        draft.kind,
        draft.label.trim(),
        base_url,
        draft.model.trim(),
    );
    if let Some(previous) = existing {
        config.created_at = previous.created_at;
        if previous.same_endpoint_as(&config) {
            config.secret_ref.clone_from(&previous.secret_ref);
        }
    }
    config
}

/// An agent as the screen shows it. Not confined when Codex's managed
/// configuration turns on what Oxyn switches off: the organization's layer
/// ranks above Oxyn's (ADR-0033). Blocks on the file system.
fn agent_view(config: &ExternalAgentConfig) -> ExternalAgent {
    let mut agent = ExternalAgent::of(config);
    agent.confined = agent.confined && !oxyn_ai::external::confine::managed_turns_on(config);
    agent
}

/// The store keeps the first `created_at` of an agent it replaces.
fn agent_of(id: ProviderId, draft: &AgentDraft) -> ExternalAgentConfig {
    let mut agent = ExternalAgentConfig::new(id, draft.label.trim(), draft.command.trim())
        .with_args(draft.args.iter().map(|arg| arg.trim().to_owned()));
    agent.env = draft
        .env
        .iter()
        .map(|var| (var.name.trim().to_owned(), var.value.clone()))
        .collect();
    agent
}

fn parse_provider_id(id: &str) -> Result<ProviderId, IpcError> {
    ProviderId::new(id).map_err(|error| IpcError::invalid(format!("invalid provider: {error}")))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// May a conversation on this tier reach this endpoint?
///
/// Asked **before** the keyring is read and before any transport exists; the
/// runtime asks again against its own measurement, so a stale answer here can
/// only cost a refusal, never a send ([I-04](../../../../CLAUDE.md#i-04)).
pub(crate) fn check_endpoint(tier: oxyn_core::PrivacyTier, reach: Reach) -> Result<(), String> {
    if oxyn_ai::privacy::allows_endpoint(tier, reach) {
        return Ok(());
    }
    Err(match reach {
        Reach::Unresolved => {
            "This connection is local-only, and this provider's endpoint could not \
             be resolved: Oxyn treats it as remote. Nothing was sent."
                .to_owned()
        }
        _ => "This connection is local-only, and this provider's endpoint leaves this machine. \
             Nothing was sent."
            .to_owned(),
    })
}

/// What the panel is told of one model: what the provider published, and
/// nothing Oxyn adds.
fn model_choice(model: oxyn_llm::ModelInfo) -> ModelChoice {
    ModelChoice {
        id: model.id,
        display_name: model.display_name,
        context_window: model.context_window,
        cost: model.cost.map(|cost| ModelCost {
            input_per_million: cost.input_per_million,
            output_per_million: cost.output_per_million,
            currency: cost.currency,
        }),
        reasoning_efforts: model.reasoning_efforts,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxyn_core::{AiProviderKind, PrivacyTier};
    use oxyn_secrets::{ExposeSecret as _, MemorySecretStore, SecretRef, SecretStore};
    use oxyn_store::Store;

    use super::*;

    #[test]
    fn a_models_declared_efforts_reach_the_panel() {
        use oxyn_llm::ReasoningEffort::{High, Low};

        let choice = model_choice(
            oxyn_llm::ModelInfo::new("claude-opus-5").with_reasoning_efforts(vec![High, Low]),
        );
        assert_eq!(choice.reasoning_efforts, vec![Low, High]);
        // None declared stays none: no selector.
        assert!(
            model_choice(oxyn_llm::ModelInfo::new("m"))
                .reasoning_efforts
                .is_empty()
        );
    }

    fn draft(id: Option<String>) -> ProviderDraft {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "kind": "openai_compatible",
            "label": "  Ollama  ",
            "baseUrl": "",
            "model": " llama3 ",
        }))
        .expect("valid draft")
    }

    #[test]
    fn a_local_tier_refuses_a_remote_or_unresolved_endpoint() {
        assert!(check_endpoint(PrivacyTier::Local, Reach::Remote).is_err());
        assert!(check_endpoint(PrivacyTier::Local, Reach::Unresolved).is_err());
        assert!(check_endpoint(PrivacyTier::Local, Reach::Local).is_ok());
        assert!(check_endpoint(PrivacyTier::Metadata, Reach::Remote).is_ok());
    }

    #[test]
    fn an_edit_keeps_identity_endpoint_and_key_reference() {
        let previous = AiProviderConfig::new(
            ProviderId::for_new_declaration(AiProviderKind::OpenAiCompatible),
            AiProviderKind::OpenAiCompatible,
            "Old",
            "http://localhost:11434/v1",
            "old",
        )
        .with_secret_ref("oxyn:llm:openai_compatible-00000000");
        let config = compose_provider(&draft(Some(previous.id.to_string())), Some(&previous));
        assert_eq!(config.id, previous.id);
        assert_eq!(config.base_url, previous.base_url);
        assert_eq!(config.secret_ref, previous.secret_ref);
        assert_eq!(config.label, "Ollama");
        assert_eq!(config.model, "llama3");
    }

    #[test]
    fn a_kind_change_drops_the_key_reference() {
        let previous = AiProviderConfig::new(
            ProviderId::for_new_declaration(AiProviderKind::Anthropic),
            AiProviderKind::Anthropic,
            "Old",
            "https://198.51.100.1/v1",
            "old",
        )
        .with_secret_ref("oxyn:llm:anthropic-00000000");
        let edited = provider_draft(
            Some(previous.id.to_string()),
            AiProviderKind::OpenAiCompatible,
            "https://198.51.100.1/v1",
            None,
        );
        let config = compose_provider(&edited, Some(&previous));
        assert_eq!(config.secret_ref, None);
    }

    /// A draft naming every field, unlike [`draft`]: the tests below edit
    /// across a host change and must control the endpoint and the kind.
    fn provider_draft(
        id: Option<String>,
        kind: AiProviderKind,
        base_url: &str,
        key: Option<&str>,
    ) -> ProviderDraft {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "kind": kind,
            "label": "Provider",
            "baseUrl": base_url,
            "model": "model",
            "key": key,
        }))
        .expect("valid draft")
    }

    fn backend_with_provider_store(path: &std::path::Path) -> (Backend, Arc<MemorySecretStore>) {
        let secrets = Arc::new(MemorySecretStore::new());
        let backend = Backend::assemble(
            Arc::new(Store::open_at(path).expect("temporary store")),
            Arc::clone(&secrets) as Arc<dyn SecretStore>,
        )
        .expect("temporary backend");
        (backend, secrets)
    }

    fn stored_provider(backend: &Backend, runtime: &tokio::runtime::Runtime) -> AiProviderConfig {
        runtime
            .block_on(backend.declared_providers())
            .expect("listed")
            .into_iter()
            .next()
            .expect("one declaration")
    }

    fn stored_key(store: &MemorySecretStore, config: &AiProviderConfig) -> Option<String> {
        let reference = SecretRef::parse(config.secret_ref.as_deref().expect("a reference"))
            .expect("valid reference");
        store
            .get(&reference)
            .expect("keychain read")
            .map(|secret| secret.expose_secret().to_owned())
    }

    #[test]
    fn a_failed_provider_edit_keeps_the_old_declaration_and_key() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime for the executor");
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("providers.sqlite3");
        let (backend, secrets) = backend_with_provider_store(&path);
        // An existing installation may still name the deterministic entry.
        let id = ProviderId::for_new_declaration(AiProviderKind::Anthropic);
        let reference = SecretRef::for_provider(id.as_str()).expect("legacy reference");
        secrets
            .put(
                &reference,
                oxyn_secrets::SecretString::from("old-test-key".to_owned()),
            )
            .expect("legacy key");
        let before = AiProviderConfig::new(
            id,
            AiProviderKind::Anthropic,
            "Provider",
            "https://198.51.100.1/v1",
            "model",
        )
        .with_secret_ref(reference.as_str());
        backend
            .inner
            .executor
            .store()
            .providers()
            .save(&before)
            .expect("legacy declaration");
        assert_eq!(
            backend
                .inner
                .credentials
                .provider_key(&before)
                .expect("legacy lookup")
                .expect("legacy key")
                .expose(),
            "old-test-key"
        );
        assert_eq!(
            stored_key(&secrets, &before).as_deref(),
            Some("old-test-key")
        );
        let connection = rusqlite::Connection::open(&path).expect("test connection");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_provider_update
                 BEFORE UPDATE ON ai_providers
                 BEGIN
                     SELECT RAISE(FAIL, 'injected provider save failure');
                 END;",
            )
            .expect("failing trigger");

        runtime
            .block_on(backend.save_ai_provider(provider_draft(
                Some(before.id.to_string()),
                AiProviderKind::Anthropic,
                "https://203.0.113.7/v1",
                Some("new-test-key"),
            )))
            .expect_err("the declaration save fails");

        let after = stored_provider(&backend, &runtime);
        assert_eq!(after.base_url, before.base_url);
        assert_eq!(after.secret_ref, before.secret_ref);
        assert_eq!(
            stored_key(&secrets, &after).as_deref(),
            Some("old-test-key")
        );
        assert_eq!(secrets.len(), 1, "the failed edit leaves no fresh orphan");
    }

    #[test]
    fn a_successful_provider_key_edit_replaces_the_reference_and_forgets_the_old_entry() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime for the executor");
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("providers.sqlite3");
        let (backend, secrets) = backend_with_provider_store(&path);
        let declared = runtime
            .block_on(backend.save_ai_provider(provider_draft(
                None,
                AiProviderKind::Anthropic,
                "https://198.51.100.1/v1",
                Some("old-test-key"),
            )))
            .expect("declared");
        let mut before = stored_provider(&backend, &runtime);
        // Replacing on the same endpoint must isolate its key just like a move.
        for endpoint in ["https://203.0.113.7/v1", "https://203.0.113.7/v1"] {
            runtime
                .block_on(backend.save_ai_provider(provider_draft(
                    Some(declared.id.clone()),
                    AiProviderKind::Anthropic,
                    endpoint,
                    Some("new-test-key"),
                )))
                .expect("edited");

            let after = stored_provider(&backend, &runtime);
            assert_ne!(after.secret_ref, before.secret_ref);
            assert_eq!(
                stored_key(&secrets, &after).as_deref(),
                Some("new-test-key")
            );
            let old_reference =
                SecretRef::parse(before.secret_ref.as_deref().expect("old reference"))
                    .expect("valid old reference");
            assert!(
                !secrets.contains(&old_reference),
                "the old key is forgotten"
            );
            assert_eq!(secrets.len(), 1);
            before = after;
        }
    }

    #[test]
    fn overlapping_provider_key_edits_leave_only_the_saved_entry() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let directory = tempfile::tempdir().expect("temporary directory");
        let (backend, secrets) =
            backend_with_provider_store(&directory.path().join("providers.sqlite3"));
        runtime.block_on(async {
            let declared = backend
                .save_ai_provider(provider_draft(
                    None,
                    AiProviderKind::Anthropic,
                    "https://198.51.100.1/v1",
                    Some("initial-test-key"),
                ))
                .await
                .expect("declared");
            let before = backend
                .declared_providers()
                .await
                .expect("listed")
                .remove(0);
            let (left, right) = tokio::join!(
                backend.save_ai_provider(provider_draft(
                    Some(declared.id.clone()),
                    AiProviderKind::Anthropic,
                    "https://203.0.113.7/v1",
                    Some("left-test-key")
                )),
                backend.save_ai_provider(provider_draft(
                    Some(declared.id.clone()),
                    AiProviderKind::Anthropic,
                    "https://198.51.100.2/v1",
                    Some("right-test-key")
                )),
            );
            left.expect("left saved");
            right.expect("right saved");
            let saved = backend
                .declared_providers()
                .await
                .expect("listed")
                .remove(0);
            assert_ne!(saved.secret_ref, before.secret_ref);
            assert_eq!(
                secrets.len(),
                1,
                "each successful save retires its actual predecessor"
            );
            let key = stored_key(&secrets, &saved).expect("saved key");
            let expected = if saved.base_url == "https://203.0.113.7/v1" {
                "left-test-key"
            } else {
                "right-test-key"
            };
            assert_eq!(key, expected);
        });
    }

    #[test]
    fn an_edit_to_another_host_without_a_key_drops_the_key_reference() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime for the executor");
        let backend = Backend::open_temporary().expect("temporary backend");
        runtime.block_on(async {
            let declared = backend
                .save_ai_provider(provider_draft(
                    None,
                    AiProviderKind::Anthropic,
                    "https://198.51.100.1/v1",
                    Some("not-a-real-key"),
                ))
                .await
                .expect("declared");
            assert!(declared.key_configured);

            let edited = backend
                .save_ai_provider(provider_draft(
                    Some(declared.id.clone()),
                    AiProviderKind::Anthropic,
                    "https://203.0.113.7/v1",
                    None,
                ))
                .await
                .expect("edited without a key, to another host");
            assert!(!edited.key_configured);

            let stored = backend
                .declared_providers()
                .await
                .expect("listed")
                .into_iter()
                .find(|config| config.id.to_string() == declared.id)
                .expect("still declared");
            assert_eq!(stored.secret_ref, None);
        });
    }

    #[test]
    fn an_edit_on_the_same_endpoint_keeps_the_key() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime for the executor");
        let backend = Backend::open_temporary().expect("temporary backend");
        runtime.block_on(async {
            let declared = backend
                .save_ai_provider(provider_draft(
                    None,
                    AiProviderKind::Anthropic,
                    "https://198.51.100.1/v1",
                    Some("not-a-real-key"),
                ))
                .await
                .expect("declared");

            let edited = backend
                .save_ai_provider(provider_draft(
                    Some(declared.id.clone()),
                    AiProviderKind::Anthropic,
                    "https://198.51.100.1:443/v1/",
                    None,
                ))
                .await
                .expect("edited without a key, to the same endpoint");
            assert!(edited.key_configured);

            let before = backend
                .declared_providers()
                .await
                .expect("listed")
                .into_iter()
                .find(|config| config.id.to_string() == declared.id)
                .expect("still declared")
                .secret_ref;
            assert!(before.is_some());
        });
    }

    #[test]
    fn models_after_a_host_change_read_no_key() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime for the executor");
        let backend = Backend::open_temporary().expect("temporary backend");
        runtime.block_on(async {
            let declared = backend
                .save_ai_provider(provider_draft(
                    None,
                    AiProviderKind::Anthropic,
                    "https://198.51.100.1/v1",
                    Some("not-a-real-key"),
                ))
                .await
                .expect("declared");
            backend
                .save_ai_provider(provider_draft(
                    Some(declared.id.clone()),
                    AiProviderKind::Anthropic,
                    "https://203.0.113.7/v1",
                    None,
                ))
                .await
                .expect("edited without a key, to another host");

            let error = backend
                .provider_models(&declared.id)
                .await
                .expect_err("no key follows a changed host");
            assert!(
                error.message.contains("An API key is needed"),
                "unexpected message: {}",
                error.message
            );
        });
    }

    #[test]
    fn two_declarations_of_the_same_draft_stay_two() {
        let first = compose_provider(&draft(None), None);
        let second = compose_provider(&draft(None), None);
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn a_new_declaration_with_credentials_in_the_endpoint_is_refused_before_the_keyring() {
        let mut typed = draft(None);
        typed.base_url = "https://alice:hunter2@api.example.com/v1".to_owned();
        let error = compose_provider(&typed, None)
            .validate()
            .expect_err("credentials in the authority are refused");
        assert!(!error.to_string().contains("hunter2"));
    }

    fn agent_draft(id: Option<String>, command: &str) -> AgentDraft {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "label": "Codex",
            "command": command,
        }))
        .expect("valid draft")
    }

    #[test]
    fn a_replacement_rewrites_the_agent_in_place_and_never_revives_a_removed_one() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime for the executor");
        let backend = Backend::open_temporary().expect("temporary backend");
        runtime.block_on(async {
            let declared = backend
                .save_external_agent(agent_draft(None, "/old/codex-acp"))
                .await
                .expect("decision answered")
                .expect("declared");

            backend
                .save_external_agent(agent_draft(Some(declared.id.clone()), "/new/codex-acp"))
                .await
                .expect("replaced");
            let agents = backend.declared_agents().await.expect("listed");
            assert_eq!(agents.len(), 1, "one declaration, not two");
            assert_eq!(agents[0].id.to_string(), declared.id);
            assert_eq!(agents[0].command, "/new/codex-acp");

            backend
                .remove_external_agent(&declared.id)
                .await
                .expect("removed");
            assert!(
                backend
                    .save_external_agent(agent_draft(Some(declared.id.clone()), "/new/codex-acp"))
                    .await
                    .is_err()
            );
            assert!(backend.declared_agents().await.expect("listed").is_empty());
        });
    }
}
