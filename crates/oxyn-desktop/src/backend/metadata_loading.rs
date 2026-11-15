//! Loading preferences cross the bus and never rewrite connection parameters.

use super::Backend;
use crate::ipc::IpcError;
use oxyn_core::{Command, CommandId, ConnectionId, MetadataLoadingPolicy, MetadataPolicySnapshot};
use oxyn_exec::Outcome;

impl Backend {
    /// Read one explicit bounded page through its human-owned bus command.
    pub async fn preload_metadata(
        &self,
        id: CommandId,
        request: oxyn_core::MetadataPreloadRequest,
    ) -> Result<oxyn_core::MetadataPreloadProgress, IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::PreloadMetadata {
                    request: Box::new(request),
                },
            )
            .await?
        {
            Outcome::MetadataPreloaded { progress } => Ok(progress),
            _ => Err(IpcError::invalid("Unexpected metadata preload outcome")),
        }
    }

    /// Snapshot only; the IPC boundary separately enforces window ownership.
    pub fn metadata_preload_progress(
        &self,
        id: CommandId,
    ) -> Option<oxyn_core::MetadataPreloadProgress> {
        self.inner.executor.metadata_preload_progress(id)
    }

    /// Read local preferences without opening or introspecting the source.
    pub async fn read_metadata_policy(
        &self,
        id: CommandId,
        connection: ConnectionId,
    ) -> Result<MetadataPolicySnapshot, IpcError> {
        match self
            .local_command_outcome(id, Command::ReadMetadataPolicy { connection })
            .await?
        {
            Outcome::MetadataPolicy { snapshot } => Ok(snapshot),
            _ => Err(IpcError::invalid("Unexpected metadata policy outcome")),
        }
    }

    /// Compare the reviewed revision and save; this starts no preload.
    pub async fn write_metadata_policy(
        &self,
        id: CommandId,
        connection: ConnectionId,
        policy: MetadataLoadingPolicy,
        expected_revision: u64,
    ) -> Result<MetadataPolicySnapshot, IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::WriteMetadataPolicy {
                    connection,
                    policy: Box::new(policy),
                    expected_revision,
                },
            )
            .await?
        {
            Outcome::MetadataPolicy { snapshot } => Ok(snapshot),
            _ => Err(IpcError::invalid("Unexpected metadata policy outcome")),
        }
    }
}

#[cfg(test)]
mod tests;
