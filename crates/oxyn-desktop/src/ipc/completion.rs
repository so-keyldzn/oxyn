//! What the editor's completion reads of one connection's catalog cache.
//!
//! A copy the webview keeps in memory and the completion source reads, so that
//! no IPC is sent on a keystroke
//! ([ADR-0057](../../../../docs/adr/0057-editor-completion-reads-the-catalog-cache.md)).
//! It holds names, kinds and types — never a row — and nothing that reaches
//! `oxyn-ai`: the assistant has its own gateway ([I-04](../../../../CLAUDE.md#i-04)).
//!
//! Every name here is the server's word, hence hostile data
//! ([SECURITY](../../../../docs/SECURITY.md)): the front displays it as text and
//! inserts the `insert` form beside it, never a form it composed itself.

use serde::{Deserialize, Serialize};

/// The cache of one connection, as completion sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionCatalog {
    /// The databases or schemas of the connection with what the cache holds of
    /// their relations. The catalog level is not offered: a PostgreSQL
    /// connection sees one database, and MySQL and SQLite have none.
    pub namespaces: Vec<CompletionNamespace>,
    /// Where an unqualified name resolves.
    pub resolution: CompletionResolution,
    /// How a name typed without quotes is compared with the names above.
    pub folding: CompletionFolding,
    /// Present when a bound cut the answer: a partial answer says so.
    pub truncated: Option<CompletionTruncation>,
}

/// How a name typed without quotes is compared with the names of the cache.
///
/// A quoted name is always compared exactly. The rule of the others is the
/// server's, so it is the host that says it, from the dialect: a second rule in
/// the webview would diverge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletionFolding {
    /// Folded to lowercase, then compared exactly — PostgreSQL and Redshift:
    /// `users` is `users`, never `"Users"`, which only a quoted name reaches.
    Lower,
    /// Compared without regard to case — the others. MySQL's rule for table
    /// names depends on a server setting and a file system the cache does not
    /// hold, and SQLite ignores case; a name written exactly as typed wins when
    /// two differ by case alone.
    Insensitive,
}

/// One level of qualification — a PostgreSQL schema, a MySQL or SQLite
/// database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionNamespace {
    /// `None` for relations that live at the root of a source without
    /// namespaces.
    pub name: Option<String>,
    /// The form to insert for the name, quoted when it must be.
    pub insert: Option<String>,
    /// Collapsed by the tree and suggested only after its qualifier.
    pub system: bool,
    /// Read, then invalidated by a DDL: still suggested, and read again.
    pub stale: bool,
    /// `false` when its relations were never listed — « not asked », which an
    /// empty `relations` would otherwise pass for « none ».
    pub listed: bool,
    pub relations: Vec<CompletionRelation>,
}

/// A table, a view, or any other relation the cache lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionRelation {
    pub name: String,
    /// The form to insert for the name alone.
    pub insert: String,
    /// The form to insert when the namespace is part of it.
    pub qualified: String,
    /// `table`, `view`, … as the catalog names the kind.
    pub kind: String,
    /// The description was read, then invalidated by a DDL.
    pub stale: bool,
    pub virtual_table: bool,
    pub shadow: bool,
    pub fields: CompletionFields,
}

/// What the cache knows of a relation's columns — three answers, not two: an
/// empty list of loaded fields and a relation nobody described must not look
/// alike.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CompletionFields {
    /// Never described: the columns are unknown.
    NotLoaded,
    Loaded {
        items: Vec<CompletionField>,
    },
    /// Described, and cut by a bound: `count` columns are left out.
    Omitted {
        count: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionField {
    pub name: String,
    pub insert: String,
    /// The type as the server spells it.
    pub data_type: String,
}

/// The namespaces an unqualified name resolves in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CompletionResolution {
    /// Never read, the source cannot say, or the answer is not one the snapshot
    /// can carry whole — a path of more entries than the bound, or an entry
    /// that hides text or is too long. A path with an entry missing would let
    /// a later namespace pass for the first match. Nothing is suggested
    /// unqualified.
    Unknown,
    /// First match first. May be empty: MySQL with no database selected
    /// resolves nothing, which is an answer.
    Known {
        namespaces: Vec<String>,
        /// Read, then invalidated by a DDL.
        stale: bool,
    },
}

/// What a bound left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionTruncation {
    /// Namespaces left out, with the relations they hold counted below. One
    /// that holds nothing in the cache counts all the same: an unqualified name
    /// may resolve in a namespace the snapshot does not carry.
    pub namespaces_left_out: u64,
    pub relations_left_out: u64,
    pub fields_left_out: u64,
}

/// One part of a name the editor lexed in the statement: `public` and
/// `Order Lines` in `public."Order Lines"`.
///
/// What the webview sends: compared with the names of the cache, never
/// composed into SQL nor into the path of a read, so a hostile script gains
/// nothing but a read the cache would have made for a real name
/// ([I-10](../../../../CLAUDE.md#i-10)).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CompletionNamePart {
    pub text: String,
    /// Written between identifier quotes: compared exactly, case included.
    pub quoted: bool,
}

/// What one pass of the editor read — counts only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionLoad {
    /// Reads submitted, successful or not. Zero: nothing was missing.
    pub attempted: u32,
    /// Reads that failed or were refused.
    pub failed: u32,
    /// Why the pass ended before it was done, if it did: `deadline` or
    /// `cancelled` (a newer pass took its place).
    pub stopped: Option<&'static str>,
}
