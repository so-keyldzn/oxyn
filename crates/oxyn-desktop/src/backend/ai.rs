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

mod conversation;
mod persistence;
mod samples;
mod threads;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oxyn_ai::external::locate::SearchPath;
use oxyn_ai::external::presets::PresetDraft;
use oxyn_core::{Actor, AiProviderConfig, CancelToken, Command, ExternalAgentConfig, ProviderId};
use oxyn_exec::Outcome;
use oxyn_llm::Reach;

pub(crate) use self::threads::AiState;
use super::Backend;
use crate::ipc::IpcError;
use crate::ipc::ai::{
    AgentDraft, AgentPresetDraft, DeclaredProvider, ExternalAgent, ModelChoice, ModelCost,
    ProviderDraft,
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
    pub async fn external_agents(&self) -> Result<Vec<ExternalAgent>, IpcError> {
        Ok(self
            .declared_agents()
            .await?
            .iter()
            .map(ExternalAgent::of)
            .collect())
    }

    /// Declares a provider, or edits one: key to the keyring first, then the
    /// declaration through the bus.
    ///
    /// The declaration is validated **before** the key is written, so a
    /// refused endpoint leaves no orphan entry. After that the order is the
    /// connection's: a failure past the keyring write leaves an unreachable
    /// entry, while the reverse would leave a declaration whose key does not
    /// exist.
    pub async fn save_ai_provider(
        &self,
        draft: ProviderDraft,
    ) -> Result<DeclaredProvider, IpcError> {
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

        self.dispatch_ai(Command::SaveAiProvider {
            config: Box::new(config.clone()),
        })
        .await?;

        // The old key goes only once the declaration no longer references it.
        if key.is_none()
            && draft.clear_key
            && let Some(reference) = existing.and_then(|previous| previous.secret_ref)
        {
            self.forget_provider_key(reference);
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
        let id = parse_provider_id(id)?;
        let secret_ref = self
            .declared_providers()
            .await?
            .into_iter()
            .find(|config| config.id == id)
            .and_then(|config| config.secret_ref);
        self.dispatch_ai(Command::RemoveAiProvider { id }).await?;
        if let Some(reference) = secret_ref {
            self.forget_provider_key(reference);
        }
        Ok(())
    }

    /// The models a declared provider says it serves.
    ///
    /// One request to the endpoint, with the stored key: it doubles as the
    /// check that the endpoint answers. Nothing from a database goes with it,
    /// so no tier applies.
    pub async fn provider_models(&self, id: &str) -> Result<Vec<ModelChoice>, IpcError> {
        let id = parse_provider_id(id)?;
        let config = self
            .declared_providers()
            .await?
            .into_iter()
            .find(|config| config.id == id)
            .ok_or_else(|| IpcError::invalid("This provider is no longer declared"))?;
        let credentials = Arc::clone(&self.inner.credentials);
        let provider = tokio::task::spawn_blocking(move || {
            let key = credentials.provider_key(&config)?;
            oxyn_llm::build_provider(config.kind, &config.base_url, key)
        })
        .await
        .map_err(|error| IpcError::invalid(format!("preparing the provider: {error}")))??;
        let models = tokio::time::timeout(MODELS_TIMEOUT, provider.models())
            .await
            .map_err(|_| IpcError {
                message: format!(
                    "The endpoint did not answer within {} seconds",
                    MODELS_TIMEOUT.as_secs()
                ),
                retryable: true,
            })??;
        Ok(models.into_iter().map(model_choice).collect())
    }

    /// Declares an external agent.
    ///
    /// The caller has already had the user confirm the exact command in a
    /// native dialog: declaring an agent is declaring a program Oxyn will run
    /// ([`crate::commands::ai::ai_save_external_agent`]).
    pub async fn save_external_agent(&self, draft: AgentDraft) -> Result<ExternalAgent, IpcError> {
        let agent = agent_of(&draft);
        agent.validate()?;
        self.dispatch_ai(Command::SaveExternalAgent {
            agent: Box::new(agent.clone()),
        })
        .await?;
        Ok(ExternalAgent::of(&agent))
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
    /// Only when the user clicks « Detect »: nothing is read at startup and
    /// nothing is saved here (ADR-0023). Reads `PATH` and `HOME` of this
    /// process, then the file system, off the IPC thread (I-05).
    pub async fn ai_detect_agent(&self, id: &str) -> Result<AgentPresetDraft, IpcError> {
        let preset = oxyn_ai::external::presets::preset(id)
            .ok_or_else(|| IpcError::invalid("Oxyn knows no such agent"))?;
        tokio::task::spawn_blocking(move || {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(std::path::PathBuf::from);
            let search = SearchPath::usual(std::env::var_os("PATH").as_deref(), home.as_deref());
            AgentPresetDraft::of(PresetDraft::detect(preset, &search), true)
        })
        .await
        .map_err(|error| IpcError::invalid(format!("looking for the agent: {error}")))
    }

    /// Removes an external agent. There is no key to forget.
    pub async fn remove_external_agent(&self, id: &str) -> Result<(), IpcError> {
        let id = parse_provider_id(id)?;
        self.dispatch_ai(Command::RemoveExternalAgent { id })
            .await?;
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
        match self.dispatch_ai(Command::ListExternalAgents).await? {
            Outcome::ExternalAgentsListed { agents } => Ok(agents),
            _ => Err(IpcError::invalid(
                "Listing the external agents gave an unexpected answer",
            )),
        }
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

    fn forget_provider_key(&self, reference: String) {
        let credentials = Arc::clone(&self.inner.credentials);
        tauri::async_runtime::spawn(async move {
            match tokio::task::spawn_blocking(move || credentials.forget_provider_key(&reference))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(error = %error, "orphan provider key left in the keyring");
                }
                Err(error) => tracing::warn!(error = %error, "the keyring task did not finish"),
            }
        });
    }
}

/// The declaration a draft describes.
///
/// A new declaration gets an opaque identity, never one derived from the
/// label: two providers both called « Prod » must stay two declarations, keys
/// included. An edit keeps its identity, its creation date, and — unless a new
/// one is typed — its endpoint and its key reference.
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
        config.secret_ref.clone_from(&previous.secret_ref);
    }
    config
}

fn agent_of(draft: &AgentDraft) -> ExternalAgentConfig {
    let mut agent = ExternalAgentConfig::new(
        ProviderId::for_new_agent(),
        draft.label.trim(),
        draft.command.trim(),
    )
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
    use oxyn_core::{AiProviderKind, PrivacyTier};

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
}
