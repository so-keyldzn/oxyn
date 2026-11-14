//! Typed local commands share cancellation, audit and shutdown ownership.

use super::Backend;
use crate::ipc::IpcError;
use oxyn_core::{Actor, Command, CommandId};
use oxyn_exec::Outcome;

impl Backend {
    pub(super) async fn local_command_outcome(
        &self,
        id: CommandId,
        command: Command,
    ) -> Result<Outcome, IpcError> {
        let cancel = self.track(id)?;
        let _local = self.local_write(&command);
        let outcome = self
            .inner
            .executor
            .dispatch_as(id, Actor::Human, command, &cancel)
            .await?;
        match outcome {
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            Outcome::NeedsApproval { command, .. } => {
                // Local settings have no approval surface: withdraw rather than
                // leaving a command pending behind a completed IPC request.
                let _withdrawn = self.inner.executor.reject(command);
                Err(IpcError::invalid(
                    "The local operation required approval and was withdrawn",
                ))
            }
            outcome => Ok(outcome),
        }
    }
}
