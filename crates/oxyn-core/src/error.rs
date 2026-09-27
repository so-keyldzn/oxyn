//! The domain error type.
//!
//! The classification comes from [`DRIVER-CONTRACT` §4](../../../docs/DRIVER-CONTRACT.md):
//! an error is **transient** (it can be retried), **permanent** (there is no
//! reason to retry) or **ambiguous** (we do not know whether the effect took
//! place — and in that case we above all do not retry).
//!
//! [`OxynError::is_retryable`] encodes this classification. The costliest case
//! is a client-side timeout during a write: the server may have applied it.
//! Replaying creates a silent duplicate in the user's data. That is why
//! [`OxynError::Timeout`] is **not** retryable.

use std::time::Duration;

use crate::ids::DriverId;

/// Result alias for the whole workspace.
pub type Result<T> = std::result::Result<T, OxynError>;

/// Error family in the sense of
/// [`DRIVER-CONTRACT` §4](../../../docs/DRIVER-CONTRACT.md).
///
/// The class is **data** carried by the error, never a deduction the caller
/// makes from the message: a message changes, and a caller that parsed it
/// breaks silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorClass {
    /// Network outage, `too many connections`, expired lock. The caller may
    /// retry, with exponential backoff.
    Transient,
    /// Syntax error, missing table, insufficient rights. Displayed, **never**
    /// retried.
    Permanent,
    /// The server-side effect is unknown — typically a client-side timeout
    /// during a write. **Never** retried, and the uncertainty is reported:
    /// replaying an expired `INSERT` creates a silent duplicate in the user's
    /// data.
    Ambiguous,
}

impl ErrorClass {
    /// Does this family allow a retry?
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient)
    }

    /// Stable name, written to the local state and read by a human without
    /// Oxyn ([I-11](../../../CLAUDE.md#i-11)).
    ///
    /// Versions before 2026-09-16 wrote this code in French; `oxyn-store`
    /// still reads these three values, but nobody writes them any more.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Permanent => "permanent",
            Self::Ambiguous => "ambiguous",
        }
    }
}

/// A single name per family: the one displayed is the one written. Two
/// renderings differing only by language left a reader unable to tell which of
/// the two was the audit code.
impl std::fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Oxyn domain error.
///
/// Messages are meant for a professional: they repeat what the server says
/// rather than a reassuring paraphrase. On the other hand they never contain a
/// secret, a connection identifier or a bound value (I-03) — that
/// responsibility lies with whoever builds the variant.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OxynError {
    /// Invalid or incomplete configuration: unreadable workspace, missing
    /// connection parameter, out-of-range value.
    #[error("invalid configuration: {0}")]
    Config(String),

    /// The connection to the server could not be established or was lost.
    ///
    /// It is the transient family: network outage, `too many connections`,
    /// server restarting.
    #[error("connection failed: {0}")]
    Connection(String),

    /// The server refused the credentials, or the account lacks the rights
    /// needed to open the session.
    #[error("authentication failed: {0}")]
    Authentication(String),

    /// Error raised by a driver, with the family it assigns it to.
    ///
    /// The driver **classifies** its error: it, and it alone, knows whether
    /// `08006` is an outage or a definitive refusal. The caller reads
    /// [`ErrorClass`], it does not re-read the message.
    #[error("driver `{driver}` ({class} error): {source}")]
    Driver {
        /// The driver the error comes from.
        driver: DriverId,
        /// Family of the error, as the driver classifies it.
        class: ErrorClass,
        /// The original error, as the driver produced it.
        ///
        /// Type erasure is unavoidable here — `oxyn-core` knows no driver
        /// implementation — but it erases **no decision information**: that is
        /// in `class`.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The statement is rejected: syntax, missing object, violated
    /// constraint, insufficient rights. Permanent family: display, do not
    /// retry.
    ///
    /// The rejection comes from the server, or from the driver when it refuses
    /// before sending what the server would refuse — a preview column the
    /// relation does not declare. Retrying will not make the column appear.
    #[error("query rejected: {0}")]
    Query(String),

    /// The allotted time has elapsed **on the client side**.
    ///
    /// Ambiguous family: nothing says the server did not apply the write.
    /// See [`OxynError::is_retryable`].
    #[error("timed out after {after:?}")]
    Timeout {
        /// Duration after which the wait was abandoned.
        after: Duration,
    },

    /// The request left and its server-side effect is unknown.
    ///
    /// Ambiguous family, like [`Timeout`](Self::Timeout), for the case where no
    /// delay expired: the connection dropped after sending. A model provider
    /// may have produced — and billed — the reply; a server may have applied
    /// the write.
    ///
    /// The text describes the fact. It carries no URL, host, header, nor the
    /// raw message of the network stack, which can contain all three (I-03).
    #[error("outcome unknown: {0}")]
    OutcomeUnknown(String),

    /// The operation was cancelled, at the user's request or by propagation
    /// of a parent [`CancelToken`](crate::cancel::CancelToken).
    #[error("operation cancelled")]
    Cancelled,

    /// The `PolicyGate` refused the command. It is not a failure: it is the
    /// product doing its job.
    #[error("denied by policy: {reason}")]
    PolicyDenied {
        /// Reason for the refusal, showable as is to the user.
        reason: String,
    },

    /// The command requires an explicit approval that was not given.
    #[error("approval required: {reason}")]
    ApprovalRequired {
        /// What the user must decide on.
        reason: String,
    },

    /// The requested capability does not exist on this session.
    ///
    /// "Not knowing how is an acceptable answer; pretending is not"
    /// ([`DRIVER-CONTRACT` §5](../../../docs/DRIVER-CONTRACT.md)).
    #[error("capability not supported: {capability}")]
    NotSupported {
        /// Name of the missing flag or flags.
        capability: String,
    },

    /// Local input-output failure: disk spill, workspace file, export.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A piece of data could not be encoded or decoded: workspace written by a
    /// future version, malformed JSON, unreadable identifier.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// The catalog is not available: introspection in progress, empty cache,
    /// insufficient rights to read the metadata.
    ///
    /// Transient family: the catalog may come back. A name that the catalog,
    /// once read, does not declare therefore does not belong here — it is
    /// [`Query`](Self::Query).
    #[error("catalog unavailable: {0}")]
    CatalogUnavailable(String),

    /// Broken internal invariant. It is an Oxyn bug, not a usage error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl OxynError {
    /// Builds a driver error naming its family.
    pub fn driver<E>(driver: DriverId, class: ErrorClass, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Driver {
            driver,
            class,
            source: Box::new(source),
        }
    }

    /// Family of the error, when it is known.
    ///
    /// Only driver errors carry it explicitly; for the other variants it
    /// follows from the variant itself, which is equally typed data.
    #[must_use]
    pub const fn class(&self) -> ErrorClass {
        match self {
            Self::Driver { class, .. } => *class,
            Self::Connection(_) | Self::CatalogUnavailable(_) => ErrorClass::Transient,
            Self::Timeout { .. } | Self::OutcomeUnknown(_) => ErrorClass::Ambiguous,
            _ => ErrorClass::Permanent,
        }
    }

    /// Can the operation be replayed as is, with exponential backoff?
    ///
    /// Only the **transient** family answers `true`. In particular:
    ///
    /// * [`Timeout`](Self::Timeout) answers `false`: the server-side effect is
    ///   unknown, and replaying an expired `INSERT` creates a duplicate;
    /// * [`OutcomeUnknown`](Self::OutcomeUnknown) answers `false` for the same
    ///   reason, without a delay having expired: the request left, the reply
    ///   did not arrive;
    /// * [`Driver`](Self::Driver) answers according to the class the driver
    ///   declared, and that alone.
    ///
    /// The retry policy itself belongs to the caller: a driver never retries
    /// on its own, because it alone does not know whether the operation can be
    /// replayed.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.class().is_retryable()
    }

    /// Does the error come from what was requested, rather than from an Oxyn
    /// defect?
    ///
    /// Used to decide the display register: a usage error is shown as is with
    /// the next action; the rest deserves to be reported as an incident.
    /// [`Cancelled`](Self::Cancelled) is neither — it is a deliberate action —
    /// and answers `false`.
    #[must_use]
    pub const fn is_user_error(&self) -> bool {
        matches!(
            self,
            Self::Config(_)
                | Self::Authentication(_)
                | Self::Query(_)
                | Self::PolicyDenied { .. }
                | Self::ApprovalRequired { .. }
                | Self::NotSupported { .. }
        )
    }

    /// Was the operation interrupted on request?
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl From<crate::ids::IdParseError> for OxynError {
    fn from(err: crate::ids::IdParseError) -> Self {
        Self::Serialization(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("the socket was closed by the peer")]
    struct ErreurDriverFactice;

    #[test]
    fn only_transient_errors_are_retried() {
        assert!(OxynError::Connection("network down".into()).is_retryable());
        assert!(OxynError::CatalogUnavailable("empty cache".into()).is_retryable());

        assert!(!OxynError::Query("syntax error at or near \"slect\"".into()).is_retryable());
        assert!(!OxynError::Authentication("password refused".into()).is_retryable());
        assert!(!OxynError::Cancelled.is_retryable());
        assert!(!OxynError::Internal("broken invariant".into()).is_retryable());
    }

    #[test]
    fn a_timeout_is_never_retried() {
        // DRIVER-CONTRACT §4: ambiguity is not retried. An INSERT expired on
        // the client side may have been applied on the server side; replaying
        // it creates a silent duplicate.
        let expiration = OxynError::Timeout {
            after: Duration::from_secs(30),
        };
        assert!(!expiration.is_retryable());
    }

    #[test]
    fn a_driver_error_follows_the_class_declared_by_the_driver() {
        let transitoire = OxynError::driver(
            DriverId::postgres(),
            ErrorClass::Transient,
            ErreurDriverFactice,
        );
        assert!(transitoire.is_retryable());
        assert!(transitoire.to_string().contains("postgres"));
        assert!(transitoire.to_string().contains("closed by the peer"));

        let permanente = OxynError::driver(
            DriverId::postgres(),
            ErrorClass::Permanent,
            ErreurDriverFactice,
        );
        assert!(!permanente.is_retryable());

        let ambigue = OxynError::driver(
            DriverId::sqlite(),
            ErrorClass::Ambiguous,
            ErreurDriverFactice,
        );
        assert!(
            !ambigue.is_retryable(),
            "ambiguity is not retried: the server may have applied the write"
        );
    }

    #[test]
    fn the_class_is_not_deduced_from_the_message() {
        // Two errors with an identical message, two different classes: that is
        // exactly what a caller parsing the text would miss.
        let a = OxynError::driver(
            DriverId::postgres(),
            ErrorClass::Transient,
            ErreurDriverFactice,
        );
        let b = OxynError::driver(
            DriverId::postgres(),
            ErrorClass::Permanent,
            ErreurDriverFactice,
        );
        assert_ne!(a.is_retryable(), b.is_retryable());
    }

    #[test]
    fn classification_by_family() {
        assert_eq!(
            OxynError::Connection("coupure".into()).class(),
            ErrorClass::Transient
        );
        assert_eq!(
            OxynError::Timeout {
                after: Duration::from_secs(1)
            }
            .class(),
            ErrorClass::Ambiguous
        );
        let inconnu = OxynError::OutcomeUnknown("connection lost after sending".into());
        assert_eq!(inconnu.class(), ErrorClass::Ambiguous);
        assert!(!inconnu.is_retryable());
        assert_eq!(
            OxynError::Query("syntaxe".into()).class(),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn classification_of_an_unknown_column() {
        // DRIVER-CONTRACT §4 and §6: a column the relation does not declare
        // will not reappear on the next attempt. It is rendered as `Query`,
        // permanent; `CatalogUnavailable` keeps the meaning of a catalog
        // really unavailable, which may come back.
        let inconnue = OxynError::Query(
            "cannot sort a preview on `absente`: the relation does not declare that column".into(),
        );
        assert_eq!(inconnue.class(), ErrorClass::Permanent);
        assert!(!inconnue.is_retryable());
        assert!(inconnue.is_user_error());
        assert_eq!(
            OxynError::CatalogUnavailable("introspection in progress".into()).class(),
            ErrorClass::Transient
        );
    }

    #[test]
    fn classification_of_usage_errors() {
        assert!(OxynError::Query("relation \"users\" does not exist".into()).is_user_error());
        assert!(
            OxynError::PolicyDenied {
                reason: "read-only connection".into()
            }
            .is_user_error()
        );
        assert!(
            OxynError::NotSupported {
                capability: "TRANSACTIONS".into()
            }
            .is_user_error()
        );

        assert!(!OxynError::Internal("broken invariant".into()).is_user_error());
        assert!(!OxynError::Cancelled.is_user_error());
        assert!(!OxynError::Connection("network down".into()).is_user_error());
    }

    #[test]
    fn an_error_is_either_a_usage_error_or_retryable_never_both() {
        let cas = [
            OxynError::Config("missing field".into()),
            OxynError::Connection("coupure".into()),
            OxynError::Authentication("refused".into()),
            OxynError::Query("syntaxe".into()),
            OxynError::Timeout {
                after: Duration::from_millis(1),
            },
            OxynError::OutcomeUnknown("inconnu".into()),
            OxynError::Cancelled,
            OxynError::PolicyDenied { reason: "r".into() },
            OxynError::ApprovalRequired { reason: "r".into() },
            OxynError::NotSupported {
                capability: "c".into(),
            },
            OxynError::Serialization("json".into()),
            OxynError::CatalogUnavailable("empty".into()),
            OxynError::Internal("bug".into()),
        ];
        for erreur in &cas {
            assert!(
                !(erreur.is_retryable() && erreur.is_user_error()),
                "{erreur} cannot be both retryable and a usage error"
            );
        }
    }

    #[test]
    fn an_unreadable_identifier_becomes_a_serialization_error() {
        let err: OxynError = "not-a-uuid"
            .parse::<crate::ids::SessionId>()
            .expect_err("invalide")
            .into();
        assert!(matches!(err, OxynError::Serialization(_)));
    }
}
