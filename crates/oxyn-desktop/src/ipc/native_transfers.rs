//! Native IPC contains host capabilities and structured options, never executable/file paths.
use oxyn_core::{
    ConnectionId, NativeGrantId, NativeTransferFormat, NativeTransferOperation, SessionId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedNativeFile {
    pub grant: NativeGrantId,
    pub name: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativePreparationDraft {
    pub connection: ConnectionId,
    pub session: SessionId,
    pub operation: NativeTransferOperation,
    pub format: NativeTransferFormat,
    pub tool: NativeGrantId,
    pub file: NativeGrantId,
    pub no_owner: bool,
    pub no_privileges: bool,
}
