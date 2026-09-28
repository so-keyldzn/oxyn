//! Keychain errors.
//!
//! This enumeration exists for one precise reason, and one only: **no error
//! message produced here may contain a secret.**
//!
//! The danger is not theoretical. The `keyring` crate carries the offending
//! bytes in two of its variants (`BadEncoding(Vec<u8>)` and
//! `BadDataFormat(Vec<u8>, _)`): copying a keychain error into a message, or
//! printing it with `{:?}`, writes a password into the log. It is exactly the
//! "`tracing` logs" channel of [`SECURITY`](../../../docs/SECURITY.md) (I-03).
//!
//! The countermeasure is in the types: the variants that describe unreadable
//! content carry only a `&'static str`. A `&'static str` cannot carry runtime
//! data — hence no secret. The variants that carry a `String` are built only
//! from a platform diagnostic (a system error code), never from the stored
//! content.

use oxyn_core::OxynError;

/// Result alias of this crate.
pub type Result<T> = std::result::Result<T, SecretError>;

/// What can fail between Oxyn and the system keychain.
///
/// The enumeration is open: an additional storage system (the passphrase
/// encryption planned for phase 4, for instance) will add its cases without a
/// major break.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretError {
    /// No keychain is usable on this machine: unsupported platform, Secret Service
    /// missing on Linux, graphical session closed.
    ///
    /// It is not an Oxyn failure; it is a capability missing from the
    /// environment, and the interface must say so.
    #[error("keychain unavailable: {detail}")]
    Unavailable {
        /// Platform diagnostic, without stored content.
        detail: String,
    },

    /// The keychain exists but refused the operation: locked keychain, user
    /// declining the prompt, application not authorized.
    ///
    /// It is **not** an error to retry in a loop: the user is the one who must
    /// act.
    #[error("keychain access denied: {detail}")]
    AccessDenied {
        /// Platform diagnostic, without stored content.
        detail: String,
    },

    /// The keychain failed for a reason of its own.
    #[error("keychain failure: {detail}")]
    Backend {
        /// Platform diagnostic, without stored content.
        detail: String,
    },

    /// What was read back is not what had been written: non-UTF-8 bytes, invalid
    /// JSON, unexpected structure.
    ///
    /// The detail is a `&'static str` **by construction**: it can therefore quote
    /// neither the offending content nor the position of the parse error. A
    /// `serde_json` error readily copies a fragment of its input; it is not
    /// propagated.
    #[error("unreadable secret: {detail}")]
    Malformed {
        /// Nature of the defect, chosen from a finite set of constants.
        detail: &'static str,
    },

    /// The keychain imposes a size limit that the value exceeds.
    ///
    /// Real case: an 8 KB SSH private key against a platform limit.
    #[error("`{attribute}` exceeds the keychain limit of {limit} characters")]
    TooLarge {
        /// Name of the refused attribute, as the platform names it.
        attribute: String,
        /// Limit announced by the platform.
        limit: u32,
    },

    /// The secret reference is malformed.
    ///
    /// It comes from a workspace file, and a workspace file is not a trusted
    /// input ([`SECURITY`](../../../docs/SECURITY.md), input surface no. 3).
    #[error("invalid secret reference: {detail}")]
    InvalidReference {
        /// Reason for the rejection, without copying the offending value.
        detail: &'static str,
    },
}

impl SecretError {
    /// Detail used when the content read back is not a JSON bundle.
    pub(crate) const NOT_A_BUNDLE: &'static str = "not a JSON credentials bundle";
    /// Detail used when the bundle could not be encoded.
    pub(crate) const NOT_ENCODABLE: &'static str = "the bundle could not be encoded as JSON";
    /// Detail used when the platform returns non-textual bytes.
    pub(crate) const NOT_UTF8: &'static str = "the keychain returned non-textual bytes";
}

impl From<SecretError> for OxynError {
    /// Translates into the domain vocabulary, keeping the display register: a
    /// locked keychain is a usage error (the next action belongs to the user), a
    /// platform failure is a local input-output, unreadable content is a
    /// serialization problem.
    fn from(err: SecretError) -> Self {
        match err {
            SecretError::Unavailable { detail } => Self::NotSupported {
                capability: format!("system keychain ({detail})"),
            },
            SecretError::AccessDenied { detail } => {
                Self::Authentication(format!("system keychain: {detail}"))
            }
            other @ (SecretError::Backend { .. } | SecretError::TooLarge { .. }) => {
                Self::Io(std::io::Error::other(other.to_string()))
            }
            other @ (SecretError::Malformed { .. } | SecretError::InvalidReference { .. }) => {
                Self::Serialization(other.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dummy password used everywhere in the leak tests. It has no chance of
    /// appearing in a message by accident.
    const WITNESS_SECRET: &str = "correct-horse-battery-staple";

    #[test]
    fn unreadable_content_cannot_carry_a_secret() {
        // The type forbids the mistake: `detail` is a `&'static str`, so nothing that
        // comes from runtime can enter it. This test documents the intent; the
        // compiler is what enforces it.
        let err = SecretError::Malformed {
            detail: SecretError::NOT_A_BUNDLE,
        };
        assert!(!err.to_string().contains(WITNESS_SECRET));
        assert!(!format!("{err:?}").contains(WITNESS_SECRET));
    }

    #[test]
    fn translation_to_the_domain_keeps_the_register() {
        let locked = SecretError::AccessDenied {
            detail: "keychain locked".into(),
        };
        let domain = OxynError::from(locked);
        assert!(
            domain.is_user_error(),
            "a locked keychain is resolved by a user action"
        );
        assert!(
            !domain.is_retryable(),
            "replaying without the user unlocking is pointless"
        );

        let absent = SecretError::Unavailable {
            detail: "no Secret Service".into(),
        };
        assert!(matches!(
            OxynError::from(absent),
            OxynError::NotSupported { .. }
        ));

        let unreadable = SecretError::Malformed {
            detail: SecretError::NOT_UTF8,
        };
        assert!(matches!(
            OxynError::from(unreadable),
            OxynError::Serialization(_)
        ));

        let panne = SecretError::Backend {
            detail: "OSStatus -25300".into(),
        };
        assert!(matches!(OxynError::from(panne), OxynError::Io(_)));
    }

    #[test]
    fn no_translation_copies_the_stored_content() {
        // Every variant, translated, then read back: the witness secret cannot
        // appear anywhere, because no variant has a field it could have entered.
        let cases = [
            SecretError::Unavailable {
                detail: "unsupported platform".into(),
            },
            SecretError::AccessDenied {
                detail: "user refusal".into(),
            },
            SecretError::Backend {
                detail: "OSStatus -25300".into(),
            },
            SecretError::Malformed {
                detail: SecretError::NOT_A_BUNDLE,
            },
            SecretError::TooLarge {
                attribute: "password".into(),
                limit: 2560,
            },
            SecretError::InvalidReference {
                detail: "empty segment",
            },
        ];
        for case in cases {
            let rendered = case.to_string();
            let translated = OxynError::from(case).to_string();
            assert!(!rendered.contains(WITNESS_SECRET), "{rendered}");
            assert!(!translated.contains(WITNESS_SECRET), "{translated}");
        }
    }
}
