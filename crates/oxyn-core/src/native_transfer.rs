//! Engine-neutral native job vocabulary; no paths, credentials or process handles.
use crate::{
    CommandId, ConnectionId, DriverId, NativeGrantId, NativeTransferId, OxynError, Result,
    SessionId, WindowId,
};
use serde::{Deserialize, Serialize};

/// Native file formats are interpreted by the selected engine adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum NativeTransferFormat {
    /// The engine's native structured archive, not an Oxyn format.
    CustomArchive,
    /// Readable SQL export; never automatically treated as an executable restore.
    Sql,
}
/// Export and restore have different authority and possible effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum NativeTransferOperation {
    /// Read the source into an explicitly granted local destination.
    Export,
    /// Possibly modify the exact reviewed destination.
    Restore,
}
/// A tool grant permits one role, never a generic program invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum NativeToolRole {
    /// Export program for the selected engine.
    Export,
    /// Archive restore program for the selected engine.
    Restore,
}
/// Non-secret public tool identity, safe to retain in readable job history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeToolIdentity {
    /// Protocol/adapter identity; variants remain subject to runtime checks.
    pub engine: DriverId,
    /// Permitted role.
    pub role: NativeToolRole,
    /// Canonical adapter-owned tool name, not raw program output.
    pub name: String,
    /// Parsed public version, never an arbitrary diagnostic.
    pub version: String,
}
/// A live tool selection; ephemeral capabilities are never persisted in job history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeToolInfo {
    /// Private host snapshot capability, confined to its original window.
    pub grant: NativeGrantId,
    /// Parsed non-secret identity.
    pub identity: NativeToolIdentity,
}
/// Preparation fixes structured options and source ownership before review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTransferRequest {
    /// Existing source/destination connection.
    pub connection: ConnectionId,
    /// Original live owner; preparing never opens a replacement session.
    pub session: SessionId,
    /// Set by the host, not accepted from a webview's owner claim.
    pub owner: WindowId,
    /// Export or restore; the later execution command must match.
    pub operation: NativeTransferOperation,
    /// Engine-interpreted format.
    pub format: NativeTransferFormat,
    /// Inspected role-bound executable snapshot.
    pub tool: NativeGrantId,
    /// One-use host-selected input/output file grant.
    pub file: NativeGrantId,
    /// Omit source ownership restoration; does not certify an archive's effects.
    pub no_owner: bool,
    /// Omit source privileges restoration; no arbitrary role/SQL argument exists.
    pub no_privileges: bool,
}
impl NativeTransferRequest {
    /// Refuse unresolved SQL restoration before any tool or source is touched.
    ///
    /// # Errors
    /// A structured combination with no implemented safe execution path.
    pub fn validate(&self) -> Result<()> {
        if self.operation == NativeTransferOperation::Restore
            && self.format == NativeTransferFormat::Sql
        {
            return Err(OxynError::NotSupported {
                capability: "SQL script restore requires a separately authorized target-bound flow"
                    .into(),
            });
        }
        Ok(())
    }
}
/// Observable work stages, never a guessed completion percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum NativeTransferStage {
    /// Local capability/file/compatibility validation.
    Preparing,
    /// The exact prepared request awaits the human's host confirmation/start.
    Ready,
    /// Native process is active.
    Running,
    /// Export is synchronized/published without replacement.
    Finalizing,
    /// Cancellation requested; cleanup is not yet confirmed.
    Cancelling,
    /// Terminal observation, not resumable authority; interruption is not process cleanup proof.
    Finished,
}
/// Terminal observations do not infer database rollback from client termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum NativeTransferOutcome {
    /// Tool and required finalization/cleanup reported success.
    Succeeded,
    /// The authority or human refused; execution did not begin.
    Refused,
    /// A local/preflight failure prevented any target execution.
    FailedBeforeStart,
    /// An export failed after startup without a successful published file.
    Failed,
    /// A read-only export was stopped and cleanup completed.
    Cancelled,
    /// A restore returned failure and may have applied part of the archive.
    PossiblePartial,
    /// A started restore or file finalization cannot be authoritatively settled.
    Unknown,
    /// A prior application launch ended without a terminal observation.
    Interrupted,
}
/// Sanitized categories replace raw SQL/data-bearing native diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum NativeTransferFailure {
    /// A capability or platform mode has no proven implementation.
    Unavailable,
    /// The selected tool could not be started/verified.
    Tool,
    /// The native archive could not pass non-executing format preflight.
    Archive,
    /// File access, identity, synchronization or no-clobber publication failed.
    File,
    /// Destination/owner/settings no longer match the prepared request.
    SourceChanged,
    /// Native program reported a nonzero exit.
    Process,
    /// Human cancellation; possible write effects remain separate.
    Cancelled,
    /// Deadline elapsed; not evidence of rollback.
    Deadline,
    /// Process/descendant/pipe cleanup did not settle as required.
    Cleanup,
    /// A previous launch left this job without a terminal observation.
    Interrupted,
}
/// Versioned readable job history; never contains credentials, argv or raw logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTransferJob {
    /// Persisted JSON schema version.
    pub version: u32,
    /// Stable identity across preparation and execution commands.
    pub id: NativeTransferId,
    /// Connection identity, retained in history even after its removal.
    pub connection: ConnectionId,
    /// Source owner at preparation; never reused as a live owner after restart.
    pub session: SessionId,
    /// Original host window; history does not grant another window execution.
    pub owner: WindowId,
    /// Current command, when owned work is active.
    pub command: Option<CommandId>,
    /// Source name as reviewed, not a URI.
    pub connection_name: String,
    /// Native database name; no server connection string is stored.
    pub database: String,
    /// Engine adapter, independent from the progress UI.
    pub engine: DriverId,
    /// Source read or destination restore.
    pub operation: NativeTransferOperation,
    /// Adapter-owned interpretation.
    pub format: NativeTransferFormat,
    /// Reviewed ownership option, retained without executable authority.
    pub no_owner: bool,
    /// Reviewed privilege option, retained without executable authority.
    pub no_privileges: bool,
    /// Inspected public tool name/version; no path or environment.
    pub tool: NativeToolIdentity,
    /// Human-selected file's basename only; never a source row or raw output.
    pub file_name: String,
    /// Observed work phase.
    pub stage: NativeTransferStage,
    /// Terminal result, absent while active/ready.
    pub outcome: Option<NativeTransferOutcome>,
    /// Sanitized failure category, when relevant.
    pub failure: Option<NativeTransferFailure>,
    /// Observed output bytes, not a completion fraction.
    pub bytes: u64,
    /// A restore may have changed its destination, including on interruption.
    pub may_have_changed: bool,
    /// Explicit human reconciliation never rewrites the original outcome.
    pub reconciled: bool,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC terminal timestamp, absent while unsettled.
    pub finished_at: Option<String>,
}
impl NativeTransferJob {
    /// Validate persistence/input bounds without reading any filesystem/source.
    ///
    /// # Errors
    /// An unsupported format revision or unbounded/unreadable identity/timestamp.
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || [
                &self.connection_name,
                &self.database,
                &self.tool.name,
                &self.tool.version,
                &self.file_name,
                &self.created_at,
            ]
            .iter()
            .any(|value| value.len() > 4096)
            || self
                .finished_at
                .as_ref()
                .is_some_and(|value| value.len() > 64)
        {
            return Err(OxynError::Config("invalid native transfer record".into()));
        }
        let finished = self.stage == NativeTransferStage::Finished;
        let role = match self.operation {
            NativeTransferOperation::Export => NativeToolRole::Export,
            NativeTransferOperation::Restore => NativeToolRole::Restore,
        };
        if finished != self.outcome.is_some()
            || finished != self.finished_at.is_some()
            || self.engine != self.tool.engine
            || role != self.tool.role
            || (self.reconciled && !finished)
            || self.created_at.is_empty()
            || self.created_at.len() > 64
            || self.finished_at.as_ref().is_some_and(String::is_empty)
        {
            return Err(OxynError::Config(
                "native transfer stage/outcome mismatch".into(),
            ));
        }
        Ok(())
    }
}
/// Common source capabilities; unsupported engines/modes still have an honest UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTransferSupport {
    /// Adapter/engine display name.
    pub engine: String,
    /// Whether export can be prepared on this original owner.
    pub export: bool,
    /// Whether restore can be prepared under read-only policy and transport.
    pub restore: bool,
    /// Reason for unavailable modes; no raw diagnostic or authentication setting.
    pub reason: Option<String>,
    /// Export formats actually available from this adapter.
    pub export_formats: Vec<NativeTransferFormat>,
    /// Restore formats actually available; SQL is not an archive fallback.
    pub restore_formats: Vec<NativeTransferFormat>,
}

/// Prepared public review; native file/tool capabilities remain in the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTransferReview {
    /// Persistent job identity and non-secret observations.
    pub job: NativeTransferJob,
    /// Exact destination description, not a URI or webview-provided message.
    pub destination: String,
}

#[cfg(test)]
mod tests;
