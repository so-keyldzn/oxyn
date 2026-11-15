//! A map's native metadata reads remain human bus commands on their original owner.
use super::Backend;
use crate::ipc::IpcError;
use oxyn_catalog::schema_map::SchemaMapLoaded;
use oxyn_core::{Command, CommandId, SchemaMapRequest};
use oxyn_exec::Outcome;

impl Backend {
    /// Read one bounded map; no connection opening, row query or automatic sample.
    pub async fn load_schema_map(
        &self,
        id: CommandId,
        request: SchemaMapRequest,
    ) -> Result<SchemaMapLoaded, IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::LoadSchemaMap {
                    request: Box::new(request),
                },
            )
            .await?
        {
            Outcome::SchemaMapLoaded { map } => Ok(map),
            Outcome::Cancelled { .. } => Err(IpcError::invalid(
                "Schema map read was cancelled before a snapshot could be returned",
            )),
            _ => Err(IpcError::invalid("Unexpected schema map outcome")),
        }
    }
}

#[cfg(test)]
mod tests;
