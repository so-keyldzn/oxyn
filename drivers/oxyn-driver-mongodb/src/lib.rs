//! The driver for the MongoDB wire protocol.
//!
//! A new protocol, not a product speaking an implemented one: OP_MSG and BSON
//! share nothing with the PostgreSQL, MySQL or SQLite drivers
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)). Products that
//! claim MongoDB compatibility — DocumentDB, Cosmos DB for MongoDB, FerretDB —
//! would be this driver's capabilities, not other crates; none is claimed,
//! since none is tested. The decisions this crate implements are those of
//! [ADR-0063](../../../docs/adr/0063-mongodb-driver-database-commands-in-extended-json.md).
//!
//! # What is in it
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`driver`] | [`MongoDriver`]: connection form, opening, capabilities | DRIVER-CONTRACT §5 |
//! | [`session`] | [`MongoSession`]: one logical session per execution, context, read-only | ADR-0063 |
//! | [`cursor`] | [`MongoCursor`]: server batches to byte-bounded Arrow batches | ADR-0002, I-06 |
//! | [`catalog`] | [`MongoCatalog`]: databases, collections, inferred fields, indexes | ARCHITECTURE §6 |
//! | [`decode`] | the BSON → Arrow table, the sample, the lossless column | DRIVER-CONTRACT §3 |
//! | `command` | the console text to a BSON command: Extended JSON, read strictly | I-10 |
//! | `ejson` | BSON to Extended JSON text, keys in order | ADR-0063 |
//! | `cancel` | `killSessions` on the execution's logical session | DRIVER-CONTRACT §2 |
//! | `error` | codes and labels to the three families | DRIVER-CONTRACT §4 |
//! | `options` | the form to client options, without a connection string | I-03 |
//! | `preview` | a bounded `find`, ordered by ADR-0028's policy | ADR-0020 |
//!
//! # The choices that govern this crate
//!
//! **The console speaks database commands, not JavaScript.** One Extended
//! JSON document, the command the server runs, sent as typed — the only
//! envelope field read is `$db`. `oxyn-query` reads the same tree to classify
//! it by command name.
//!
//! **No retry, ever.** The library's retryable reads and writes are off: a
//! write retried after a cut may apply twice ([I-13](../../../CLAUDE.md#i-13)).
//!
//! **Inferred, declared, lossless.** The columns come from a sample and say
//! so; the canonical Extended JSON column carries every document whole, so a
//! field outside the sample is never lost, and a type outside it ends the
//! stream with an error naming it.
//!
//! **Cancellation is the server's.** Every execution has its own logical
//! session, and the stop is `killSessions` on it; the verdict waits for the
//! server's answer to the stopped command.
//!
//! # Integration tests
//!
//! Everything that needs a server is `#[ignore]` and **fails** without
//! `OXYN_MONGODB_TEST_URL` when run (the CI refuses a skipped suite):
//!
//! ```sh
//! docker run -d --name oxyn-it-mongo -p 127.0.0.1:27018:27017 \
//!   -e MONGO_INITDB_ROOT_USERNAME=root -e MONGO_INITDB_ROOT_PASSWORD=oxyn mongo:8.0.34
//! OXYN_MONGODB_TEST_URL='mongodb://root:oxyn@127.0.0.1:27018/' \
//!   cargo test -p oxyn-driver-mongodb -- --ignored --test-threads=1
//! ```

pub mod catalog;
pub mod cursor;
pub mod decode;
pub mod driver;
pub mod session;

mod cancel;
mod command;
mod ejson;
mod error;
mod options;
mod preview;

/// The tests that need a server. All `#[ignore]`.
#[cfg(test)]
mod integration;

pub use catalog::MongoCatalog;
pub use cursor::MongoCursor;
pub use decode::{
    DOCUMENT_COLUMN, DecodeError, META_BSON_SUBTYPE, META_BSON_TYPE, META_ENCODING, META_INFERRED,
    META_OMITTED_FIELDS, META_SAMPLE_DOCUMENTS, Plan,
};
pub use driver::{MongoDriver, base_capabilities, driver_capabilities, mongodb_metadata};
pub use error::MongoError;
pub use options::{ConnectSpec, DEFAULT_PORT, Mechanism, TlsMode};
pub use session::{DEFAULT_DATABASE, LANGUAGE, MongoSession};
