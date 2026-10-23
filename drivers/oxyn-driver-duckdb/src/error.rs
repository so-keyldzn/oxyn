//! Error classification at the DuckDB boundary.

use duckdb::{Error, ErrorCode};
use oxyn_core::{DriverId, ErrorClass, OxynError};

/// Converts a DuckDB error without exposing a connection path or parameter.
pub(crate) fn engine(error: Error, class: ErrorClass) -> OxynError {
    let message = match &error {
        Error::DuckDBFailure(failure) => failure.message.clone(),
        _ => "DuckDB driver error".to_owned(),
    };
    OxynError::driver(DriverId::duckdb(), class, DuckDbError { message })
}

/// Classifies an error from a read-only operation.
pub(crate) fn read(error: Error) -> OxynError {
    let class = match &error {
        Error::DuckDBFailure(failure)
            if matches!(
                failure.code,
                ErrorCode::ConnectionFailure | ErrorCode::IoError
            ) =>
        {
            ErrorClass::Transient
        }
        _ => ErrorClass::Permanent,
    };
    engine(error, class)
}

/// The user-visible portion of an engine error.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct DuckDbError {
    message: String,
}
