//! The driver for the MySQL **protocol** — hence also for MariaDB.
//!
//! There is no `oxyn-driver-mariadb` crate: MariaDB speaks the same protocol,
//! and what sets it apart is read from the server at connection time
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)). The decisions
//! this crate implements are those of
//! [ADR-0050](../../../docs/adr/0050-mysql-driver-on-mysql-async-prepared-first.md).
//!
//! # What is in it
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`driver`] | [`MysqlDriver`]: connection form, opening | DRIVER-CONTRACT |
//! | [`session`] | [`MysqlSession`]: prepared first, read-only, transactions | ADR-0050 §2-9 |
//! | [`cursor`] | [`MysqlCursor`]: the batch stream and its back-pressure | ADR-0002, I-06 |
//! | [`catalog`] | [`MysqlCatalog`]: introspection through `information_schema` | ARCHITECTURE §6 |
//! | [`types`] | the MySQL → Arrow mapping, and its losses | ADR-0050 §10 |
//! | [`decode`] | the server's values to `RecordBatch` | I-09 |
//! | [`variant`] | [`MysqlVariant`]: MySQL or MariaDB, and what the session can do | ADR-0003 |
//! | `connection` | the session's single connection | ADR-0050 §1 |
//! | `cancel` | `KILL QUERY` from a second connection | ADR-0050 §7 |
//! | `params` | bound parameters, the other direction of the type table | DRIVER-CONTRACT §7 |
//!
//! # The choices that govern this crate
//!
//! **Every statement is prepared first.** The server's prepare proves a text is
//! one statement; the text protocol serves only what MySQL cannot prepare
//! (1295), unbound, and split by `oxyn-query` into exactly one statement. The
//! driver never declares `MULTIPLE_STATEMENTS`.
//!
//! **One connection per session.** The current database, a transaction and
//! user variables are the user's; a pool would scatter them. The transaction
//! state is the server's `SERVER_STATUS_IN_TRANS`, never read from the text.
//!
//! **Cancellation reaches the server, and spares the transaction when it can.**
//! `KILL QUERY` goes out from a second connection of the same account while
//! the stream task holds the targeted one; the connection is kept only once
//! the server confirmed the kill on that very statement.
//!
//! **No type fails a result.** A code the driver does not decode becomes bytes
//! marked with the server's type; zero dates are refused naming the column,
//! never turned into `NULL`.
//!
//! **What the library would do behind the driver's back is refused.**
//! `mysql_async` rewrites `:name` into a placeholder, panics decoding the
//! server's internal type codes in the binary protocol, and answers a
//! `LOAD DATA LOCAL INFILE` request only if a handler is set — the driver
//! refuses the first two before they happen and sets no handler.
//!
//! # Integration tests
//!
//! Everything that needs a server is `#[ignore]` and skipped without
//! `OXYN_MYSQL_TEST_URL`:
//!
//! ```sh
//! docker run -d --name oxyn-it-mysql -e MYSQL_ROOT_PASSWORD=oxyn -e MYSQL_DATABASE=t \
//!   -p 127.0.0.1:13306:3306 mysql:8.4
//! OXYN_MYSQL_TEST_URL='mysql://root:oxyn@127.0.0.1:13306/t' \
//!   cargo test -p oxyn-driver-mysql -- --ignored --test-threads=1
//! ```

pub mod catalog;
pub mod cursor;
pub mod decode;
pub mod driver;
pub mod session;
pub mod types;
pub mod variant;

mod cancel;
mod connection;
mod error;
mod options;
mod params;
mod preview;

/// The tests that need a server. All `#[ignore]`.
#[cfg(test)]
mod integration;

pub use catalog::MysqlCatalog;
pub use cursor::MysqlCursor;
pub use decode::{BatchAssembler, DecodeError};
pub use driver::{MysqlDriver, mysql_metadata};
pub use error::MysqlError;
pub use options::{ConnectSpec, DEFAULT_PORT, TlsMode};
pub use session::MysqlSession;
pub use types::{META_FALLBACK, META_MYSQL_TYPE, MyDecoding, schema_for};
pub use variant::{MysqlFlavor, MysqlVariant, base_capabilities, driver_capabilities};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxyn_core::{Capabilities, DriverId, QueryLanguage, SqlDialect};
    use oxyn_driver::DriverRegistry;

    use crate::MysqlDriver;

    /// The path `oxyn-desktop` takes at startup: register, look up, declare.
    #[test]
    fn the_driver_registers_and_announces_what_it_can_do() {
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::new(MysqlDriver::new()))
            .expect("the driver is consistent");
        let driver = registry
            .require(&DriverId::mysql())
            .expect("it has just been registered");
        assert_eq!(driver.metadata().display_name, "MySQL");
        assert!(
            driver
                .capabilities()
                .supports_language(QueryLanguage::Sql(SqlDialect::MySql))
        );
        assert!(
            !driver
                .capabilities()
                .contains(Capabilities::MULTIPLE_STATEMENTS)
        );
    }
}
