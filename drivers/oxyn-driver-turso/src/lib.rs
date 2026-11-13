//! The driver for Turso Cloud: libSQL over Hrana on HTTP.
//!
//! The decisions this crate implements are those of
//! [ADR-0069](../../../docs/adr/0069-turso-driver-hrana-over-http.md).
//!
//! Turso Cloud is not a SQLite file: it is a server, queried with
//! `POST /v2/pipeline` or, for retained previews, `/v3/cursor`. Its SQL is SQLite's, so the catalog reads
//! `sqlite_master` and the `pragma_*` functions and the types follow SQLite's
//! storage classes, but nothing is shared with `oxyn-driver-sqlite`, and
//! **nothing links `libsql` or `libsqlite3-sys`**: the workspace holds one
//! version of `libsqlite3-sys`
//! ([ADR-0010](../../../docs/adr/0010-contraintes-natives-sqlite.md)) and this
//! crate speaks the protocol itself.
//!
//! # What is in it
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`driver`] | [`TursoDriver`]: the form (URL, token), opening | DRIVER-CONTRACT |
//! | [`session`] | [`TursoSession`]: stateless statements, stop, ping | ADR-0069 |
//! | [`cursor`] | [`TursoCursor`]: the batches of a whole answer | ADR-0002, I-06 |
//! | [`catalog`] | [`TursoCatalog`]: `sqlite_master` and `pragma_*` | ARCHITECTURE §6 |
//! | [`convert`] | Hrana values to Arrow, typed from every value | ADR-0002 |
//! | [`error`] | classification: transient, permanent, ambiguous | DRIVER-CONTRACT §4 |
//! | [`variant`] | what a session declares | ADR-0003 |
//! | `options`, `wire`, `http`, `params`, `refuse`, `preview`, `base64` | parameters, messages, transport, binding, refusals, previews | I-03, I-09, I-10 |
//!
//! # The choices that govern this crate
//!
//! **One statement, one stream.** Ordinary statements close in their pipeline
//! request; retained previews own their stream through EOF or bounded cleanup. No transaction, no `PRAGMA` setting, no
//! `ATTACH`: they would last one request and be forgotten, so they are
//! refused naming the way out.
//!
//! **A stop is a dropped request, and a stopped write is never "cancelled".**
//! Turso has no interrupt call; `SERVER_SIDE_CANCEL` is not declared.
//!
//! **Pipeline answers have a whole-response budget** ([`http::MAX_RESPONSE_BYTES`]).
//! Hrana 3 previews instead stream byte-bounded NDJSON frames into Arrow batches
//! ([ADR-0078](../../../docs/adr/0078-page-keyless-relations-with-isolated-retained-reads.md)).
//!
//! **The token is a secret in the keyring**, sent as a sensitive header,
//! scrubbed from every message, absent from every `Debug`.
//!
//! # Tests
//!
//! Everything runs against a local server that speaks Hrana
//! (`testserver`): no Turso account, no secret.

pub mod catalog;
pub mod convert;
pub mod cursor;
pub mod driver;
pub mod error;
pub mod http;
pub mod session;
pub mod variant;

mod base64;
mod options;
mod params;
mod preview;
mod refuse;
mod streaming;
mod wire;

#[cfg(test)]
mod integration;
/// A local server that speaks Hrana, and the tests that use it.
#[cfg(test)]
mod testserver;

pub use catalog::{MAIN, TursoCatalog, logical_type};
pub use convert::{Batches, Kind, META_DECLARED_TYPE, META_INFERRED, META_STORAGE_CLASSES, Plan};
pub use cursor::TursoCursor;
pub use driver::{TursoDriver, turso_metadata};
pub use error::TursoError;
pub use http::{Http, MAX_RESPONSE_BYTES};
pub use options::{ConnectSpec, TOKEN_FIELD, URL_FIELD};
pub use session::TursoSession;
pub use variant::{LANGUAGE, capabilities};
