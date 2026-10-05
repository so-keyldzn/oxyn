//! Gemini's model list: `GET /{version}/models`, paginated by `pageToken`.
//!
//! Path, header, page size and fields read on 2026-10-04 (RESEARCH-NOTES,
//! "Model listing"). The list is the provider's own, unfiltered by Oxyn except
//! for models that cannot answer `generateContent` — embeddings, which no
//! conversation can use.

use reqwest::header::HeaderValue;
use serde::Deserialize;
use serde_json::Value;

use super::{API_KEY_HEADER, GeminiProvider};
use crate::error::LlmError;
use crate::http;
use crate::provider::ProviderId;
use crate::types::ModelInfo;

/// Entries asked per page: the documented maximum of `pageSize`.
const PAGE_SIZE: u32 = 1000;

/// Pages walked at most. With [`PAGE_SIZE`], ten pages already exceed
/// [`http::MAX_MODELS`]: the bound exists so that a server that keeps handing
/// out tokens cannot hold the call forever.
const MAX_PAGES: usize = 10;

/// The method a conversation calls.
const GENERATE_CONTENT: &str = "generateContent";

/// The prefix `name` carries: `models/gemini-…`, where the stream path wants
/// the bare id.
const NAME_PREFIX: &str = "models/";

/// One page of `GET /models`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelsPage {
    /// Untyped for the same reason as the other families: one malformed
    /// entry must not hide the others.
    #[serde(default, deserialize_with = "crate::http::bounded_entries")]
    models: Vec<Value>,
    #[serde(default)]
    next_page_token: Option<String>,
}

/// An entry of the list.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireModel {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    input_token_limit: Option<Value>,
    #[serde(default)]
    supported_generation_methods: Option<Vec<String>>,
}

impl GeminiProvider {
    /// The model list, every page walked under a bound, failures typed.
    ///
    /// The key goes in the `x-goog-api-key` header, never in `?key=`: a query
    /// parameter ends up in access logs ([I-03](../../../../CLAUDE.md#i-03)).
    pub(crate) async fn fetch_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let id = ProviderId::gemini();
        let mut key =
            HeaderValue::from_str(self.api_key.expose()).map_err(|_| LlmError::Config {
                provider: id.clone(),
                detail: "the API key contains a character that is not valid in an HTTP header"
                    .to_owned(),
            })?;
        key.set_sensitive(true);
        if self.api_key.is_blank() {
            return Err(LlmError::MissingApiKey { provider: id });
        }

        let mut infos = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let url = self.models_url(token.as_deref())?;
            let response = self
                .client
                .get(url)
                .header(API_KEY_HEADER, key.clone())
                .send()
                .await
                .map_err(|err| {
                    LlmError::from_transport(id.clone(), &err, Some(http::IDLE_TIMEOUT))
                })?;
            if !response.status().is_success() {
                return Err(http::failure(&id, response, Some(&self.api_key), None).await);
            }
            let page: ModelsPage = http::read_json(&id, response, "model list", None).await?;
            infos.extend(parse_models(page.models));
            if infos.len() > http::MAX_MODELS {
                return Err(http::too_many_models(&id));
            }
            // An empty or repeated token ends the walk: asking for the same page
            // again would loop until the bound.
            match page.next_page_token {
                Some(next) if !next.is_empty() && token.as_deref() != Some(next.as_str()) => {
                    token = Some(next);
                }
                _ => break,
            }
        }
        Ok(infos)
    }

    /// `{base}/{version}/models`, resolved like the stream path.
    pub(super) fn models_url(&self, page_token: Option<&str>) -> Result<reqwest::Url, LlmError> {
        let path = format!("{}/models", self.api_version);
        let mut url = self.base_url.join(&path).map_err(|err| LlmError::Config {
            provider: ProviderId::gemini(),
            detail: format!("cannot build the request path: {err}"),
        })?;
        url.query_pairs_mut()
            .append_pair("pageSize", &PAGE_SIZE.to_string());
        if let Some(token) = page_token {
            url.query_pairs_mut().append_pair("pageToken", token);
        }
        Ok(url)
    }
}

/// Parses one page, entry by entry; keeps what can answer a conversation.
fn parse_models(entries: Vec<Value>) -> Vec<ModelInfo> {
    let mut infos = Vec::with_capacity(entries.len());
    let mut ignored = 0_usize;
    for entry in entries {
        let Ok(raw) = serde_json::from_value::<WireModel>(entry) else {
            ignored += 1;
            continue;
        };
        if raw
            .supported_generation_methods
            .as_ref()
            .is_some_and(|methods| !methods.iter().any(|m| m == GENERATE_CONTENT))
        {
            continue;
        }
        let id = raw
            .name
            .strip_prefix(NAME_PREFIX)
            .unwrap_or(&raw.name)
            .to_owned();
        if id.is_empty() {
            ignored += 1;
            continue;
        }
        let mut info = ModelInfo::new(id);
        if let Some(display_name) = raw.display_name {
            info = info.with_display_name(display_name);
        }
        if let Some(window) = raw
            .input_token_limit
            .and_then(|limit| limit.as_u64())
            .and_then(|limit| u32::try_from(limit).ok())
        {
            info = info.with_context_window(window);
        }
        infos.push(info);
    }
    if ignored > 0 {
        tracing::debug!(ignored, "unreadable entries in the Gemini model list");
    }
    infos
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_list_path_follows_the_stream_path() {
        let provider = GeminiProvider::with_base_url("k", "https://proxy.example/gemini")
            .expect("construction");
        let url = provider.models_url(Some("abc")).expect("URL");
        assert_eq!(
            url.as_str(),
            "https://proxy.example/gemini/v1beta/models?pageSize=1000&pageToken=abc"
        );
        let stream = provider.stream_url("gemini-x").expect("URL");
        assert!(
            stream
                .as_str()
                .starts_with("https://proxy.example/gemini/v1beta/models/"),
            "{stream}"
        );
    }

    #[test]
    fn names_lose_their_prefix_and_embeddings_are_left_out() {
        let infos = parse_models(vec![
            json!({
                "name": "models/gemini-pro",
                "displayName": "Gemini Pro",
                "inputTokenLimit": 1_048_576,
                "supportedGenerationMethods": ["generateContent", "countTokens"],
            }),
            json!({
                "name": "models/text-embedding",
                "supportedGenerationMethods": ["embedContent"],
            }),
            json!({ "name": "models/bare" }),
            json!({ "displayName": "no name" }),
            json!({ "name": "models/huge", "inputTokenLimit": 1e20 }),
        ]);
        let ids: Vec<&str> = infos.iter().map(|info| info.id.as_str()).collect();
        assert_eq!(ids, ["gemini-pro", "bare", "huge"]);
        assert_eq!(
            infos.first().and_then(|i| i.context_window),
            Some(1_048_576)
        );
        assert_eq!(
            infos.first().map(|i| i.display_name.as_str()),
            Some("Gemini Pro")
        );
        assert_eq!(infos.get(2).and_then(|i| i.context_window), None);
    }
}
