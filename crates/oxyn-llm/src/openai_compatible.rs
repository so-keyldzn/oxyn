//! One implementation for seven providers.
//!
//! Ollama, LM Studio, `llama.cpp`, OpenAI, Azure OpenAI, OpenRouter and any
//! endpoint that speaks `POST /chat/completions` share this code
//! ([`ARCHITECTURE` §7.5](../../../docs/ARCHITECTURE.md)). What distinguishes
//! them fits in four pieces of data: the base URL, the presence of a key, the
//! way to present it, and the shape of the path. Anthropic and Gemini are
//! **not** here: their protocols differ enough for an adapter to be a lie, and
//! they have their own modules.
//!
//! # What is not done
//!
//! * **No automatic detection.** Nothing here probes `localhost` to see
//!   whether an Ollama is running. A provider exists because the user
//!   configured it (ADR-0006).
//! * **No retry.** A transient error is reported as such and the caller
//!   decides, because only it knows whether the user is still waiting.
//! * **No global timeout on the request.** A long generation is normal; a
//!   global timeout would kill it in the middle. Only the *connection* is
//!   bounded.

mod decode;
mod stream;
mod wire;

use std::fmt;
use std::pin::pin;

use async_trait::async_trait;
use futures::future::{Either, select};
use futures::stream::{BoxStream, StreamExt};
use oxyn_core::{CancelToken, OxynError, Result};
use reqwest::header::{AUTHORIZATION, HeaderName, HeaderValue};
use reqwest::{Client, RequestBuilder, Url};

use crate::error::LlmError;
use crate::http;
use crate::provider::{self, LlmProvider, ProviderId};
use crate::reach;
use crate::secret::ApiKey;
use crate::types::{ChatEvent, ChatRequest, ModelInfo};

/// Ollama's local endpoint.
///
/// TODO(phase 2): port and path to confirm against the project's registry and
/// to date in `RESEARCH-NOTES` (I-12).
pub const OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";

/// LM Studio's local endpoint. Same I-12 caveat as [`OLLAMA_BASE_URL`].
pub const LM_STUDIO_BASE_URL: &str = "http://localhost:1234/v1";

/// Local endpoint of the `llama.cpp` server. Same I-12 caveat.
pub const LLAMA_CPP_BASE_URL: &str = "http://localhost:8080/v1";

/// Endpoint of OpenAI's API. Same I-12 caveat.
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// OpenRouter's endpoint. Same I-12 caveat.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Default API version of Azure OpenAI.
///
/// TODO(phase 2): **unchecked value**. Azure carries the version in the
/// request and refuses the ones it no longer knows. To confirm in Azure's
/// documentation and date in `RESEARCH-NOTES` before any real use (I-12);
/// meanwhile, prefer
/// [`with_azure_api_version`](OpenAiCompatibleProvider::with_azure_api_version).
pub const AZURE_DEFAULT_API_VERSION: &str = "2024-10-21";

/// How the key is presented to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStyle {
    /// `Authorization: Bearer <key>` — OpenAI, OpenRouter, most compatible
    /// endpoints.
    Bearer,
    /// `api-key: <key>` — Azure OpenAI, which does not use `Authorization`.
    ApiKeyHeader,
}

/// Shape of the endpoint's paths.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Route {
    /// `<base>/chat/completions` and `<base>/models`.
    OpenAi,
    /// `<base>/openai/deployments/<deployment>/chat/completions?api-version=…`
    AzureDeployment {
        deployment: String,
        api_version: String,
    },
}

/// A provider speaking OpenAI's `chat/completions` protocol.
///
/// The `Debug` is written by hand: the key does not appear in it, and the base
/// URL is scrubbed of any credentials (I-03).
pub struct OpenAiCompatibleProvider {
    id: ProviderId,
    base_url: Url,
    api_key: Option<ApiKey>,
    auth: AuthStyle,
    route: Route,
    client: Client,
    extra_headers: Vec<(HeaderName, HeaderValue)>,
    /// Does the provider refuse to serve without a key?
    ///
    /// True for remote providers: going out without a key would give a `401`
    /// the user would read as an account problem, whereas the configuration is
    /// simply incomplete.
    requires_key: bool,
    /// Should the usage be requested in the stream?
    ///
    /// `stream_options` is only understood by gateways; local servers ignore
    /// it, but a few strict implementations reject unknown fields. Hence a flag
    /// rather than a systematic sending.
    include_usage: bool,
    /// Does the endpoint understand `reasoning_effort`?
    ///
    /// Same reason as [`include_usage`](Self::include_usage), with a more
    /// visible consequence: a strict local server that receives this field
    /// rejects the **whole** request, and the user sees their assistant break
    /// down with no apparent link to the setting they just changed.
    reasoning_effort: bool,
}

impl OpenAiCompatibleProvider {
    /// Builds a provider on an arbitrary base URL.
    ///
    /// The URL is normalized to end with `/`: without it, [`Url::join`] would
    /// replace the last segment and `…/v1` would become `…/chat/completions`
    /// instead of `…/v1/chat/completions`.
    ///
    /// This unclassified transport performs no DNS measurement. For a
    /// privacy-gated request, use [`crate::build_provider`] with its measured reach.
    ///
    /// # Errors
    /// Unreadable URL, or HTTP client impossible to build.
    pub fn new(id: ProviderId, base_url: &str) -> Result<Self> {
        Ok(Self::new_with_reach(
            id,
            base_url,
            crate::Reach::Unresolved,
        )?)
    }

    pub(crate) fn new_with_reach(
        id: ProviderId,
        base_url: &str,
        reach: crate::Reach,
    ) -> std::result::Result<Self, LlmError> {
        let parsed = Url::parse(base_url).map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot parse the base URL: {err}"),
        })?;
        let client = http::client(&id, &parsed, reach)?;
        Ok(Self {
            base_url: provider::normalize_base_url(parsed),
            api_key: None,
            auth: AuthStyle::Bearer,
            route: Route::OpenAi,
            client,
            extra_headers: Vec::new(),
            requires_key: false,
            include_usage: false,
            reasoning_effort: false,
            id,
        })
    }

    /// Ollama, local, without a key.
    ///
    /// # Errors
    /// See [`new`](Self::new).
    pub fn ollama() -> Result<Self> {
        Self::new(ProviderId::ollama(), OLLAMA_BASE_URL)
    }

    /// LM Studio, local, without a key.
    ///
    /// # Errors
    /// See [`new`](Self::new).
    pub fn lm_studio() -> Result<Self> {
        Self::new(ProviderId::lm_studio(), LM_STUDIO_BASE_URL)
    }

    /// The HTTP server of `llama.cpp`, local, without a key.
    ///
    /// # Errors
    /// See [`new`](Self::new).
    pub fn llama_cpp() -> Result<Self> {
        Self::new(ProviderId::llama_cpp(), LLAMA_CPP_BASE_URL)
    }

    /// OpenAI's API.
    ///
    /// The key is **required**: without it, the call would go out and come
    /// back as a `401`, a message the user would read as an account problem.
    ///
    /// # Errors
    /// See [`new`](Self::new).
    pub fn openai(api_key: impl Into<ApiKey>) -> Result<Self> {
        Ok(Self::new(ProviderId::openai(), OPENAI_BASE_URL)?
            .with_api_key(api_key.into())
            .requiring_api_key()
            .with_usage_reporting(true)
            .supporting_reasoning_effort())
    }

    /// OpenRouter.
    ///
    /// # Errors
    /// See [`new`](Self::new).
    pub fn openrouter(api_key: impl Into<ApiKey>) -> Result<Self> {
        Ok(Self::new(ProviderId::openrouter(), OPENROUTER_BASE_URL)?
            .with_api_key(api_key.into())
            .requiring_api_key()
            .with_usage_reporting(true)
            .supporting_reasoning_effort())
    }

    /// Azure OpenAI Service.
    ///
    /// `endpoint` is the URL of the resource (`https://<name>.openai.azure.com`),
    /// `deployment` the name of the deployment — **not** that of the model.
    ///
    /// The deployment name is validated before being inserted into the path:
    /// it is an identifier received from the user, and Oxyn never concatenates
    /// a received identifier without checking it (I-10). A name containing `/`,
    /// `?` or `#` would rewrite the request.
    ///
    /// # Errors
    /// Unreadable URL, invalid deployment name, or HTTP client impossible to
    /// build.
    pub fn azure(endpoint: &str, deployment: &str, api_key: impl Into<ApiKey>) -> Result<Self> {
        let id = ProviderId::azure_openai();
        let deployment = validate_deployment(&id, deployment)?;
        let mut provider = Self::new(id, endpoint)?
            .with_api_key(api_key.into())
            .requiring_api_key()
            .with_usage_reporting(true)
            .supporting_reasoning_effort();
        provider.auth = AuthStyle::ApiKeyHeader;
        provider.route = Route::AzureDeployment {
            deployment,
            api_version: AZURE_DEFAULT_API_VERSION.to_owned(),
        };
        Ok(provider)
    }

    /// Attaches a key.
    #[must_use]
    pub fn with_api_key(mut self, api_key: ApiKey) -> Self {
        self.api_key = Some(api_key);
        self
    }

    /// Requires a key: its absence becomes a local error, not a `401`.
    #[must_use]
    pub fn requiring_api_key(mut self) -> Self {
        self.requires_key = true;
        self
    }

    /// Chooses the way to present the key.
    #[must_use]
    pub fn with_auth_style(mut self, auth: AuthStyle) -> Self {
        self.auth = auth;
        self
    }

    /// Requests — or not — the usage in the stream.
    #[must_use]
    pub fn with_usage_reporting(mut self, enabled: bool) -> Self {
        self.include_usage = enabled;
        self
    }

    /// Declares that this endpoint understands `reasoning_effort`.
    ///
    /// Only to enable for an endpoint where it is documented. Otherwise the
    /// field is **omitted**: an omission degrades the response, an unknown
    /// field makes the whole request fail.
    #[must_use]
    pub const fn supporting_reasoning_effort(mut self) -> Self {
        self.reasoning_effort = true;
        self
    }

    /// Sets Azure's API version.
    ///
    /// No effect on a provider that is not an Azure deployment.
    #[must_use]
    pub fn with_azure_api_version(mut self, version: impl Into<String>) -> Self {
        if let Route::AzureDeployment { api_version, .. } = &mut self.route {
            *api_version = version.into();
        }
        self
    }

    /// Adds a header sent with every request.
    ///
    /// Used by gateways that ask for one (attribution, project). The value is
    /// marked sensitive: it will not appear in the HTTP stack's traces.
    ///
    /// # Errors
    /// Name or value not representable in an HTTP header. The message does
    /// **not** copy the faulty value, which may be a secret.
    pub fn with_header(mut self, name: &str, value: &str) -> Result<Self> {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| LlmError::Config {
                provider: self.id.clone(),
                detail: format!("`{name}` is not a valid HTTP header name"),
            })?;
        let mut header_value = HeaderValue::from_str(value).map_err(|_| LlmError::Config {
            provider: self.id.clone(),
            detail: format!("the value given for header `{name}` is not valid in an HTTP header"),
        })?;
        header_value.set_sensitive(true);
        self.extra_headers.push((header_name, header_value));
        Ok(self)
    }

    /// Base URL, normalized.
    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// URL de `chat/completions`.
    fn chat_url(&self) -> std::result::Result<Url, LlmError> {
        match &self.route {
            Route::OpenAi => self.join("chat/completions"),
            Route::AzureDeployment {
                deployment,
                api_version,
            } => {
                // `deployment` was validated at construction: it contains
                // neither a path separator nor a query separator (I-10).
                let mut url =
                    self.join(&format!("openai/deployments/{deployment}/chat/completions"))?;
                url.query_pairs_mut()
                    .append_pair("api-version", api_version);
                Ok(url)
            }
        }
    }

    /// URL of the model list.
    fn models_url(&self) -> std::result::Result<Url, LlmError> {
        match &self.route {
            Route::OpenAi => self.join("models"),
            Route::AzureDeployment { api_version, .. } => {
                let mut url = self.join("openai/models")?;
                url.query_pairs_mut()
                    .append_pair("api-version", api_version);
                Ok(url)
            }
        }
    }

    /// Assembles a relative path onto the base URL.
    fn join(&self, path: &str) -> std::result::Result<Url, LlmError> {
        self.base_url.join(path).map_err(|err| LlmError::Config {
            provider: self.id.clone(),
            detail: format!("cannot append path `{path}` to the base URL: {err}"),
        })
    }

    /// Sets the authentication headers and the additional headers.
    fn apply_auth(
        &self,
        mut builder: RequestBuilder,
    ) -> std::result::Result<RequestBuilder, LlmError> {
        for (header_name, value) in &self.extra_headers {
            builder = builder.header(header_name.clone(), value.clone());
        }

        let missing = || LlmError::MissingApiKey {
            provider: self.id.clone(),
        };
        let Some(key) = &self.api_key else {
            if self.requires_key {
                return Err(missing());
            }
            return Ok(builder);
        };
        if key.is_blank() {
            return Err(missing());
        }

        let (header_name, raw) = match self.auth {
            AuthStyle::Bearer => (AUTHORIZATION, format!("Bearer {}", key.expose())),
            AuthStyle::ApiKeyHeader => {
                (HeaderName::from_static("api-key"), key.expose().to_owned())
            }
        };
        let mut value = HeaderValue::from_str(&raw).map_err(|_| LlmError::Config {
            provider: self.id.clone(),
            detail: "the API key contains a character that is not valid in an HTTP header"
                .to_owned(),
        })?;
        // Marked sensitive: the HTTP stack will not render it in its traces.
        value.set_sensitive(true);
        Ok(builder.header(header_name, value))
    }

    /// Refuses a reasoning request this protocol cannot carry.
    ///
    /// Two cases, and the asymmetry is intended:
    ///
    /// * **the thinking budget has no equivalent here**, at any provider of
    ///   this family. It is refused everywhere;
    /// * **effort only exists on the endpoints that document it.** It is
    ///   refused on the others.
    ///
    /// Refusing rather than omitting because silence costs more than failure:
    /// a response produced without the requested setting is billed, and
    /// nothing tells the user they did not get what they asked for. Nothing
    /// regresses for all that — a request that asks for no reasoning never
    /// meets this path.
    fn check_reasoning(&self, request: &ChatRequest) -> std::result::Result<(), LlmError> {
        let refusal = |capability: &str| LlmError::Unsupported {
            provider: self.id.clone(),
            capability: capability.to_owned(),
        };
        if request.reasoning_budget_tokens.is_some() {
            return Err(refusal(
                "a thinking budget in tokens; this protocol has no such setting",
            ));
        }
        if request.reasoning_effort.is_some() && !self.reasoning_effort {
            return Err(refusal("a reasoning effort (`reasoning_effort`)"));
        }
        Ok(())
    }

    /// The model list, failures typed — see
    /// [`AnthropicProvider::fetch_models`](crate::anthropic::AnthropicProvider).
    ///
    /// One request: no endpoint of this family documents pagination of
    /// `GET /models` (RESEARCH-NOTES, "Model listing", 2026-10-04). OpenRouter
    /// paginates only when `offset` or `limit` is sent, and returns the whole
    /// list without them.
    pub(crate) async fn fetch_models(&self) -> std::result::Result<Vec<ModelInfo>, LlmError> {
        let url = self.models_url()?;
        let request = self.apply_auth(self.client.get(url))?;
        let response = request.send().await.map_err(|err| self.transport(&err))?;
        if !response.status().is_success() {
            return Err(self.failure(response, None).await);
        }
        // No token here: the trait does not pass one. The reading stays bounded
        // in size and in time.
        let raw: wire::ModelsResponse =
            http::read_json(&self.id, response, "model list", None).await?;
        Ok(wire::parse_models(raw))
    }

    /// Classifies a transport error, without ever copying the key.
    fn transport(&self, err: &reqwest::Error) -> LlmError {
        // The idle bound also covers the wait for the headers.
        LlmError::from_transport(self.id.clone(), err, Some(crate::http::IDLE_TIMEOUT))
    }

    /// Turns a failure response into an error, body scrubbed and read under a
    /// bound — cancellable when the caller holds a token.
    async fn failure(&self, response: reqwest::Response, cancel: Option<&CancelToken>) -> LlmError {
        http::failure(&self.id, response, self.api_key.as_ref(), cancel).await
    }
}

/// Validates an Azure deployment name before inserting it into a path.
///
/// Accepts letters, digits, `-`, `_` and `.` — the set Azure allows.
/// Everything else is refused rather than escaped: an exotic name is far more
/// likely a typing mistake than a real need, and refusing can be explained.
fn validate_deployment(id: &ProviderId, deployment: &str) -> Result<String> {
    let invalid = |detail: &str| {
        OxynError::from(LlmError::Config {
            provider: id.clone(),
            detail: detail.to_owned(),
        })
    };
    if deployment.is_empty() {
        return Err(invalid("the deployment name is empty"));
    }
    if deployment.len() > 64 {
        return Err(invalid("the deployment name is longer than 64 characters"));
    }
    if !deployment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(invalid(
            "the deployment name accepts only A-Z, a-z, 0-9, `-`, `_` and `.`",
        ));
    }
    Ok(deployment.to_owned())
}

impl fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatibleProvider")
            .field("id", &self.id)
            .field("base_url", &reach::redacted(&self.base_url))
            .field(
                "api_key",
                &if self.api_key.is_some() {
                    "<present>"
                } else {
                    "<absente>"
                },
            )
            .field("auth", &self.auth)
            .field("route", &self.route)
            .field(
                "extra_headers",
                &self
                    .extra_headers
                    .iter()
                    .map(|(header_name, _)| header_name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("include_usage", &self.include_usage)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn endpoint(&self) -> Option<&Url> {
        Some(&self.base_url)
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.fetch_models().await?)
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        if request.model.trim().is_empty() {
            return Err(LlmError::Config {
                provider: self.id.clone(),
                detail: "no model requested".to_owned(),
            }
            .into());
        }

        self.check_reasoning(&request)?;

        let url = self.chat_url()?;
        let body = wire::ChatCompletionRequest::from_request(
            &request,
            self.include_usage,
            self.reasoning_effort,
        );
        let http_request = self.apply_auth(self.client.post(url).json(&body))?;

        // The sending itself must yield to cancellation: an endpoint that does
        // not answer would otherwise leave the user in front of a "Cancel"
        // button with no effect.
        // `std::pin::pin!` and not `futures::pin_mut!`: the standard library's
        // pinning introduces no `unsafe` block in this crate, where it is
        // refused.
        let send = pin!(http_request.send());
        let pending = pin!(cancel.cancelled());
        let response = match select(pending, send).await {
            Either::Left(((), _)) => return Err(OxynError::Cancelled),
            Either::Right((result, _)) => result.map_err(|err| self.transport(&err))?,
        };

        if !response.status().is_success() {
            // The error body is read under the same token as the sending: a
            // failure status followed by a body that never ends must not make
            // "Cancel" ineffective.
            return Err(self.failure(response, Some(cancel)).await.into());
        }

        let bytes = response
            .bytes_stream()
            .map(|result| result.map_err(|err| crate::stream::describe_stream_error(&err)));
        Ok(stream::openai_events(
            Box::pin(bytes),
            cancel.clone(),
            self.api_key.clone(),
        ))
    }
}

#[cfg(test)]
mod tests;
