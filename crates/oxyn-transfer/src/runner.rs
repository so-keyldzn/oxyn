//! Host-issued capabilities feed a narrow blocking transfer boundary, never a generic launcher.
use crate::files::{Input, Output, SelectedTool, ToolSnapshot};
use crate::postgres::Target;
use crate::{Result, TransferError, TransferSource};
mod prepared;
use prepared::PreparedFile;
pub use prepared::{PreparedTransfer, TransferResult};

use oxyn_core::{
    CancelToken, ConnectionId, DriverId, NativeGrantId, NativeToolInfo, NativeToolRole,
    NativeTransferFormat, NativeTransferOperation, NativeTransferRequest, SessionId, WindowId,
};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_GRANTS_PER_WINDOW: usize = 32;
/// File selections bind to their original native source, not its display label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceBinding {
    /// Original connection.
    pub connection: ConnectionId,
    /// Original live session.
    pub session: SessionId,
}
struct Granted {
    owner: WindowId,
    value: Grant,
}
enum Grant {
    Selected(SelectedTool),
    Tool(Arc<ToolSnapshot>),
    Input(SourceBinding, Input),
    Output(SourceBinding, NativeTransferFormat, Output),
}
/// Engine-neutral capability registry and native adapter boundary.
///
/// File/inspection/preparation/run methods block: the host issues grants after
/// native selection and the executor executes inspection/run on owned workers.
#[derive(Default)]
pub struct TransferRunner {
    grants: Mutex<HashMap<NativeGrantId, Granted>>,
    closed_owners: Mutex<HashSet<WindowId>>,
    closed_sources: Mutex<HashSet<SourceBinding>>,
    closing: AtomicBool,
}
impl std::fmt::Debug for TransferRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferRunner")
            .field("grants", &self.grants.lock().len())
            .finish()
    }
}
impl TransferRunner {
    /// Start with no executable or file authority.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    fn insert(&self, owner: WindowId, value: Grant) -> Result<NativeGrantId> {
        let closed = self.closed_owners.lock();
        if self.closing.load(Ordering::Acquire) || closed.contains(&owner) {
            return Err(TransferError::Grant);
        }
        let mut grants = self.grants.lock();
        if grants.values().filter(|grant| grant.owner == owner).count() >= MAX_GRANTS_PER_WINDOW {
            return Err(TransferError::Unavailable(
                "This window has too many native selections; finish or discard a transfer first",
            ));
        }
        let id = NativeGrantId::new();
        grants.insert(id, Granted { owner, value });
        Ok(id)
    }
    /// Grant only the tool selected by the native host, not a webview path.
    ///
    /// # Errors
    /// Missing/non-regular executable, unsupported adapter or capability capacity.
    pub fn select_tool(
        &self,
        owner: WindowId,
        engine: DriverId,
        role: NativeToolRole,
        path: &Path,
    ) -> Result<NativeGrantId> {
        self.insert(
            owner,
            Grant::Selected(SelectedTool::select(path, engine, role)?),
        )
    }
    /// Grant one regular archive for the exact original source/window.
    ///
    /// # Errors
    /// Non-archive, unreadable file or capability capacity.
    pub fn select_archive(
        &self,
        owner: WindowId,
        source: SourceBinding,
        path: &Path,
    ) -> Result<NativeGrantId> {
        let closed = self.closed_sources.lock();
        if closed.contains(&source) {
            return Err(TransferError::Grant);
        }
        self.insert(owner, Grant::Input(source, Input::select(path)?))
    }
    /// Grant an unused native destination. Existing paths are never overwritten.
    ///
    /// # Errors
    /// Missing/unsafe directory, existing path or capability capacity.
    pub fn select_output(
        &self,
        owner: WindowId,
        source: SourceBinding,
        format: NativeTransferFormat,
        path: &Path,
    ) -> Result<NativeGrantId> {
        let closed = self.closed_sources.lock();
        if closed.contains(&source) {
            return Err(TransferError::Grant);
        }
        self.insert(owner, Grant::Output(source, format, Output::select(path)?))
    }
    /// Inspect a host-issued tool once, behind its human bus command.
    ///
    /// # Errors
    /// Wrong owner, consumed grant or failed role/version/confinement checks.
    pub fn inspect_tool(
        &self,
        owner: WindowId,
        id: NativeGrantId,
        cancel: &CancelToken,
    ) -> Result<NativeToolInfo> {
        let selected = {
            let mut grants = self.grants.lock();
            let grant = grants
                .get(&id)
                .filter(|grant| grant.owner == owner)
                .ok_or(TransferError::Grant)?;
            if let Grant::Tool(tool) = &grant.value {
                return Ok(tool.info.clone());
            }
            if !matches!(grant.value, Grant::Selected(_)) {
                return Err(TransferError::Grant);
            }
            match grants.remove(&id).ok_or(TransferError::Grant)?.value {
                Grant::Selected(selected) => selected,
                _ => return Err(TransferError::Grant),
            }
        };
        let tool = Arc::new(selected.inspect(id, cancel)?);
        let info = tool.info.clone();
        let closed = self.closed_owners.lock();
        if self.closing.load(Ordering::Acquire) || closed.contains(&owner) || cancel.is_cancelled()
        {
            return Err(TransferError::Cancelled);
        }
        self.grants.lock().insert(
            id,
            Granted {
                owner,
                value: Grant::Tool(tool),
            },
        );
        Ok(info)
    }
    /// Revoke selections only after the executor joined this owner's active work.
    pub fn revoke_owner(&self, owner: WindowId) {
        self.closed_owners.lock().insert(owner);
        self.grants.lock().retain(|_, grant| grant.owner != owner);
    }
    /// Revoke one unused selection without affecting another operation/window.
    pub fn revoke(&self, owner: WindowId, id: NativeGrantId) -> Result<()> {
        let mut grants = self.grants.lock();
        if grants.get(&id).is_some_and(|grant| grant.owner != owner) {
            return Err(TransferError::Grant);
        }
        grants.remove(&id);
        Ok(())
    }
    /// Consume a one-use file grant into a fixed prepared plan, without target writes.
    ///
    /// # Errors
    /// Source/mode/ownership/format/version mismatch or non-executing archive preflight failure.
    pub fn prepare(
        &self,
        request: NativeTransferRequest,
        source: &TransferSource,
        cancel: &CancelToken,
        closed: &CancelToken,
    ) -> Result<PreparedTransfer> {
        request.validate().map_err(|_| {
            TransferError::Unavailable(
                "SQL-script restore is unavailable; select a supported native archive",
            )
        })?;
        if request.connection != source.connection || request.session != source.session {
            return Err(TransferError::Grant);
        }
        if request.operation == NativeTransferOperation::Restore && source.read_only {
            return Err(TransferError::Unavailable(
                "Restoration to a read-only connection is refused",
            ));
        }
        let target = Target::from_source(source)?;
        let mut grants = self.grants.lock();
        let tool = match &grants
            .get(&request.tool)
            .filter(|grant| grant.owner == request.owner)
            .ok_or(TransferError::Grant)?
            .value
        {
            Grant::Tool(tool) => Arc::clone(tool),
            _ => return Err(TransferError::Grant),
        };
        let role = match request.operation {
            NativeTransferOperation::Export => NativeToolRole::Export,
            NativeTransferOperation::Restore => NativeToolRole::Restore,
            _ => return Err(TransferError::Grant),
        };
        if tool.info.identity.engine != source.driver || tool.info.identity.role != role {
            return Err(TransferError::Grant);
        }
        target.check_tool(tool.version)?;
        let file = grants
            .get(&request.file)
            .filter(|grant| grant.owner == request.owner)
            .ok_or(TransferError::Grant)?;
        let binding = SourceBinding {
            connection: source.connection,
            session: source.session,
        };
        match &file.value {
            Grant::Input(source, _)
                if request.operation == NativeTransferOperation::Restore && *source == binding => {}
            Grant::Output(source, format, _)
                if request.operation == NativeTransferOperation::Export
                    && *source == binding
                    && *format == request.format => {}
            _ => return Err(TransferError::Grant),
        }
        let file = grants
            .remove(&request.file)
            .ok_or(TransferError::Grant)?
            .value;
        drop(grants);
        let file = match file {
            Grant::Input(_, input) => PreparedFile::Input(input),
            Grant::Output(_, _, output) => PreparedFile::Output(output),
            _ => return Err(TransferError::Grant),
        };
        let mut prepared = PreparedTransfer::new(request, target, tool, file);
        prepared.preflight(cancel, closed)?;
        Ok(prepared)
    }
}

impl TransferRunner {
    /// Release unused files belonging to a retired original session; tools remain reusable.
    pub fn revoke_source(&self, source: SourceBinding) {
        self.closed_sources.lock().insert(source);
        self.grants.lock().retain(|_,grant|!matches!(&grant.value,Grant::Input(bound,_)|Grant::Output(bound,_,_) if *bound==source));
    }
    /// Called only after the executor joined every native worker at shutdown.
    pub fn revoke_all(&self) {
        self.closing.store(true, Ordering::Release);
        self.grants.lock().clear();
    }
}

#[cfg(test)]
mod tests;
