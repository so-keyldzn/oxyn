//! The reference to a secret, and the contract every keychain honors.
//!
//! The division of roles is the one of [`SECURITY`](../../../docs/SECURITY.md)
//! and of [`ARCHITECTURE` §8](../../../docs/ARCHITECTURE.md):
//!
//! * what is **persisted** in a workspace file is a [`SecretRef`] — a stable,
//!   public string, derived from the connection identifier;
//! * what is **stored** in the system keychain is the value, under that
//!   reference.
//!
//! The failure avoided is concrete: a workspace file containing a production
//! password, committed by the user to their team's repository because the file
//! looked like plain configuration.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use oxyn_core::{ConnectionConfig, ConnectionId};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::bundle::CredentialBundle;
use crate::error::{Result, SecretError};

/// Stable designation of a secret in the system keychain.
///
/// Canonical form: `oxyn:<kind>:<name>`, for example
/// `oxyn:conn:018f0000-0000-7000-8000-000000000000`. The `oxyn` prefix confines
/// the entries written by Oxyn; the kind says what the secret is attached to;
/// the name identifies the object.
///
/// # A reference is not a secret
///
/// Its `Debug` is **complete**, and on purpose: the reference is written in
/// clear text in workspace files, it is meant to be seen. Masking what is not
/// secret dilutes the signal of the masks that matter — those of
/// [`CredentialBundle`] and of [`oxyn_core::ConnectionConfig`].
///
/// # Why validation is strict
///
/// A reference read back from a workspace file becomes an **account name in the
/// system keychain**. A workspace file is an untrusted input
/// ([`SECURITY`](../../../docs/SECURITY.md), input surface no. 3): it may have
/// been written by a third party. Letting through a control byte, a space or an
/// extra `:` lets a foreign string decide which keychain entry Oxyn will read.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SecretRef(Arc<str>);

impl SecretRef {
    /// Prefix of every entry written by Oxyn.
    pub const SCHEME: &'static str = "oxyn";
    /// Kind of the secrets attached to a database connection.
    pub const KIND_CONNECTION: &'static str = "conn";
    /// Kind of the secrets attached to a model provider (phase 2).
    pub const KIND_PROVIDER: &'static str = "llm";

    /// Maximum length of the full reference.
    const MAX_LEN: usize = 160;
    /// Maximum length of the kind segment.
    const MAX_KIND_LEN: usize = 16;
    /// Maximum length of the name segment.
    const MAX_NAME_LEN: usize = 128;

    /// Reference of a connection's secret, derived from its identifier.
    ///
    /// The derivation is total and deterministic: two calls on the same
    /// [`ConnectionId`] return the same reference, including after a restart. That
    /// is what finds again the password of a connection whose file only ever
    /// carried the identifier.
    #[must_use]
    pub fn for_connection(id: ConnectionId) -> Self {
        Self(Arc::from(format!(
            "{}:{}:{id}",
            Self::SCHEME,
            Self::KIND_CONNECTION
        )))
    }

    /// Reference of a model provider's secret.
    ///
    /// # Errors
    /// Returns [`SecretError::InvalidReference`] if the provider name does not
    /// follow the naming rules of a segment.
    pub fn for_provider(provider: impl AsRef<str>) -> Result<Self> {
        let provider = provider.as_ref();
        validate_name(provider)?;
        Ok(Self(Arc::from(format!(
            "{}:{}:{provider}",
            Self::SCHEME,
            Self::KIND_PROVIDER
        ))))
    }

    /// Reference to query for a given connection.
    ///
    /// If the configuration already carries a
    /// [`secret_ref`](ConnectionConfig::secret_ref), it is authoritative — after
    /// validation, because it comes from a file. Otherwise, the reference is
    /// derived from the connection identifier.
    ///
    /// # Errors
    /// Returns [`SecretError::InvalidReference`] if the reference written in the
    /// file is malformed. There is **no** silent fallback to the derived reference
    /// in that case: reading another secret than the one requested would be worse
    /// than reading nothing.
    pub fn for_connection_config(config: &ConnectionConfig) -> Result<Self> {
        match config.secret_ref.as_deref() {
            Some(existing) => Self::parse(existing),
            None => Ok(Self::for_connection(config.id)),
        }
    }

    /// Parses a reference read back from a workspace file.
    ///
    /// # Errors
    /// Returns [`SecretError::InvalidReference`] if the string is not of the form
    /// `oxyn:<kind>:<name>` with normalized segments. The message never copies the
    /// offending value.
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > Self::MAX_LEN {
            return Err(invalid("reference too long"));
        }
        let mut segments = text.split(':');
        let (Some(scheme), Some(kind), Some(name), None) = (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ) else {
            return Err(invalid("expected form: oxyn:<kind>:<name>"));
        };
        if scheme != Self::SCHEME {
            return Err(invalid("the prefix must be `oxyn`"));
        }
        validate_kind(kind)?;
        validate_name(name)?;
        Ok(Self(Arc::from(text)))
    }

    /// Borrowed view of the full reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Kind segment (`conn`, `llm`…).
    #[must_use]
    pub fn kind(&self) -> &str {
        self.segment(1)
    }

    /// Name segment: the connection identifier, the provider name…
    #[must_use]
    pub fn name(&self) -> &str {
        self.segment(2)
    }

    /// Does the reference designate a connection's secret?
    #[must_use]
    pub fn is_connection(&self) -> bool {
        self.kind() == Self::KIND_CONNECTION
    }

    /// Segment at index `index`, the form being guaranteed by construction.
    fn segment(&self, index: usize) -> &str {
        // Every `SecretRef` went through `parse` or one of the factories: it
        // therefore has exactly three segments. Return `""` rather than panic if
        // this invariant were ever broken.
        self.0.split(':').nth(index).unwrap_or_default()
    }
}

/// Builds a reference error.
fn invalid(detail: &'static str) -> SecretError {
    SecretError::InvalidReference { detail }
}

/// Validates a kind segment: ASCII lowercase, digits, `-` and `_`.
fn validate_kind(kind: &str) -> Result<()> {
    if kind.is_empty() {
        return Err(invalid("the kind is empty"));
    }
    if kind.len() > SecretRef::MAX_KIND_LEN {
        return Err(invalid("kind too long"));
    }
    if !kind.starts_with(|c: char| c.is_ascii_lowercase()) {
        return Err(invalid("the kind must start with a lowercase letter"));
    }
    if !kind
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(invalid("allowed characters in the kind: a-z, 0-9, -, _"));
    }
    Ok(())
}

/// Validates a name segment.
///
/// The allowed set is deliberately narrow: this segment becomes an account name
/// in the system keychain, and it must be impossible to slip a space, a control
/// byte or a separator into it.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(invalid("the name is empty"));
    }
    if name.len() > SecretRef::MAX_NAME_LEN {
        return Err(invalid("name too long"));
    }
    // The first character is alphanumeric, which excludes `.` and `..`: a
    // reference ends up one day in a cache file name, and a name designating a
    // parent directory would be a path traversal there.
    if !name.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return Err(invalid("the name must start with a letter or a digit"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(invalid(
            "allowed characters in the name: A-Z, a-z, 0-9, -, _, .",
        ));
    }
    Ok(())
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretRef({:?})", self.as_str())
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for SecretRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for SecretRef {
    type Err = SecretError;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl TryFrom<String> for SecretRef {
    type Error = SecretError;

    fn try_from(value: String) -> Result<Self> {
        Self::parse(&value)
    }
}

impl From<SecretRef> for String {
    fn from(reference: SecretRef) -> Self {
        reference.as_str().to_owned()
    }
}

/// What Oxyn expects from a keychain.
///
/// The API is **synchronous**: an access to the system keychain is a local call
/// in the order of a millisecond, and wrapping it in `async` would charge a
/// `spawn_blocking` on every read to solve nothing. Implementations are still
/// `Send + Sync`: they are called from the Tokio runtime, never from the
/// interface thread (I-05).
///
/// # `Debug` is part of the contract
///
/// The trait requires `Debug` so that a structure holding an
/// `Arc<dyn SecretStore>` can derive one — and it thereby requires every
/// implementation to write a `Debug` that shows **no stored value**. An
/// implementation that printed its entries would violate I-03.
///
/// # Idempotence
///
/// [`delete`](Self::delete) succeeds when the reference is unknown: deleting
/// what does not exist is the desired state, not a failure. That is what allows
/// deleting a connection without knowing whether it had a password.
pub trait SecretStore: fmt::Debug + Send + Sync {
    /// Writes — or replaces — the designated secret.
    ///
    /// # Errors
    /// See [`SecretError`]: keychain unavailable, access denied, value too large
    /// for the platform.
    fn put(&self, reference: &SecretRef, secret: SecretString) -> Result<()>;

    /// Reads back the designated secret, or `None` if the reference is unknown.
    ///
    /// An unknown reference is not an error: a connection may have no secret
    /// (SQLite on a file, Unix socket authentication, `~/.pgpass`).
    ///
    /// # Errors
    /// See [`SecretError`]. In particular [`Malformed`](SecretError::Malformed) if
    /// the keychain returns something other than what was written to it.
    fn get(&self, reference: &SecretRef) -> Result<Option<SecretString>>;

    /// Deletes the designated secret. Succeeds if the reference is already unknown.
    ///
    /// # Errors
    /// See [`SecretError`].
    fn delete(&self, reference: &SecretRef) -> Result<()>;

    /// Writes a complete set of credentials, encoded as JSON.
    ///
    /// It is **the** write path for everything that is not a plain password: the
    /// JSON envelope lives here and nowhere else, so that a calling crate does not
    /// invent its own format.
    ///
    /// # Errors
    /// See [`SecretError`].
    fn put_bundle(&self, reference: &SecretRef, bundle: &CredentialBundle) -> Result<()> {
        self.put(reference, bundle.to_secret_json()?)
    }

    /// Reads back a complete set of credentials.
    ///
    /// # Errors
    /// [`Malformed`](SecretError::Malformed) if the keychain entry exists but is
    /// not a bundle — typically a bare password written by an earlier version, or
    /// by another tool under the same reference.
    fn get_bundle(&self, reference: &SecretRef) -> Result<Option<CredentialBundle>> {
        match self.get(reference)? {
            Some(json) => CredentialBundle::from_secret_json(&json).map(Some),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests;
