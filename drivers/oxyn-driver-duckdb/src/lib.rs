//! Oxyn's DuckDB driver: an embedded analytical engine, in Oxyn's process.
//!
//! DuckDB is not SQLite under another name: an analytical type system, a
//! vectorized engine exporting Arrow, and SQL that reaches the file system
//! and the network. The decisions are argued in
//! [ADR-0059](../../../docs/adr/0059-duckdb-driver-on-duckdb-rs-bundled.md);
//! the contract is [DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md).
//!
//! | Module | Subject |
//! |---|---|
//! | [`driver`] | [`DuckDbDriver`]: identity, connection form, capabilities |
//! | `open` | one engine per file in the process, and the access policy |
//! | `worker`, `interrupt` | the carrier thread, and the interruption that targets one task |
//! | `stream` | executing a request: splitting, read-only bound, counted writes, streaming |
//! | [`cursor`] | [`DuckDbCursor`]: Arrow batches, and the Stop |
//! | [`session`] | [`DuckDbSession`]: transactions, session context, preview |
//! | [`catalog`] | [`DuckDbCatalog`]: the `duckdb_*()` functions |
//! | `convert`, [`logical`] | DuckDB's Arrow 58 export rebuilt as Arrow 59, type by type |
//! | [`types`] | a catalog column's written type as a logical type |
//! | [`error`] | the transient / permanent / ambiguous classification |
//! | [`options`] | [`BatchLimits`]: a batch bounded in bytes |
//!
//! # The decisions that govern this crate
//!
//! **One engine per file.** Every session of a file shares one DuckDB
//! instance; two instances on one file in one process would corrupt it
//! (`open`).
//!
//! **SQL reaches no file and no network unless the connection grants a
//! folder.** External access is turned off before the first statement, and
//! DuckDB refuses to turn it back on (`open`).
//!
//! **No Arrow 58 value crosses the crate.** `duckdb-rs` speaks Arrow 58; each
//! chunk is rebuilt as the workspace's Arrow 59 in safe, validated code
//! (`convert`).
//!
//! **No panic reachable from engine data.** The release profile aborts on a
//! panic. Of `duckdb-rs`, the crate calls the statement API —
//! `prepare`, `stream_arrow`, `execute`, `step`, `schema`,
//! `column_logical_type`, `parameter_count` — and the logical type handle's
//! accessors, under the conditions ADR-0059 lists; never the row interface,
//! whose value decoding ends in `unreachable!`.

#[cfg(test)]
mod cancel_tests;
pub mod catalog;
mod convert;
pub mod cursor;
pub mod driver;
pub mod error;
mod interrupt;
pub mod logical;
mod open;
pub mod options;
mod params;
mod preview;
mod query;
pub mod session;
#[cfg(test)]
mod session_tests;
mod stream;
#[cfg(test)]
mod test_support;
mod transaction;
pub mod types;
#[cfg(test)]
mod types_tests;
mod worker;

pub use catalog::DuckDbCatalog;
pub use convert::METADATA_TYPE;
pub use cursor::DuckDbCursor;
pub use driver::DuckDbDriver;
pub use error::{DuckDbError, EngineKind};
pub use options::BatchLimits;
pub use session::DuckDbSession;
