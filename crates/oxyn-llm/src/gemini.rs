//! The Gemini provider — complete request structure, sending in phase 2.
//!
//! # Why not an OpenAI-compatible adapter
//!
//! Google does publish a compatibility endpoint, but the native protocol
//! differs on points that touch exactly what Oxyn needs:
//!
//! 1. the **model is in the path** of the URL, not in the body;
//! 2. the turns are called `contents`, and the assistant's role is `model`;
//! 3. a tool call is a `functionCall` designated by **name**, without an
//!    identifier: matching a response to its request requires finding the name
//!    again from the previous turn;
//! 4. the tool response must be an **object**, never a bare string.
//!
//! Going through the compatibility layer would lose point 3, that is multiple
//! tool calls — the case that matters
//! ([`ARCHITECTURE` §7.5](../../../docs/ARCHITECTURE.md)).
//!
//! # What this module already does
//!
//! Building the request is written and tested. Sending and decoding the stream
//! explicitly **refuse** ([`LlmError::NotImplemented`]): an unfinished path
//! that panics kills the application the day someone configures this provider
//! ([I-09](../../../CLAUDE.md#i-09)), whereas a refusal is read, displayed and
//! worked around by switching provider.

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxyn_core::{CancelToken, Result};
use reqwest::header::HeaderValue;
use reqwest::{Client, Url};
use serde_json::{Map, Value, json};

use crate::error::LlmError;
use crate::http;
use crate::provider::{self, LlmProvider, ProviderId};
use crate::reach;
use crate::secret::ApiKey;
use crate::types::{ChatEvent, ChatMessage, ChatRequest, ModelInfo, Role};

/// Endpoint of the Gemini API.
///
/// Source: docs/RESEARCH-NOTES.md, "Gemini provider".
pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";

/// API version in the path.
///
/// `v1beta` rather than `v1`: the streamGenerateContent reference only
/// documents the path under `/v1beta/…`, and it remains the default used by
/// the official SDKs. Source: docs/RESEARCH-NOTES.md, "Gemini provider".
pub const GEMINI_API_VERSION: &str = "v1beta";

/// Header carrying the key.
///
/// Source: docs/RESEARCH-NOTES.md, "Gemini provider".
pub const API_KEY_HEADER: &str = "x-goog-api-key";

/// Gemini provider.
///
/// `Debug` written by hand: the key does not appear in it (I-03).
pub struct GeminiProvider {
    base_url: Url,
    api_key: ApiKey,
    api_version: String,
    client: Client,
}

impl GeminiProvider {
    /// Builds the provider on the public endpoint.
    ///
    /// # Errors
    /// Unreadable base URL, or HTTP client impossible to build.
    pub fn new(api_key: impl Into<ApiKey>) -> Result<Self> {
        Self::with_base_url(api_key, GEMINI_BASE_URL)
    }

    /// Builds the provider on a chosen endpoint.
    ///
    /// # Errors
    /// Unreadable base URL, or HTTP client impossible to build.
    pub fn with_base_url(api_key: impl Into<ApiKey>, base_url: &str) -> Result<Self> {
        let id = ProviderId::gemini();
        let analysee = Url::parse(base_url).map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot parse the base URL: {err}"),
        })?;
        let client = http::client(&id)?;
        Ok(Self {
            base_url: provider::normalize_base_url(analysee),
            api_key: api_key.into(),
            api_version: GEMINI_API_VERSION.to_owned(),
            client,
        })
    }

    /// Sets the API version used in the path.
    #[must_use]
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }

    /// Streaming generation URL for a given model.
    ///
    /// The model name is **validated** before entering the path: it is a
    /// received identifier, and Oxyn never concatenates one without checking
    /// it (I-10). A name containing `/` or `?` would rewrite the request.
    ///
    /// # Errors
    /// Model name empty or containing a character outside `[A-Za-z0-9._-]`, or
    /// path that cannot be assembled.
    pub fn stream_url(&self, model: &str) -> Result<Url> {
        let invalide = |detail: &str| LlmError::Config {
            provider: ProviderId::gemini(),
            detail: detail.to_owned(),
        };
        let modele = model.trim();
        if modele.is_empty() {
            return Err(invalide("no model requested").into());
        }
        if !modele
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(
                invalide("the model name accepts only A-Z, a-z, 0-9, `-`, `_` and `.`").into(),
            );
        }

        let chemin = format!("{}/models/{modele}:streamGenerateContent", self.api_version);
        let mut url = self
            .base_url
            .join(&chemin)
            .map_err(|err| invalide(&format!("cannot build the request path: {err}")))?;
        // `alt=sse`: without it, the API returns a whole JSON array rather than
        // a stream — which would amount to waiting for the end before
        // displaying anything, and that is precisely what Oxyn refuses.
        url.query_pairs_mut().append_pair("alt", "sse");
        Ok(url)
    }

    /// Builds the body of the generation request.
    ///
    /// # Errors
    /// Request without any turn apart from the system instruction.
    pub fn wire_request(&self, request: &ChatRequest) -> Result<Value> {
        let contenus = contents(&request.messages);
        if contenus.is_empty() {
            return Err(LlmError::Config {
                provider: ProviderId::gemini(),
                detail: "no message to send".to_owned(),
            }
            .into());
        }

        let mut corps = Map::new();
        corps.insert("contents".to_owned(), Value::Array(contenus));

        if let Some(consigne) = system_instruction(&request.messages) {
            corps.insert(
                "systemInstruction".to_owned(),
                json!({ "parts": [{ "text": consigne }] }),
            );
        }

        if !request.tools.is_empty() {
            let declarations: Vec<Value> = request
                .tools
                .iter()
                .map(|outil| {
                    json!({
                        "name": outil.name,
                        "description": outil.description,
                        "parameters": outil.parameters,
                    })
                })
                .collect();
            corps.insert(
                "tools".to_owned(),
                json!([{ "functionDeclarations": declarations }]),
            );
        }

        let mut config = Map::new();
        if let Some(temperature) = request.temperature {
            config.insert("temperature".to_owned(), json!(temperature));
        }
        if let Some(max_tokens) = request.max_tokens {
            // `maxOutputTokens`, not `max_tokens`: the name differs.
            config.insert("maxOutputTokens".to_owned(), json!(max_tokens));
        }
        if !config.is_empty() {
            corps.insert("generationConfig".to_owned(), Value::Object(config));
        }

        Ok(Value::Object(corps))
    }

    /// Prepares the HTTP request: model URL, then the key.
    ///
    /// The key is only set here — never in the URL, where it would end up in
    /// the provider's access logs — and the header is marked sensitive: the
    /// HTTP stack will not render it in its traces (I-03).
    ///
    /// # Errors
    /// Refused model name, or key not representable in an HTTP header — a key
    /// pasted from a terminal often carries a line break.
    pub(crate) fn prepared_request(&self, model: &str) -> Result<reqwest::RequestBuilder> {
        let url = self.stream_url(model)?;
        let mut cle =
            HeaderValue::from_str(self.api_key.expose()).map_err(|_| LlmError::Config {
                provider: ProviderId::gemini(),
                detail: "the API key contains a character that is not valid in an HTTP header"
                    .to_owned(),
            })?;
        cle.set_sensitive(true);
        Ok(self
            .client
            .post(url)
            .header("content-type", "application/json")
            .header(API_KEY_HEADER, cle))
    }
}

/// Gathers the system instructions.
fn system_instruction(messages: &[ChatMessage]) -> Option<String> {
    let morceaux: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == Role::System && !m.content.is_empty())
        .map(|m| m.content.as_str())
        .collect();
    if morceaux.is_empty() {
        None
    } else {
        Some(morceaux.join("\n\n"))
    }
}

/// Projects the conversation onto the protocol's `contents`.
///
/// The tricky point is the tool response: this protocol designates it by
/// **function name**, whereas the domain only carries a call identifier. The
/// name is therefore found again in the previous assistant turns. Without it,
/// the response would be attached at random as soon as the model asks for two
/// tools.
fn contents(messages: &[ChatMessage]) -> Vec<Value> {
    let mut noms_par_appel: HashMap<&str, &str> = HashMap::new();
    let mut sorties: Vec<(&'static str, Vec<Value>)> = Vec::new();

    for message in messages {
        let (role, parts) = match message.role {
            Role::System => continue,
            Role::User => ("user", vec![json!({ "text": message.content })]),
            Role::Assistant => {
                let mut parts = Vec::new();
                if !message.content.is_empty() {
                    parts.push(json!({ "text": message.content }));
                }
                for appel in &message.tool_calls {
                    noms_par_appel.insert(appel.id.as_str(), appel.name.as_str());
                    parts.push(json!({
                        "functionCall": { "name": appel.name, "args": appel.arguments }
                    }));
                }
                if parts.is_empty() {
                    continue;
                }
                ("model", parts)
            }
            Role::Tool => {
                let identifiant = message.tool_call_id.as_deref().unwrap_or_default();
                let nom = noms_par_appel
                    .get(identifiant)
                    .copied()
                    .unwrap_or(identifiant);
                (
                    "user",
                    vec![json!({
                        "functionResponse": {
                            "name": nom,
                            // The response must be an object: a bare string
                            // is refused by the API.
                            "response": { "result": message.content },
                        }
                    })],
                )
            }
        };

        // `last()` then `last_mut()` in two steps: a `match` on `last_mut()`
        // would keep the mutable borrow alive in the arm that pushes, and the
        // borrow checker refuses it.
        if sorties
            .last()
            .is_some_and(|(precedent, _)| *precedent == role)
        {
            if let Some((_, accumulees)) = sorties.last_mut() {
                accumulees.extend(parts);
            }
        } else {
            sorties.push((role, parts));
        }
    }

    sorties
        .into_iter()
        .map(|(role, parts)| json!({ "role": role, "parts": parts }))
        .collect()
}

impl fmt::Debug for GeminiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiProvider")
            .field("base_url", &reach::redacted(&self.base_url))
            .field("api_key", &"<present>")
            .field("api_version", &self.api_version)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::gemini()
    }

    fn endpoint(&self) -> Option<&Url> {
        Some(&self.base_url)
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        // Neither the path nor the schema of the list is checked against the
        // registry (I-12): a list written from memory would be plausible and wrong.
        Err(LlmError::NotImplemented {
            provider: ProviderId::gemini(),
            operation: "listing models (endpoint path and response shape still unverified)"
                .to_owned(),
        }
        .into())
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>> {
        // The request is built and validated before the refusal: a malformed
        // request must report itself as such rather than be hidden by the
        // absence of sending.
        let corps = self.wire_request(&request)?;
        let _requete = self.prepared_request(&request.model)?.json(&corps);
        // What is missing is sending and decoding Gemini's SSE frames
        // (`candidates[].content.parts`, `functionCall`, `usageMetadata`,
        // `finishReason`).
        Err(LlmError::NotImplemented {
            provider: ProviderId::gemini(),
            operation: "streaming a generation (streamGenerateContent)".to_owned(),
        }
        .into())
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::OxynError;

    use super::*;
    use crate::types::{ToolCall, ToolSpec};

    fn fournisseur() -> GeminiProvider {
        GeminiProvider::new("clé-de-test").expect("construction")
    }

    #[test]
    fn an_unimplemented_exchange_refuses_instead_of_panicking() {
        // I-09: a panic kills the application. A provider that is configured
        // but whose protocol is not written must give a message.
        let f = fournisseur();
        let liste = futures::executor::block_on(f.models()).expect_err("not implemented yet");
        assert!(matches!(&liste, OxynError::NotSupported { .. }), "{liste}");
        assert!(!liste.is_retryable(), "retrying will not write the code");

        let requete = ChatRequest::new("gemini-2.0-flash", vec![ChatMessage::user("bonjour")]);
        let flux = futures::executor::block_on(f.stream(requete, &CancelToken::new()))
            .err()
            .expect("not implemented yet");
        let rendu = flux.to_string();
        assert!(rendu.contains("gemini"), "{rendu}");
        assert!(
            rendu.contains("Oxyn does not implement"),
            "the message must point at Oxyn, not the provider: {rendu}"
        );
    }

    #[test]
    fn an_invalid_request_reports_itself_before_the_refusal() {
        // The order matters: otherwise the refusal would hide a defect of the
        // request, and the validation would stop being tested until phase 2.
        let f = fournisseur();
        let requete = ChatRequest::new("  ", vec![ChatMessage::user("bonjour")]);
        let err = futures::executor::block_on(f.stream(requete, &CancelToken::new()))
            .err()
            .expect("empty model");
        assert!(matches!(&err, OxynError::Config(_)), "{err}");
    }

    #[test]
    fn the_model_is_in_the_path_with_streaming_requested() {
        let url = fournisseur()
            .stream_url("gemini-2.0-flash")
            .expect("valid URL");
        assert_eq!(
            url.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn a_model_name_that_would_rewrite_the_request_is_refused() {
        // I-10: a received identifier is not concatenated without checking.
        let f = fournisseur();
        for tordu in ["", "../autre", "modele?key=vole", "modele#x", "mo dele"] {
            assert!(
                f.stream_url(tordu).is_err(),
                "`{tordu}` should have been refused"
            );
        }
    }

    #[test]
    fn the_system_instruction_is_a_separate_field() {
        let requete = ChatRequest::new(
            "gemini-2.0-flash",
            vec![
                ChatMessage::system("tu es un assistant SQL"),
                ChatMessage::user("bonjour"),
            ],
        );
        let corps = fournisseur().wire_request(&requete).expect("valid request");
        assert_eq!(
            corps["systemInstruction"]["parts"][0]["text"],
            "tu es un assistant SQL"
        );
        assert_eq!(corps["contents"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn the_assistant_role_is_called_model() {
        let requete = ChatRequest::new(
            "m",
            vec![ChatMessage::user("a"), ChatMessage::assistant("b")],
        );
        let corps = fournisseur().wire_request(&requete).expect("valid request");
        assert_eq!(corps["contents"][0]["role"], "user");
        assert_eq!(corps["contents"][1]["role"], "model");
    }

    #[test]
    fn a_tool_response_finds_the_function_name_again() {
        // The point that rules out the adapter: this protocol designates by name.
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("compte"),
                ChatMessage::assistant("").with_tool_calls(vec![ToolCall::new(
                    "call_1",
                    "execute_query",
                    json!({"sql": "SELECT 1"}),
                )]),
                ChatMessage::tool_result("call_1", "42"),
            ],
        );
        let corps = fournisseur().wire_request(&requete).expect("valid request");
        let reponse = &corps["contents"][2]["parts"][0]["functionResponse"];
        assert_eq!(reponse["name"], "execute_query");
        assert_eq!(reponse["response"]["result"], "42");
    }

    #[test]
    fn two_tool_responses_do_not_mix() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("fais les deux"),
                ChatMessage::assistant("").with_tool_calls(vec![
                    ToolCall::new("c1", "lire", json!({})),
                    ToolCall::new("c2", "compter", json!({})),
                ]),
                ChatMessage::tool_result("c2", "deux"),
                ChatMessage::tool_result("c1", "un"),
            ],
        );
        let corps = fournisseur().wire_request(&requete).expect("valid request");
        let parts = corps["contents"][2]["parts"]
            .as_array()
            .expect("both responses are merged into one turn");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["functionResponse"]["name"], "compter");
        assert_eq!(parts[1]["functionResponse"]["name"], "lire");
    }

    #[test]
    fn a_tool_call_becomes_a_function_call() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("a"),
                ChatMessage::assistant("je regarde").with_tool_calls(vec![ToolCall::new(
                    "c1",
                    "lister",
                    json!({"schema": "public"}),
                )]),
            ],
        );
        let corps = fournisseur().wire_request(&requete).expect("valid request");
        let parts = corps["contents"][1]["parts"].as_array().expect("parts");
        assert_eq!(parts[0]["text"], "je regarde");
        assert_eq!(parts[1]["functionCall"]["name"], "lister");
        assert_eq!(parts[1]["functionCall"]["args"]["schema"], "public");
    }

    #[test]
    fn tools_are_grouped_into_declarations() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![
            ToolSpec::new("lister", "liste", json!({"type": "object"})),
            ToolSpec::new("compter", "compte", json!({"type": "object"})),
        ]);
        let corps = fournisseur().wire_request(&requete).expect("valid request");
        let declarations = corps["tools"][0]["functionDeclarations"]
            .as_array()
            .expect("a single group of declarations");
        assert_eq!(declarations.len(), 2);
        assert_eq!(declarations[0]["name"], "lister");
    }

    #[test]
    fn the_token_ceiling_carries_the_protocol_name() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_max_tokens(256);
        let corps = fournisseur().wire_request(&requete).expect("valid request");
        assert_eq!(corps["generationConfig"]["maxOutputTokens"], json!(256));
        assert!(corps["generationConfig"].get("max_tokens").is_none());
    }

    #[test]
    fn a_request_without_a_useful_turn_is_refused() {
        let requete = ChatRequest::new("m", vec![ChatMessage::system("seule")]);
        assert!(fournisseur().wire_request(&requete).is_err());
    }

    #[test]
    fn debug_does_not_show_the_key() {
        let rendu = format!(
            "{:?}",
            GeminiProvider::new("CECINEDOITPASFUIR").expect("ok")
        );
        assert!(!rendu.contains("CECINEDOITPASFUIR"), "{rendu}");
    }

    #[test]
    fn a_key_with_a_line_break_is_refused_without_being_displayed() {
        // A key pasted from a terminal often carries a `\n`.
        let f = GeminiProvider::new("cle-avec\nsaut").expect("construction");
        let err = f
            .prepared_request("gemini-2.0-flash")
            .expect_err("invalid header");
        assert!(!err.to_string().contains("cle-avec"), "{err}");
    }

    #[test]
    fn a_prepared_request_does_not_put_the_key_in_the_url() {
        // A key as a query parameter ends up in the access logs.
        let f = fournisseur();
        let url = f.stream_url("gemini-2.0-flash").expect("valid URL");
        assert!(!url.as_str().contains("cle-de-test"), "{url}");
        assert!(f.prepared_request("gemini-2.0-flash").is_ok());
    }
}
