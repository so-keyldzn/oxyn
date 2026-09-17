//! What crosses the IPC boundary for the AI workspace.
//!
//! Narrower than the domain, like the rest of [`crate::ipc`]: a declaration's
//! secret reference, an API key, an agent's arguments and environment never
//! reach the webview ([I-03](../../../../CLAUDE.md#i-03)). A key travels **once**,
//! front to back, inside [`ProviderDraft`], and nothing sends it back.
//!
//! Conversations and their events live in [`conversation`], re-exported here.

use std::fmt;

use oxyn_ai::external::presets::{PRESETS, PresetDraft};
use oxyn_core::{AiProviderConfig, AiProviderKind, ExternalAgentConfig};
use oxyn_llm::Reach;
use serde::{Deserialize, Serialize};

mod conversation;

pub use self::conversation::*;

/// Where an endpoint resolved, as the screen says it.
///
/// `Unresolved` stays `Unresolved`: it counts as remote for every decision,
/// but showing it as « remote » would claim a measurement that did not happen
/// ([UX-SPEC](../../../../docs/UX-SPEC.md#le-niveau-se-lit-avant-de-parler-pas-après)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderReach {
    Local,
    Remote,
    Unresolved,
}

impl From<Reach> for ProviderReach {
    fn from(reach: Reach) -> Self {
        match reach {
            Reach::Local => Self::Local,
            Reach::Remote => Self::Remote,
            Reach::Unresolved => Self::Unresolved,
        }
    }
}

/// A declared model provider, as the settings screen and the panel show it.
///
/// `id` is opaque and only addresses the declaration in later calls; the views
/// never render it. `endpoint` is [`redacted_endpoint`]: the domain refuses
/// credentials in the authority, but a query string may still carry a key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredProvider {
    pub id: String,
    pub label: String,
    pub kind: AiProviderKind,
    pub endpoint: String,
    /// Whether `endpoint` lost a query or fragment on the way: an edit then
    /// keeps the stored endpoint unless a new one is typed.
    pub endpoint_redacted: bool,
    pub model: String,
    /// « a key is stored », never where nor what.
    pub key_configured: bool,
    pub reach: ProviderReach,
    /// Milliseconds since the Unix epoch. Never persisted: it dates this
    /// session's resolution ([ADR-0023](../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    pub measured_at_ms: u64,
}

impl DeclaredProvider {
    pub fn of(config: &AiProviderConfig, reach: Reach, measured_at_ms: u64) -> Self {
        let (endpoint, endpoint_redacted) = redacted_endpoint(&config.base_url);
        Self {
            id: config.id.as_str().to_owned(),
            label: config.label.clone(),
            kind: config.kind,
            endpoint,
            endpoint_redacted,
            model: config.model.clone(),
            key_configured: config.secret_ref.is_some(),
            reach: reach.into(),
            measured_at_ms,
        }
    }
}

/// An endpoint as it may be shown: no user information, no query, no fragment.
///
/// The domain already refuses `user:password@` ([`AiProviderConfig::validate`]);
/// stripping it again costs nothing and keeps this function honest on a value
/// written by another binary.
pub fn redacted_endpoint(base_url: &str) -> (String, bool) {
    let trimmed = base_url.trim();
    let mut redacted = false;
    let without_tail = match trimmed.find(['?', '#']) {
        Some(end) => {
            redacted = true;
            trimmed.get(..end).unwrap_or_default()
        }
        None => trimmed,
    };
    let (scheme, rest) = match without_tail.find("://") {
        Some(position) => (
            without_tail.get(..position + 3).unwrap_or_default(),
            without_tail.get(position + 3..).unwrap_or_default(),
        ),
        None => ("", without_tail),
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = rest.get(..authority_end).unwrap_or_default();
    let path = rest.get(authority_end..).unwrap_or_default();
    let host = match authority.rfind('@') {
        Some(at) => {
            redacted = true;
            authority.get(at + 1..).unwrap_or_default()
        }
        None => authority,
    };
    (format!("{scheme}{host}{path}"), redacted)
}

/// A declared external agent: a program, and nothing to measure
/// ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
///
/// The argument **count**, not the arguments: a command line is where a token
/// ends up when someone pastes one, and the list does not need the values. The
/// environment likewise travels as names only.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgent {
    pub id: String,
    pub label: String,
    pub command: String,
    pub arg_count: usize,
    pub env_names: Vec<String>,
    /// The preset the declaration was made from, recognized by its package.
    pub preset: Option<&'static str>,
}

impl ExternalAgent {
    pub fn of(config: &ExternalAgentConfig) -> Self {
        Self {
            id: config.id.as_str().to_owned(),
            label: config.label.clone(),
            command: config.command.clone(),
            arg_count: config.args.len(),
            env_names: config.env.iter().map(|(name, _)| name.clone()).collect(),
            preset: preset_of(config).map(|preset| preset.id),
        }
    }
}

/// The preset whose package this declaration runs, if any.
pub fn preset_of(config: &ExternalAgentConfig) -> Option<oxyn_ai::external::presets::AgentPreset> {
    PRESETS.into_iter().find(|preset| {
        config
            .args
            .iter()
            .any(|arg| arg == preset.package || arg.starts_with(&format!("{}@", preset.package)))
    })
}

/// One environment variable of a draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// A ready-made declaration to review, filled from what « Detect » found.
///
/// Sent to the webview **before** anything is saved: it is the user's to read
/// and confirm. It holds no secret — only paths found on the machine.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPresetDraft {
    pub id: &'static str,
    pub label: &'static str,
    pub package: &'static str,
    pub version: &'static str,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
    /// Whether the machine was looked at. `false` for the static list.
    pub detected: bool,
    /// Where the launcher was found, if it was.
    pub launcher: Option<String>,
    /// Where the agent's own program was found, if it was.
    pub agent_program: Option<String>,
    /// The terminal command that signs in with this agent.
    pub sign_in: &'static str,
}

impl AgentPresetDraft {
    pub fn of(draft: PresetDraft, detected: bool) -> Self {
        Self {
            id: draft.preset.id,
            label: draft.preset.label,
            package: draft.preset.package,
            version: draft.preset.version,
            command: draft.command,
            args: draft.args,
            env: draft
                .env
                .into_iter()
                .map(|(name, value)| EnvVar { name, value })
                .collect(),
            detected,
            launcher: draft.launcher,
            agent_program: draft.agent_program,
            sign_in: draft.preset.sign_in,
        }
    }
}

/// What the user filled in to declare or edit a provider.
///
/// **No `Debug` derive**: `key` is an API key in clear, and `base_url` has not
/// been validated yet — it may carry the `user:password` the domain is about to
/// refuse ([I-03](../../../../CLAUDE.md#i-03)).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDraft {
    /// `None` declares a new provider; `Some` edits that declaration.
    #[serde(default)]
    pub id: Option<String>,
    pub kind: AiProviderKind,
    pub label: String,
    /// Empty on an edit keeps the stored endpoint.
    #[serde(default)]
    pub base_url: String,
    pub model: String,
    /// Empty or absent keeps the stored key on an edit, and declares none on a
    /// new provider.
    #[serde(default)]
    pub key: Option<String>,
    /// Removes the stored key. Ignored when a new key is typed.
    #[serde(default)]
    pub clear_key: bool,
}

impl fmt::Debug for ProviderDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderDraft")
            .field("editing", &self.id.is_some())
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("model", &self.model)
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .field("clear_key", &self.clear_key)
            .finish_non_exhaustive()
    }
}

/// What the user filled in to declare an external agent.
///
/// No secret belongs here — the agent carries its own authentication — but the
/// arguments may still hold one someone pasted, so `Debug` counts them.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDraft {
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

impl fmt::Debug for AgentDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentDraft")
            .field("label", &self.label)
            .field("args", &self.args.len())
            .field("env", &self.env.len())
            .finish_non_exhaustive()
    }
}

/// A model a provider says it serves.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    pub id: String,
    pub display_name: String,
    /// `None` means « not declared », never « unlimited ».
    pub context_window: Option<u32>,
    /// The price the provider publishes, per million tokens. Never a value
    /// written in Oxyn (I-12).
    pub cost: Option<ModelCost>,
    /// The reasoning efforts the provider declares for this model, in order.
    /// Empty means « not declared »: no selector, and no effort is sent.
    pub reasoning_efforts: Vec<oxyn_llm::ReasoningEffort>,
}

/// A provider's published price.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCost {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub currency: String,
}

/// Who answers a question: a declared provider, or a declared external agent.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DestinationChoice {
    #[serde(rename_all = "camelCase")]
    Provider {
        id: String,
        /// Overrides the declaration's default model for this question.
        #[serde(default)]
        model: Option<String>,
        /// The reasoning effort for this question. Checked against the
        /// model's declared efforts before anything is sent; `None` sends
        /// none.
        #[serde(default)]
        effort: Option<oxyn_llm::ReasoningEffort>,
    },
    #[serde(rename_all = "camelCase")]
    Agent { id: String },
}

/// A schema-change template to review ([ADR-0025](../../../../docs/adr/0025-proposition-de-changement-de-schema.md)).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaProposal {
    /// Every line commented: nothing runs on a distracted `Run`.
    pub sql: String,
    pub title: String,
    pub origin: String,
}

/// What a schema-change proposal is about.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProposalTarget {
    #[serde(rename_all = "camelCase")]
    Column { name: String },
    #[serde(rename_all = "camelCase")]
    Constraint { name: String },
}

#[cfg(test)]
mod tests {
    use oxyn_core::ProviderId;

    use super::*;

    #[test]
    fn an_effort_crosses_as_the_provider_writes_it_and_nothing_else() {
        let read = |json: &str| serde_json::from_str::<DestinationChoice>(json);
        assert!(matches!(
            read(r#"{"kind":"provider","id":"p","model":null,"effort":"xhigh"}"#),
            Ok(DestinationChoice::Provider {
                effort: Some(oxyn_llm::ReasoningEffort::XHigh),
                ..
            })
        ));
        assert!(matches!(
            read(r#"{"kind":"provider","id":"p"}"#),
            Ok(DestinationChoice::Provider { effort: None, .. })
        ));
        // A word the backend does not know is not rounded to a neighbour.
        assert!(read(r#"{"kind":"provider","id":"p","effort":"turbo"}"#).is_err());

        let json = serde_json::to_string(&ModelChoice {
            id: "m".to_owned(),
            display_name: "M".to_owned(),
            context_window: None,
            cost: None,
            reasoning_efforts: vec![
                oxyn_llm::ReasoningEffort::Low,
                oxyn_llm::ReasoningEffort::XHigh,
            ],
        })
        .expect("serializable");
        assert!(
            json.contains(r#""reasoningEfforts":["low","xhigh"]"#),
            "{json}"
        );
    }

    #[test]
    fn a_declared_provider_sends_no_secret_reference() {
        let config = AiProviderConfig::new(
            ProviderId::for_new_declaration(AiProviderKind::Anthropic),
            AiProviderKind::Anthropic,
            "Work",
            "https://api.anthropic.com",
            "claude-sonnet-5",
        )
        .with_secret_ref("oxyn:llm:anthropic-1234abcd");
        let json = serde_json::to_string(&DeclaredProvider::of(&config, Reach::Remote, 0))
            .expect("serializable");
        assert!(!json.contains("oxyn:llm"), "keyring reference sent: {json}");
        assert!(json.contains(r#""keyConfigured":true"#), "{json}");
    }

    #[test]
    fn an_endpoint_loses_its_query_and_user_information() {
        let (shown, redacted) =
            redacted_endpoint("https://alice:hunter2@generativelanguage.example/v1?key=sk-abc#x");
        assert_eq!(shown, "https://generativelanguage.example/v1");
        assert!(redacted);
        assert!(!shown.contains("hunter2") && !shown.contains("sk-abc"));

        let (plain, redacted) = redacted_endpoint("http://localhost:11434/v1");
        assert_eq!(plain, "http://localhost:11434/v1");
        assert!(!redacted);
    }

    #[test]
    fn an_unresolved_endpoint_is_not_rounded_to_remote() {
        assert_eq!(
            ProviderReach::from(Reach::Unresolved),
            ProviderReach::Unresolved
        );
    }

    #[test]
    fn a_draft_debug_never_shows_the_key_or_the_endpoint() {
        let draft: ProviderDraft = serde_json::from_str(
            r#"{"kind":"openai","label":"Work","baseUrl":"https://bob:pw@api.example.com",
                "model":"gpt","key":"sk-live-secret"}"#,
        )
        .expect("valid draft");
        let rendered = format!("{draft:?}");
        assert!(!rendered.contains("sk-live-secret"), "{rendered}");
        assert!(!rendered.contains("bob:pw"), "{rendered}");
    }

    #[test]
    fn an_agent_sends_its_argument_count_not_its_arguments() {
        let agent = ExternalAgentConfig::new(ProviderId::for_new_agent(), "Claude", "claude")
            .with_args(["--token", "tok-secret"]);
        let json = serde_json::to_string(&ExternalAgent::of(&agent)).expect("serializable");
        assert!(!json.contains("tok-secret"), "{json}");
        assert!(json.contains(r#""argCount":2"#), "{json}");
    }
}
