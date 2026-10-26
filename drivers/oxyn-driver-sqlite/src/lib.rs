//! Oxyn's SQLite driver: embedded, synchronous, without network.
//!
//! It is the simple case of the driver layer, and that is why it must be
//! beyond reproach: the PostgreSQL driver will be reviewed against it. The
//! underlying contract is authoritative in
//! [DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md); it is not copied here.
//!
//! | Module | Subject |
//! |---|---|
//! | [`driver`] | [`SqliteDriver`]: identity, connection form, capabilities |
//! | [`session`] | [`SqliteSession`]: execution, transactions, closing |
//! | [`cursor`] | [`SqliteCursor`]: Arrow batches, and the cancellation that cuts |
//! | [`catalog`] | [`SqliteCatalog`]: `sqlite_master` and the `PRAGMA`s |
//! | [`convert`] | SQLite's dynamic typing brought down to Arrow columns |
//! | [`params`] | `ScalarValue` to the five storage classes |
//! | [`error`] | the transient / permanent / ambiguous classification |
//! | [`options`] | [`BatchLimits`]: a batch bounded in rows **and** in bytes |
//!
//! # The four decisions that govern this crate
//!
//! **The connection lives on a thread of its own.** `rusqlite::Connection` is
//! not `Sync`, and a `Statement` borrows the connection: streaming a result batch
//! by batch requires keeping this borrow alive between two `await`s, which no
//! future can do without `unsafe`. The connection therefore stays on a dedicated
//! thread, and what crosses the channel is already converted `RecordBatch`. The
//! full reasoning is in [`crate::worker`].
//!
//! **Cancellation is local, and it is stated as such.** SQLite has no server:
//! `sqlite3_interrupt` stops a statement in **our** process. The session
//! therefore never declares `SERVER_SIDE_CANCEL`, and `SqliteSession::cancel`
//! refuses — but interruption does exist, through the
//! [`CancelToken`](oxyn_core::CancelToken) handed to `execute`, checked between
//! two batches and watched while waiting for a batch.
//!
//! **A column's type is decided once, from the data.** In SQLite the type
//! belongs to the value, not to the column; Arrow requires the opposite. The
//! first batch is set aside as raw values while deciding, and a column mixing
//! storage classes falls back to text — or to bytes — **declaring it** in the
//! metadata of its field. See [`convert`].
//!
//! **A batch is measured in bytes as much as in rows.** A thousand rows each
//! carrying a one-megabyte BLOB make a gigabyte ([`BatchLimits`]).
//!
//! # Example
//!
//! ```no_run
//! use oxyn_core::{CancelToken, ConnectionConfig, DriverId, Environment, ExecRequest,
//!                 QueryLanguage};
//! use oxyn_driver::{Credentials, Cursor, Driver, Session};
//! use oxyn_driver_sqlite::SqliteDriver;
//!
//! # async fn example() -> oxyn_core::Result<()> {
//! let driver = SqliteDriver::new();
//! let connection = ConnectionConfig::new("workshop", DriverId::sqlite())
//!     .with_environment(Environment::Local)
//!     .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
//!
//! let token = CancelToken::new();
//! let session = driver.connect(&connection, &Credentials::new(), &token).await?;
//!
//! // The default limits are cautious: bounded and read-only.
//! let mut cursor = session
//!     .execute(ExecRequest::new(QueryLanguage::SQL, "SELECT 1 AS n"), &token)
//!     .await?;
//! while let Some(batch) = cursor.next_batch().await? {
//!     println!("{} row(s)", batch.num_rows());
//! }
//! # Ok(())
//! # }
//! ```

#[cfg(test)]
mod busy_batch_tests;
pub mod catalog;
mod constraints;
pub mod convert;
pub mod cursor;
#[cfg(test)]
mod ddl_tests;
mod definition;
pub mod driver;
pub mod error;
mod incoming_keys;
mod interrupt;
pub mod options;
pub mod params;
mod preview;
mod schema_sql;
pub mod session;
#[cfg(test)]
mod stopped_write_tests;
pub mod stream;
mod vector_extension;
#[cfg(test)]
mod vector_tests;
mod virtual_tables;
#[cfg(test)]
mod virtual_tables_tests;
pub mod worker;

pub use catalog::{MAIN, SqliteCatalog, logical_type, partial_predicate};
pub use convert::{
    ColumnKind, METADATA_DECLARED_TYPE, METADATA_INFERRED, METADATA_STORAGE_CLASSES, Observed,
    affinity,
};
pub use cursor::SqliteCursor;
pub use driver::SqliteDriver;
pub use error::{SqliteError, classify};
pub use options::BatchLimits;
pub use session::SqliteSession;

#[cfg(test)]
mod tests;
