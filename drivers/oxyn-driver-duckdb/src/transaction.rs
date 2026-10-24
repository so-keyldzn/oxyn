//! Whether a transaction is open on a connection, as the engine says it.
//!
//! The C API DuckDB v1.5.6 exposes has no autocommit query, and `duckdb-rs`'s
//! `Connection::is_autocommit` returns `true` unconditionally
//! (`inner_connection.rs`, 1.10506.1). What the engine does expose is
//! `current_transaction_id()`, bound to the transaction the statement runs in
//! (`src/function/scalar/system/current_transaction_id.cpp`). In autocommit
//! mode each statement runs in a transaction of its own, numbered anew
//! (`TransactionContext::BeginTransaction`); inside a transaction the user
//! opened, two statements share its number. Two reads therefore tell the two
//! states apart, without writing anything and without reading the text the
//! user submitted (ADR-0039).
//!
//! A transaction an error invalidated refuses every statement with DuckDB's
//! `TransactionContext Error: Current transaction is aborted (please
//! ROLLBACK)` (`src/main/error_manager.cpp`): it is open, and only a
//! `ROLLBACK` closes it.

use duckdb::Connection;
use oxyn_core::{OxynError, TransactionState};

use crate::error::{DuckDbError, EngineKind};
use crate::query;

/// The probe: the number of the transaction this statement runs in.
const PROBE: &str = "SELECT current_transaction_id()";

/// Reads the transaction state; [`TransactionState::Unknown`] when the engine
/// cannot answer.
///
/// Blocking, on the carrier thread, ordered after every task submitted
/// before it.
pub(crate) fn state(connection: &Connection) -> TransactionState {
    let first = number(connection);
    if let Err(aborted) = &first {
        return if *aborted {
            TransactionState::Open
        } else {
            TransactionState::Unknown
        };
    }
    match (first, number(connection)) {
        (Ok(Some(first)), Ok(Some(second))) if first == second => TransactionState::Open,
        (Ok(Some(_)), Ok(Some(_))) => TransactionState::Idle,
        _ => TransactionState::Unknown,
    }
}

/// The current transaction's number; `Err(true)` when the transaction is
/// aborted, `Err(false)` for any other failure.
///
/// Compared as text: the number is a `UBIGINT`, and DuckDB starts global
/// transaction numbers high.
fn number(connection: &Connection) -> Result<Option<String>, bool> {
    match query::read(connection, PROBE, &[]) {
        Ok(table) => Ok(table.rows().next().and_then(|row| row.text(0))),
        Err(OxynError::Driver { source, .. }) => Err(matches!(
            source.downcast_ref::<DuckDbError>(),
            Some(DuckDbError::Engine {
                kind: EngineKind::Transaction,
                message,
            }) if message.contains("Current transaction is aborted")
        )),
        Err(_) => Err(false),
    }
}
