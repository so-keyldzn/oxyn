//! The thin, bounded HTTP layer shared by execution and catalog discovery.

use std::time::Duration;

use futures::StreamExt as _;
use oxyn_core::{ConnectionConfig, OxynError, Result, StatementIntent};
use oxyn_driver::{Credentials, ExposeSecret};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Client, Method};
use serde_json::Value;
use url::Url;

use crate::error::{response_error, transport_error};

/// A single response is capped independently from the cursor's batch budget.
pub(crate) const RESPONSE_BYTE_LIMIT: usize = 1 << 20;

/// HTTP state for one session. It intentionally has no `Debug`: its default
/// headers can carry credentials.
#[derive(Clone)]
pub(crate) struct Api {
    base: Url,
    client: Client,
}

/// Percent-encodes a server-provided path segment before Oxyn composes it.
#[must_use]
pub(crate) fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

impl Api {
    pub(crate) fn from_config(
        config: &ConnectionConfig,
        credentials: &Credentials,
    ) -> Result<Self> {
        let raw_url = config
            .params
            .get("url")
            .ok_or_else(|| OxynError::Config("the Elasticsearch URL is required".to_owned()))?;
        let base = Url::parse(raw_url)
            .map_err(|_| OxynError::Config("the Elasticsearch URL is invalid".to_owned()))?;
        if !matches!(base.scheme(), "https" | "http") || base.host().is_none() {
            return Err(OxynError::Config(
                "the Elasticsearch URL must be an HTTP or HTTPS origin".to_owned(),
            ));
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(OxynError::Config(
                "credentials in an Elasticsearch URL are refused; use the secret fields".to_owned(),
            ));
        }
        if base.scheme() == "http" && !matches!(config.environment, oxyn_core::Environment::Local) {
            return Err(OxynError::Config(
                "an HTTP Elasticsearch URL is accepted only for a Local connection".to_owned(),
            ));
        }
        let mut headers = HeaderMap::new();
        match config
            .params
            .get("authentication")
            .map(String::as_str)
            .unwrap_or("none")
        {
            "none" => {}
            "basic" => {
                let username = config
                    .params
                    .get("username")
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| {
                        OxynError::Config("basic authentication requires a username".to_owned())
                    })?;
                let password = credentials.password().ok_or_else(|| {
                    OxynError::Authentication("basic authentication requires a password".to_owned())
                })?;
                let mut value = HeaderValue::from_str(&format!(
                    "Basic {}",
                    base64(&format!("{username}:{}", password.expose_secret()))
                ))
                .map_err(|_| {
                    OxynError::Config("the basic authentication header is invalid".to_owned())
                })?;
                value.set_sensitive(true);
                headers.insert(AUTHORIZATION, value);
            }
            "api-key" | "bearer" => {
                let token = credentials.token().ok_or_else(|| {
                    OxynError::Authentication("token authentication requires a token".to_owned())
                })?;
                let prefix = if config
                    .params
                    .get("authentication")
                    .is_some_and(|kind| kind == "api-key")
                {
                    "ApiKey"
                } else {
                    "Bearer"
                };
                let mut value =
                    HeaderValue::from_str(&format!("{prefix} {}", token.expose_secret())).map_err(
                        |_| OxynError::Config("the authorization header is invalid".to_owned()),
                    )?;
                value.set_sensitive(true);
                headers.insert(AUTHORIZATION, value);
            }
            _ => {
                return Err(OxynError::Config(
                    "the authentication method is invalid".to_owned(),
                ));
            }
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .default_headers(headers)
            .build()
            .map_err(|_| OxynError::Connection("the HTTP client could not start".to_owned()))?;
        Ok(Self { base, client })
    }

    pub(crate) async fn json(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        intent: StatementIntent,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|_| OxynError::Config("the HTTP method is invalid".to_owned()))?;
        let url = self
            .base
            .join(path)
            .map_err(|_| OxynError::Config("the API path is invalid".to_owned()))?;
        if url.origin() != self.base.origin() {
            return Err(OxynError::Config(
                "the API path changes the configured origin".to_owned(),
            ));
        }
        let mut request = self.client.request(method, url);
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let response = request
            .send()
            .await
            .map_err(|_| transport_error(intent, "the HTTP request failed"))?;
        let status = response.status();
        let bytes = read_bounded(response, intent).await?;
        if !status.is_success() {
            let kind = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/error/type")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "HTTP response".to_owned());
            // The server can quote submitted JSON in `reason`; never show it.
            return Err(response_error(
                status.as_u16(),
                kind,
                "the server rejected the request",
                intent,
            ));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| OxynError::Query("the server returned invalid JSON".to_owned()))
    }
}

async fn read_bounded(response: reqwest::Response, intent: StatementIntent) -> Result<Vec<u8>> {
    if let Some(length) = response.content_length()
        && length > RESPONSE_BYTE_LIMIT as u64
    {
        return Err(transport_error(
            intent,
            "the server response exceeds the one-megabyte page limit",
        ));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| transport_error(intent, "the HTTP response ended mid-stream"))?;
        let remaining = RESPONSE_BYTE_LIMIT.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            return Err(transport_error(
                intent,
                "the server response exceeds the one-megabyte page limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn base64(bytes: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = bytes.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        out.push(char::from(TABLE[usize::from(first >> 2)]));
        out.push(char::from(
            TABLE[usize::from((first & 0b11) << 4 | second >> 4)],
        ));
        out.push(if chunk.len() > 1 {
            char::from(TABLE[usize::from((second & 0b1111) << 2 | third >> 6)])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(TABLE[usize::from(third & 0b11_1111)])
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn encodes_basic_auth_without_another_dependency() {
        assert_eq!(base64("user:password"), "dXNlcjpwYXNzd29yZA==");
    }
}
