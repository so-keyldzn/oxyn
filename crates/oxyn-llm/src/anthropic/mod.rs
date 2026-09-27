//! The Anthropic provider.
//!
//! # Why not an OpenAI-compatible adapter
//!
//! The `/v1/messages` protocol differs on four points that a thin translation
//! layer cannot make up for:
//!
//! 1. the system instruction is a **top-level field**, not a message;
//! 2. the content of a message is a **list of typed blocks**, not a string;
//! 3. a tool result is a `tool_result` block in a message of role `user`, not
//!    a `tool` role;
//! 4. the SSE stream is **named** (`content_block_delta`, `message_delta`…)
//!    and uses no end sentinel.
//!
//! An adapter that claimed to cover both protocols would be wrong on tool
//! calls, that is precisely where Oxyn needs them
//! ([`ARCHITECTURE` §7.5](../../../docs/ARCHITECTURE.md)).
//!
//! # Every external fact here is dated and sourced
//!
//! Path, headers, API version, event names, field names, stop reason values,
//! status mapping: checked on **2026-09-16** in the official documentation and
//! recorded in [`RESEARCH-NOTES`](../../../docs/RESEARCH-NOTES.md) §
//! "Anthropic provider" (I-12).
//!
//! # What is not done, and why
//!
//! * **No retry.** A transient error is reported as such and the caller
//!   decides (I-13). The documentation describes resuming an interrupted
//!   stream; it asks to send the partial response back to the model, which is
//!   a product decision, not a transport one.
//! * **No global timeout on the request.** A long generation is normal; a
//!   global timeout would kill it in the middle. Only the *connection* is
//!   bounded.
//! * **No server-side tool.** Oxyn offers none: its tools are bus `Command`s
//!   and nothing else (I-01).

mod decode;
mod wire;

use std::fmt;
use std::pin::pin;

use async_trait::async_trait;
use futures::future::{Either, select};
use futures::stream::{BoxStream, StreamExt};
use oxyn_core::{CancelToken, OxynError, Result};
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Client, RequestBuilder, Url};
use serde_json::Value;

use crate::error::LlmError;
use crate::http;
use crate::provider::{self, LlmProvider, ProviderId};
use crate::reach;
use crate::secret::ApiKey;
use crate::stream::{describe_stream_error, events_stream};
use crate::types::{ChatEvent, ChatRequest, ModelInfo};

/// Endpoint of Anthropic's API.
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// Path of the conversation endpoint.
pub const MESSAGES_PATH: &str = "v1/messages";

/// Path of the token counting endpoint.
pub const COUNT_TOKENS_PATH: &str = "v1/messages/count_tokens";

/// Path of the model list endpoint.
pub const MODELS_PATH: &str = "v1/models";

/// Header carrying the key. This protocol does not use `Authorization`.
pub const API_KEY_HEADER: &str = "x-api-key";

/// Value of the `anthropic-version` header.
///
/// The API refuses the versions it does not know. This value is the one the
/// documentation gives as an example on each of its pages, the most recent
/// included: it is a **contract** version, not a release date, and it does
/// not follow model releases.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Maximum number of models requested per page.
///
/// The documentation bounds this parameter to 1000 and defaults it to 20.
/// Twenty is not enough — the catalog has more —, and the maximum value avoids
/// a pagination that would bring nothing here.
const MODELS_PAGE_SIZE: u32 = 1000;

/// Maximum number of pages walked when listing models.
///
/// A hard bound rather than trust in `has_more`: a server that always
/// answered "there are more" would make the call loop forever.
const MAX_MODEL_PAGES: usize = 10;

/// Ceiling of produced tokens, when the caller sets none.
///
/// It is not an external value but an **Oxyn choice**: `/v1/messages` requires
/// `max_tokens`, and something has to be answered. The value is deliberately
/// modest — a cut response reports itself
/// ([`StopReason::is_truncated`](crate::types::StopReason::is_truncated)), an
/// invoice cannot be taken back.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Anthropic provider.
///
/// `Debug` written by hand: the key does not appear in it (I-03).
pub struct AnthropicProvider {
    base_url: Url,
    api_key: ApiKey,
    version: String,
    default_max_tokens: u32,
    client: Client,
}

impl AnthropicProvider {
    /// Builds the provider on the public endpoint.
    ///
    /// # Errors
    /// Unreadable base URL, or HTTP client impossible to build.
    pub fn new(api_key: impl Into<ApiKey>) -> Result<Self> {
        Self::with_base_url(api_key, ANTHROPIC_BASE_URL)
    }

    /// Builds the provider on a chosen endpoint (proxy, corporate gateway).
    ///
    /// # Errors
    /// Unreadable base URL, or HTTP client impossible to build.
    pub fn with_base_url(api_key: impl Into<ApiKey>, base_url: &str) -> Result<Self> {
        let id = ProviderId::anthropic();
        let analysee = Url::parse(base_url).map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot parse the base URL: {err}"),
        })?;
        let client = http::client(&id)?;
        Ok(Self {
            base_url: provider::normalize_base_url(analysee),
            api_key: api_key.into(),
            version: ANTHROPIC_VERSION.to_owned(),
            default_max_tokens: DEFAULT_MAX_TOKENS,
            client,
        })
    }

    /// Sets the value of the `anthropic-version` header.
    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Sets the token ceiling used when the request carries none.
    #[must_use]
    pub const fn with_default_max_tokens(mut self, max_tokens: u32) -> Self {
        self.default_max_tokens = max_tokens;
        self
    }

    /// The headers every request must carry, **apart from** the key.
    ///
    /// Returned separately: the key is only set at sending time, and never
    /// copied elsewhere.
    #[must_use]
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("anthropic-version", self.version.clone()),
            ("content-type", "application/json".to_owned()),
        ]
    }

    /// Builds the body of `POST /v1/messages`.
    ///
    /// # Errors
    /// Request without a model, or without any message apart from the system
    /// instruction — the API refuses them, and failing here avoids a useless
    /// round trip.
    pub fn wire_request(&self, request: &ChatRequest) -> Result<Value> {
        self.build_body(request, true)
    }

    /// Builds a body, streamed or not.
    fn build_body(&self, request: &ChatRequest, stream: bool) -> Result<Value> {
        let invalide = |detail: &str| LlmError::Config {
            provider: ProviderId::anthropic(),
            detail: detail.to_owned(),
        };
        if request.model.trim().is_empty() {
            return Err(invalide("no model requested").into());
        }

        // Refuse rather than send a conversation from which a reasoning block
        // would have disappeared: the signature would no longer hold.
        let corps =
            wire::build_request(request, self.default_max_tokens, stream).map_err(|_| {
                LlmError::Unsupported {
                    provider: ProviderId::anthropic(),
                    capability: "replaying this kind of reasoning block".to_owned(),
                }
            })?;
        let vide = corps
            .get("messages")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty);
        if vide {
            return Err(invalide("no message to send").into());
        }
        Ok(corps)
    }

    /// Prepares an HTTP request: URL, protocol headers, then the key.
    ///
    /// The key is only set here, and the header is marked sensitive: the HTTP
    /// stack will not render it in its traces (I-03). The error message never
    /// copies the faulty value.
    ///
    /// # Errors
    /// Key not representable in an HTTP header — a key pasted from a terminal
    /// often carries a line break.
    fn authorize(
        &self,
        mut builder: RequestBuilder,
    ) -> std::result::Result<RequestBuilder, LlmError> {
        for (nom, valeur) in self.headers() {
            builder = builder.header(nom, valeur);
        }
        if self.api_key.is_blank() {
            return Err(LlmError::MissingApiKey {
                provider: ProviderId::anthropic(),
            });
        }
        let mut cle =
            HeaderValue::from_str(self.api_key.expose()).map_err(|_| LlmError::Config {
                provider: ProviderId::anthropic(),
                detail: "the API key contains a character that is not valid in an HTTP header"
                    .to_owned(),
            })?;
        cle.set_sensitive(true);
        Ok(builder.header(HeaderName::from_static(API_KEY_HEADER), cle))
    }

    /// Prepares the `POST /v1/messages` of a generation.
    ///
    /// # Errors
    /// URL that cannot be assembled, or key unusable in a header.
    fn prepared_request(&self) -> Result<RequestBuilder> {
        Ok(self.authorize(self.client.post(self.messages_url()?))?)
    }

    /// Assembles a relative path onto the base URL.
    fn join(&self, chemin: &str) -> std::result::Result<Url, LlmError> {
        self.base_url.join(chemin).map_err(|err| LlmError::Config {
            provider: ProviderId::anthropic(),
            detail: format!("cannot append path `{chemin}` to the base URL: {err}"),
        })
    }

    /// URL de `/v1/messages`.
    fn messages_url(&self) -> Result<Url> {
        Ok(self.join(MESSAGES_PATH)?)
    }

    /// Classifies a transport error, without ever copying the key.
    fn transport(&self, err: &reqwest::Error) -> LlmError {
        // No response timeout is configured (see `CONNECT_TIMEOUT`).
        LlmError::from_transport(ProviderId::anthropic(), err, None)
    }

    /// Turns a failure response into an error, body scrubbed.
    ///
    /// The status decides the retry; the body only serves display, and it is
    /// truncated and rid of the key by [`LlmError::from_response`].
    async fn failure(&self, response: reqwest::Response, cancel: Option<&CancelToken>) -> LlmError {
        http::failure(
            &ProviderId::anthropic(),
            response,
            Some(&self.api_key),
            cancel,
        )
        .await
    }

    /// Sends a request and returns its response, yielding to cancellation.
    ///
    /// The sending itself must yield: an endpoint that does not answer would
    /// otherwise leave the user in front of a "Cancel" button with no effect.
    async fn send(
        &self,
        builder: RequestBuilder,
        cancel: &CancelToken,
    ) -> Result<reqwest::Response> {
        // `std::pin::pin!` and not `futures::pin_mut!`: the standard library's
        // pinning introduces no `unsafe` block in this crate, where it is
        // refused.
        let envoi = pin!(builder.send());
        let attente = pin!(cancel.cancelled());
        let reponse = match select(attente, envoi).await {
            Either::Left(((), _)) => return Err(OxynError::Cancelled),
            Either::Right((resultat, _)) => resultat.map_err(|err| self.transport(&err))?,
        };
        if !reponse.status().is_success() {
            // Under the same token as the sending: an error body that never
            // ends must not make "Cancel" ineffective.
            return Err(self.failure(reponse, Some(cancel)).await.into());
        }
        Ok(reponse)
    }

    /// Reads a JSON body under a bound, classifying a decoding defect.
    ///
    /// Without a token: the calls that use it receive no cancellation from the
    /// trait. Size and timeout remain bounded.
    async fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        reponse: reqwest::Response,
        subject: &str,
    ) -> Result<T> {
        Ok(http::read_json(&ProviderId::anthropic(), reponse, subject, None).await?)
    }
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &reach::redacted(&self.base_url))
            .field("api_key", &"<present>")
            .field("version", &self.version)
            .field("default_max_tokens", &self.default_max_tokens)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> ProviderId {
        ProviderId::anthropic()
    }

    fn endpoint(&self) -> Option<&Url> {
        Some(&self.base_url)
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        let mut fiches = Vec::new();
        let mut apres: Option<String> = None;

        for _ in 0..MAX_MODEL_PAGES {
            let mut url = self.join(MODELS_PATH)?;
            url.query_pairs_mut()
                .append_pair("limit", &MODELS_PAGE_SIZE.to_string());
            if let Some(curseur) = &apres {
                url.query_pairs_mut().append_pair("after_id", curseur);
            }

            let requete = self.authorize(self.client.get(url))?;
            // No cancellation here: listing models is a short request, and the
            // trait passes no token.
            let reponse = requete.send().await.map_err(|err| self.transport(&err))?;
            if !reponse.status().is_success() {
                return Err(self.failure(reponse, None).await.into());
            }

            let brut: wire::ModelsResponse = self.read_json(reponse, "model list").await?;
            let encore = brut.has_more;
            let dernier = brut.last_id.clone();
            fiches.extend(wire::parse_models(brut));
            // Cumulative bound: each page is bounded, not their sum.
            if fiches.len() > http::MAX_MODELS {
                return Err(http::too_many_models(&ProviderId::anthropic()).into());
            }

            // `last_id` missing while there would be more: stop rather than ask
            // for the same page again forever.
            match (encore, dernier) {
                (true, Some(curseur)) => apres = Some(curseur),
                _ => break,
            }
        }

        Ok(fiches)
    }

    async fn count_tokens(&self, request: &ChatRequest) -> Result<Option<u32>> {
        // `stream` is refused by this endpoint: the body is therefore built
        // without it, and without `max_tokens` which it does not expect either.
        let corps = self.build_body(request, false)?;
        let url = self.join(COUNT_TOKENS_PATH)?;
        let requete = self.authorize(self.client.post(url))?.json(&corps);

        let reponse = requete.send().await.map_err(|err| self.transport(&err))?;
        if !reponse.status().is_success() {
            return Err(self.failure(reponse, None).await.into());
        }
        let brut: wire::CountResponse = self.read_json(reponse, "token count").await?;
        Ok(Some(brut.count()))
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        // The request is built and validated **before** any network call: a
        // malformed request must report itself as such rather than come back
        // as a `400` a few hundred milliseconds later.
        let corps = self.wire_request(&request)?;
        let requete = self.prepared_request()?.json(&corps);
        let reponse = self.send(requete, cancel).await?;

        let octets = reponse
            .bytes_stream()
            .map(|resultat| resultat.map_err(|err| describe_stream_error(&err)));
        Ok(events_stream(
            Box::pin(octets),
            decode::MessageDecoder::new(),
            cancel.clone(),
            Some(self.api_key.clone()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::OxynError;
    use serde_json::json;

    use super::*;
    use crate::reasoning::{ReasoningBlock, ReasoningEffort};
    use crate::types::{ChatMessage, ToolCall, ToolSpec};

    fn fournisseur() -> AnthropicProvider {
        AnthropicProvider::new("sk-ant-test").expect("construction")
    }

    fn corps(requete: &ChatRequest) -> Value {
        fournisseur().wire_request(requete).expect("valid request")
    }

    // ── System instruction, blocks, alternation ────────────────────────────

    #[test]
    fn the_system_instruction_leaves_the_messages() {
        // The point that distinguishes this protocol: `system` is a field, not
        // a message.
        let requete = ChatRequest::new(
            "claude-modele",
            vec![
                ChatMessage::system("tu es un assistant SQL"),
                ChatMessage::user("bonjour"),
            ],
        );
        let corps = corps(&requete);

        assert_eq!(corps["system"][0]["type"], "text");
        assert_eq!(corps["system"][0]["text"], "tu es un assistant SQL");
        assert_eq!(corps["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(corps["messages"][0]["role"], "user");
    }

    #[test]
    fn several_system_instructions_are_concatenated_not_lost() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::system("règle 1"),
                ChatMessage::system("règle 2"),
                ChatMessage::user("go"),
            ],
        );
        assert_eq!(corps(&requete)["system"][0]["text"], "règle 1\n\nrègle 2");
    }

    #[test]
    fn the_content_is_a_list_of_blocks() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("bonjour")]);
        let corps = corps(&requete);
        assert_eq!(corps["messages"][0]["content"][0]["type"], "text");
        assert_eq!(corps["messages"][0]["content"][0]["text"], "bonjour");
    }

    #[test]
    fn a_tool_call_becomes_a_tool_use_block() {
        let appel = ToolCall::new("call_1", "execute", json!({"sql": "SELECT 1"}));
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("compte les lignes"),
                ChatMessage::assistant("").with_tool_calls(vec![appel]),
            ],
        );
        let bloc = corps(&requete)["messages"][1]["content"][0].clone();
        assert_eq!(bloc["type"], "tool_use");
        assert_eq!(bloc["id"], "call_1");
        assert_eq!(bloc["name"], "execute");
        assert_eq!(
            bloc["input"],
            json!({"sql": "SELECT 1"}),
            "the input is an object, not a string: this protocol differs from OpenAI"
        );
    }

    #[test]
    fn a_tool_result_becomes_a_user_block() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("compte"),
                ChatMessage::assistant("").with_tool_calls(vec![ToolCall::new(
                    "call_1",
                    "execute",
                    json!({}),
                )]),
                ChatMessage::tool_result("call_1", "42"),
            ],
        );
        let dernier = corps(&requete)["messages"][2].clone();
        assert_eq!(dernier["role"], "user", "this protocol has no `tool` role");
        assert_eq!(dernier["content"][0]["type"], "tool_result");
        assert_eq!(dernier["content"][0]["tool_use_id"], "call_1");
        assert_eq!(dernier["content"][0]["content"], "42");
    }

    #[test]
    fn two_consecutive_messages_of_the_same_role_are_merged() {
        // The API requires alternation; two successive tool results are the
        // common case when the model asked for several.
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("a"),
                ChatMessage::tool_result("call_1", "r1"),
                ChatMessage::tool_result("call_2", "r2"),
            ],
        );
        let corps = corps(&requete);
        let messages = corps["messages"].as_array().expect("array");
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert_eq!(messages[0]["content"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn tools_use_input_schema() {
        let requete =
            ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![ToolSpec::new(
                "lister",
                "liste les tables",
                json!({"type": "object"}),
            )]);
        let corps = corps(&requete);
        assert_eq!(corps["tools"][0]["name"], "lister");
        assert_eq!(corps["tools"][0]["input_schema"]["type"], "object");
        assert!(
            corps["tools"][0].get("parameters").is_none(),
            "`parameters` est le nom d'OpenAI, pas celui-ci"
        );
    }

    #[test]
    fn the_token_ceiling_is_always_present() {
        // `max_tokens` is mandatory in this protocol.
        let sans = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        assert_eq!(corps(&sans)["max_tokens"], json!(DEFAULT_MAX_TOKENS));

        let avec = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_max_tokens(128);
        assert_eq!(corps(&avec)["max_tokens"], json!(128));
    }

    #[test]
    fn a_request_without_model_or_message_is_refused() {
        let f = fournisseur();
        assert!(
            f.wire_request(&ChatRequest::new("", vec![ChatMessage::user("a")]))
                .is_err()
        );
        assert!(
            f.wire_request(&ChatRequest::new("m", vec![ChatMessage::system("seule")]))
                .is_err(),
            "a system instruction alone does not make a conversation"
        );
    }

    // ── Raisonnement ───────────────────────────────────────────────────────

    #[test]
    fn effort_goes_through_output_config_and_leaves_the_thinking_mode_alone() {
        // Sending a thinking mode that was not requested would make the request
        // fail on models that do not know it.
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_reasoning_effort(ReasoningEffort::XHigh);
        let corps = corps(&requete);
        assert_eq!(corps["output_config"]["effort"], "xhigh");
        assert!(corps.get("thinking").is_none(), "{corps}");
    }

    #[test]
    fn a_thinking_budget_requires_the_explicit_mode() {
        let requete =
            ChatRequest::new("m", vec![ChatMessage::user("a")]).with_reasoning_budget_tokens(8192);
        let corps = corps(&requete);
        assert_eq!(corps["thinking"]["type"], "enabled");
        assert_eq!(corps["thinking"]["budget_tokens"], 8192);
        assert_eq!(
            corps["thinking"]["display"], "summarized",
            "sans cela le raisonnement revient vide"
        );
    }

    #[test]
    fn both_settings_coexist() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_reasoning_effort(ReasoningEffort::Low)
            .with_reasoning_budget_tokens(1024);
        let corps = corps(&requete);
        assert_eq!(corps["output_config"]["effort"], "low");
        assert_eq!(corps["thinking"]["budget_tokens"], 1024);
    }

    #[test]
    fn a_request_without_reasoning_request_carries_no_trace_of_it() {
        let corps = corps(&ChatRequest::new("m", vec![ChatMessage::user("a")]));
        assert!(corps.get("thinking").is_none(), "{corps}");
        assert!(corps.get("output_config").is_none(), "{corps}");
    }

    #[test]
    fn reasoning_blocks_go_back_first_and_intact() {
        // The API checks their signature: reordering them, editing them or
        // losing one makes the request refused.
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("compte"),
                ChatMessage::assistant("voici")
                    .with_reasoning(vec![
                        ReasoningBlock::summarized("je réfléchis", Some("SIG".to_owned())),
                        ReasoningBlock::redacted("CHIFFRE"),
                    ])
                    .with_tool_calls(vec![ToolCall::new("c1", "execute", json!({}))]),
            ],
        );
        let contenu = corps(&requete)["messages"][1]["content"].clone();
        let blocs = contenu.as_array().expect("array");

        assert_eq!(blocs[0]["type"], "thinking");
        assert_eq!(blocs[0]["thinking"], "je réfléchis");
        assert_eq!(blocs[0]["signature"], "SIG");
        assert_eq!(blocs[1]["type"], "redacted_thinking");
        assert_eq!(blocs[1]["data"], "CHIFFRE");
        assert_eq!(blocs[2]["type"], "text", "reasoning comes first");
        assert_eq!(blocs[3]["type"], "tool_use");
    }

    #[test]
    fn a_reasoning_block_without_signature_does_not_carry_the_field() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("a"),
                ChatMessage::assistant("b")
                    .with_reasoning(vec![ReasoningBlock::summarized("t", None)]),
            ],
        );
        let bloc = corps(&requete)["messages"][1]["content"][0].clone();
        assert!(
            bloc.get("signature").is_none(),
            "un champ vide n'est pas une absence : {bloc}"
        );
    }

    // ── Cache de prompt ────────────────────────────────────────────────────

    #[test]
    fn a_marked_message_carries_the_cache_marker() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::system("le schéma complet").cached(),
                ChatMessage::user("et les doublons ?"),
            ],
        );
        let corps = corps(&requete);
        assert_eq!(corps["system"][0]["cache_control"]["type"], "ephemeral");
        assert!(
            corps["messages"][0]["content"][0]
                .get("cache_control")
                .is_none(),
            "the question changes at every turn: marking it would be useless"
        );
    }

    #[test]
    fn the_marker_is_set_on_the_last_block_of_the_message() {
        // A marker closes a prefix; setting it at the head would cache nothing.
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::assistant("texte")
                    .with_tool_calls(vec![ToolCall::new("c1", "execute", json!({}))])
                    .cached(),
                ChatMessage::user("suite"),
            ],
        );
        let contenu = corps(&requete)["messages"][0]["content"].clone();
        let blocs = contenu.as_array().expect("array");
        assert!(blocs[0].get("cache_control").is_none(), "{blocs:?}");
        assert_eq!(blocs[blocs.len() - 1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn marked_tools_carry_the_marker_on_the_last_one() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_tools(vec![
                ToolSpec::new("un", "d1", json!({})),
                ToolSpec::new("deux", "d2", json!({})),
            ])
            .with_cached_tools();
        let corps = corps(&requete);
        assert!(corps["tools"][0].get("cache_control").is_none());
        assert_eq!(corps["tools"][1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn the_number_of_markers_is_bounded() {
        // Beyond the bound, the API refuses the whole request: Oxyn stops
        // before rather than let a `400` reach the user.
        let mut messages = vec![ChatMessage::system("contexte").cached()];
        for numero in 0..10 {
            messages.push(ChatMessage::user(format!("question {numero}")).cached());
            messages.push(ChatMessage::assistant(format!("réponse {numero}")).cached());
        }
        let requete = ChatRequest::new("m", messages)
            .with_tools(vec![ToolSpec::new("t", "d", json!({}))])
            .with_cached_tools();

        let marqueurs = compter_marqueurs(&corps(&requete));
        assert!(
            marqueurs <= wire::MAX_CACHE_BREAKPOINTS,
            "{marqueurs} markers set"
        );
    }

    /// Counts the `cache_control`s present anywhere in the body.
    fn compter_marqueurs(valeur: &Value) -> usize {
        match valeur {
            Value::Object(objet) => {
                let ici = usize::from(objet.contains_key("cache_control"));
                ici + objet.values().map(compter_marqueurs).sum::<usize>()
            }
            Value::Array(items) => items.iter().map(compter_marqueurs).sum(),
            _ => 0,
        }
    }

    // ── Comptage de jetons ─────────────────────────────────────────────────

    #[test]
    fn the_counting_body_carries_neither_stream_nor_ceiling() {
        // The counting endpoint refuses `stream`.
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        let corps = fournisseur()
            .build_body(&requete, false)
            .expect("valid request");
        assert!(corps.get("stream").is_none(), "{corps}");
        assert!(corps.get("max_tokens").is_none(), "{corps}");
        assert_eq!(corps["model"], "m");
    }

    // ── Privacy and headers ────────────────────────────────────────────────

    #[test]
    fn debug_does_not_show_the_key() {
        let rendu = format!("{:?}", AnthropicProvider::new("sk-ant-CECI").expect("ok"));
        assert!(!rendu.contains("CECI"), "{rendu}");
        assert!(rendu.contains("<present>"), "{rendu}");
    }

    #[test]
    fn a_key_with_a_line_break_is_refused_without_being_displayed() {
        // A key pasted from a terminal often carries a `\n`.
        let f = AnthropicProvider::new("sk-ant-avec\nsaut").expect("construction");
        let err = f.prepared_request().expect_err("invalid header");
        assert!(!err.to_string().contains("sk-ant-avec"), "{err}");
    }

    #[test]
    fn a_blank_key_is_refused_locally() {
        // A `401` would suggest an account problem whereas the configuration is
        // simply incomplete.
        let f = AnthropicProvider::new("   ").expect("construction");
        let err = f.prepared_request().expect_err("blank key");
        assert!(matches!(err, OxynError::Authentication(_)), "{err}");
    }

    #[test]
    fn a_prepared_request_builds_with_a_valid_key() {
        assert!(fournisseur().prepared_request().is_ok());
    }

    #[test]
    fn urls_are_assembled() {
        let f = fournisseur();
        assert_eq!(
            f.messages_url().expect("URL").as_str(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            f.join(COUNT_TOKENS_PATH).expect("URL").as_str(),
            "https://api.anthropic.com/v1/messages/count_tokens"
        );
        assert_eq!(
            f.join(MODELS_PATH).expect("URL").as_str(),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn a_base_url_without_final_slash_keeps_its_path() {
        // The `Url::join` trap: without a final `/`, the last segment is
        // replaced — a corporate gateway often serves under a prefix.
        let f = AnthropicProvider::with_base_url("sk-ant-test", "https://passerelle.example/api")
            .expect("construction");
        assert_eq!(
            f.messages_url().expect("URL").as_str(),
            "https://passerelle.example/api/v1/messages"
        );
    }

    #[test]
    fn the_headers_carry_the_version() {
        let f = fournisseur().with_version("2099-01-01");
        let entetes = f.headers();
        assert!(
            entetes
                .iter()
                .any(|(nom, valeur)| *nom == "anthropic-version" && valeur == "2099-01-01"),
            "{entetes:?}"
        );
        assert!(
            entetes.iter().all(|(nom, _)| *nom != API_KEY_HEADER),
            "the key does not go through there: {entetes:?}"
        );
    }

    #[test]
    fn an_unreadable_base_url_is_a_configuration_error() {
        let err = AnthropicProvider::with_base_url("sk-ant-test", "pas une url")
            .expect_err("invalid URL");
        assert!(matches!(err, OxynError::Config(_)), "{err}");
    }

    // ── Annulation avant l'envoi ───────────────────────────────────────────

    #[test]
    fn an_already_cancelled_token_short_circuits_the_call() {
        let f = fournisseur();
        let jeton = CancelToken::new();
        jeton.cancel();
        let requete = ChatRequest::new("claude-modele", vec![ChatMessage::user("bonjour")]);
        let issue = futures::executor::block_on(f.stream(requete, &jeton));
        let err = match issue {
            Ok(_) => panic!("cancelled beforehand"),
            Err(err) => err,
        };
        assert!(err.is_cancelled(), "{err}");
    }

    // ── Failure statuses ───────────────────────────────────────────────────

    #[test]
    fn failure_statuses_carry_their_class() {
        // The class decides a retry, and it is the caller that decides — never
        // this crate (I-13). The values come from the error table checked on
        // 2026-09-16.
        let cle = ApiKey::new("sk-ant-test");
        let cas = [
            (
                401,
                r#"{"type":"error","error":{"type":"authentication_error"}}"#,
                false,
            ),
            (
                413,
                r#"{"type":"error","error":{"type":"request_too_large"}}"#,
                false,
            ),
            (
                429,
                r#"{"type":"error","error":{"type":"rate_limit_error"}}"#,
                true,
            ),
            (
                500,
                r#"{"type":"error","error":{"type":"api_error"}}"#,
                true,
            ),
            (
                529,
                r#"{"type":"error","error":{"type":"overloaded_error"}}"#,
                true,
            ),
        ];
        for (statut, corps, retentable) in cas {
            let err = LlmError::from_response(ProviderId::anthropic(), statut, corps, Some(&cle));
            assert_eq!(err.is_retryable(), retentable, "HTTP {statut} : {err}");
        }
    }

    #[test]
    fn an_authentication_refusal_never_copies_the_key() {
        // This provider sometimes copies the received key into its message.
        let cle = ApiKey::new("sk-ant-api03-TRES-SECRET");
        let err = LlmError::from_response(
            ProviderId::anthropic(),
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key: sk-ant-api03-TRES-SECRET"}}"#,
            Some(&cle),
        );
        let rendu = err.to_string();
        assert!(!rendu.contains("TRES-SECRET"), "{rendu}");
        assert!(rendu.contains(crate::secret::REDACTED), "{rendu}");

        let projetee: OxynError = err.into();
        assert!(matches!(projetee, OxynError::Authentication(_)));
        assert!(!projetee.to_string().contains("TRES-SECRET"));
    }

    // ── Exact shape of the request ─────────────────────────────────────────

    #[test]
    fn a_complete_request_has_the_expected_shape() {
        // A deliberately readable snapshot rather than a file on the side: what
        // matters is that a change of shape shows **in review**, and a JSON
        // value comparison says exactly what moved.
        let requete = ChatRequest::new(
            "claude-modele",
            vec![
                ChatMessage::system("tu es un assistant SQL").cached(),
                ChatMessage::user("compte les clients"),
            ],
        )
        .with_max_tokens(256)
        .with_tools(vec![ToolSpec::new(
            "execute_query",
            "exécute une requête",
            json!({"type": "object", "properties": {"sql": {"type": "string"}}}),
        )])
        .with_cached_tools()
        .with_reasoning_effort(ReasoningEffort::Medium);

        assert_eq!(
            corps(&requete),
            json!({
                "model": "claude-modele",
                "tools": [{
                    "name": "execute_query",
                    "description": "exécute une requête",
                    "input_schema": {
                        "type": "object",
                        "properties": {"sql": {"type": "string"}}
                    },
                    "cache_control": {"type": "ephemeral"}
                }],
                "system": [{
                    "type": "text",
                    "text": "tu es un assistant SQL",
                    "cache_control": {"type": "ephemeral"}
                }],
                "messages": [{
                    "role": "user",
                    "content": [{"type": "text", "text": "compte les clients"}]
                }],
                "stream": true,
                "max_tokens": 256,
                "output_config": {"effort": "medium"}
            })
        );
    }

    #[test]
    fn a_local_connection_cannot_reach_this_provider() {
        // ADR-0006: `Local` promises that nothing leaves the machine. This
        // provider is remote by construction — there is no Anthropic
        // deployment on the loopback —, so the combination is refused. The
        // test resolves no name: it relies on the fact that the tier, itself,
        // does not depend on resolution.
        assert!(
            !oxyn_core::PrivacyTier::Local.allows_remote_provider(),
            "un niveau qui laisserait passer un fournisseur distant ne promettrait plus rien"
        );
        for niveau in [
            oxyn_core::PrivacyTier::Metadata,
            oxyn_core::PrivacyTier::Sampled,
        ] {
            assert!(niveau.allows_remote_provider(), "{niveau:?}");
        }

        // And the default endpoint is not a literal loopback address: nothing
        // here can pass itself off as local.
        let f = fournisseur();
        let point = f.endpoint().expect("this provider has a URL");
        assert_ne!(
            crate::reach::literal_reach(point),
            Some(crate::reach::Reach::Local),
            "{point}"
        );
    }

    #[test]
    fn an_invalid_request_reports_itself_before_any_network_call() {
        let f = fournisseur();
        let requete = ChatRequest::new("  ", vec![ChatMessage::user("bonjour")]);
        let issue = futures::executor::block_on(f.stream(requete, &CancelToken::new()));
        let err = match issue {
            Ok(_) => panic!("empty model"),
            Err(err) => err,
        };
        assert!(matches!(&err, OxynError::Config(_)), "{err}");
    }
}
