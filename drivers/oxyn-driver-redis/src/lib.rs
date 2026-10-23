//! The driver for the Redis serialization protocol (RESP) — hence Redis and
//! Valkey, which speak it ([ADR-0061](../../../docs/adr/0061-redis-driver-speaks-resp3-itself.md)).
//!
//! There is no `oxyn-driver-valkey` crate: Valkey speaks the same protocol,
//! and what differs is read from the server when a session opens
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! # What is in it
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`driver`] | [`RedisDriver`]: connection form, opening | DRIVER-CONTRACT |
//! | [`session`] | [`RedisSession`]: the console, the read-only guard, the database context | ADR-0061 |
//! | [`cursor`] | [`RedisCursor`]: a reply as a stream of batches, and how it stops | ADR-0002, I-06, I-13 |
//! | [`catalog`] | [`RedisCatalog`]: databases, a bounded sample of keys, a key's facts | ARCHITECTURE §6 |
//! | [`commands`] | the server's command table and the intent it proves | ADR-0061, I-02 |
//! | [`shape`] | the columns of a reply, batches bounded in bytes | ADR-0002 |
//! | [`render`] | a reply element as text and bytes, without loss | DRIVER-CONTRACT §3 |
//! | [`resp`] | RESP2 and RESP3, read without trusting a length | I-09 |
//! | [`argv`] | a command line as `redis-cli` reads it; a key written to be read back | I-10 |
//! | [`cancel`] | `CLIENT UNBLOCK`, `SCRIPT KILL`, `FUNCTION KILL` from a second connection | DRIVER-CONTRACT §2 |
//! | [`error`] | the class of an error, from its prefix | DRIVER-CONTRACT §4 |
//! | [`options`] | the connection parameters, never displayable | I-03, ADR-0052 |
//! | [`tls`] | rustls on aws-lc-rs, Mozilla roots or a named CA file | ADR-0052 |
//! | [`connection`] | one connection: transport, `HELLO`, a command and its reply | ADR-0061 |
//! | [`preview`] | a key's read-only preview, as a command | ADR-0020 |
//!
//! # Integration tests
//!
//! Everything that needs a server is `#[ignore]` and skipped without
//! `OXYN_REDIS_TEST_URL`:
//!
//! ```sh
//! docker run -d --name oxyn-it-redis -p 127.0.0.1:16379:6379 redis:8.10.2
//! OXYN_REDIS_TEST_URL='redis://127.0.0.1:16379/0?tls=disable' \
//!   cargo test -p oxyn-driver-redis -- --ignored --test-threads=1
//! ```

pub mod argv;
pub mod cancel;
pub mod capture;
pub mod catalog;
pub mod commands;
pub mod connection;
pub mod cursor;
pub mod driver;
pub mod error;
pub mod options;
pub mod preview;
pub mod render;
pub mod resp;
pub mod session;
pub mod shape;
pub mod tls;

/// The tests that need a server. All `#[ignore]`.
#[cfg(test)]
mod integration;

/// The driver against a fake server that sends what Redis never does.
#[cfg(test)]
mod wire_tests;

/// Captures against a fake server that keeps the stream open.
#[cfg(test)]
mod capture_tests;

/// Captures against a real server. All `#[ignore]`.
#[cfg(test)]
mod capture_integration;

pub use catalog::RedisCatalog;
pub use cursor::RedisCursor;
pub use driver::{DRIVER_ID, RedisDriver, redis_metadata};
pub use error::RedisError;
pub use session::{RedisSession, base_capabilities};
