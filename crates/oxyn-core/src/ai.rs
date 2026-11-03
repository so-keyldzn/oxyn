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
//! `user:password` pair, a query string or a fragment: cleaning it silently
//! would give the user back a configuration different from the one they
//! entered, without telling them their key just went through a field not meant
//! for it ([I-03](../../../CLAUDE.md#i-03)).
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
    /// The endpoint, **free of credentials, query and fragment**:
    /// [`validate`](Self::validate) refuses a value that carries any of them
    /// rather than cleaning it. The store rewrites rows saved before that rule
    /// when it lists them.
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
    /// The rule that matters: a base URL carrying a `user:password` pair, a
    /// query string or a fragment is **refused**. Cleaning it silently would
    /// write a configuration the user did not enter and leave their key without
    /// an owner — a password or an API key pasted into the "endpoint" field
    /// must produce a message, not an invisible correction (I-03). The query
    /// loses nothing by being refused: the transports build every request URL
    /// with a relative join, which discards the base's query, so it was never
    /// sent anywhere — it was only stored, in clear.
    ///
    /// # Errors
    /// [`OxynError::Config`]: credentials, query string or fragment in the URL,
    /// unreadable URL or URL without a host, empty name, or a field outside its
    /// bound. No message copies the faulty value; the one for a query or a
    /// fragment points to the key field and quotes nothing of the URL.
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
    /// The query is still compared although [`validate`](Self::validate) now
    /// refuses one: a **validated** declaration has none, so for it the clause
    /// is vacuous. It earns its keep on the values `validate` has not seen —
    /// the candidate a form submits before it is checked, and a row written by
    /// another binary — where, in doubt, a different query must not inherit a
    /// stored key.
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

/// Refuses an unusable base URL, one carrying credentials, or one carrying a
/// query string or a fragment.
///
/// The scheme is not constrained here: what a transport can reach is
/// `oxyn-llm`'s business, and `oxyn-core` knows no transport. A host is
/// required, though — a URL without an authority (`data:`, `mailto:`) is not an
/// endpoint.
///
/// A query or a fragment is refused rather than stripped, for the reason the
/// credentials are: a silent cleanup stores an endpoint the user did not type,
/// and a key pasted there (`?key=…`, the shape some vendors document) would go
/// on being believed saved while it was thrown away. Refusing costs no working
/// configuration: every request path is built with a relative [`Url::join`],
/// which replaces the base's query and fragment, so a query on the base never
/// reached a server. The presence is tested, not the content — a bare `?` or
/// `#` parses to `Some("")` and is as much a query as `?a=b`.
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
    if url.query().is_some() || url.fragment().is_some() {
        // Same rule for the message: it names neither the query nor any value
        // of it, only the place where a key belongs.
        return Err(OxynError::Config(
            "provider base URL must not carry a query string or a fragment; \
             enter the key in its own field, it is kept in the system keychain"
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

/// Product bound on a command shown in a native declaration confirmation.
pub const MAX_AGENT_COMMAND_BYTES: usize = 4096;

/// Product bound on each argument, independently of the argument count.
pub const MAX_AGENT_ARGUMENT_BYTES: usize = 4096;

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

// A declaration must stay readable wherever it is shown, not just in the host
// dialog. Cf/Zl/Zp ranges were checked against UnicodeData.txt on 2026-10-03
// (RESEARCH-NOTES); also refuse the invisible fillers used by dialog escaping.
fn obscures_agent_declaration(character: char) -> bool {
    character.is_control()
        || matches!(character,
            '\u{00AD}' | '\u{034F}' | '\u{0600}'..='\u{0605}' | '\u{061C}'
            | '\u{06DD}' | '\u{070F}' | '\u{0890}'..='\u{0891}' | '\u{08E2}'
            | '\u{115F}'..='\u{1160}' | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{206F}'
            | '\u{3164}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}'
            | '\u{FFA0}' | '\u{FFF9}'..='\u{FFFB}' | '\u{110BD}' | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}' | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}' | '\u{E0000}'..='\u{E0FFF}'
        )
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
    /// [`OxynError::Config`]: empty name or command, control or invisible character in the
    /// name, the command, an argument or an environment variable, fields or lists out of
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
        if self.label.chars().any(obscures_agent_declaration) {
            return Err(OxynError::Config(
                "agent label must not contain control or invisible characters".into(),
            ));
        }
        if self.command.trim().is_empty() {
            return Err(OxynError::Config("agent command must be nonempty".into()));
        }
        if self.command.len() > MAX_AGENT_COMMAND_BYTES
            || self
                .args
                .iter()
                .any(|arg| arg.len() > MAX_AGENT_ARGUMENT_BYTES)
        {
            return Err(OxynError::Config(
                "agent command and each argument must fit 4096 UTF-8 bytes".into(),
            ));
        }
        // A control character in a command or an argument has no legitimate
        // use, and it makes everything that displays the declaration unreadable
        // — the same reason that makes us refuse a catalog level name carrying
        // a control character.
        if self.command.chars().any(obscures_agent_declaration)
            || self
                .args
                .iter()
                .any(|arg| arg.chars().any(obscures_agent_declaration))
        {
            return Err(OxynError::Config(
                "agent command and arguments must not contain control or invisible characters"
                    .into(),
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
            name.trim().is_empty()
                || name.contains('=')
                || name.chars().any(obscures_agent_declaration)
        }) {
            return Err(OxynError::Config(
                "agent environment variable names must be nonempty and free of '=' and control or invisible \
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
            .any(|(_, value)| value.chars().any(obscures_agent_declaration))
        {
            return Err(OxynError::Config(
                "agent environment variable values must not contain control or invisible characters".into(),
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
        if self.env.len().saturating_add(self.env_secret_refs.len()) > MAX_AGENT_ENV {
            return Err(OxynError::Config(
                "agent has too many environment variables".into(),
            ));
        }
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
mod tests;
