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
pub(in crate::backend) mod catalog_fill;
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
                missing_module: None,
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
mod tests;
