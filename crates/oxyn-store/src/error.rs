//! Local persistence errors.
//!
//! A single enum for the whole `oxyn-store` boundary, per the repository rule:
//! the caller must be able to tell cases apart without re-reading a string. A
//! schema written by a future version
//! ([`SchemaTooRecent`](StoreError::SchemaTooRecent)) and a locked database
//! ([`Sqlite`](StoreError::Sqlite)) do not call for the same reaction.
//!
//! No message repeats a stored **value**: identifiers, column names and
//! parameter names are allowed, never their content (I-03).

use oxyn_core::OxynError;

/// The crate's result alias.
///
/// Distinct from [`oxyn_core::Result`]: what fails here is the **local**
/// state, not a server. The conversion to `OxynError` exists for when the
/// error goes up to the bus (`impl From<StoreError> for OxynError`).
pub type Result<T> = std::result::Result<T, StoreError>;

/// What can fail while reading or writing the local state.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The local operation stopped before completion at the caller's request.
    #[error("local operation cancelled")]
    Cancelled,
    /// The system exposes no usable user data directory.
    #[error("no usable system data directory")]
    DataDirUnavailable,

    /// SQLite refused the operation: locked database, violated constraint,
    /// audit journal protected by its trigger.
    #[error("local state: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// I/O failure on the file or its directory.
    #[error("I/O on the local state: {0}")]
    Io(#[from] std::io::Error),

    /// A migration could not be applied. The transaction was rolled back: the
    /// schema stayed in its previous state.
    #[error("migration {version} (`{name}`): {source}")]
    Migration {
        /// Number of the failing migration.
        version: u32,
        /// Name of the migration, as written in `schema_version`.
        name: &'static str,
        /// The cause as SQLite returned it.
        #[source]
        source: rusqlite::Error,
    },

    /// The local state was written by a newer version of Oxyn.
    ///
    /// Opening is refused rather than guessed: an older version writing into
    /// a schema it does not understand would corrupt the audit trail.
    #[error("local state written by a newer version (schema {found}, supported up to {supported})")]
    SchemaTooRecent {
        /// Version found in the file.
        found: u32,
        /// Highest version this binary can apply.
        supported: u32,
    },

    /// A column holds a value the domain cannot read back.
    ///
    /// The message names the column and the reason, **never** the value.
    #[error("column `{field}` is unreadable: {detail}")]
    Corrupted {
        /// Column name.
        field: &'static str,
        /// Reason for the rejection, without repeating the value.
        detail: String,
    },

    /// A connection parameter carries a secret's name.
    ///
    /// Refused **on write**: it is the last point where a password can be
    /// prevented from reaching the disk in clear (I-03). Only the key is
    /// named; the value goes up nowhere.
    #[error(
        "parameter `{key}` is named like a secret: \
         only a secret reference (`secret_ref`) is persisted"
    )]
    SecretInParams {
        /// The name of the refused parameter.
        key: String,
    },

    /// A write exceeds a local-state bound.
    ///
    /// Refused **on write**, and never truncated: what would be written by
    /// half — a conversation turn, a transcript — would read back as a
    /// complete text that lies. Only the caller can tell the user to open a
    /// new thread, hence the error rather than a `warn`.
    ///
    /// The message names the column and the bound, never the value (I-03).
    #[error("`{field}` exceeds its local-state limit of {limit}")]
    TooLarge {
        /// Name of the exceeded column or bound.
        field: &'static str,
        /// The bound, in the column's unit — bytes or number of items.
        limit: u64,
    },

    /// Writing an answer, a reasoning or a tool call for an exchange that
    /// received a sample of rows.
    ///
    /// The user's decision: such an exchange only keeps its question, its
    /// counters and its outcome. The refusal is here **and** in a trigger of
    /// the file; there is nothing to retry, the caller was not supposed to
    /// write.
    #[error("exchange {node} kept an approved sample: its answer is not stored")]
    SampleWithheld {
        /// The exchange concerned.
        node: u32,
    },

    /// A JSON encoding or decoding failed (connection parameters, enum tag,
    /// catalog snapshot).
    #[error("local state JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<StoreError> for OxynError {
    /// Maps the error to the bus vocabulary.
    ///
    /// The classification follows what the caller must **do**:
    /// [`Config`](OxynError::Config) is what the user can correct,
    /// [`Serialization`](OxynError::Serialization) is data that cannot be read
    /// back, [`Internal`](OxynError::Internal) is a defect of Oxyn.
    fn from(err: StoreError) -> Self {
        let message = err.to_string();
        match err {
            StoreError::Cancelled => Self::Cancelled,
            StoreError::Io(io) => Self::Io(io),
            // `TooLarge` is on the user's side: what they can do — open a new
            // thread, shorten a query — is an action, not a bug report.
            // Retrying identically changes nothing.
            StoreError::DataDirUnavailable
            | StoreError::SchemaTooRecent { .. }
            | StoreError::SecretInParams { .. }
            | StoreError::TooLarge { .. } => Self::Config(message),
            StoreError::Corrupted { .. } | StoreError::Json(_) => Self::Serialization(message),
            // A caller that writes the answer of a withheld exchange has a
            // defect: the user has nothing to correct.
            StoreError::Sqlite(_)
            | StoreError::Migration { .. }
            | StoreError::SampleWithheld { .. } => Self::Internal(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_refusal_only_shows_the_key() {
        let error = StoreError::SecretInParams {
            key: "password".to_owned(),
        };
        let message = error.to_string();
        assert!(message.contains("password"), "the key must be named");
        assert!(!message.contains("hunter2"), "no value may appear here");
    }

    #[test]
    fn a_secret_in_the_parameters_is_a_configuration_error() {
        let error: OxynError = StoreError::SecretInParams {
            key: "api_key".to_owned(),
        }
        .into();
        assert!(matches!(error, OxynError::Config(_)));
        assert!(error.is_user_error(), "the user can correct it");
        assert!(!error.is_retryable(), "retrying changes nothing");
    }

    #[test]
    fn a_too_recent_schema_is_not_retried() {
        let error: OxynError = StoreError::SchemaTooRecent {
            found: 9,
            supported: 1,
        }
        .into();
        assert!(!error.is_retryable());
    }
}
