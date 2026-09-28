//! A connection's secrets, for the time it takes to hand them to a driver.
//!
//! [`ConnectionConfig`](oxyn_core::ConnectionConfig) carries **no secret**: it
//! stores only a *reference*, resolved by `oxyn-secrets` against the system
//! keychain. [`Credentials`] is what that reference becomes once resolved, and
//! its only use is to be passed to
//! [`Driver::connect`](crate::traits::Driver::connect).
//!
//! # Why one more type
//!
//! `oxyn_secrets::CredentialBundle` does the same job, but `oxyn-secrets` is
//! not in this crate's dependency contract — and putting it there would
//! reverse the direction of dependencies: the system keychain is a detail of
//! the host, not of the driver contract. A driver receives its configuration;
//! it never goes looking for it ([`DRIVER-CONTRACT`](../../../docs/DRIVER-CONTRACT.md)).
//! The conversion fits in a few lines, on the side of the caller that knows
//! both.

use std::fmt;

use indexmap::IndexMap;
use secrecy::SecretString;

/// A connection's secrets, resolved from the system keychain.
///
/// # What the type guarantees
///
/// No derived `Debug`, no `Display`, no `Serialize`, no `Clone`: the only
/// exit paths are [`password`](Self::password), [`token`](Self::token) and
/// [`extra`](Self::extra), which return a [`SecretString`] — itself without
/// `Display`, with a masked `Debug`, and zeroed when dropped.
///
/// The lack of `Clone` is not an oversight: [`SecretString`] is not `Clone`
/// either, because a copy of a secret is one more copy to zero.
#[derive(Default)]
pub struct Credentials {
    password: Option<SecretString>,
    token: Option<SecretString>,
    extras: IndexMap<String, SecretString>,
}

impl Credentials {
    /// No credentials. That is the case of SQLite and DuckDB.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches a password.
    #[must_use]
    pub fn with_password(mut self, password: impl Into<SecretString>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Attaches a token — API key, service token, Redis `AUTH`.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<SecretString>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Attaches a driver-specific secret: passphrase of a client key, SSH
    /// private key…
    ///
    /// The key is a **field name**, not a secret: it appears in the type's
    /// `Debug`.
    #[must_use]
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<SecretString>) -> Self {
        self.extras.insert(key.into(), value.into());
        self
    }

    /// The password, if there is one.
    #[must_use]
    pub fn password(&self) -> Option<&SecretString> {
        self.password.as_ref()
    }

    /// The token, if there is one.
    #[must_use]
    pub fn token(&self) -> Option<&SecretString> {
        self.token.as_ref()
    }

    /// A driver-specific secret.
    #[must_use]
    pub fn extra(&self, key: &str) -> Option<&SecretString> {
        self.extras.get(key)
    }

    /// No secret is carried.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.password.is_none() && self.token.is_none() && self.extras.is_empty()
    }

    /// The **names** of the secrets present, never their values.
    ///
    /// That is all a diagnostic may say about a secret carrier, and it is
    /// exactly what its `Debug` returns.
    #[must_use]
    pub fn filled_fields(&self) -> Vec<&str> {
        let mut names = Vec::new();
        if self.password.is_some() {
            names.push("password");
        }
        if self.token.is_some() {
            names.push("token");
        }
        names.extend(self.extras.keys().map(String::as_str));
        names
    }
}

impl fmt::Debug for Credentials {
    /// Written by hand: a derived `Debug` on a secret carrier is the most
    /// frequent leak, because it is invisible in review (I-03). It is the
    /// `tracing::debug!("{creds:?}")` added six months later that leaks.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Credentials(<redacted: {:?}>)", self.filled_fields())
    }
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    const PASSWORD: &str = "hunter2";

    #[test]
    fn empty_credentials_declare_themselves_as_such() {
        let credentials = Credentials::new();
        assert!(credentials.is_empty());
        assert!(credentials.password().is_none());
        assert!(credentials.token().is_none());
        assert!(credentials.extra("ssh_passphrase").is_none());
        assert!(credentials.filled_fields().is_empty());
    }

    #[test]
    fn each_secret_reads_back_through_its_accessor() {
        let credentials = Credentials::new()
            .with_password(PASSWORD)
            .with_token("service-token")
            .with_extra("ssh_passphrase", "pass-phrase");

        assert_eq!(
            credentials.password().map(ExposeSecret::expose_secret),
            Some(PASSWORD)
        );
        assert_eq!(
            credentials.token().map(ExposeSecret::expose_secret),
            Some("service-token")
        );
        assert_eq!(
            credentials
                .extra("ssh_passphrase")
                .map(ExposeSecret::expose_secret),
            Some("pass-phrase")
        );
        assert!(!credentials.is_empty());
    }

    #[test]
    fn debug_names_only_the_filled_fields() {
        // I-03, checkable corollary: no `Debug` shows a secret.
        let credentials = Credentials::new()
            .with_password(PASSWORD)
            .with_token("service-token")
            .with_extra("ssh_passphrase", "pass-phrase");

        let rendered = format!("{credentials:?}");

        assert!(!rendered.contains(PASSWORD), "leak: {rendered}");
        assert!(!rendered.contains("service-token"), "leak: {rendered}");
        assert!(!rendered.contains("pass-phrase"), "leak: {rendered}");

        // What remains must stay useful for diagnostics.
        assert!(rendered.contains("password"), "{rendered}");
        assert!(rendered.contains("token"), "{rendered}");
        assert!(rendered.contains("ssh_passphrase"), "{rendered}");
    }

    #[test]
    fn debug_of_empty_credentials_does_not_lie() {
        let rendered = format!("{:?}", Credentials::new());
        assert_eq!(rendered, "Credentials(<redacted: []>)");
    }

    #[test]
    fn a_rewritten_secret_replaces_the_previous_one() {
        let credentials = Credentials::new()
            .with_password("old")
            .with_password(PASSWORD);
        assert_eq!(
            credentials.password().map(ExposeSecret::expose_secret),
            Some(PASSWORD)
        );
        assert_eq!(credentials.filled_fields(), ["password"]);
    }
}
