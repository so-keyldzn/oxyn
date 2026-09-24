//! What crosses the IPC boundary, in both directions.
//!
//! Every type here is **narrower** than the domain type it comes from. The
//! webview is a new input surface ([ADR-0029](../../../docs/adr/0029-interface-tauri-shadcn.md)):
//! whatever reaches JavaScript can be read by a script injected through a cell
//! value, so a configuration's parameters, its secret reference and its bound
//! values never do ([I-03](../../../CLAUDE.md#i-03)).
//!
//! Identifiers do cross, as opaque strings: the front needs them to address a
//! session or a result. The views never render them.

use std::collections::BTreeMap;
use std::fmt;

use oxyn_catalog::{CatalogPath, Field, RelationKind};
use oxyn_core::{Capabilities, ConnectionConfig, Environment, OxynError, Preview, PrivacyTier};
use oxyn_data::CellValue;
use oxyn_driver::{ConnectionField, DriverMetadata, FieldKind};
use serde::{Deserialize, Serialize};

/// An error as the front shows it: the server's words, and whether retrying
/// makes sense.
///
/// `retryable` is data, not a guess made in JavaScript from the message
/// ([I-13](../../../CLAUDE.md#i-13)).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub message: String,
    pub retryable: bool,
}

impl From<OxynError> for IpcError {
    fn from(error: OxynError) -> Self {
        Self {
            retryable: error.is_retryable(),
            message: error.to_string(),
        }
    }
}

impl From<anyhow::Error> for IpcError {
    fn from(error: anyhow::Error) -> Self {
        // `{:#}` keeps the whole context chain on one line: « opening a session
        // on « prod »: password authentication failed » is what a professional
        // needs, the outer context alone is not.
        Self {
            message: format!("{error:#}"),
            retryable: false,
        }
    }
}

impl IpcError {
    /// An error that is not the server's: a malformed request from the front.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
}

/// A database type the connection screen may offer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverChoice {
    pub id: String,
    pub display_name: String,
    pub family: String,
    pub default_port: Option<u16>,
    pub fields: Vec<FormField>,
}

/// One field of a driver's connection form, in the driver's order.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub key: String,
    pub label: String,
    pub kind: FormFieldKind,
    pub required: bool,
    pub secret: bool,
    /// Never set on a secret field: a default password shipped to the webview
    /// is a password in the DOM.
    pub default: Option<String>,
    pub help: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FormFieldKind {
    Text,
    Password,
    Number,
    Bool,
    Choice { options: Vec<String> },
    Path,
}

impl DriverChoice {
    pub fn of(metadata: &DriverMetadata) -> Self {
        Self {
            id: metadata.id.to_string(),
            display_name: metadata.display_name.clone(),
            family: metadata.family.to_string(),
            default_port: metadata.default_port,
            fields: metadata
                .connection_fields
                .iter()
                .map(FormField::of)
                .collect(),
        }
    }
}

impl FormField {
    fn of(field: &ConnectionField) -> Self {
        let kind = match &field.kind {
            FieldKind::Text => FormFieldKind::Text,
            FieldKind::Password => FormFieldKind::Password,
            FieldKind::Number => FormFieldKind::Number,
            FieldKind::Bool => FormFieldKind::Bool,
            FieldKind::Choice(options) => FormFieldKind::Choice {
                options: options.clone(),
            },
            FieldKind::Path => FormFieldKind::Path,
            other => {
                tracing::warn!(kind = %other, field = %field.key, "unknown field kind, sent as text");
                FormFieldKind::Text
            }
        };
        Self {
            key: field.key.clone(),
            label: field.label.clone(),
            kind,
            required: field.required,
            secret: field.is_secret(),
            default: if field.is_secret() {
                None
            } else {
                field.default.clone()
            },
            help: field.help.clone(),
        }
    }
}

/// A connection saved in the workspace, as the list shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedConnection {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub environment: Environment,
    pub read_only: bool,
    /// What may leave the machine when an agent speaks about this connection
    /// ([I-04](../../../CLAUDE.md#i-04)).
    pub privacy_tier: PrivacyTier,
}

impl SavedConnection {
    pub fn of(config: &ConnectionConfig) -> Self {
        Self {
            id: config.id.to_string(),
            name: config.name.clone(),
            driver: config.driver.to_string(),
            environment: config.environment,
            read_only: config.read_only,
            privacy_tier: config.privacy_tier,
        }
    }
}

/// What the user filled in on the connection screen.
///
/// **No `Debug` derive**: `secrets` carries passwords in clear, and the manual
/// implementation below renders the keys only ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDraft {
    pub driver: String,
    pub name: String,
    /// Required from the front, never defaulted here: a missing marking is a
    /// front bug, and silently choosing one would hide it. The domain default
    /// is still `Production` ([I-02](../../../CLAUDE.md#i-02)).
    pub environment: Environment,
    /// Required, like the environment: a default chosen here would hide a
    /// front that forgot to send the user's choice (I-04).
    pub privacy_tier: PrivacyTier,
    pub read_only: bool,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

impl fmt::Debug for ConnectionDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionDraft")
            .field("driver", &self.driver)
            .field("name", &self.name)
            .field("environment", &self.environment)
            .field("privacy_tier", &self.privacy_tier)
            .field("read_only", &self.read_only)
            .field("values", &self.values.keys().collect::<Vec<_>>())
            .field("secrets", &self.secrets.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// A connection with an open session, as the workspace needs it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenConnection {
    pub connection: String,
    pub session: String,
    pub name: String,
    pub driver: String,
    pub environment: Environment,
    pub read_only: bool,
    /// Copied from the configuration when the session opened. A screen that
    /// edits the connection must refresh it, or the assistant keeps speaking
    /// under the old tier ([I-04](../../../CLAUDE.md#i-04)).
    pub privacy_tier: PrivacyTier,
    /// Capability names, as `oxyn-core` spells them. The interface is
    /// conditional on them ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
    pub capabilities: Vec<String>,
    /// The first console's own session. `session` above stays the catalog's
    /// and previews' ([ADR-0015](../../../docs/adr/0015-consoles-independantes.md)).
    pub console: consoles::ConsoleSession,
}

pub fn capability_names(capabilities: Capabilities) -> Vec<String> {
    capabilities
        .iter_names()
        .map(|(name, _)| name.to_owned())
        .collect()
}

/// What the approval dialog shows. The connection **name**, never its id.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalPreview {
    pub statement: String,
    pub connection: String,
    pub estimated_rows: Option<u64>,
}

impl From<Preview> for ApprovalPreview {
    fn from(preview: Preview) -> Self {
        Self {
            statement: preview.statement,
            connection: preview.connection,
            estimated_rows: preview.estimated_rows,
        }
    }
}

/// The answer to a connection attempt.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConnectResponse {
    Open(OpenConnection),
    /// The policy wants a decision first. The configuration stays in the
    /// backend: the front only holds the command id to answer with.
    #[serde(rename_all = "camelCase")]
    Approval {
        command: String,
        reason: String,
        preview: Option<ApprovalPreview>,
    },
}

/// The answer to a connection test.
///
/// A failed test is an answer, not an IPC error: the form shows it in its
/// status bar next to the name, where `Not tested` stood. The message is the
/// driver's, and like every `OxynError` it carries no secret (I-03).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConnectionTest {
    /// A session opened and was closed at once; nothing was saved.
    #[serde(rename_all = "camelCase")]
    Succeeded {
        elapsed_ms: u64,
    },
    /// The server, the network or the configuration refused.
    #[serde(rename_all = "camelCase")]
    Failed {
        message: String,
        /// `ErrorClass::as_str`: `transient`, `permanent` or `ambiguous`.
        class: &'static str,
        retryable: bool,
    },
    Cancelled,
}

/// A column of a result, as the grid header shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

/// The answer to a command the workspace dispatched.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CommandOutcome {
    #[serde(rename_all = "camelCase")]
    Executed {
        result: String,
        columns: Vec<ResultColumn>,
        rows: u64,
        elapsed_ms: u64,
        /// Only an exhausted stream describes a whole result.
        complete: bool,
        cancelled: bool,
        truncated: bool,
    },
    #[serde(rename_all = "camelCase")]
    NeedsApproval {
        command: String,
        reason: String,
        preview: Option<ApprovalPreview>,
    },
    #[serde(rename_all = "camelCase")]
    Denied {
        reason: String,
    },
    CatalogRefreshed,
    #[serde(rename_all = "camelCase")]
    Exported {
        rows: usize,
        bytes: u64,
    },
    Cancelled,
    Done,
}

/// One cell, formatted by `oxyn-data` — the formatter the export relies on.
///
/// Serialized compactly because a page holds thousands of them: `null`, a
/// string, or an object for the two cases the grid draws differently.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Cell {
    Null,
    Text(String),
    #[serde(rename_all = "camelCase")]
    Truncated {
        text: String,
        full_bytes: usize,
    },
    Unrenderable {
        unrenderable: String,
    },
}

impl From<CellValue<'_>> for Cell {
    fn from(value: CellValue<'_>) -> Self {
        match value {
            CellValue::Null => Self::Null,
            CellValue::Text(text) => Self::Text(text.into_owned()),
            CellValue::Truncated { text, full_bytes } => Self::Truncated {
                text: text.into_owned(),
                full_bytes,
            },
            CellValue::Unrenderable { reason } => Self::Unrenderable {
                unrenderable: reason.into_owned(),
            },
            other => Self::Unrenderable {
                unrenderable: format!("unsupported cell: {other:?}"),
            },
        }
    }
}

/// A bounded window of a result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultPage {
    pub offset: usize,
    pub rows: Vec<Vec<Cell>>,
    /// Rows received so far; grows while the stream is running.
    pub total_rows: usize,
    pub complete: bool,
}

/// A catalog level, addressed by its segments rather than a joined string: a
/// relation called `a.b` is legal, and splitting a dotted path would address
/// another object ([I-10](../../../CLAUDE.md#i-10)).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogAddress {
    pub catalog: Option<String>,
    pub namespace: Option<String>,
    pub relation: Option<String>,
}

impl CatalogAddress {
    pub fn of(path: &CatalogPath) -> Self {
        Self {
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: path.relation().map(str::to_owned),
        }
    }

    pub fn to_path(&self) -> Result<CatalogPath, IpcError> {
        CatalogPath::from_levels(
            self.catalog.clone(),
            self.namespace.clone(),
            self.relation.clone(),
        )
        .map_err(|error| IpcError::invalid(error.to_string()))
    }
}

/// A node of the catalog tree, with the children the cache already holds.
///
/// `loaded` distinguishes « no children » from « never asked »: an empty schema
/// and an unexplored one must not look the same.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogNode {
    pub address: CatalogAddress,
    pub name: String,
    /// `catalog`, `namespace`, or the relation kind (`table`, `view`, …).
    pub kind: String,
    pub holds_records: bool,
    pub system: bool,
    pub comment: Option<String>,
    pub loaded: bool,
    /// Read, then invalidated by a DDL sent from Oxyn: stale whatever its age,
    /// and read again when it is on screen (ADR-0022).
    pub stale: bool,
    pub children: Vec<CatalogNode>,
}

impl CatalogNode {
    pub fn relation(
        path: CatalogPath,
        name: &str,
        kind: RelationKind,
        comment: Option<String>,
    ) -> Self {
        Self {
            address: CatalogAddress::of(&path),
            name: name.to_owned(),
            kind: kind.as_str().to_owned(),
            holds_records: kind.holds_records(),
            system: false,
            comment,
            loaded: true,
            stale: false,
            children: Vec::new(),
        }
    }
}

/// A relation's structure, when the cache holds it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationDetail {
    pub name: String,
    pub kind: String,
    pub comment: Option<String>,
    pub estimated_rows: Option<u64>,
    pub size_bytes: Option<u64>,
    pub fields: Vec<RelationField>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationField {
    pub name: String,
    pub position: u32,
    pub logical_type: String,
    pub raw_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub comment: Option<String>,
    pub primary_key: bool,
}

impl From<&Field> for RelationField {
    fn from(field: &Field) -> Self {
        Self {
            name: field.name.clone(),
            position: field.position,
            logical_type: field.logical_type.to_string(),
            raw_type: field.raw_type.clone(),
            nullable: field.nullable,
            default: field.default.clone(),
            comment: field.comment.clone(),
            primary_key: field.is_primary_key,
        }
    }
}

/// An execution event, forwarded to the front as it happens.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionEvent {
    pub command: String,
    #[serde(flatten)]
    pub kind: ExecutionEventKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExecutionEventKind {
    #[serde(rename_all = "camelCase")]
    SchemaReady {
        result: String,
    },
    #[serde(rename_all = "camelCase")]
    BatchReady {
        result: String,
        rows: usize,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        rows: u64,
    },
    #[serde(rename_all = "camelCase")]
    Completed {
        result: String,
        rows: u64,
        elapsed_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        error: String,
        retryable: bool,
    },
    #[serde(rename_all = "camelCase")]
    ApprovalRequested {
        reason: String,
    },
    Cancelled,
    CatalogUpdated,
}

impl ExecutionEventKind {
    pub fn of(event: &oxyn_core::Event) -> Option<Self> {
        use oxyn_core::Event;
        Some(match event {
            Event::SchemaReady { result } => Self::SchemaReady {
                result: result.to_string(),
            },
            Event::BatchReady { result, rows } => Self::BatchReady {
                result: result.to_string(),
                rows: *rows,
            },
            Event::Progress { rows } => Self::Progress { rows: *rows },
            Event::Completed { result, stats, .. } => Self::Completed {
                result: result.to_string(),
                rows: stats.rows,
                elapsed_ms: millis(stats.total_time),
            },
            Event::Failed { error, retryable } => Self::Failed {
                error: error.clone(),
                retryable: *retryable,
            },
            // The preview is not forwarded here: it travels once, with the
            // outcome the front is awaiting, rather than to every subscriber.
            Event::ApprovalRequested { reason, .. } => Self::ApprovalRequested {
                reason: reason.clone(),
            },
            Event::Cancelled => Self::Cancelled,
            Event::CatalogUpdated => Self::CatalogUpdated,
            #[allow(unreachable_patterns)]
            _ => return None,
        })
    }
}

/// Milliseconds, saturating: a duration beyond `u64` milliseconds is not a
/// value the status bar needs to be exact about.
pub fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;
    use oxyn_driver::DriverFamily;

    use super::*;

    #[test]
    fn a_secret_field_never_carries_its_default() {
        let metadata =
            DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
                .with_field(
                    ConnectionField::new("host", "Host", FieldKind::Text).with_default("localhost"),
                )
                .with_field(
                    ConnectionField::new("password", "Password", FieldKind::Password)
                        .with_default("hunter2"),
                );
        let json = serde_json::to_string(&DriverChoice::of(&metadata)).expect("serializable");
        assert!(
            !json.contains("hunter2"),
            "secret default sent to the webview: {json}"
        );
        assert!(json.contains("localhost"));
    }

    #[test]
    fn a_saved_connection_sends_no_parameter() {
        let config = ConnectionConfig::new("prod", DriverId::postgres())
            .with_environment(Environment::Production)
            .with_param("host", "db.internal")
            .with_secret_ref("connection:0123");
        let json = serde_json::to_string(&SavedConnection::of(&config)).expect("serializable");
        assert!(!json.contains("db.internal"), "parameter sent: {json}");
        assert!(
            !json.contains("connection:0123"),
            "secret reference sent: {json}"
        );
    }

    #[test]
    fn a_draft_debug_shows_keys_not_values() {
        let draft: ConnectionDraft = serde_json::from_str(
            r#"{"driver":"postgres","name":"prod","environment":"production","privacyTier":"metadata","readOnly":false,
                "values":{"host":"db.internal"},"secrets":{"password":"hunter2"}}"#,
        )
        .expect("valid draft");
        let rendered = format!("{draft:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("db.internal"), "{rendered}");
        assert!(rendered.contains("password"));
    }

    #[test]
    fn a_draft_without_environment_is_rejected() {
        let draft = serde_json::from_str::<ConnectionDraft>(r#"{"driver":"sqlite","name":"x"}"#);
        assert!(
            draft.is_err(),
            "an unmarked draft must not be silently accepted"
        );
    }

    #[test]
    fn a_hostile_relation_name_keeps_its_dots() {
        let name = r#"users"; DROP TABLE audit; --.x"#;
        let path = CatalogPath::for_relation(Some("db"), Some("public"), name).expect("legal name");
        let address = CatalogAddress::of(&path);
        assert_eq!(address.relation.as_deref(), Some(name));
        assert_eq!(address.to_path().expect("round trip"), path);
    }

    #[test]
    fn cells_serialize_compactly() {
        let cells = vec![
            Cell::Null,
            Cell::Text("42".into()),
            Cell::Truncated {
                text: "abc".into(),
                full_bytes: 9000,
            },
        ];
        assert_eq!(
            serde_json::to_string(&cells).expect("serializable"),
            r#"[null,"42",{"text":"abc","fullBytes":9000}]"#
        );
    }
}

pub mod ai;
pub mod consoles;
pub mod library;
pub mod metadata;
pub mod recovery;
pub mod results;
pub mod settings;
