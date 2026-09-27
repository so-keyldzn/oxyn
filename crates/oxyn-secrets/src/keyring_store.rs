//! The operating system's keychain.
//!
//! It is the production implementation of [`SecretStore`]: Keychain on macOS,
//! Secret Service on Linux, Credential Manager on Windows. The `keyring` crate
//! picks the store according to the platform; Oxyn writes no code specific to a
//! system.
//!
//! # The rule that governs this module
//!
//! **A `keyring` error is never propagated as it is.** Two of its variants carry
//! the offending bytes (`BadEncoding(Vec<u8>)`, `BadDataFormat(Vec<u8>, _)`):
//! printing them with `{:?}`, or copying them into a message, writes a password
//! into the log. Everything goes through `map_keyring`, which translates **per
//! variant** and never returns the stored content (I-03).
//!
//! The same caution applies to the unknown variant: `keyring::Error` is
//! `#[non_exhaustive]`, and nothing says that a future variant will not carry
//! stored bytes too. The default case therefore returns a **constant** message,
//! never the `Display` of the received error.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroize;

use crate::error::{Result, SecretError};
use crate::store::{SecretRef, SecretStore};

/// The system keychain, addressed by a service name.
///
/// The service name is what the user sees in Keychain Access or in `seahorse`
/// next to the entries written by Oxyn. It is not a secret and does not need to
/// be.
///
/// The structure keeps **no** value: each operation opens the matching entry,
/// uses it and closes it. There is thus no password cache in memory, and nothing
/// to erase on destruction.
#[derive(Clone)]
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    /// Service name used by Oxyn.
    ///
    /// It is stable: changing it would make invisible every credential users have
    /// already saved.
    pub const DEFAULT_SERVICE: &'static str = "oxyn";

    /// Opens the system keychain under Oxyn's service name.
    ///
    /// Construction does not touch the keychain yet: the first operation triggers
    /// the platform's initialization, and thus, if any, the authorization prompt. To
    /// check availability without writing, see [`availability`](Self::availability).
    #[must_use]
    pub fn new() -> Self {
        Self::with_service(Self::DEFAULT_SERVICE)
    }

    /// Opens the keychain under a chosen service name.
    ///
    /// Used by integration tests, which must be able to write to the development
    /// machine's keychain without overwriting the user's real credentials.
    #[must_use]
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// Service name used by this instance.
    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    /// Is the keychain usable on this machine?
    ///
    /// The call **initializes** the platform store if it was not yet. On Linux
    /// without a graphical session or Secret Service, it returns
    /// [`SecretError::Unavailable`]: a capability missing from the environment,
    /// which the interface must announce rather than let each connection fail one
    /// after the other.
    ///
    /// # Errors
    /// See [`SecretError`].
    pub fn availability() -> Result<()> {
        match keyring::Entry::store_status() {
            Ok(()) => Ok(()),
            Err(err) => Err(map_keyring(err)),
        }
    }

    /// Opens the keychain entry matching a reference.
    fn entry(&self, reference: &SecretRef) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, reference.as_str()).map_err(|err| map_keyring(&err))
    }
}

impl Default for KeyringSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for KeyringSecretStore {
    /// Written by hand, like everything touching secrets in this crate.
    ///
    /// There is only a service name to show here — but a derived `Debug` on a type
    /// of this module would become wrong the day a cache was added to it, and
    /// nobody would notice (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyringSecretStore")
            .field("service", &self.service)
            .finish()
    }
}

impl SecretStore for KeyringSecretStore {
    fn put(&self, reference: &SecretRef, secret: SecretString) -> Result<()> {
        self.entry(reference)?
            .set_password(secret.expose_secret())
            .map_err(|err| map_keyring(&err))?;
        // The reference is public; the value is not and does not appear here. That is
        // the whole point of separating the two.
        tracing::debug!(secret_ref = %reference, "secret written to the keychain");
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<Option<SecretString>> {
        match self.entry(reference)?.get_password() {
            Ok(mut clair) => {
                // `keyring` returns a `String` whose capacity and buffer we do not
                // control. We copy it into an exact allocation — which `SecretString`
                // will erase on destruction — then erase the original, which would
                // otherwise stay readable in the heap.
                let secret = SecretString::from(clair.as_str());
                clair.zeroize();
                Ok(Some(secret))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_keyring(&err)),
        }
    }

    fn delete(&self, reference: &SecretRef) -> Result<()> {
        match self.entry(reference)?.delete_credential() {
            // Deleting what does not exist gives the desired state: it is a
            // success. Otherwise, deleting a connection without a password would
            // surface an error to the user for nothing.
            Ok(()) | Err(keyring::Error::NoEntry) => {
                tracing::debug!(secret_ref = %reference, "secret removed from the keychain");
                Ok(())
            }
            Err(err) => Err(map_keyring(&err)),
        }
    }
}

/// Translates a `keyring` error **per variant**.
///
/// No branch copies the stored content:
///
/// * `BadEncoding` and `BadDataFormat` carry the offending bytes — they are
///   dropped, and the returned detail is a constant;
/// * `NoStorageAccess` and `PlatformFailure` carry a platform diagnostic (a
///   system error code): that one is useful and safe;
/// * the default branch exists because `keyring::Error` is
///   `#[non_exhaustive]`. It returns a **constant** message: a variant added
///   tomorrow could, like two of today's, carry stored bytes, and
///   `err.to_string()` would publish them without anyone noticing.
fn map_keyring(err: &keyring::Error) -> SecretError {
    use keyring::Error as K;

    match err {
        K::NoEntry => SecretError::Backend {
            detail: "no secret under this reference".into(),
        },
        K::NoDefaultStore => SecretError::Unavailable {
            detail: "no credential store could be initialized".into(),
        },
        K::Invalid(attribute, _) if attribute.as_str() == "platform" => SecretError::Unavailable {
            detail: "platform not supported by the keychain".into(),
        },
        K::Invalid(attribute, _) => SecretError::Backend {
            detail: format!("parameter `{attribute}` rejected by the keychain"),
        },
        K::NoStorageAccess(cause) => SecretError::AccessDenied {
            detail: cause.to_string(),
        },
        K::PlatformFailure(cause) => SecretError::Backend {
            detail: cause.to_string(),
        },
        K::BadEncoding(_) => SecretError::Malformed {
            detail: SecretError::NOT_UTF8,
        },
        K::BadDataFormat(_, _) => SecretError::Malformed {
            detail: "the keychain could not decode what it stores",
        },
        K::BadStoreFormat(raison) => SecretError::Backend {
            detail: format!("unreadable credential store: {raison}"),
        },
        K::TooLong(attribute, limite) => SecretError::TooLarge {
            attribute: attribute.clone(),
            limit: *limite,
        },
        K::Ambiguous(_) => SecretError::Backend {
            detail: "several keychain entries match this reference".into(),
        },
        _ => SecretError::Backend {
            detail: "unknown keychain error".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No test of this crate touches the machine's real keychain: that would
    /// trigger an authorization prompt during `make qualite` and pollute the
    /// developer's keychain. Write behavior is covered by `MemorySecretStore`;
    /// what is tested here is the translation of errors, which is the sensitive
    /// part.
    const SECRET_TEMOIN: &str = "hunter2";

    #[test]
    fn offending_bytes_never_leave_the_translation() {
        // This is the leak scenario: the keychain returns non-UTF-8 bytes, and the
        // resulting error carries the stored content.
        let brut = SECRET_TEMOIN.as_bytes().to_vec();
        let cas = [
            keyring::Error::BadEncoding(brut.clone()),
            keyring::Error::BadDataFormat(brut, Box::new(std::fmt::Error)),
        ];

        for erreur in &cas {
            let traduite = map_keyring(erreur);
            assert!(matches!(traduite, SecretError::Malformed { .. }));

            let rendu = traduite.to_string();
            let debug = format!("{traduite:?}");
            assert!(!rendu.contains(SECRET_TEMOIN), "secret leaked: {rendu}");
            assert!(!debug.contains(SECRET_TEMOIN), "secret leaked: {debug}");
        }
    }

    #[test]
    fn a_platform_without_keychain_is_a_missing_capability() {
        let sans_magasin = map_keyring(&keyring::Error::NoDefaultStore);
        assert!(matches!(sans_magasin, SecretError::Unavailable { .. }));

        let hors_plateforme = map_keyring(&keyring::Error::Invalid(
            "platform".into(),
            "must be macOS, Windows or *nix".into(),
        ));
        assert!(matches!(hors_plateforme, SecretError::Unavailable { .. }));
    }

    #[test]
    fn an_access_refusal_differs_from_a_failure() {
        let verrouille = map_keyring(&keyring::Error::NoStorageAccess(Box::new(
            std::io::Error::other("keychain is locked"),
        )));
        assert!(matches!(verrouille, SecretError::AccessDenied { .. }));
        assert!(verrouille.to_string().contains("keychain is locked"));

        let panne = map_keyring(&keyring::Error::PlatformFailure(Box::new(
            std::io::Error::other("OSStatus -25300"),
        )));
        assert!(matches!(panne, SecretError::Backend { .. }));
    }

    #[test]
    fn a_value_too_large_is_named_as_such() {
        // Real case: an SSH private key against a platform limit.
        let trop_grand = map_keyring(&keyring::Error::TooLong("password".into(), 2560));
        match trop_grand {
            SecretError::TooLarge { attribute, limit } => {
                assert_eq!(attribute, "password");
                assert_eq!(limit, 2560);
            }
            autre => panic!("attendu TooLarge, obtenu {autre:?}"),
        }
    }

    #[test]
    fn a_refused_parameter_publishes_only_its_name() {
        // The second member of `Invalid` is an explanation from the platform:
        // nothing guarantees it does not quote the refused value.
        let refuse = map_keyring(&keyring::Error::Invalid(
            "password".into(),
            format!("`{SECRET_TEMOIN}` is not acceptable"),
        ));
        let rendu = refuse.to_string();
        assert!(rendu.contains("password"));
        assert!(!rendu.contains(SECRET_TEMOIN), "value leaked: {rendu}");
    }

    #[test]
    fn the_service_name_is_stable() {
        assert_eq!(KeyringSecretStore::DEFAULT_SERVICE, "oxyn");
        assert_eq!(KeyringSecretStore::new().service(), "oxyn");
        assert_eq!(
            KeyringSecretStore::with_service("oxyn-tests").service(),
            "oxyn-tests"
        );
    }

    #[test]
    fn the_store_debug_shows_only_a_service_name() {
        let magasin = KeyringSecretStore::new();
        let rendu = format!("{magasin:?}");
        assert!(rendu.contains("oxyn"));
        assert!(!rendu.contains("password"));
    }
}
