//! The driver for the PostgreSQL **protocol** — hence also for Redshift,
//! TimescaleDB, pgvector and Citus.
//!
//! There is no `oxyn-driver-redshift` crate, and there will not be one. Redshift
//! speaks the PostgreSQL protocol; what sets it apart is a set of capabilities,
//! not one more decoder. The ~30 systems of the vision thus come down to ~14
//! real implementations
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! # What is in it
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`driver`] | [`PostgresDriver`]: connection form, opening | DRIVER-CONTRACT |
//! | [`session`] | [`PostgresSession`]: execute, cancel, probe | ARCHITECTURE §4.1 |
//! | [`cursor`] | [`PostgresCursor`]: the batch stream and its back-pressure | ADR-0002, I-06 |
//! | [`catalog`] | [`PostgresCatalog`]: introspection through `pg_catalog` | ARCHITECTURE §6 |
//! | [`variant`] | [`PostgresVariant`]: what the session can do | ADR-0003 |
//! | [`types`] | the PostgreSQL → Arrow mapping, and its losses | DRIVER-CONTRACT §7 |
//! | [`decode`] | the server's binary to `RecordBatch` | I-09 |
//! | `numeric` | exact decoding of a `NUMERIC` (internal) | DRIVER-CONTRACT §7 |
//!
//! # The five choices that govern this crate
//!
//! **Capabilities are evaluated at connection time.** `version()` and
//! `pg_extension` are queried once, and that is where `VECTOR_SEARCH` or
//! `TIME_SERIES` get enabled — or where Redshift loses `EXPLAIN ANALYZE`. A
//! missing capability means "I cannot do it", never "I will pretend": this
//! driver declares neither `TRANSACTIONS`, nor `MULTIPLE_STATEMENTS`, nor
//! `BULK_LOAD`, because it does not implement them
//! ([`variant::base_capabilities`] says why).
//!
//! **Cancellation reaches the server.** Closing a tab drops the cursor, which
//! cancels its token, which issues `pg_cancel_backend` from a **second**
//! connection — not from the pool, which is precisely saturated when one wants
//! to cancel. An abandoned future releases neither the connection nor the lock
//! ([DRIVER-CONTRACT §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **Nothing is materialized.** The cursor pushes batches sized **in bytes**
//! into a one-slot channel: the task decodes the next batch only once the
//! previous one has been taken. A `SELECT *` over 500 GB therefore does not
//! inflate memory ([I-06](../../../CLAUDE.md#i-06)).
//!
//! **Unknown wire types keep their bytes and PostgreSQL type metadata.**
//! The grid formats Arrow binary values explicitly, without guessing text from
//! bytes that happen to be valid UTF-8. Preview composition requests server text
//! for internal types and OID aliases; user SQL is never rewritten or retried.
//!
//! **The user's SQL is sent as is; Oxyn's concatenates nothing.** The text of a
//! query is neither parsed nor rewritten: that is the feature of a professional
//! tool. The queries the driver composes — introspection, cancellation — are
//! literals with bound parameters ([I-10](../../../CLAUDE.md#i-10)).
//!
//! # Example
//!
//! ```no_run
//! use oxyn_core::prelude::*;
//! use oxyn_driver::{Credentials, Cursor as _, Driver as _, Session as _};
//! use oxyn_driver_postgres::PostgresDriver;
//!
//! # async fn example() -> Result<()> {
//! let driver = PostgresDriver::new();
//!
//! // What is persisted carries no secret.
//! let connection = ConnectionConfig::new("checkout", DriverId::postgres())
//!     .with_param("host", "internal.example")
//!     .with_param("database", "checkout")
//!     .with_param("user", "reader")
//!     .with_environment(Environment::Development);
//!
//! // The password comes from the system keychain, separately.
//! let credentials = Credentials::new().with_password("resolved-at-the-last-moment");
//!
//! let token = CancelToken::new();
//! let session = driver.connect(&connection, &credentials, &token).await?;
//!
//! // The capabilities are those of *this* session: pgvector installed here
//! // says nothing about the neighboring database.
//! if session.capabilities().contains(Capabilities::VECTOR_SEARCH) {
//!     // … the vector search surface has a reason to exist.
//! }
//!
//! let mut cursor = session
//!     .execute(
//!         ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), "SELECT 1")
//!             .with_intent(StatementIntent::Read),
//!         &token,
//!     )
//!     .await?;
//!
//! // The schema is known before the first row.
//! assert_eq!(cursor.schema().fields().len(), 1);
//! while let Some(_batch) = cursor.next_batch().await? {}
//! # Ok(())
//! # }
//! ```
//!
//! # Integration tests
//!
//! Everything that needs a server is marked `#[ignore]`. To run them:
//!
//! ```sh
//! docker run --rm -d -p 5433:5432 -e POSTGRES_PASSWORD=oxyn --name oxyn-pg postgres:17
//! OXYN_PG_TEST_URL='postgres://postgres:oxyn@localhost:5433/postgres?sslmode=disable' \
//!   cargo test -p oxyn-driver-postgres -- --ignored --test-threads=1
//! ```
//!
//! The cancellation tests additionally need a server that accepts
//! `pg_cancel_backend` on its own processes, which is the default.

mod cancel;
pub mod catalog;
pub mod cursor;
pub mod decode;
pub mod driver;
mod lease;
mod preview;
pub mod session;
mod transaction_text;
pub mod types;
pub mod variant;

pub(crate) mod error;
pub(crate) mod numeric;
pub(crate) mod options;
pub(crate) mod render;

/// The `Session::cancel` tests that need a server. All `#[ignore]`.
#[cfg(test)]
mod cancel_tests;
/// The session state a connection carries back to the pool. All `#[ignore]`.
#[cfg(test)]
mod context_tests;
#[cfg(test)]
mod ddl_tests;
#[cfg(test)]
mod definition_tests;
/// The tests that need a server. All `#[ignore]`.
#[cfg(test)]
mod integration;
/// Every rendered type compared with the server's own output. `#[ignore]`.
#[cfg(test)]
mod type_render_tests;

pub use catalog::{PostgresCatalog, logical_type};
pub use cursor::PostgresCursor;
pub use decode::{BatchAssembler, DecodeError};
pub use driver::{PostgresDriver, postgres_metadata};
pub use error::PostgresError;
pub use options::{ConnectSpec, DEFAULT_APPLICATION_NAME, DEFAULT_PORT, LEAKY_ENV};
pub use session::PostgresSession;
pub use types::{META_FALLBACK, META_PG_TYPE, PgDecoding, TextFormat, decoding_for, schema_for};
pub use variant::{PostgresFlavor, PostgresVariant, base_capabilities, driver_capabilities};

#[cfg(test)]
mod tests {
    use oxyn_core::{Capabilities, DriverId, QueryLanguage, SqlDialect};
    use oxyn_driver::DriverRegistry;
    use std::sync::Arc;

    use crate::PostgresDriver;

    /// The driver registers, declares itself, and its declaration holds up.
    ///
    /// It is the only path of the crate that needs no server, and it is the
    /// one `oxyn-desktop` takes at startup.
    #[test]
    fn the_driver_registers_and_announces_what_it_can_do() {
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::new(PostgresDriver::new()))
            .expect("the driver is consistent");

        let driver = registry
            .require(&DriverId::postgres())
            .expect("it has just been registered");

        assert_eq!(driver.metadata().display_name, "PostgreSQL");
        assert!(
            driver
                .capabilities()
                .supports_language(QueryLanguage::Sql(SqlDialect::Postgres))
        );
        assert!(
            driver
                .capabilities()
                .contains(Capabilities::SERVER_SIDE_CANCEL),
            "without this flag, the \"Cancel\" button can promise nothing"
        );
    }

    /// Redshift has no crate of its own: it is the same driver, a different
    /// dialect (ADR-0003).
    #[test]
    fn redshift_goes_through_this_driver_and_no_other() {
        use crate::PostgresVariant;

        let redshift = PostgresVariant::detect(
            "PostgreSQL 8.0.2 on i686-pc-linux-gnu, Redshift 1.0.75008",
            "8.0.2",
            Vec::new(),
        );
        assert_eq!(redshift.dialect(), SqlDialect::Redshift);
        assert!(redshift.capabilities().contains(Capabilities::SQL));
        assert!(
            !redshift
                .capabilities()
                .contains(Capabilities::EXPLAIN_ANALYZE),
            "an \"Execution plan\" panel must not exist here"
        );
    }
}
