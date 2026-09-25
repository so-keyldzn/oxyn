//! What crosses the IPC boundary for `Drop…`, `Truncate…` and `Rename…`
//! ([ADR-0042](../../../../docs/adr/0042-revue-sur-place-des-operations-destructrices.md)).
//!
//! The front sends an operation and receives the statement the backend
//! composed; it never composes one. What it sends back to run is that text,
//! which the backend checks is one statement of the announced kind, and the
//! gate then reclassifies like any user SQL.

use oxyn_core::Environment;
use serde::{Deserialize, Serialize};

use crate::ipc::metadata::{Facet, IncomingKeyRow};

/// What the review composes, as the user chose it in the box.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ObjectOperation {
    /// `DROP TABLE|VIEW|MATERIALIZED VIEW`, by the relation's kind.
    Drop { cascade: bool },
    /// `TRUNCATE TABLE`.
    Truncate { cascade: bool },
    /// `ALTER TABLE … RENAME TO`, or `RENAME COLUMN` when `column` is set.
    #[serde(rename_all = "camelCase")]
    Rename {
        column: Option<String>,
        new_name: String,
    },
}

impl ObjectOperation {
    pub(crate) const fn kind(&self) -> OperationKind {
        match self {
            Self::Drop { .. } => OperationKind::Drop,
            Self::Truncate { .. } => OperationKind::Truncate,
            Self::Rename { .. } => OperationKind::Rename,
        }
    }
}

/// The operation a submitted statement must be: what the button said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    Drop,
    Truncate,
    Rename,
}

/// Everything the box shows, read from the configuration and the catalog
/// cache. No secret: the connection's name and environment only (I-03).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectOperationReview {
    /// The one statement that will be submitted, exactly.
    pub sql: String,
    pub connection_name: String,
    /// `production` when the connection never said otherwise (I-02).
    pub environment: Environment,
    /// The unqualified name to type on production: the object's current name,
    /// or the column's.
    pub object_name: String,
    /// `table`, `view`, `materialized_view`.
    pub relation_kind: String,
    pub transactional_ddl: bool,
    /// The engine refuses without `CASCADE`; only then is the box offered.
    pub restrict_dependents: bool,
    pub dependents: Dependents,
}

/// What the box can say about the objects that depend on this one.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Dependents {
    /// A rename, or a view: no foreign key points at it to list.
    NotApplicable,
    /// The session does not declare `INCOMING_FOREIGN_KEYS`.
    NotReported,
    /// The incoming foreign keys, as the cache holds them: `never` read means
    /// the box asks for them before it lets anything run.
    IncomingKeys(Facet<Vec<IncomingKeyRow>>),
}

/// How a submission ended, in the words the box needs to pick its screen.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ObjectOperationOutcome {
    /// Run and committed. The catalog is invalidated by the executor.
    Applied,
    /// The gate holds it: `ApprovalDialog` takes over, and its decision goes
    /// through `decide`.
    #[serde(rename_all = "camelCase")]
    NeedsApproval {
        command: String,
        reason: String,
        preview: Option<crate::ipc::ApprovalPreview>,
    },
    /// The gate, or the host's dialog, refused. Nothing ran.
    Denied { reason: String },
    /// The server refused; its words, as it wrote them.
    Failed { message: String },
    /// Sent, and nobody knows whether it applied: a timeout, a lost
    /// connection, a Stop after sending. Never retried (I-13).
    Ambiguous { message: String },
    /// Nothing was sent — the review's session did not open. May be
    /// submitted again.
    NotSent { message: String },
}
