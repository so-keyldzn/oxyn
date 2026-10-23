//! DuckDB: an embedded analytical SQL engine.
//!
//! DuckDB is not SQLite under another name: it has an analytical type system,
//! an Arrow-oriented execution engine, and SQL statements that can reach the
//! local file system or the network. The driver therefore opens the engine
//! with external access and extension autoloading disabled by default.

mod catalog;
mod cursor;
mod driver;
mod error;
mod options;
mod session;

pub use driver::DuckDbDriver;
pub use options::BatchLimits;
