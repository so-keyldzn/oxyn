//! The driver for ClickHouse, over its HTTP interface.
//!
//! The decisions this crate implements are those of
//! [ADR-0060](../../../docs/adr/0060-clickhouse-driver-over-http-rowbinary.md).
//!
//! # What is in it
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`driver`] | [`ClickhouseDriver`]: connection form, opening, probes | DRIVER-CONTRACT |
//! | [`session`] | [`ClickhouseSession`]: stateless requests, read-only, context | ADR-0060 |
//! | [`stream`] | the task that runs a statement, and stops it with `KILL QUERY` | DRIVER-CONTRACT §2, §4 |
//! | [`cursor`] | [`ClickhouseCursor`]: the batches, under back-pressure | ADR-0002, I-06 |
//! | [`catalog`] | [`ClickhouseCatalog`]: introspection through `system` tables | ARCHITECTURE §6 |
//! | [`types`] | ClickHouse type names, parsed | I-09 |
//! | [`schema`] | the ClickHouse → Arrow mapping, and its losses | ADR-0060 |
//! | [`measure`], [`decode`] | `RowBinary` rows, measured whole then decoded | I-06, I-09 |
//! | [`body`] | the response body, and the exception it may carry | ADR-0060 |
//! | [`error`] | classification: transient, permanent, ambiguous | DRIVER-CONTRACT §4 |
//! | [`variant`] | what the server and the account allow | ADR-0003 |
//! | `options`, `http`, `fetch`, `refuse`, `preview` | parameters, transport, driver reads, refusals, previews | I-03, I-10 |
//!
//! # The choices that govern this crate
//!
//! **One statement, one request, no server session.** Everything that decides
//! a statement's meaning travels with it; a typed `USE`, `SET` or
//! `CREATE TEMPORARY TABLE` is refused, since it would change nothing past
//! its own request. The server refuses a second statement in a request.
//!
//! **Types are the server's.** Results arrive as `RowBinaryWithNamesAndTypes`:
//! the exact ClickHouse type of every column, then the rows. No integer
//! becomes a float, no instant gets a zone the server did not give. A type
//! whose wire layout the driver does not know is refused naming its column,
//! since the wire gives no length to skip it.
//!
//! **A stop reaches the server, and a stopped write is never "cancelled".**
//! `KILL QUERY` on the driver's own `query_id`; ClickHouse has no transaction,
//! so a stopped statement that may have written ends on an unknown outcome.
//!
//! # Integration tests
//!
//! Everything that needs a server is `#[ignore]` and skipped without
//! `OXYN_CLICKHOUSE_TEST_URL`:
//!
//! ```sh
//! docker run -d --name oxyn-it-clickhouse -e CLICKHOUSE_USER=oxyn \
//!   -e CLICKHOUSE_PASSWORD=oxyn -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
//!   -p 127.0.0.1:18123:8123 clickhouse/clickhouse-server:26.3.59.10
//! OXYN_CLICKHOUSE_TEST_URL='http://oxyn:oxyn@127.0.0.1:18123' \
//!   cargo test -p oxyn-driver-clickhouse -- --ignored --test-threads=1
//! ```

pub mod body;
pub mod catalog;
pub mod cursor;
pub mod decode;
pub mod driver;
pub mod error;
pub mod measure;
pub mod schema;
pub mod session;
pub mod stream;
pub mod types;
pub mod variant;

mod fetch;
mod http;
mod options;
mod params;
mod preview;
mod refuse;

/// The tests that need a server. All `#[ignore]`.
#[cfg(test)]
mod integration;

pub use catalog::ClickhouseCatalog;
pub use cursor::ClickhouseCursor;
pub use decode::RowDecoder;
pub use driver::{ClickhouseDriver, clickhouse_metadata};
pub use error::{ClickhouseError, ServerError, classify};
pub use measure::{DecodeError, MAX_ROW_BYTES, measure};
pub use options::{ConnectSpec, HTTP_PORT, HTTPS_PORT, TlsMode};
pub use schema::{Column, META_CLICKHOUSE_TYPE, logical_type, schema_for};
pub use session::ClickhouseSession;
pub use types::{ChType, TypeSyntaxError};
pub use variant::{LANGUAGE, ServerFacts, base_capabilities, driver_capabilities};
