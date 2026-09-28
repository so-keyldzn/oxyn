//! What can go wrong in the data layer.
//!
//! One enum per boundary ([`rust.md`](../../../.claude/rules/rust.md)):
//! this one is the boundary between a stream of `RecordBatch`es and the rest
//! of the workspace. It converts into [`OxynError`] to go up, but the
//! immediate caller — `oxyn-exec`, `oxyn-driver` — tells cases apart **by
//! variant**, never by parsing a message.
//!
//! The distinction that matters here is between *the buffer refused* and *the
//! disk refused*: the first is a policy decision, the second a machine
//! failure. Confusing them leads to retrying a disk spill on a full file
//! system.

use std::io;

use arrow::error::ArrowError;
use oxyn_core::OxynError;

/// Result of a data layer operation.
pub type Result<T> = std::result::Result<T, DataError>;

/// Failure of a result buffer, a batch sink or an export.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DataError {
    /// A batch does not carry the schema announced by the buffer.
    ///
    /// It is always a driver defect: the schema is set by
    /// `Cursor::schema()` before the first batch and no longer changes. It is
    /// refused rather than letting the grid read a column that is not the one
    /// it draws.
    #[error("batch schema does not match the result schema: expected {expected}, found {found}")]
    SchemaMismatch {
        /// Schema of the buffer, in Arrow notation.
        expected: String,
        /// Schema of the rejected batch.
        found: String,
    },

    /// A batch arrived after [`mark_complete`](crate::ResultBuffer::mark_complete).
    ///
    /// Accepting it would mean publishing rows no consumer will see: the
    /// displayed row count is already frozen.
    #[error("the result is already complete; no further batch can be accepted")]
    AlreadyComplete,

    /// The buffer reached one of its bounds and refuses to take more.
    ///
    /// It is **not** an error in the sense of a failure: it is back-pressure.
    /// `BatchSink` handles it by cleanly truncating the result, marked as such
    /// in [`ExecStats`](oxyn_core::ExecStats).
    #[error("the result buffer is full: {reason}")]
    Full {
        /// Which bound was reached.
        reason: &'static str,
    },

    /// Writing or reading back the spill file failed.
    ///
    /// Distinct from [`Io`](Self::Io): here, the result already accumulated
    /// stays readable, only the extension failed.
    #[error("spill file failure: {0}")]
    Spill(#[source] io::Error),

    /// Arrow refused the encoding or decoding.
    #[error("arrow failure: {0}")]
    Arrow(#[from] ArrowError),

    /// Local input/output: export file, temporary file.
    #[error("i/o failure: {0}")]
    Io(#[from] io::Error),

    /// The requested export format is not implemented yet.
    ///
    /// "Not knowing how is an acceptable answer; pretending is not" — a
    /// silently degraded export produces a file the user believes faithful.
    #[error("export format `{format}` is not supported yet")]
    UnsupportedFormat {
        /// Name of the format, as it appears in
        /// [`ExportFormat`](oxyn_core::ExportFormat).
        format: &'static str,
    },

    /// An export was requested on a result still being received.
    ///
    /// Writing a partial file that looks like a complete file is silent data
    /// loss. The caller that accepts it declares so through
    /// [`ExportOptions::allow_incomplete`](crate::ExportOptions::allow_incomplete).
    #[error("the result is still streaming; exporting it now would truncate it silently")]
    IncompleteResult,

    /// An export was asked of a truncated result: closed by a row limit,
    /// saturation, a cancellation or a timeout.
    ///
    /// No option allows it: a truncated buffer has finished loading, nothing
    /// tells it from a whole result, and the file would pass for the table.
    #[error(
        "the result is truncated (row limit, memory budget, cancellation or timeout); \
         exporting it would pass a partial result off as complete"
    )]
    TruncatedResult,

    /// The operation was interrupted by a
    /// [`CancelToken`](oxyn_core::CancelToken).
    #[error("cancelled")]
    Cancelled,
}

impl DataError {
    /// Can the operation be retried as is?
    ///
    /// Conservative by construction ([I-13](../../../CLAUDE.md#i-13)): when in
    /// doubt, `false`. Only a write failure of the spill file is declared
    /// replayable, because it does not touch the data already received.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Spill(_))
    }

    /// Does the error come from a cancellation requested by the user?
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl From<DataError> for OxynError {
    fn from(error: DataError) -> Self {
        match error {
            DataError::Cancelled => Self::Cancelled,
            DataError::Io(source) => Self::Io(source),
            DataError::Spill(source) => Self::Io(source),
            DataError::UnsupportedFormat { format } => Self::NotSupported {
                capability: format!("export:{format}"),
            },
            // `Arrow` and `SchemaMismatch` are encoding defects: putting them
            // in `Internal` would hide that they come from received data, and
            // therefore that they may happen again at the next execution of the
            // same query.
            other @ (DataError::Arrow(_) | DataError::SchemaMismatch { .. }) => {
                Self::Serialization(other.to_string())
            }
            // A refusal of use, not a bug: `Internal` would present it as a
            // defect of Oxyn.
            other @ DataError::TruncatedResult => Self::Config(other.to_string()),
            other => Self::Internal(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancellation_stays_a_cancellation_after_conversion() {
        let error: OxynError = DataError::Cancelled.into();
        assert!(error.is_cancelled());
    }

    #[test]
    fn a_missing_format_becomes_a_missing_capability() {
        let error: OxynError = DataError::UnsupportedFormat { format: "parquet" }.into();
        match error {
            OxynError::NotSupported { capability } => assert_eq!(capability, "export:parquet"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    /// A schema defect must not be classified replayable: replaying the same
    /// query will produce the same inconsistent schema.
    #[test]
    fn a_schema_defect_is_not_replayable() {
        let error = DataError::SchemaMismatch {
            expected: "a: Int32".into(),
            found: "a: Utf8".into(),
        };
        assert!(!error.is_retryable());
    }
}
