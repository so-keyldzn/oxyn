//! Dedicated native IPC still traverses the human bus, policy and host-confirmation port.
use super::Backend;
use crate::ipc::IpcError;
use oxyn_core::{
    Actor, Command, CommandId, ConnectionId, NativeGrantId, NativeToolInfo, NativeTransferId,
    NativeTransferJob, NativeTransferOperation, NativeTransferRequest, NativeTransferReview,
    NativeTransferSupport, SessionId, WindowId,
};
use oxyn_exec::Outcome;

impl Backend {
    /// Actual source capabilities; unsupported engines/modes have explicit reasons.
    pub async fn native_transfer_support(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
    ) -> Result<NativeTransferSupport, IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::ReadNativeTransferSupport {
                    connection,
                    session,
                },
            )
            .await?
        {
            Outcome::NativeTransferSupport { support } => Ok(support),
            _ => Err(IpcError::invalid(
                "Native source inspection did not complete",
            )),
        }
    }
    /// Verify one host-issued selection on an owned human worker.
    pub async fn inspect_native_tool(
        &self,
        id: CommandId,
        owner: WindowId,
        grant: NativeGrantId,
    ) -> Result<NativeToolInfo, IpcError> {
        let inspected = self
            .local_command_outcome(id, Command::InspectNativeTool { owner, grant })
            .await;
        match inspected {
            Ok(Outcome::NativeToolInspected { tool }) => Ok(tool),
            other => {
                // Cancellation/capacity/policy may refuse before the runner
                // consumes the host's raw selection. The webview never received
                // this grant, so the host must release it before returning.
                self.revoke_native_selection(CommandId::new(), owner, grant)
                    .await?;
                match other {
                    Err(error) => Err(error),
                    _ => Err(IpcError::invalid("Native tool inspection did not complete")),
                }
            }
        }
    }
    /// Idempotently release one window's unused selection through the bus.
    pub async fn revoke_native_selection(
        &self,
        id: CommandId,
        owner: WindowId,
        grant: NativeGrantId,
    ) -> Result<(), IpcError> {
        match self
            .local_command_outcome(id, Command::RevokeNativeSelection { owner, grant })
            .await?
        {
            Outcome::NativeSelectionRevoked { .. } => Ok(()),
            _ => Err(IpcError::invalid(
                "Native selection cleanup did not complete",
            )),
        }
    }
    /// Prepare fixed files/tool/source/options without modifying a destination.
    pub async fn prepare_native_transfer(
        &self,
        id: CommandId,
        request: NativeTransferRequest,
    ) -> Result<NativeTransferReview, IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::PrepareNativeTransfer {
                    request: Box::new(request),
                },
            )
            .await?
        {
            Outcome::NativeTransferPrepared { review } => Ok(*review),
            _ => Err(IpcError::invalid(
                "Native transfer preparation did not complete",
            )),
        }
    }
    /// Discard one unused plan; it never starts a native program.
    pub async fn discard_native_transfer(
        &self,
        id: CommandId,
        connection: ConnectionId,
        owner: WindowId,
        transfer: NativeTransferId,
    ) -> Result<NativeTransferJob, IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::DiscardNativeTransfer {
                    connection,
                    owner,
                    transfer,
                },
            )
            .await?
        {
            Outcome::NativeTransferUpdated { job } => Ok(*job),
            _ => Err(IpcError::invalid(
                "Native preparation could not be discarded",
            )),
        }
    }
    /// Run only the exact prepared operation. Restores always require the host's review.
    pub async fn start_native_transfer(
        &self,
        id: CommandId,
        connection: ConnectionId,
        owner: WindowId,
        transfer: NativeTransferId,
    ) -> Result<NativeTransferJob, IpcError> {
        let review = self
            .inner
            .executor
            .native_review(connection, owner, transfer)?;
        let operation = if review.job.operation == NativeTransferOperation::Restore {
            Command::RestoreDatabase {
                connection,
                owner,
                transfer,
            }
        } else {
            Command::ExportDatabase {
                connection,
                owner,
                transfer,
            }
        };
        let cancel = self.track(id)?;
        let _writing = self.local_write(&operation);
        let mut outcome = self
            .inner
            .executor
            .dispatch_as(id, Actor::Human, operation, &cancel)
            .await?;
        if matches!(outcome, Outcome::NeedsApproval { .. }) {
            if review.job.operation != NativeTransferOperation::Restore {
                self.inner.executor.reject(id);
                return Err(IpcError::invalid(
                    "This export policy requires a host review that is not implemented; nothing was started",
                ));
            }
            let held = self.inner.executor.approvals().peek(id);
            let Some(confirmation) = super::confirm::text::restore_database(&review) else {
                self.inner.executor.reject(id);
                let _ = self
                    .discard_native_transfer(CommandId::new(), connection, owner, transfer)
                    .await;
                return Err(IpcError::invalid(
                    "The native destination could not be shown completely; nothing was started",
                ));
            };
            let slot = match self.inner.confirmations.reserve() {
                Ok(slot) => slot,
                Err(error) => {
                    self.inner.executor.reject(id);
                    return Err(error);
                }
            };
            let answer =
                tokio::select! {answer=slot.ask(confirmation)=>answer,_=cancel.cancelled()=>false};
            if !answer || self.inner.executor.approvals().peek(id) != held {
                self.inner.executor.reject(id);
                return self
                    .discard_native_transfer(CommandId::new(), connection, owner, transfer)
                    .await;
            }
            outcome = self.inner.executor.approve("human", id, &cancel).await?;
        }
        match outcome {
            Outcome::NativeTransferUpdated { job } => Ok(*job),
            Outcome::Denied { reason, .. } => {
                let _ = self
                    .discard_native_transfer(CommandId::new(), connection, owner, transfer)
                    .await;
                Err(IpcError::invalid(reason))
            }
            _ => Err(IpcError::invalid(
                "Native transfer did not return a settled observation",
            )),
        }
    }
    /// Bounded local history and live progress; no source or process is resumed.
    pub async fn native_transfer_jobs(
        &self,
        id: CommandId,
        connection: ConnectionId,
    ) -> Result<Vec<NativeTransferJob>, IpcError> {
        match self
            .local_command_outcome(id, Command::ReadNativeTransfers { connection })
            .await?
        {
            Outcome::NativeTransfers { jobs } => Ok(jobs),
            _ => Err(IpcError::invalid("Native job history could not be read")),
        }
    }
    /// A human inspected the outcome; the original observation is retained.
    pub async fn reconcile_native_transfer(
        &self,
        id: CommandId,
        connection: ConnectionId,
        transfer: NativeTransferId,
    ) -> Result<NativeTransferJob, IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::ReconcileNativeTransfer {
                    connection,
                    transfer,
                },
            )
            .await?
        {
            Outcome::NativeTransferUpdated { job } => Ok(*job),
            _ => Err(IpcError::invalid("Native outcome could not be reconciled")),
        }
    }
}

#[cfg(test)]
mod tests;
