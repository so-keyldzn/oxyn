//! The connection parameters, never displayable with their secret.
//!
//! [`ConnectSpec`] holds the `Authorization` header and has **no derived
//! `Debug`**: the credentials leave it only as that header, marked sensitive,
//! on a request built by [`crate::http`] — never in the URL, which proxies
//! and server logs keep ([I-03](../../../CLAUDE.md#i-03)).
//!
//! # TLS is required, verified, outside a local cluster
//!
//! [ADR-0052](../../../docs/adr/0052-verified-tls-outside-local.md):
//! `verify-full` — HTTPS, certificate chain and host name verified against the
//! system's trust store — is the only mode accepted on a connection whose
//! environment is not [`Environment::Local`]. A connection with no environment
//! counts as production ([I-02](../../../CLAUDE.md#i-02)). The URL's scheme
//! says whether TLS is used at all: `http://` is the `disable` mode, and is
//! refused outside `Local` like an unverified `https://`.

use oxyn_core::{ConnectionConfig, Environment, OxynError, Result};
use oxyn_driver::{Credentials, DriverMetadata, ExposeSecret as _};
use reqwest::header::HeaderValue;
use url::Url;

/// How the server's certificate is checked on an `https://` URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TlsMode {
    /// Chain and host name verified. The default, and the only mode outside
    /// [`Environment::Local`].
    VerifyFull,
    /// Encrypted, certificate not verified: the server is not authenticated.
    /// The escape hatch of a disposable local cluster with its generated
    /// certificate.
    Require,
}

impl TlsMode {
    /// Every mode, in the order the form offers them.
    pub(crate) const ALL: [Self; 2] = [Self::VerifyFull, Self::Require];

    /// The mode's name, as the form spells it.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::VerifyFull => "verify-full",
            Self::Require => "require",
        }
    }
}

/// How requests authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthMethod {
    /// No `Authorization` header: a cluster without security, or a proxy that
    /// authenticates on its own.
    None,
    /// `Basic`, from the username and the password.
    Basic,
    /// `ApiKey`, the encoded key Elasticsearch and OpenSearch return.
    ApiKey,
    /// `Bearer`, a token from the cluster's token service or an identity
    /// provider.
    Bearer,
}

impl AuthMethod {
    /// Every method, in the order the form offers them.
    pub(crate) const ALL: [Self; 4] = [Self::None, Self::Basic, Self::ApiKey, Self::Bearer];

    /// The method's name, as the form spells it.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Basic => "basic",
            Self::ApiKey => "api-key",
            Self::Bearer => "bearer",
        }
    }
}

/// The parameters of a connection, credentials included, not displayable.
pub(crate) struct ConnectSpec {
    /// The origin every request goes to: scheme, host, port, and a path
    /// prefix when the cluster sits behind a reverse proxy.
    pub(crate) base: Url,
    pub(crate) tls: Option<TlsMode>,
    /// The `Authorization` header, already marked sensitive.
    pub(crate) authorization: Option<HeaderValue>,
}

impl ConnectSpec {
    /// Builds the parameters from a configuration and resolved credentials.
    ///
    /// The recognized keys are `url`, `sslmode`, `authentication` and
    /// `username`. **Any other key is refused**: an option the driver does
    /// not apply would otherwise be silently ignored while the user believes
    /// it holds.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the URL is missing, not an HTTP origin, or
    /// carries credentials, a query or a fragment; if a key is unknown; if
    /// plain HTTP or an unverified certificate is asked for outside
    /// [`Environment::Local`]; [`OxynError::Authentication`] if the method's
    /// secret is missing.
    pub(crate) fn from_config(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
        credentials: &Credentials,
    ) -> Result<Self> {
        metadata.validate(config)?;
        let mut url = None;
        let mut tls = TlsMode::VerifyFull;
        let mut method = AuthMethod::None;
        let mut username = None;
        for (key, value) in &config.params {
            let value = value.trim();
            match key.as_str() {
                "url" => url = Some(value),
                "sslmode" => {
                    tls = TlsMode::ALL
                        .into_iter()
                        .find(|mode| mode.name().eq_ignore_ascii_case(value))
                        .ok_or_else(|| {
                            OxynError::Config(
                                "parameter `sslmode` expects verify-full or require".to_owned(),
                            )
                        })?;
                }
                "authentication" => {
                    method = AuthMethod::ALL
                        .into_iter()
                        .find(|method| method.name().eq_ignore_ascii_case(value))
                        .ok_or_else(|| {
                            OxynError::Config(
                                "parameter `authentication` expects none, basic, api-key or bearer"
                                    .to_owned(),
                            )
                        })?;
                }
                "username" => username = Some(value).filter(|name| !name.is_empty()),
                other => {
                    return Err(OxynError::Config(format!(
                        "unknown Elasticsearch connection parameter `{other}`"
                    )));
                }
            }
        }
        let base =
            base_url(url.ok_or_else(|| {
                OxynError::Config("the Elasticsearch URL is required".to_owned())
            })?)?;
        let tls = (base.scheme() == "https").then_some(tls);
        let local = config.environment == Environment::Local;
        if tls.is_none() && !local {
            return Err(OxynError::Config(
                "an `http://` URL sends credentials and data in clear: it is accepted only on a \
                 connection marked Local; use `https://` with sslmode verify-full"
                    .to_owned(),
            ));
        }
        if tls == Some(TlsMode::Require) && !local {
            return Err(OxynError::Config(
                "sslmode `require` does not verify the server's certificate: it is accepted only \
                 on a connection marked Local; use verify-full"
                    .to_owned(),
            ));
        }
        Ok(Self {
            base,
            tls,
            authorization: authorization(method, username, credentials)?,
        })
    }
}

/// The configured URL, as an origin requests can be joined to.
fn base_url(raw: &str) -> Result<Url> {
    let mut base = Url::parse(raw)
        .map_err(|_| OxynError::Config("the Elasticsearch URL is not a valid URL".to_owned()))?;
    if !matches!(base.scheme(), "https" | "http") || base.host().is_none() {
        return Err(OxynError::Config(
            "the Elasticsearch URL must be an `https://` or `http://` address".to_owned(),
        ));
    }
    if !base.username().is_empty() || base.password().is_some() {
        // Never echo the URL: it is the part that carries the password.
        return Err(OxynError::Config(
            "credentials in an Elasticsearch URL are refused; use the authentication fields"
                .to_owned(),
        ));
    }
    if base.query().is_some() || base.fragment().is_some() {
        return Err(OxynError::Config(
            "the Elasticsearch URL takes no query or fragment".to_owned(),
        ));
    }
    // A path prefix (a reverse proxy's `/es`) must end with `/`, or joining
    // `_search` to it would replace its last segment.
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    Ok(base)
}

fn authorization(
    method: AuthMethod,
    username: Option<&str>,
    credentials: &Credentials,
) -> Result<Option<HeaderValue>> {
    let value = match method {
        AuthMethod::None => return Ok(None),
        AuthMethod::Basic => {
            let username = username.ok_or_else(|| {
                OxynError::Config("basic authentication requires a username".to_owned())
            })?;
            if username.contains(':') {
                return Err(OxynError::Config(
                    "a basic-authentication username cannot contain `:`".to_owned(),
                ));
            }
            let password = credentials.password().ok_or_else(|| {
                OxynError::Authentication("basic authentication requires a password".to_owned())
            })?;
            let pair = format!("{username}:{}", password.expose_secret());
            format!("Basic {}", base64(pair.as_bytes()))
        }
        AuthMethod::ApiKey | AuthMethod::Bearer => {
            let token = credentials.token().ok_or_else(|| {
                OxynError::Authentication(
                    "API-key and bearer authentication require a token".to_owned(),
                )
            })?;
            let scheme = if method == AuthMethod::ApiKey {
                "ApiKey"
            } else {
                "Bearer"
            };
            format!("{scheme} {}", token.expose_secret().trim())
        }
    };
    let mut header = HeaderValue::from_str(&value).map_err(|_| {
        OxynError::Config(
            "the credentials contain a character an HTTP header cannot carry".to_owned(),
        )
    })?;
    header.set_sensitive(true);
    Ok(Some(header))
}

/// Standard base64 with padding, the encoding `Basic` takes (RFC 7617).
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let symbol = |index: u32| {
        let position = usize::try_from(index & 0x3f).unwrap_or_default();
        char::from(TABLE.get(position).copied().unwrap_or(b'A'))
    };
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let [a, b, c] = [0, 1, 2].map(|i| u32::from(chunk.get(i).copied().unwrap_or(0)));
        let triple = (a << 16) | (b << 8) | c;
        out.push(symbol(triple >> 18));
        out.push(symbol(triple >> 12));
        out.push(if chunk.len() > 1 {
            symbol(triple >> 6)
        } else {
            '='
        });
        out.push(if chunk.len() > 2 { symbol(triple) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;

    use super::*;
    use crate::driver::elasticsearch_metadata;

    fn config(environment: Environment, params: &[(&str, &str)]) -> ConnectionConfig {
        params.iter().fold(
            ConnectionConfig::new("test", DriverId::elasticsearch()).with_environment(environment),
            |config, (key, value)| config.with_param(*key, *value),
        )
    }

    fn spec(environment: Environment, params: &[(&str, &str)]) -> Result<ConnectSpec> {
        ConnectSpec::from_config(
            &elasticsearch_metadata(),
            &config(environment, params),
            &Credentials::new()
                .with_password("s3cret")
                .with_token("key=="),
        )
    }

    #[test]
    fn basic_authentication_is_encoded_as_rfc_7617_says() {
        assert_eq!(base64(b"user:password"), "dXNlcjpwYXNzd29yZA==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b""), "");
        let spec = spec(
            Environment::Production,
            &[
                ("url", "https://search.example:9200"),
                ("authentication", "basic"),
                ("username", "ana"),
            ],
        )
        .expect("valid");
        let header = spec.authorization.expect("a header");
        assert!(header.is_sensitive());
        assert_eq!(header.to_str().ok(), Some("Basic YW5hOnMzY3JldA=="));
    }

    #[test]
    fn an_api_key_and_a_bearer_token_take_their_own_scheme() {
        for (method, expected) in [("api-key", "ApiKey key=="), ("bearer", "Bearer key==")] {
            let spec = spec(
                Environment::Production,
                &[
                    ("url", "https://search.example"),
                    ("authentication", method),
                ],
            )
            .expect("valid");
            let header = spec.authorization.expect("a header");
            assert_eq!(header.to_str().ok(), Some(expected));
        }
    }

    #[test]
    fn weaker_transport_is_local_only() {
        for (url, mode) in [
            ("http://search.example:9200", "verify-full"),
            ("https://search.example:9200", "require"),
        ] {
            for environment in [
                Environment::Production,
                Environment::Staging,
                Environment::Development,
            ] {
                let refused = spec(environment, &[("url", url), ("sslmode", mode)])
                    .err()
                    .expect("refused outside Local");
                assert!(refused.to_string().contains("Local"), "{refused}");
            }
            assert!(spec(Environment::Local, &[("url", url), ("sslmode", mode)]).is_ok());
        }
    }

    #[test]
    fn a_url_never_carries_credentials_and_errors_never_quote_it() {
        let refused = spec(
            Environment::Local,
            &[("url", "https://ana:hunter2@search.example")],
        )
        .err()
        .expect("refused");
        assert!(!refused.to_string().contains("hunter2"), "{refused}");
        assert!(
            spec(
                Environment::Local,
                &[("url", "https://search.example/?a=1")]
            )
            .is_err()
        );
        assert!(spec(Environment::Local, &[("url", "ftp://search.example")]).is_err());
        assert!(
            spec(
                Environment::Local,
                &[("url", "https://search.example"), ("color", "blue")]
            )
            .is_err()
        );
    }

    #[test]
    fn a_proxy_prefix_keeps_its_last_segment() {
        let spec = spec(Environment::Local, &[("url", "https://proxy.example/es")]).expect("valid");
        assert_eq!(
            spec.base.join("_search").expect("joined").as_str(),
            "https://proxy.example/es/_search"
        );
    }

    #[test]
    fn a_header_breaking_secret_is_refused_without_being_quoted() {
        let refused = ConnectSpec::from_config(
            &elasticsearch_metadata(),
            &config(
                Environment::Local,
                &[
                    ("url", "https://search.example"),
                    ("authentication", "bearer"),
                ],
            ),
            &Credentials::new().with_token("abc\r\nX-Evil: 1"),
        )
        .err()
        .expect("refused");
        assert!(!refused.to_string().contains("Evil"), "{refused}");
    }
}
