//! Hrana 3 negotiation, authenticated requests and bounded stream cleanup.

use oxyn_core::{CancelToken, DriverId, OxynError, Result, StatementIntent};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde_json::json;
use url::Url;

use super::{CONNECT_TIMEOUT, Http, read_body};
use crate::{error, wire};

impl Http {
    pub(crate) fn authenticated(&self, method: Method, url: Url) -> Result<RequestBuilder> {
        if !self.trusted_cursor_origin(&url) {
            return Err(OxynError::Config(
                "cursor routing outside the configured database service is refused".into(),
            ));
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", self.spec.token()))
            .map_err(|_| OxynError::Config("the database token cannot be sent".into()))?;
        authorization.set_sensitive(true);
        Ok(self
            .client
            .request(method, url)
            .header(AUTHORIZATION, authorization)
            .header(CONTENT_TYPE, "application/json"))
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.spec
            .endpoint()
            .join(path)
            .map_err(|_| OxynError::Internal("the Hrana endpoint cannot be joined".into()))
    }

    /// The protocol advertises its version with GET, without executing SQL.
    pub(crate) async fn supports_cursor(
        &self,
        driver: &DriverId,
        cancel: &CancelToken,
    ) -> Result<bool> {
        let probe = self.cursor_support.get_or_try_init(|| async {
            let response = self
                .authenticated(Method::GET, self.endpoint("v3")?)?
                .send()
                .await
                .map_err(|error| {
                    error::transport(StatementIntent::Read, error, self.spec.token())
                })?;
            if matches!(
                response.status(),
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ) {
                return Ok(false);
            }
            if response.status().is_success() {
                return Ok(true);
            }
            Err(self.response_error(driver, response).await)
        });
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(OxynError::Cancelled),
            result = tokio::time::timeout(CONNECT_TIMEOUT, probe) => {
                result.map_err(|_| OxynError::Timeout { after: CONNECT_TIMEOUT })?.copied()
            }
        }
    }

    pub(crate) async fn cursor_response(
        &self,
        driver: &DriverId,
        body: Vec<u8>,
    ) -> Result<Response> {
        let response = self
            .authenticated(Method::POST, self.endpoint("v3/cursor")?)?
            .body(body)
            .send()
            .await
            .map_err(|error| error::transport(StatementIntent::Read, error, self.spec.token()))?;
        if response.status() != StatusCode::OK {
            return Err(self.response_error(driver, response).await);
        }
        Ok(response)
    }

    async fn response_error(&self, driver: &DriverId, response: Response) -> OxynError {
        let status = response.status().as_u16();
        let body = read_body(response, error::MAX_MESSAGE_BYTES)
            .await
            .map(|(bytes, _)| bytes)
            .unwrap_or_default();
        let fault = wire::parse_http_fault(&body);
        error::status(
            driver,
            StatementIntent::Read,
            status,
            fault.as_ref(),
            self.spec.token(),
        )
    }

    pub(crate) fn cursor_base(&self, base: Option<&str>) -> Result<Url> {
        let base = match base {
            Some(base) => Url::parse(base)
                .map_err(|_| OxynError::Config("the cursor routing URL is invalid".into()))?,
            None => self.spec.endpoint().clone(),
        };
        if !self.trusted_cursor_origin(&base) {
            return Err(OxynError::Config(
                "cursor routing outside the configured database service is refused".into(),
            ));
        }
        Ok(base)
    }

    /// No arbitrary redirect target receives credentials. Turso Cloud may pin
    /// a stream to another HTTPS node of its own database service; custom Hrana
    /// endpoints stay confined to the exact configured origin (ADR-0078).
    fn trusted_cursor_origin(&self, target: &Url) -> bool {
        if !target.username().is_empty()
            || target.password().is_some()
            || target.query().is_some()
            || target.fragment().is_some()
        {
            return false;
        }
        let source = self.spec.endpoint();
        if source.origin() == target.origin() {
            return true;
        }
        let cloud = |url: &Url| {
            url.scheme() == "https"
                && url.port_or_known_default() == Some(443)
                && url.host_str().is_some_and(|host| {
                    host.strip_suffix(".turso.io")
                        .is_some_and(|label| !label.is_empty())
                })
        };
        cloud(source) && cloud(target)
    }

    /// Called after the cursor response is exhausted or dropped, never in
    /// parallel with that stream's HTTP request. Failure cannot replay SQL.
    pub(crate) async fn close_cursor(
        &self,
        driver: &DriverId,
        base: &Url,
        baton: &str,
    ) -> Result<()> {
        let close = async {
            let endpoint = base.join("v3/pipeline").map_err(|_| {
                OxynError::Internal("the Hrana close endpoint cannot be joined".into())
            })?;
            let body = serde_json::to_vec(&json!({"baton": baton, "requests": [{"type":"close"}]}))
                .map_err(|_| {
                    OxynError::Serialization("the cursor close request cannot be encoded".into())
                })?;
            let response = self
                .authenticated(Method::POST, endpoint)?
                .body(body)
                .send()
                .await
                .map_err(|error| {
                    error::transport(StatementIntent::Read, error, self.spec.token())
                })?;
            if response.status() != StatusCode::OK {
                return Err(self.response_error(driver, response).await);
            }
            let (body, overflow) = read_body(response, error::MAX_MESSAGE_BYTES)
                .await
                .map_err(|error| {
                    error::transport(StatementIntent::Read, error, self.spec.token())
                })?;
            let value: serde_json::Value = serde_json::from_slice(&body)
                .map_err(|_| error::malformed(driver, StatementIntent::Read))?;
            if overflow
                || !value.get("baton").is_some_and(serde_json::Value::is_null)
                || value
                    .get("results")
                    .and_then(|r| r.as_array())
                    .is_none_or(|results| {
                        results.len() != 1
                            || results
                                .first()
                                .and_then(|r| r.get("response"))
                                .and_then(|r| r.get("type"))
                                .and_then(|r| r.as_str())
                                != Some("close")
                    })
            {
                return Err(error::malformed(driver, StatementIntent::Read));
            }
            Ok(())
        };
        tokio::time::timeout(CONNECT_TIMEOUT, close)
            .await
            .map_err(|_| OxynError::Timeout {
                after: CONNECT_TIMEOUT,
            })?
    }
}
