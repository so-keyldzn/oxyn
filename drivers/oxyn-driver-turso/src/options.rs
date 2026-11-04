//! The connection parameters, never displayable with their secret.
//!
//! A Turso connection is two values: the database's URL and a **database**
//! token (`turso db tokens create <db>`), not the Platform API token. The
//! token is the field `token` of the form, a secret: it lives in the keyring
//! and reaches this module through [`Credentials::token`], never through the
//! persisted parameters ([I-03](../../../CLAUDE.md#i-03)).
//!
//! [`ConnectSpec`] has **no derived `Debug`**: its own shows the endpoint and
//! nothing of the token, which leaves it only as the `Authorization` header
//! built by [`crate::http`] — never in the URL, which proxies and logs keep.
//!
//! # What a URL may be
//!
//! `https://<db>-<org>.turso.io` or the same with `libsql://`, which is the
//! form `turso db show` prints and which this driver reads as `https://`.
//! Nothing else: `http://` would carry the token in clear
//! ([ADR-0052](../../../docs/adr/0052-verified-tls-outside-local.md)), and
//! `ws://`, `wss://`, `file:` are other transports. A user name, a password, a
//! query or a fragment is refused too, because a pasted `?authToken=…` would
//! otherwise be persisted with the connection. **No refusal echoes the URL**:
//! what the user pasted may hold a secret.

use std::fmt;

use oxyn_core::{ConnectionConfig, OxynError, Result};
use oxyn_driver::{Credentials, DriverMetadata};
use reqwest::header::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

/// The form's key for the database URL.
pub const URL_FIELD: &str = "url";
/// The form's key for the database token. The keyring maps this key to the
/// bundle's token slot.
pub const TOKEN_FIELD: &str = "token";

/// Why a URL is refused. One text for every case: a longer one would have to
/// say which part was wrong, which means quoting it.
const URL_REFUSED: &str = "parameter `url` expects the database URL as `turso db show <db> --url` \
    prints it: `libsql://<db>-<org>.turso.io` or `https://<db>-<org>.turso.io`, with no user, \
    password, path, query or fragment";

/// The parameters of a Turso connection, secret included, not displayable.
#[derive(Clone)]
pub struct ConnectSpec {
    /// `https://host[:port]/`, as Hrana's paths are joined to it.
    endpoint: Url,
    token: SecretString,
}

impl ConnectSpec {
    /// Builds the parameters from a configuration and resolved credentials.
    ///
    /// The only recognized key is `url`. **Any other key is refused**: an
    /// option the driver does not apply would be silently ignored while the
    /// user believes it holds.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the URL is missing or not one of the accepted
    /// forms, a key is unknown, or the token is missing or holds a character
    /// an HTTP header cannot carry (the text never repeats it).
    pub fn from_config(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
        credentials: &Credentials,
    ) -> Result<Self> {
        metadata.validate(config)?;

        let mut url = None;
        for (key, value) in &config.params {
            match key.trim().to_ascii_lowercase().as_str() {
                URL_FIELD => url = Some(value.trim()),
                other => {
                    return Err(OxynError::Config(format!(
                        "parameter `{other}` is not a Turso connection option Oxyn applies"
                    )));
                }
            }
        }
        let url = url
            .filter(|url| !url.is_empty())
            .ok_or_else(|| OxynError::Config("parameter `url` is required".to_owned()))?;
        let endpoint = parse_endpoint(url)?;

        let token = credentials
            .token()
            .filter(|token| !token.expose_secret().trim().is_empty())
            .ok_or_else(|| {
                OxynError::Config(
                    "the database token is required: create one with \
                     `turso db tokens create <db>` (not the Platform API token)"
                        .to_owned(),
                )
            })?;
        let bearer = token.expose_secret().trim();
        // Sent as a header: a character it cannot carry would leave the header
        // out, and the server would answer "unauthorized" about a token the
        // user typed correctly.
        if HeaderValue::from_str(bearer).is_err() {
            return Err(OxynError::Config(
                "the database token holds a character an HTTP header cannot carry".to_owned(),
            ));
        }
        Ok(Self {
            endpoint,
            token: SecretString::from(bearer.to_owned()),
        })
    }

    /// A spec on an arbitrary base URL, for the tests' local server.
    #[cfg(test)]
    pub(crate) fn plaintext(base: &str, token: &str) -> Self {
        Self {
            endpoint: Url::parse(base).expect("a test URL"),
            token: SecretString::from(token.to_owned()),
        }
    }

    /// The database's HTTPS endpoint, without credentials or path.
    #[must_use]
    pub const fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// The token, for the request header and for scrubbing messages only.
    pub(crate) fn token(&self) -> &str {
        self.token.expose_secret()
    }

    /// Whether the endpoint is plain HTTP — only a test's local server. Not
    /// compiled into a release: no code path may relax the `https` rule.
    #[cfg(test)]
    pub(crate) fn is_plaintext(&self) -> bool {
        self.endpoint.scheme() == "http"
    }
}

impl fmt::Debug for ConnectSpec {
    /// The endpoint, which is not a secret; the token is not even said to
    /// exist.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectSpec")
            .field("endpoint", &self.endpoint.as_str())
            .finish_non_exhaustive()
    }
}

/// `libsql://host` and `https://host` become `https://host/`; anything else is
/// refused.
fn parse_endpoint(text: &str) -> Result<Url> {
    let refused = || OxynError::Config(URL_REFUSED.to_owned());
    let (scheme, rest) = text.split_once("://").ok_or_else(refused)?;
    if !scheme.eq_ignore_ascii_case("libsql") && !scheme.eq_ignore_ascii_case("https") {
        return Err(refused());
    }
    // Parsed as `https` in both cases: `libsql` is not a scheme the `url`
    // crate treats as special, so its host would not be normalized.
    let url = Url::parse(&format!("https://{rest}")).map_err(|_| refused())?;
    let plain_root = matches!(url.path(), "" | "/");
    if url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !plain_root
        || url.port() == Some(0)
    {
        return Err(refused());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use oxyn_core::Environment;
    use rstest::rstest;

    use super::*;
    use crate::driver::turso_metadata;

    fn config(url: &str) -> ConnectionConfig {
        ConnectionConfig::new("edge", oxyn_core::DriverId::turso())
            .with_environment(Environment::Production)
            .with_param(URL_FIELD, url)
    }

    fn credentials() -> Credentials {
        Credentials::new().with_token(SecretString::from("eyJ-secret-token"))
    }

    fn spec(url: &str) -> Result<ConnectSpec> {
        ConnectSpec::from_config(&turso_metadata(), &config(url), &credentials())
    }

    #[rstest]
    #[case("libsql://db-org.turso.io", "https://db-org.turso.io/")]
    #[case("LIBSQL://db-org.turso.io", "https://db-org.turso.io/")]
    #[case("https://db-org.turso.io", "https://db-org.turso.io/")]
    #[case("https://db-org.turso.io/", "https://db-org.turso.io/")]
    #[case("  libsql://db-org.turso.io  ", "https://db-org.turso.io/")]
    #[case("https://db.example.com:8443", "https://db.example.com:8443/")]
    fn accepted_urls_become_https_roots(#[case] given: &str, #[case] endpoint: &str) {
        assert_eq!(spec(given).expect(given).endpoint().as_str(), endpoint);
    }

    #[rstest]
    #[case("http://db-org.turso.io")]
    #[case("ws://db-org.turso.io")]
    #[case("wss://db-org.turso.io")]
    #[case("file:///tmp/db.sqlite")]
    #[case("db-org.turso.io")]
    #[case("libsql:db-org.turso.io")]
    #[case("libsql://")]
    #[case("https://")]
    #[case("libsql://user:pw@db-org.turso.io")]
    #[case("https://user@db-org.turso.io")]
    #[case("libsql://db-org.turso.io?authToken=eyJ-leaked")]
    #[case("libsql://db-org.turso.io/?tls=0")]
    #[case("libsql://db-org.turso.io#frag")]
    #[case("libsql://db-org.turso.io/v2/pipeline")]
    #[case("https://db-org.turso.io:0")]
    #[case("https://exa mple.com")]
    fn every_other_url_is_refused_without_being_echoed(#[case] given: &str) {
        let error = spec(given).expect_err(given);
        assert!(matches!(error, OxynError::Config(_)), "{given}: {error:?}");
        // One fixed text for every refusal: nothing of the input is in it.
        assert_eq!(
            error.to_string(),
            format!("invalid configuration: {URL_REFUSED}"),
            "{given}"
        );
    }

    #[test]
    fn the_token_never_shows_in_debug_nor_in_the_endpoint() {
        let spec = spec("libsql://db-org.turso.io").expect("valid");
        assert!(!format!("{spec:?}").contains("eyJ-secret"));
        assert!(!spec.endpoint().as_str().contains("eyJ-secret"));
        assert_eq!(spec.token(), "eyJ-secret-token");
    }

    #[test]
    fn a_missing_or_blank_token_is_refused() {
        for credentials in [
            Credentials::new(),
            Credentials::new().with_token(SecretString::from("   ")),
            // The password slot is not where this driver looks.
            Credentials::new().with_password(SecretString::from("eyJ-secret-token")),
        ] {
            let error = ConnectSpec::from_config(
                &turso_metadata(),
                &config("libsql://db-org.turso.io"),
                &credentials,
            )
            .expect_err("no token");
            assert!(matches!(error, OxynError::Config(_)), "{error:?}");
            assert!(!error.to_string().contains("eyJ-secret"), "{error}");
        }
    }

    #[test]
    fn a_token_a_header_cannot_carry_is_refused_without_echo() {
        let error = ConnectSpec::from_config(
            &turso_metadata(),
            &config("libsql://db-org.turso.io"),
            &Credentials::new().with_token(SecretString::from("eyJ-secret\nsecond-line")),
        )
        .expect_err("a line feed");
        assert!(!error.to_string().contains("eyJ-secret"), "{error}");
    }

    #[test]
    fn an_unknown_option_is_refused_rather_than_ignored() {
        let error = ConnectSpec::from_config(
            &turso_metadata(),
            &config("libsql://db-org.turso.io").with_param("branch", "main"),
            &credentials(),
        )
        .expect_err("refused");
        assert!(error.to_string().contains("branch"), "{error}");
    }

    #[test]
    fn a_token_among_the_persisted_parameters_is_refused() {
        let error = ConnectSpec::from_config(
            &turso_metadata(),
            &config("libsql://db-org.turso.io").with_param(TOKEN_FIELD, "eyJ-secret-token"),
            &credentials(),
        )
        .expect_err("a secret never persists with the connection");
        assert!(!error.to_string().contains("eyJ-secret"), "{error}");
    }
}
