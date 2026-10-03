//! What the domain knows about a model provider: its identity, its protocol
//! family, and the declaration the user made of it.
//!
//! Authority: [ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md),
//! which clarifies [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md).
//!
//! # Why these types live here and not in `oxyn-llm`
//!
//! A [`Command`](crate::Command) carries a provider's identity and its full
//! declaration: it is the bus that registers, lists and removes a provider,
//! as it does a connection ([I-01](../../../CLAUDE.md#i-01)). `oxyn-core`
//! depends on no crate of the workspace, `oxyn-llm` depends on it: this is
//! therefore the only possible placement. It is the exact precedent of
//! [`PrivacyTier`](crate::PrivacyTier), defined next to the
//! [`ConnectionConfig`](crate::ConnectionConfig) that carries it rather than in
//! the AI crate. [`ProviderId`] is re-exported by `oxyn_llm::provider`: it has
//! **a single** definition in the repository.
//!
//! # No key enters here
//!
//! [`AiProviderConfig`] carries a secret *reference*, never a secret — same
//! shape as [`ConnectionConfig::secret_ref`](crate::ConnectionConfig). Its
//! `Debug` is written by hand to mask that reference, and
//! [`AiProviderConfig::validate`] **refuses** a base URL carrying a
//! `user:password` pair: cleaning it silently would give the user back a
//! configuration different from the one they entered, without telling them
//! their key just went through a field not meant for it
//! ([I-03](../../../CLAUDE.md#i-03)).
//!
//! # What is **not** here
//!
//! The local/remote classification (`oxyn_llm::Reach`). It is computed after
//! DNS resolution, on every registration and every runtime opening, and is
//! never persisted: a value in the database would be yesterday's DNS answer
//! applied to today's send (ADR-0023).

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{OxynError, Result};
use crate::ids::IdParseError;

mod provenance;
pub use provenance::{MAX_PROVENANCE_BYTES, Provenance};

mod turn;
pub use turn::{ReasoningBlock, Role, StopReason};

/// Stable identifier of a model provider.
///
/// Same constraints as [`DriverId`](crate::DriverId) and for the same reason:
/// this value ends up in the local state and in a keychain key. Lowercase
/// ASCII, digits, `-` and `_`, first character a letter, 32 characters at
/// most.
///
/// The identifier names a **configuration**, not a protocol: `ollama`,
/// `lm-studio` and `openai` share the same transport implementation, and yet
/// they are three distinct providers for the user — three endpoints, three
/// levels of data egress. It is [`AiProviderKind`] that says the protocol.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProviderId(Arc<str>);

impl ProviderId {
    /// Ollama, locally.
    pub const OLLAMA: &'static str = "ollama";
    /// LM Studio, locally.
    pub const LM_STUDIO: &'static str = "lm-studio";
    /// `llama.cpp` and its HTTP server, locally.
    pub const LLAMA_CPP: &'static str = "llama-cpp";
    /// OpenAI's API.
    pub const OPENAI: &'static str = "openai";
    /// Azure OpenAI Service.
    pub const AZURE_OPENAI: &'static str = "azure-openai";
    /// OpenRouter, multi-provider gateway.
    pub const OPENROUTER: &'static str = "openrouter";
    /// Anthropic's API (own protocol).
    pub const ANTHROPIC: &'static str = "anthropic";
    /// Google's Gemini API (own protocol).
    pub const GEMINI: &'static str = "gemini";
    /// An OpenAI-compatible endpoint that has no other name.
    pub const OPENAI_COMPATIBLE: &'static str = "openai-compatible";

    /// Builds an identifier after validation.
    ///
    /// # Errors
    /// Returns [`IdParseError`] if the string is empty, exceeds 32 characters,
    /// does not start with a lowercase ASCII letter, or contains a character
    /// outside `[a-z0-9_-]`. The faulty value is never repeated in the message.
    pub fn new(name: impl AsRef<str>) -> std::result::Result<Self, IdParseError> {
        let name = name.as_ref();
        if name.is_empty() {
            return Err(IdParseError::new("ProviderId", "the string is empty"));
        }
        if name.len() > 32 {
            return Err(IdParseError::new("ProviderId", "longer than 32 characters"));
        }
        if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(IdParseError::new(
                "ProviderId",
                "must start with an ASCII lowercase letter",
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(IdParseError::new(
                "ProviderId",
                "allowed characters: a-z, 0-9, `-`, `_`",
            ));
        }
        Ok(Self(Arc::from(name)))
    }

    /// Builds an identifier whose validity is guaranteed by this module.
    fn known(name: &'static str) -> Self {
        debug_assert!(Self::new(name).is_ok(), "invalid provider constant");
        Self(Arc::from(name))
    }

    /// Ollama identifier.
    #[must_use]
    pub fn ollama() -> Self {
        Self::known(Self::OLLAMA)
    }

    /// LM Studio identifier.
    #[must_use]
    pub fn lm_studio() -> Self {
        Self::known(Self::LM_STUDIO)
    }

    /// `llama.cpp` identifier.
    #[must_use]
    pub fn llama_cpp() -> Self {
        Self::known(Self::LLAMA_CPP)
    }

    /// OpenAI identifier.
    #[must_use]
    pub fn openai() -> Self {
        Self::known(Self::OPENAI)
    }

    /// Azure OpenAI identifier.
    #[must_use]
    pub fn azure_openai() -> Self {
        Self::known(Self::AZURE_OPENAI)
    }

    /// OpenRouter identifier.
    #[must_use]
    pub fn openrouter() -> Self {
        Self::known(Self::OPENROUTER)
    }

    /// Anthropic identifier.
    #[must_use]
    pub fn anthropic() -> Self {
        Self::known(Self::ANTHROPIC)
    }

    /// Gemini identifier.
    #[must_use]
    pub fn gemini() -> Self {
        Self::known(Self::GEMINI)
    }

    /// Identifier of an OpenAI-compatible endpoint without a proper name.
    #[must_use]
    pub fn openai_compatible() -> Self {
        Self::known(Self::OPENAI_COMPATIBLE)
    }

    /// Borrowed view of the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Mints the identity of a new declaration: `<family>-<8 hex digits>`.
    ///
    /// # Why it is not derived from the label
    ///
    /// The label is a display name: the user chooses it freely, with accents,
    /// spaces, and nothing stops them from calling two declarations "Prod". An
    /// identifier derived from it would turn two distinct declarations into
    /// one — hence a **silent replacement**, keychain key included, at the
    /// moment the user believed they were adding one. An opaque identity makes
    /// that confusion impossible.
    ///
    /// The family stays as a prefix for a single reason: this identifier
    /// becomes an entry name in the system keychain, which the user sees in
    /// Keychain Access. `anthropic-3f2a9b1c` is recognizable there,
    /// `3f2a9b1c` is not.
    ///
    /// The length fits in the 32 characters that [`new`](Self::new) accepts,
    /// longest family included — `openai-compatible` makes 26 with its suffix —
    /// and the first character is a lowercase letter, as required.
    #[must_use]
    pub fn for_new_declaration(kind: AiProviderKind) -> Self {
        // The first 32 bits of a UUID v4: enough to make a collision unlikely
        // among the few declarations of one machine, without claiming a global
        // uniqueness nobody has any use for here.
        let suffix = uuid::Uuid::new_v4().as_u128() >> 96;
        Self(Arc::from(format!("{}-{suffix:08x}", kind.as_str())))
    }

    /// Mints the identity of a new **external agent**: `agent-<8 hex digits>`.
    ///
    /// Counterpart of [`for_new_declaration`](Self::for_new_declaration), and
    /// for the same reasons — an opaque identity rather than one derived from
    /// the label, which the user can give twice identically without meaning to
    /// replace anything.
    ///
    /// The prefix is `agent` and not a protocol family: an external agent has
    /// none, and inventing one for it would make it pass for a provider in
    /// everything that reads this identifier.
    #[must_use]
    pub fn for_new_agent() -> Self {
        let suffix = uuid::Uuid::new_v4().as_u128() >> 96;
        Self(Arc::from(format!("agent-{suffix:08x}")))
    }
}

impl fmt::Debug for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProviderId({:?})", self.as_str())
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for ProviderId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for ProviderId {
    type Err = IdParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for ProviderId {
    type Error = IdParseError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ProviderId> for String {
    fn from(id: ProviderId) -> Self {
        id.as_str().to_owned()
    }
}

/// The protocol family of an endpoint.
///
/// Distinct from [`ProviderId`], which names **a declaration**: three
/// declarations of the same user can share
/// [`OpenAiCompatible`](Self::OpenAiCompatible) and target three different
/// machines. It is this value, and it alone, that says which transport
/// `oxyn-llm` must instantiate.
///
/// `#[non_exhaustive]`: one more protocol family is an ordinary extension, not
/// a break for callers.
///
/// It says **nothing** about local or remote. An OpenAI-compatible endpoint is
/// as much an Ollama on the loopback as a gateway in the cloud, and that is
/// exactly why the classification is made after resolution (ADR-0023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AiProviderKind {
    /// Anthropic's own protocol.
    #[serde(rename = "anthropic")]
    Anthropic,
    /// OpenAI's own protocol, as OpenAI's API serves it.
    #[serde(rename = "openai")]
    OpenAi,
    /// Google Gemini's own protocol.
    #[serde(rename = "gemini")]
    Gemini,
    /// Any endpoint that speaks "OpenAI-compatible": Ollama, LM Studio,
    /// `llama.cpp`, Azure, OpenRouter.
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
}

impl AiProviderKind {
    /// Stable name, the one written in the local state and in a provenance.
    ///
    /// The `match` is exhaustive inside the crate that defines the type:
    /// adding a family breaks here, at compile time, rather than labeling two
    /// protocols the same way in a persisted file.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Gemini => "gemini",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }
}

impl fmt::Display for AiProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AiProviderKind {
    type Err = IdParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            "gemini" => Ok(Self::Gemini),
            "openai_compatible" => Ok(Self::OpenAiCompatible),
            _ => Err(IdParseError::new(
                "AiProviderKind",
                "expected: anthropic, openai, gemini or openai_compatible",
            )),
        }
    }
}

/// Maximum length of the name shown to the user.
pub const MAX_PROVIDER_LABEL_BYTES: usize = 128;

/// Maximum length of a model name.
///
/// Bounded because a model name is copied into a [`Provenance`], whose total
/// budget is [`MAX_PROVENANCE_BYTES`]: without this bound, a valid declaration
/// would produce a provenance impossible to write.
pub const MAX_PROVIDER_MODEL_BYTES: usize = 128;

/// Maximum length of a base URL.
///
/// An endpoint longer than an address bar is an accidental paste, not a
/// configuration.
pub const MAX_PROVIDER_BASE_URL_BYTES: usize = 2048;

/// The declaration a user made of a model provider.
///
/// **Per machine, not per workspace** (ADR-0023): an Ollama listening on the
/// machine serves every workspace, and duplicating it per workspace would
/// create as many places where its configuration can diverge. What stays per
/// connection is the [`PrivacyTier`](crate::PrivacyTier) — a shared provider
/// does not make a shared tier.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProviderConfig {
    /// Identifier of this declaration.
    pub id: ProviderId,
    /// The protocol family to instantiate.
    pub kind: AiProviderKind,
    /// The name the user gives this declaration. It is **this** that is
    /// shown.
    pub label: String,
    /// The endpoint, **stripped of its credentials**: see
    /// [`validate`](Self::validate).
    pub base_url: String,
    /// The default model of this declaration.
    pub model: String,
    /// Reference to the system keychain, `None` for an endpoint without a key.
    /// Never the key itself.
    #[serde(default)]
    pub secret_ref: Option<String>,
    /// Date of the declaration.
    pub created_at: DateTime<Utc>,
    /// Date of the last modification.
    pub updated_at: DateTime<Utc>,
}

impl AiProviderConfig {
    /// Declares a provider, without a secret and dated now.
    #[must_use]
    pub fn new(
        id: ProviderId,
        kind: AiProviderKind,
        label: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            kind,
            label: label.into(),
            base_url: base_url.into(),
            model: model.into(),
            secret_ref: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Attaches a secret reference.
    #[must_use]
    pub fn with_secret_ref(mut self, secret_ref: impl Into<String>) -> Self {
        self.secret_ref = Some(secret_ref.into());
        self
    }

    /// Checks what must be checked **before** the declaration reaches the disk.
    ///
    /// The rule that matters: a base URL carrying a `user:password` pair is
    /// **refused**. Cleaning it silently would write a configuration the user
    /// did not enter and leave their key without an owner — a password pasted
    /// into the "endpoint" field must produce a message, not an invisible
    /// correction (I-03).
    ///
    /// # Errors
    /// [`OxynError::Config`]: credentials in the URL, unreadable URL or URL
    /// without a host, empty name, or a field outside its bound. No message
    /// copies the faulty value.
    pub fn validate(&self) -> Result<()> {
        if self.label.trim().is_empty() || self.label.len() > MAX_PROVIDER_LABEL_BYTES {
            return Err(OxynError::Config(
                "provider label must be nonempty and fit 128 UTF-8 bytes".into(),
            ));
        }
        if self.model.trim().is_empty() || self.model.len() > MAX_PROVIDER_MODEL_BYTES {
            return Err(OxynError::Config(
                "provider model must be nonempty and fit 128 UTF-8 bytes".into(),
            ));
        }
        if self.base_url.len() > MAX_PROVIDER_BASE_URL_BYTES {
            return Err(OxynError::Config(
                "provider base URL exceeds 2048 bytes".into(),
            ));
        }
        if self
            .secret_ref
            .as_ref()
            .is_some_and(|reference| reference.trim().is_empty())
        {
            return Err(OxynError::Config(
                "provider secret reference must not be blank; omit it instead".into(),
            ));
        }
        validate_base_url(&self.base_url)
    }

    /// Whether a key typed for `self` may follow it to `other`'s endpoint.
    ///
    /// True only when both hold: (a) `self.kind == other.kind` — the protocol
    /// family decides which header carries the key (`x-api-key` for
    /// Anthropic, `Authorization` for an OpenAI-compatible transport), so two
    /// declarations of different families never share a key even at the same
    /// URL; (b) both `base_url` parse and resolve to the same access point —
    /// same scheme, same host, same port (default ports included, via
    /// [`Url::port_or_known_default`]), same path with trailing `/` ignored,
    /// same query. The fragment is not compared: it never leaves the process
    /// on the wire.
    ///
    /// An unreadable `base_url` on either side is never equal to anything —
    /// in doubt, the key is forgotten, which only costs a retype
    /// ([I-03](../../../CLAUDE.md#i-03)). No I/O, and no URL is ever quoted in a
    /// message, matching `validate_base_url`: an endpoint that fails to
    /// parse says nothing here about what it contains.
    #[must_use]
    pub fn same_endpoint_as(&self, other: &Self) -> bool {
        if self.kind != other.kind {
            return false;
        }
        let (Ok(mine), Ok(theirs)) = (Url::parse(&self.base_url), Url::parse(&other.base_url))
        else {
            return false;
        };
        mine.scheme() == theirs.scheme()
            && mine.host_str() == theirs.host_str()
            && mine.port_or_known_default() == theirs.port_or_known_default()
            && mine.path().trim_end_matches('/') == theirs.path().trim_end_matches('/')
            && mine.query() == theirs.query()
    }
}

/// Refuses an unusable base URL or one carrying credentials.
///
/// The scheme is not constrained here: what a transport can reach is
/// `oxyn-llm`'s business, and `oxyn-core` knows no transport. A host is
/// required, though — a URL without an authority (`data:`, `mailto:`) is not an
/// endpoint.
fn validate_base_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw)
        .map_err(|_| OxynError::Config("provider base URL is not a valid absolute URL".into()))?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(OxynError::Config(
            "provider base URL must name a host".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        // The message does not quote the URL: it carries precisely what must
        // not be written.
        return Err(OxynError::Config(
            "provider base URL must not carry credentials; \
             keep the key in the system keychain and reference it"
                .into(),
        ));
    }
    Ok(())
}

/// How many arguments an agent command can carry.
///
/// An agent command line has a handful. The bound exists so that a state file
/// written by a third party does not make us build a `Vec` of arbitrary size
/// on opening.
pub const MAX_AGENT_ARGS: usize = 32;

/// How many environment variables an agent declaration can carry.
pub const MAX_AGENT_ENV: usize = 32;

/// Maximum length of an environment variable **value**.
///
/// A product choice, not an external limit: what justifies this bound is that
/// an environment value declared by hand has no reason to be long, and that
/// non-secret values are persisted in the local state. Without a bound, it
/// becomes a convenient place to store anything.
pub const MAX_AGENT_ENV_VALUE_BYTES: usize = 4096;

/// A declared external agent: a program to launch, and nothing more.
///
/// # Why this type is not an [`AiProviderConfig`]
///
/// An external agent has no endpoint or model. It can carry its own
/// authentication or receive explicitly declared secrets at launch.
/// Fitting it into `AiProviderConfig` would produce a structure half of whose
/// fields mean nothing depending on the variant, and the question "does this
/// field count here?" would come up again at every read.
///
/// # What Oxyn will never know about it
///
/// Where its model goes. The agent is an opaque process: it can talk to a
/// local model, to a remote service, or switch between two turns.
///
/// This type therefore carries **no** reach, and exposes nothing to set one.
/// The classification lives in `oxyn-ai`, with the rest of privacy —
/// `oxyn-core` does not know `Reach`, and depending on `oxyn-llm` to get it
/// would reverse the direction of dependencies.
/// No `#[non_exhaustive]`, unlike this crate's public enumerations:
/// `oxyn-store` must be able to **rebuild** a declaration read back from disk,
/// as it already does for [`AiProviderConfig`]. The attribute would prevent it
/// without protecting anything — an added field breaks the rebuilding anyway,
/// and it is better that it happen at compile time.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentConfig {
    /// Identifier of this declaration.
    pub id: ProviderId,
    /// The name the user gives this declaration. It is **this** that is
    /// shown.
    pub label: String,
    /// The program to launch.
    pub command: String,
    /// Its arguments, in order.
    #[serde(default)]
    pub args: Vec<String>,
    /// Non-secret environment values. Token-like names must be moved to
    /// `env_secret_refs` by the host before persistence.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// Environment names and OS keychain references, never secret values.
    /// Resolved only when preparing the child process, never for the webview.
    #[serde(default)]
    pub env_secret_refs: Vec<(String, String)>,
    /// Date of the declaration.
    pub created_at: DateTime<Utc>,
    /// Date of the last modification.
    pub updated_at: DateTime<Utc>,
}

impl ExternalAgentConfig {
    /// Declares an external agent, dated now.
    #[must_use]
    pub fn new(id: ProviderId, label: impl Into<String>, command: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id,
            label: label.into(),
            command: command.into(),
            args: Vec::new(),
            env: Vec::new(),
            env_secret_refs: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Adds the command's arguments.
    #[must_use]
    pub fn with_args<S: Into<String>>(mut self, args: impl IntoIterator<Item = S>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Checks what must be checked **before** the declaration reaches the disk.
    ///
    /// # Errors
    /// [`OxynError::Config`]: empty name or command, control character in the
    /// name, the command, an argument or an environment variable, lists out of
    /// bounds, environment variable without a name. No message copies the
    /// faulty value.
    pub fn validate(&self) -> Result<()> {
        if self.label.trim().is_empty() || self.label.len() > MAX_PROVIDER_LABEL_BYTES {
            return Err(OxynError::Config(
                "agent label must be nonempty and fit 128 UTF-8 bytes".into(),
            ));
        }
        // The label heads the native confirmation dialog, above the command it
        // asks the user to read. A newline or two there pushes the real
        // command out of view — and the declaration can come from any script
        // running in the webview.
        if self.label.chars().any(char::is_control) {
            return Err(OxynError::Config(
                "agent label must not contain control characters".into(),
            ));
        }
        if self.command.trim().is_empty() {
            return Err(OxynError::Config("agent command must be nonempty".into()));
        }
        // A control character in a command or an argument has no legitimate
        // use, and it makes everything that displays the declaration unreadable
        // — the same reason that makes us refuse a catalog level name carrying
        // a control character.
        if self.command.chars().any(char::is_control)
            || self
                .args
                .iter()
                .any(|arg| arg.chars().any(char::is_control))
        {
            return Err(OxynError::Config(
                "agent command and arguments must not contain control characters".into(),
            ));
        }
        if self.args.len() > MAX_AGENT_ARGS {
            return Err(OxynError::Config("agent has too many arguments".into()));
        }
        if self.env.len() > MAX_AGENT_ENV {
            return Err(OxynError::Config(
                "agent has too many environment variables".into(),
            ));
        }
        if self.env.iter().any(|(name, _)| {
            name.trim().is_empty() || name.contains('=') || name.chars().any(char::is_control)
        }) {
            return Err(OxynError::Config(
                "agent environment variable names must be nonempty and free of '=' and control \
                 characters"
                    .into(),
            ));
        }
        // The **values** were the unchecked half: bounded by nothing, and
        // free to carry a NUL or a control character. A value with a NUL is
        // silently truncated by the system call when launching the process —
        // the agent then receives something other than what is displayed, and
        // than what is persisted. The rest of the checks would prevent nothing
        // without this one.
        if self
            .env
            .iter()
            .any(|(_, value)| value.len() > MAX_AGENT_ENV_VALUE_BYTES)
        {
            return Err(OxynError::Config(
                "agent environment variable value is too long".into(),
            ));
        }
        if self
            .env
            .iter()
            .any(|(_, value)| value.chars().any(char::is_control))
        {
            return Err(OxynError::Config(
                "agent environment variable values must not contain control characters".into(),
            ));
        }
        Ok(())
    }
}

/// Token-like names are secret even when an IPC caller marks them otherwise.
#[must_use]
pub fn agent_env_is_secret(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name.ends_with("_API_KEY")
        || name.ends_with("_TOKEN")
        || name.ends_with("_SECRET")
        || name.contains("PASSWORD")
        || matches!(name.as_str(), "API_KEY" | "TOKEN" | "SECRET")
}

impl ExternalAgentConfig {
    /// Validates the complete persisted environment without resolving secrets.
    ///
    /// # Errors
    /// Rejects clear token-like variables, duplicate names and invalid/bounded
    /// environment names or references. No error repeats a value.
    pub fn validate_stored_environment(&self) -> Result<()> {
        if self.env.iter().any(|(name, _)| agent_env_is_secret(name)) {
            return Err(OxynError::Config(
                "agent tokens must use the system keychain".into(),
            ));
        }
        let mut combined = self.clone();
        combined.env.extend(self.env_secret_refs.iter().cloned());
        combined.validate()?;
        let mut names = std::collections::BTreeSet::new();
        if combined
            .env
            .iter()
            .any(|(name, _)| !names.insert(name.to_ascii_uppercase()))
        {
            return Err(OxynError::Config(
                "agent environment names must be unique".into(),
            ));
        }
        if self.env_secret_refs.iter().any(|(_, reference)| {
            !reference
                .strip_prefix("oxyn:agent-env:")
                .is_some_and(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        }) {
            return Err(OxynError::Config("invalid agent secret reference".into()));
        }
        Ok(())
    }
}

/// Manual rendering keeps transient draft values out of logs.
impl fmt::Debug for ExternalAgentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalAgentConfig")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("command", &self.command)
            .field("args", &self.args.len())
            .field("env", &format_args!("{} variables", self.env.len()))
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for AiProviderConfig {
    /// Deliberately partial rendering: the secret reference is not printed. A
    /// derived `Debug` is the most frequent leak because it is invisible in
    /// review (I-03).
    ///
    /// Of the base URL, only the **host** is shown — not the value entered.
    ///
    /// This field used to render the whole URL, on the grounds that
    /// [`validate`](Self::validate) refuses a URL carrying credentials. That
    /// was true of an already validated declaration, and false everywhere
    /// else: `Command::SaveAiProvider` carries the configuration **before** the
    /// executor validates it. A key pasted into the "endpoint" field — the most
    /// ordinary typing mistake there is — therefore lived in memory in a full
    /// `Debug`, and a `tracing::debug!` added six months later was enough to
    /// log it ([I-03](../../../CLAUDE.md#i-03)).
    ///
    /// The host alone keeps the diagnosis — knowing where the requests were
    /// going — without depending on a validation that may not have happened.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let host = url::Url::parse(&self.base_url).ok().map_or_else(
            // An unreadable URL is not shown: what we could not parse is
            // precisely what we do not know the contents of.
            || "<unreadable endpoint>".to_owned(),
            |analysee| match (analysee.host_str(), analysee.port()) {
                (Some(host), Some(port)) => format!("{host}:{port}"),
                (Some(host), None) => host.to_owned(),
                (None, _) => "<no host>".to_owned(),
            },
        );
        f.debug_struct("AiProviderConfig")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("base_url_host", &host)
            .field("model", &self.model)
            .field(
                "secret_ref",
                &self.secret_ref.as_ref().map(|_| "<redacted reference>"),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ollama() -> AiProviderConfig {
        AiProviderConfig::new(
            ProviderId::ollama(),
            AiProviderKind::OpenAiCompatible,
            "Laptop Ollama",
            "http://localhost:11434/v1",
            "llama3.2",
        )
    }

    #[test]
    fn a_url_carrying_credentials_is_refused() {
        // ADR-0023: the `base_url` is stored stripped of its credentials. The
        // refusal is the chosen form — a silent cleanup would give the user
        // back a configuration they did not enter.
        let mut config = ollama();
        config.base_url = "https://alice:p4ssw0rd@api.example.com/v1".to_owned();

        let error = config
            .validate()
            .expect_err("a URL with credentials is not written");
        let message = error.to_string();
        assert!(!message.contains("p4ssw0rd"), "{message}");
        assert!(!message.contains("alice"), "{message}");

        // A user name alone is enough to refuse: it is already half of a
        // pair, and the password field will follow.
        config.base_url = "https://alice@api.example.com/v1".to_owned();
        assert!(config.validate().is_err());
    }

    /// Environment **values** are checked, not only the names.
    ///
    /// The value half was not checked at all. The case that hurts is not
    /// cosmetic: a value containing a NUL is silently truncated by the system
    /// call at launch, so that the agent receives something other than what
    /// the screen shows and what the local state persisted.
    #[test]
    fn a_hostile_environment_value_is_refused() {
        let base = ExternalAgentConfig::new(
            ProviderId::new("claude-code").expect("valid identifier"),
            "Claude Code",
            "claude",
        );
        let with_value = |value: String| {
            let mut agent = base.clone();
            agent.env = vec![("MODE".to_owned(), value)];
            agent
        };
        assert!(
            with_value("acp".to_owned()).validate().is_ok(),
            "an ordinary value stays accepted"
        );

        for (case, value) in [
            ("NUL", "acp\0rest".to_owned()),
            ("line break", "acp\nrest".to_owned()),
            ("too long", "v".repeat(MAX_AGENT_ENV_VALUE_BYTES + 1)),
        ] {
            assert!(
                with_value(value).validate().is_err(),
                "{case} must be refused"
            );
        }
    }

    /// The label and the variable names head the confirmation dialog: a
    /// newline there pushes the command the user must read out of view.
    #[test]
    fn a_control_character_in_the_label_or_a_variable_name_is_refused() {
        let base = ExternalAgentConfig::new(
            ProviderId::new("claude-code").expect("valid identifier"),
            "Claude Code",
            "npx",
        );
        assert!(base.validate().is_ok());

        let mut agent = base.clone();
        agent.label = format!("Claude Code{}", "\n".repeat(40));
        assert!(agent.validate().is_err(), "newlines in the label");

        let mut agent = base.clone();
        agent.label = "Claude\u{1b}[2J".to_owned();
        assert!(agent.validate().is_err(), "escape in the label");

        let mut agent = base;
        agent.env = vec![("MODE\n\n\n".to_owned(), "acp".to_owned())];
        assert!(agent.validate().is_err(), "newlines in a variable name");
    }

    #[test]
    fn a_url_without_credentials_is_accepted() {
        assert!(ollama().validate().is_ok());
        let mut config = ollama();
        config.base_url = "https://api.anthropic.com".to_owned();
        config.kind = AiProviderKind::Anthropic;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn a_url_without_host_or_unreadable_is_refused() {
        for brut in [
            "",
            "not a url",
            "/v1/chat",
            "mailto:someone@example.com",
            "data:text/plain,hello",
        ] {
            let mut config = ollama();
            config.base_url = brut.to_owned();
            assert!(config.validate().is_err(), "wrongly accepted: `{brut}`");
        }
    }

    #[test]
    fn the_bounds_of_showable_fields_hold() {
        let mut config = ollama();
        config.label = "   ".to_owned();
        assert!(
            config.validate().is_err(),
            "an empty name cannot be selected"
        );

        config = ollama();
        config.label = "a".repeat(MAX_PROVIDER_LABEL_BYTES + 1);
        assert!(config.validate().is_err());

        config = ollama();
        config.model = String::new();
        assert!(config.validate().is_err());

        config = ollama();
        // The model bound exists so that the provenance fits in its budget:
        // exceeding it would make a valid declaration unusable at the moment of
        // writing a document.
        config.model = "m".repeat(MAX_PROVIDER_MODEL_BYTES + 1);
        assert!(config.validate().is_err());

        config = ollama();
        config.base_url = format!("http://h/{}", "p".repeat(MAX_PROVIDER_BASE_URL_BYTES));
        assert!(config.validate().is_err());
    }

    #[test]
    fn no_key_goes_through_this_configuration() {
        // I-03: the only path is a reference, and the `Debug` does not render
        // it. The test fails if someone replaces the hand-written `Debug` with
        // a `#[derive(Debug)]`.
        let config = ollama().with_secret_ref("keychain://oxyn/ollama");

        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains("keychain://oxyn/ollama"),
            "reference leaked: {rendered}"
        );
        assert!(rendered.contains("Laptop Ollama"), "{rendered}");
        assert!(rendered.contains("11434"), "the host stays diagnosable");

        // And nothing in the structure can carry the key itself: the only
        // field meant for the keychain is a reference.
        let json = serde_json::to_string(&config).expect("serialization");
        assert!(json.contains("keychain://oxyn/ollama"));
        assert!(!json.contains("api_key"), "{json}");
        assert!(!json.contains("password"), "{json}");
    }

    /// The `Debug` protects a configuration **not yet validated**.
    ///
    /// It is the real case: `Command::SaveAiProvider` carries the
    /// configuration, and `validate` is only called at the other end, in the
    /// executor. In between, a key pasted into the "endpoint" field — the most
    /// common typing mistake — must not be able to reach a log
    /// ([I-03](../../../CLAUDE.md#i-03)).
    #[test]
    fn a_url_carrying_credentials_is_not_rendered_before_validation() {
        let mut config = ollama();
        config.base_url = "https://login:p4ssw0rd@api.example.com/v1".to_owned();

        // The test's premise: this configuration is not validated, and would
        // not be. Without this line, the test would prove the easy case.
        assert!(
            config.validate().is_err(),
            "validation does refuse a URL carrying credentials"
        );

        let rendered = format!("{config:?}");
        assert!(!rendered.contains("p4ssw0rd"), "secret leaked: {rendered}");
        assert!(
            !rendered.contains("login:"),
            "identifier leaked: {rendered}"
        );
        assert!(
            rendered.contains("api.example.com"),
            "the host stays diagnosable: {rendered}"
        );
    }

    #[test]
    fn an_empty_secret_reference_is_refused() {
        // A blank reference is a field left half filled: it designates
        // nothing in the keychain and the failure would occur when the runtime
        // opens, far from the input.
        let config = ollama().with_secret_ref("  ");
        assert!(config.validate().is_err());
    }

    #[test]
    fn the_declaration_round_trips_faithfully() {
        let config = ollama().with_secret_ref("keychain://oxyn/ollama");
        let json = serde_json::to_string(&config).expect("serialization");
        let read_back: AiProviderConfig = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(read_back, config);
    }

    #[test]
    fn protocol_families_have_a_stable_name() {
        // This name is written to the database and into a provenance:
        // changing it would make what was already written unreadable.
        for (kind, name) in [
            (AiProviderKind::Anthropic, "anthropic"),
            (AiProviderKind::OpenAi, "openai"),
            (AiProviderKind::Gemini, "gemini"),
            (AiProviderKind::OpenAiCompatible, "openai_compatible"),
        ] {
            assert_eq!(kind.as_str(), name);
            assert_eq!(name.parse::<AiProviderKind>(), Ok(kind));
            assert_eq!(
                serde_json::to_string(&kind).expect("serialization"),
                format!("\"{name}\"")
            );
        }
        assert!("mistral".parse::<AiProviderKind>().is_err());
        assert!(serde_json::from_str::<AiProviderKind>("\"OpenAI\"").is_err());
    }

    #[test]
    fn the_family_is_not_deduced_from_the_identifier() {
        // Three OpenAI-compatible declarations, three endpoints: it is the
        // family that says the transport, not the name.
        for id in [
            ProviderId::ollama(),
            ProviderId::openrouter(),
            ProviderId::azure_openai(),
        ] {
            let config = AiProviderConfig::new(
                id,
                AiProviderKind::OpenAiCompatible,
                "endpoint",
                "http://127.0.0.1:8080/v1",
                "model",
            );
            assert_eq!(config.kind, AiProviderKind::OpenAiCompatible);
            assert!(config.validate().is_ok());
        }
    }

    #[test]
    fn invalid_identifiers_are_refused() {
        assert!(ProviderId::new("").is_err());
        assert!(ProviderId::new("OpenAI").is_err(), "uppercase");
        assert!(ProviderId::new("1ollama").is_err(), "leading digit");
        assert!(ProviderId::new("open ai").is_err(), "space");
        assert!(ProviderId::new("open.ai").is_err(), "dot");
        assert!(ProviderId::new("a".repeat(33)).is_err(), "too long");
        assert!(ProviderId::new("a").is_ok());
        assert!(ProviderId::new("lm-studio").is_ok());
        assert!(ProviderId::new("openai_v2").is_ok());
    }

    #[test]
    fn the_error_does_not_copy_the_faulty_value() {
        // A malformed provider identifier may be a key pasted into the wrong
        // field (I-03).
        let err = ProviderId::new("sk-proj-THISMUSTNOTLEAK").expect_err("invalid");
        let rendered = err.to_string();
        assert!(!rendered.contains("THISMUSTNOTLEAK"), "{rendered}");
    }

    #[test]
    fn the_constants_are_valid_identifiers() {
        for name in [
            ProviderId::OLLAMA,
            ProviderId::LM_STUDIO,
            ProviderId::LLAMA_CPP,
            ProviderId::OPENAI,
            ProviderId::AZURE_OPENAI,
            ProviderId::OPENROUTER,
            ProviderId::ANTHROPIC,
            ProviderId::GEMINI,
            ProviderId::OPENAI_COMPATIBLE,
        ] {
            assert!(ProviderId::new(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn an_identifier_serializes_as_a_bare_string() {
        let json = serde_json::to_string(&ProviderId::openai()).expect("serialization");
        assert_eq!(json, "\"openai\"");
        let read_back: ProviderId = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(read_back, ProviderId::openai());
        assert!(serde_json::from_str::<ProviderId>("\"OPENAI\"").is_err());
    }

    #[test]
    fn a_new_declaration_gets_a_valid_and_distinct_identity() {
        // The longest family is the one that would overflow if the shape
        // changed: it is the one tested, not the shortest.
        for family in [
            AiProviderKind::OpenAiCompatible,
            AiProviderKind::Anthropic,
            AiProviderKind::OpenAi,
            AiProviderKind::Gemini,
        ] {
            let minted = ProviderId::for_new_declaration(family);
            ProviderId::new(minted.as_str())
                .unwrap_or_else(|error| panic!("identity refused by its own validation: {error}"));
            assert!(
                minted.as_str().starts_with(family.as_str()),
                "the family stays readable in the keychain: {minted:?}"
            );
        }

        // Two declarations of the same family and label remain two
        // declarations. The trap this test closes: an identifier derived from
        // the label would turn the second into a silent replacement of the
        // first, keychain key included.
        let first = ProviderId::for_new_declaration(AiProviderKind::Anthropic);
        let second = ProviderId::for_new_declaration(AiProviderKind::Anthropic);
        assert_ne!(first, second);
    }

    fn config_with(kind: AiProviderKind, base_url: &str) -> AiProviderConfig {
        let mut config = ollama();
        config.kind = kind;
        config.base_url = base_url.to_owned();
        config
    }

    #[test]
    fn same_endpoint_as_is_true_for_a_host_written_in_a_different_case() {
        let mine = config_with(
            AiProviderKind::OpenAiCompatible,
            "HTTPS://API.Example.com/v1",
        );
        let theirs = config_with(
            AiProviderKind::OpenAiCompatible,
            "https://api.example.com/v1",
        );
        assert!(mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_true_for_an_explicit_default_port() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com:443/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        assert!(mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_true_for_a_trailing_slash() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1/");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        assert!(mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_host() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.evil.example/v1");
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_port() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com:8443/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_http_against_https() {
        let mine = config_with(AiProviderKind::OpenAi, "http://api.example.com/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_path() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v2");
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_query() {
        let mine = config_with(
            AiProviderKind::OpenAi,
            "https://api.example.com/v1?region=eu",
        );
        let theirs = config_with(
            AiProviderKind::OpenAi,
            "https://api.example.com/v1?region=us",
        );
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_kind_with_the_same_url() {
        let mine = config_with(AiProviderKind::Anthropic, "https://api.example.com/v1");
        let theirs = config_with(
            AiProviderKind::OpenAiCompatible,
            "https://api.example.com/v1",
        );
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_an_unreadable_url_on_either_side() {
        let readable = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        let unreadable = config_with(AiProviderKind::OpenAi, "not a url");
        assert!(!readable.same_endpoint_as(&unreadable));
        assert!(!unreadable.same_endpoint_as(&readable));
        assert!(!unreadable.same_endpoint_as(&unreadable));
    }
}
