//! What crosses the boundary when files are dropped on the window
//! ([ADR-0041](../../../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md),
//! point 9).
//!
//! Only Rust reads a dropped file, and no command reads a path the front
//! sends: a `.sql` file arrives as its text, and a database file as the value
//! of one form field — its absolute path — that the user still has to submit.

use serde::Serialize;

/// One dropped file, classified and validated by `file_drop.rs`.
///
/// No `Debug`: a `.sql` file is the user's text, and may hold a password
/// written in a `CREATE USER` (I-03).
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DroppedFile {
    /// A `.sql` file's text, to open in a new console. Nothing runs it.
    Sql { name: String, text: String },
    /// A file a registered driver reads: the connection screen is offered,
    /// its `field` filled with `path`. Nothing is created until submitted.
    Database {
        name: String,
        driver: String,
        field: String,
        path: String,
    },
    /// Not opened, and why, in words the user can act on.
    Refused { name: String, reason: String },
}
