//! A connection's set of credentials.
//!
//! An operating system keychain stores only **one** value per entry. Yet a
//! connection may require a password *and* a client certificate *and* the
//! passphrase of a tunnel SSH key. [`CredentialBundle`] gathers these values and
//! encodes them as JSON before storage: one keychain entry, one logical secret.
//!
//! Three properties are held by the type, not by the caller's discipline:
//!
//! 1. **No derived `Debug`.** `Debug` is written by hand and renders
//!    `<redacted>`. It is the checkable corollary of I-03: it is the
//!    `tracing::debug!("{bundle:?}")` added six months later that leaks.
//! 2. **Erasure on destruction.** [`Zeroize`] is implemented field by field, and
//!    [`Drop`] calls it. A password freed without erasure stays readable in the
//!    heap until the page is reused.
//! 3. **No `Clone`.** Copying a set of credentials multiplies the buffers to
//!    erase without any caller needing it. What must travel is the
//!    [`SecretRef`](crate::SecretRef).

use std::collections::BTreeMap;
use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{Result, SecretError};

/// What must be given to a server to open a session.
///
/// Every field is optional: an SQLite connection on a file fills none, a
/// PostgreSQL connection by password a single one, a mutually authenticated
/// connection three.
///
/// # Serialization
///
/// The JSON produced writes only the filled fields, and reading back tolerates
/// unknown fields: a workspace written by a later version of Oxyn stays
/// readable, at the cost of ignoring what it brings.
#[derive(Default, Serialize, Deserialize)]
pub struct CredentialBundle {
    /// Password of the database account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,

    /// Authentication token: API token of a model provider, bearer token, session
    /// token of a cloud provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,

    /// TLS client certificate, in PEM format.
    ///
    /// The certificate is not secret in itself, but it is useless without its key
    /// and travels with it: separating them would only add a keychain entry to
    /// manage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls_client_cert: Option<String>,

    /// Private key of the TLS client certificate, in PEM format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls_client_key: Option<String>,

    /// SSH private key of a tunnel, in PEM or OpenSSH format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh_private_key: Option<String>,

    /// Passphrase protecting the SSH key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh_passphrase: Option<String>,

    /// Driver-specific secrets that the common model does not provide for: AWS
    /// session token, BigQuery service account, Snowflake private key.
    ///
    /// The key is a short name chosen and documented by the driver; it is not a
    /// secret. The value is one, and it is erased like the others.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    extra: BTreeMap<String, String>,
}

impl CredentialBundle {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the password.
    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Sets the authentication token.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Sets the TLS client certificate (PEM).
    #[must_use]
    pub fn with_tls_client_cert(mut self, pem: impl Into<String>) -> Self {
        self.tls_client_cert = Some(pem.into());
        self
    }

    /// Sets the private key of the TLS client certificate (PEM).
    #[must_use]
    pub fn with_tls_client_key(mut self, pem: impl Into<String>) -> Self {
        self.tls_client_key = Some(pem.into());
        self
    }

    /// Sets the SSH private key of the tunnel.
    #[must_use]
    pub fn with_ssh_private_key(mut self, pem: impl Into<String>) -> Self {
        self.ssh_private_key = Some(pem.into());
        self
    }

    /// Sets the passphrase of the SSH key.
    #[must_use]
    pub fn with_ssh_passphrase(mut self, passphrase: impl Into<String>) -> Self {
        self.ssh_passphrase = Some(passphrase.into());
        self
    }

    /// Adds a driver-specific secret.
    #[must_use]
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// Exposes the password.
    ///
    /// Like every accessor of this type, the returned value is the secret in clear
    /// text: it is handed to the driver and forgotten. It is not logged, not
    /// cached, and does not reach an AI prompt (I-03, I-04).
    #[must_use]
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// Exposes the authentication token. Same precautions as
    /// [`password`](Self::password).
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Exposes the TLS client certificate. Same precautions as
    /// [`password`](Self::password).
    #[must_use]
    pub fn tls_client_cert(&self) -> Option<&str> {
        self.tls_client_cert.as_deref()
    }

    /// Exposes the TLS private key. Same precautions as
    /// [`password`](Self::password).
    #[must_use]
    pub fn tls_client_key(&self) -> Option<&str> {
        self.tls_client_key.as_deref()
    }

    /// Exposes the SSH private key. Same precautions as
    /// [`password`](Self::password).
    #[must_use]
    pub fn ssh_private_key(&self) -> Option<&str> {
        self.ssh_private_key.as_deref()
    }

    /// Exposes the SSH passphrase. Same precautions as
    /// [`password`](Self::password).
    #[must_use]
    pub fn ssh_passphrase(&self) -> Option<&str> {
        self.ssh_passphrase.as_deref()
    }

    /// Exposes a driver-specific secret. Same precautions as
    /// [`password`](Self::password).
    #[must_use]
    pub fn extra(&self, key: &str) -> Option<&str> {
        self.extra.get(key).map(String::as_str)
    }

    /// Does the set contain no secret?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.password.is_none()
            && self.token.is_none()
            && self.tls_client_cert.is_none()
            && self.tls_client_key.is_none()
            && self.ssh_private_key.is_none()
            && self.ssh_passphrase.is_none()
            && self.extra.is_empty()
    }

    /// Names of the filled fields, for diagnostics.
    ///
    /// It is the explicit escape hatch from the masked `Debug`: knowing *that* a
    /// password is present helps understand an authentication failure, and says
    /// nothing of its value. The keys of [`extra`](Self::with_extra) are returned
    /// as they are — they are names chosen by a driver, not secrets.
    #[must_use]
    pub fn filled_fields(&self) -> Vec<&str> {
        let mut names = Vec::new();
        for (present, name) in [
            (self.password.is_some(), "password"),
            (self.token.is_some(), "token"),
            (self.tls_client_cert.is_some(), "tls_client_cert"),
            (self.tls_client_key.is_some(), "tls_client_key"),
            (self.ssh_private_key.is_some(), "ssh_private_key"),
            (self.ssh_passphrase.is_some(), "ssh_passphrase"),
        ] {
            if present {
                names.push(name);
            }
        }
        names.extend(self.extra.keys().map(String::as_str));
        names
    }

    /// Encodes the set as JSON, wrapped in a [`SecretString`].
    ///
    /// # Errors
    /// [`SecretError::Malformed`] if encoding fails — which, on a structure of
    /// strings, would signal a `serde_json` bug rather than faulty data.
    pub fn to_secret_json(&self) -> Result<SecretString> {
        let mut json = serde_json::to_string(self).map_err(|_| SecretError::Malformed {
            detail: SecretError::NOT_ENCODABLE,
        })?;

        // `SecretString::from(&str)` allocates exactly the needed length, so the
        // conversion to `Box<str>` it then does does not reallocate and leaves no
        // copy behind. The `serde_json` buffer, however, has an arbitrary capacity:
        // it is erased explicitly, otherwise the clear-text JSON would survive in
        // the heap after being freed.
        let secret = SecretString::from(json.as_str());
        json.zeroize();
        Ok(secret)
    }

    /// Decodes a set from the JSON read back from the keychain.
    ///
    /// # Errors
    /// [`SecretError::Malformed`] if the content is not a bundle. The `serde_json`
    /// error is **dropped**, neither returned nor logged: its message readily
    /// quotes a fragment of its input, which here is the secret itself.
    pub fn from_secret_json(secret: &SecretString) -> Result<Self> {
        let brut = secret.expose_secret();

        // A bundle is an **object**, and nothing else. Without this guard, `[]`
        // is accepted: serde can build a structure from a sequence, and since
        // every field carries `#[serde(default)]`, an empty sequence gives a
        // perfectly valid empty set. A corrupted keychain entry would then pass
        // for "no secret set", and the connection failure that follows would not
        // point at the keychain.
        //
        // The check is on the first significant character rather than on an
        // intermediate decoding: going through `serde_json::Value` would copy the
        // secret into one more structure, which would then have to be erased.
        if !brut.trim_start().starts_with('{') {
            return Err(SecretError::Malformed {
                detail: SecretError::NOT_A_BUNDLE,
            });
        }

        serde_json::from_str(brut).map_err(|_| SecretError::Malformed {
            detail: SecretError::NOT_A_BUNDLE,
        })
    }
}

impl fmt::Debug for CredentialBundle {
    /// Total rendering: `CredentialBundle(<redacted>)`.
    ///
    /// Not even the list of filled fields — a `Debug` is called by paths nobody
    /// rereads (a `#[derive(Debug)]` of an enclosing structure, a `tracing`
    /// macro), and its output lands in channels nobody chooses. What we want to
    /// show on purpose goes through [`filled_fields`](Self::filled_fields).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialBundle(<redacted>)")
    }
}

impl Zeroize for CredentialBundle {
    fn zeroize(&mut self) {
        self.password.zeroize();
        self.token.zeroize();
        self.tls_client_cert.zeroize();
        self.tls_client_key.zeroize();
        self.ssh_private_key.zeroize();
        self.ssh_passphrase.zeroize();
        // `BTreeMap` has no `Zeroize` implementation: each value is erased in
        // place before the structure is emptied, otherwise `clear()` would merely
        // free buffers that are still readable.
        for value in self.extra.values_mut() {
            value.zeroize();
        }
        self.extra.clear();
    }
}

impl Drop for CredentialBundle {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for CredentialBundle {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_shows_nothing() {
        let bundle = CredentialBundle::new()
            .with_password("hunter2")
            .with_token("sk-ant-secret")
            .with_ssh_passphrase("phrase de passe");

        let rendered = format!("{bundle:?}");
        assert_eq!(rendered, "CredentialBundle(<redacted>)");
        for secret in ["hunter2", "sk-ant-secret", "phrase de passe"] {
            assert!(!rendered.contains(secret), "secret leaked: {rendered}");
        }
    }

    #[test]
    fn the_debug_of_an_enclosing_structure_shows_nothing_either() {
        // This is the real leak path: nobody writes `{bundle:?}`; `Debug` is
        // derived on a structure that contains one.
        #[derive(Debug)]
        struct Enclosing {
            #[allow(dead_code)]
            name: &'static str,
            #[allow(dead_code)]
            credentials: CredentialBundle,
        }

        let enclosing = Enclosing {
            name: "prod-eu",
            credentials: CredentialBundle::new().with_password("hunter2"),
        };
        let rendered = format!("{enclosing:?}");
        assert!(!rendered.contains("hunter2"), "secret leaked: {rendered}");
        assert!(rendered.contains("prod-eu"));
    }

    #[test]
    fn filled_fields_are_named_without_being_shown() {
        let bundle = CredentialBundle::new()
            .with_password("hunter2")
            .with_extra("aws_session_token", "AQoDYXdz");
        assert_eq!(bundle.filled_fields(), ["password", "aws_session_token"]);
        assert!(!bundle.is_empty());
        assert!(CredentialBundle::new().is_empty());
    }

    #[test]
    fn the_json_round_trip_is_faithful() {
        let bundle = CredentialBundle::new()
            .with_password("hunter2")
            .with_token("sk-ant-secret")
            .with_tls_client_cert("-----BEGIN CERTIFICATE-----")
            .with_tls_client_key("-----BEGIN PRIVATE KEY-----")
            .with_ssh_private_key("-----BEGIN OPENSSH PRIVATE KEY-----")
            .with_ssh_passphrase("phrase")
            .with_extra("aws_session_token", "AQoDYXdz");

        let json = bundle.to_secret_json().expect("encoding");
        let read_back = CredentialBundle::from_secret_json(&json).expect("decoding");

        assert_eq!(read_back.password(), Some("hunter2"));
        assert_eq!(read_back.token(), Some("sk-ant-secret"));
        assert_eq!(
            read_back.tls_client_cert(),
            Some("-----BEGIN CERTIFICATE-----")
        );
        assert_eq!(
            read_back.tls_client_key(),
            Some("-----BEGIN PRIVATE KEY-----")
        );
        assert_eq!(
            read_back.ssh_private_key(),
            Some("-----BEGIN OPENSSH PRIVATE KEY-----")
        );
        assert_eq!(read_back.ssh_passphrase(), Some("phrase"));
        assert_eq!(read_back.extra("aws_session_token"), Some("AQoDYXdz"));
        assert_eq!(read_back.extra("unknown"), None);
    }

    #[test]
    fn only_filled_fields_are_written() {
        let bundle = CredentialBundle::new().with_password("hunter2");
        let json = bundle.to_secret_json().expect("encoding");
        assert_eq!(json.expose_secret(), r#"{"password":"hunter2"}"#);

        let empty = CredentialBundle::new().to_secret_json().expect("encoding");
        assert_eq!(empty.expose_secret(), "{}");
    }

    #[test]
    fn a_bundle_written_by_a_later_version_stays_readable() {
        // I-11: what Oxyn writes stays readable, and the reverse too — a field not
        // known yet must not make the keychain unusable.
        let json = SecretString::from(r#"{"password":"hunter2","kerberos_keytab":"…"}"#);
        let bundle = CredentialBundle::from_secret_json(&json).expect("unknown field tolerated");
        assert_eq!(bundle.password(), Some("hunter2"));
    }

    #[test]
    fn what_is_not_a_bundle_is_refused_without_being_copied() {
        for content in ["hunter2", "[]", "{\"password\": 42}", ""] {
            let secret = SecretString::from(content);
            let err = CredentialBundle::from_secret_json(&secret)
                .expect_err("this is not a valid bundle");
            assert!(matches!(err, SecretError::Malformed { .. }));
            assert!(
                !err.to_string().contains("hunter2"),
                "content leaked: {err}"
            );
        }
    }

    #[test]
    fn erasure_empties_every_field() {
        // The heap cannot be observed from a portable test; what can be checked is
        // that `zeroize` resets the structure to the empty state — so that no field
        // was forgotten in the manual implementation.
        let mut bundle = CredentialBundle::new()
            .with_password("hunter2")
            .with_token("sk-ant-secret")
            .with_tls_client_cert("cert")
            .with_tls_client_key("key")
            .with_ssh_private_key("ssh key")
            .with_ssh_passphrase("phrase")
            .with_extra("aws_session_token", "AQoDYXdz");

        bundle.zeroize();

        assert!(bundle.is_empty(), "a field escaped erasure");
        assert!(bundle.filled_fields().is_empty());
    }
}
