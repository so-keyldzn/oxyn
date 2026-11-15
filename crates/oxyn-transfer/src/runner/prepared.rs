//! One fixed plan owns its files/tool through native execution and finalization.
use crate::files::{Input, Output, ToolSnapshot};
use crate::postgres::Target;
use crate::{Result, TransferError};
use oxyn_core::{
    CancelToken, NativeToolInfo, NativeTransferFailure, NativeTransferOperation,
    NativeTransferOutcome, NativeTransferRequest, NativeTransferStage,
};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

const RUN_DEADLINE: Duration = Duration::from_secs(24 * 60 * 60);
pub(super) enum PreparedFile {
    Input(Input),
    Output(Output),
}
/// A fixed owned plan: it can run once and cannot be redirected by a later UI edit.
pub struct PreparedTransfer {
    request: NativeTransferRequest,
    target: Target,
    tool: Arc<ToolSnapshot>,
    file: PreparedFile,
}
impl std::fmt::Debug for PreparedTransfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedTransfer")
            .field("connection", &self.request.connection)
            .field("session", &self.request.session)
            .field("operation", &self.request.operation)
            .finish_non_exhaustive()
    }
}
/// Sanitized terminal observation; a failed restore is never labelled rolled back.
#[derive(Debug, Clone)]
pub struct TransferResult {
    /// Actual terminal classification.
    pub outcome: NativeTransferOutcome,
    /// Safe diagnostic category.
    pub failure: Option<NativeTransferFailure>,
    /// Observed output bytes.
    pub bytes: u64,
    /// Possible database changes, independent from success.
    pub may_have_changed: bool,
}
impl PreparedTransfer {
    pub(super) fn new(
        request: NativeTransferRequest,
        target: Target,
        tool: Arc<ToolSnapshot>,
        file: PreparedFile,
    ) -> Self {
        Self {
            request,
            target,
            tool,
            file,
        }
    }

    /// Public tool identity, without a path or invocation.
    #[must_use]
    pub fn tool(&self) -> &NativeToolInfo {
        &self.tool.info
    }
    /// File label selected by the human; not a path lookup for execution.
    #[must_use]
    pub fn file_name(&self) -> &str {
        match &self.file {
            PreparedFile::Input(file) => &file.label,
            PreparedFile::Output(file) => &file.label,
        }
    }
    /// Exact database selected on the original live owner.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.target.database
    }
    /// Host-confirmation destination details; no credential or URI.
    #[must_use]
    pub fn destination(&self) -> String {
        format!(
            "database {} on {}:{} as {}",
            self.target.database, self.target.host, self.target.port, self.target.user
        )
    }
    pub(super) fn preflight(&mut self, cancel: &CancelToken, closed: &CancelToken) -> Result<()> {
        let PreparedFile::Input(input) = &mut self.file else {
            return Ok(());
        };
        let directory = tempfile::Builder::new()
            .prefix("oxyn-native-check-")
            .tempdir()
            .map_err(|_| TransferError::File)?;
        let mut command = self.tool.command(directory.path());
        crate::postgres::check_archive_header(&mut input.reader()?)?;
        command
            .arg("--list")
            .arg("--no-password")
            .stdin(Stdio::from(input.reader()?))
            .stdout(Stdio::null());
        let result = crate::process::run(
            command,
            cancel,
            closed,
            Duration::from_secs(60),
            None,
            &mut |_| {},
        )?;
        if result.cancelled {
            return Err(TransferError::Cancelled);
        }
        if result.deadline {
            return Err(TransferError::Deadline);
        }
        if result.code != Some(0) || !result.cleanup {
            return Err(TransferError::Archive);
        }
        Ok(())
    }
    /// Execute once, only after the scheduler rechecked and authorized this plan.
    ///
    /// Blocks and owns cleanup through reap. `progress` receives no stdout/stderr
    /// or values. Callers keep the blocking task owned even if a view disappears.
    #[must_use]
    pub fn run(
        mut self,
        command: oxyn_core::CommandId,
        cancel: &CancelToken,
        closed: &CancelToken,
        progress: &mut dyn FnMut(NativeTransferStage, u64),
    ) -> TransferResult {
        let restore = self.request.operation == NativeTransferOperation::Restore;
        let before = |failure| TransferResult {
            outcome: NativeTransferOutcome::FailedBeforeStart,
            failure: Some(failure),
            bytes: 0,
            may_have_changed: false,
        };
        let directory = match tempfile::Builder::new()
            .prefix("oxyn-native-run-")
            .tempdir()
        {
            Ok(dir) => dir,
            Err(_) => return before(NativeTransferFailure::File),
        };
        let passfile = directory.path().join("password.disabled");
        let empty = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&passfile);
        if empty.is_err() {
            return before(NativeTransferFailure::File);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::set_permissions(&passfile, std::fs::Permissions::from_mode(0o600)).is_err()
            {
                return before(NativeTransferFailure::File);
            }
        }
        let args =
            match self
                .target
                .arguments(&self.request, &passfile, &format!("oxyn-native-{command}"))
            {
                Ok(args) => args,
                Err(_) => return before(NativeTransferFailure::Unavailable),
            };
        let mut invocation = self.tool.command(directory.path());
        invocation.args(args);
        match &mut self.file {
            PreparedFile::Input(file) => match file.reader() {
                Ok(reader) => {
                    invocation.stdin(Stdio::from(reader)).stdout(Stdio::null());
                }
                Err(_) => return before(NativeTransferFailure::Archive),
            },
            PreparedFile::Output(file) => match file.writer() {
                Ok(writer) => {
                    invocation.stdin(Stdio::null()).stdout(Stdio::from(writer));
                }
                Err(_) => return before(NativeTransferFailure::File),
            },
        }
        progress(NativeTransferStage::Running, 0);
        let observed = crate::process::run(
            invocation,
            cancel,
            closed,
            RUN_DEADLINE,
            None,
            &mut |stopping| {
                let bytes = match &self.file {
                    PreparedFile::Output(file) => file.bytes(),
                    _ => 0,
                };
                progress(
                    if stopping {
                        NativeTransferStage::Cancelling
                    } else {
                        NativeTransferStage::Running
                    },
                    bytes,
                );
            },
        );
        let bytes = match &self.file {
            PreparedFile::Output(file) => file.bytes(),
            _ => 0,
        };
        let mut result = match observed {
            Err(TransferError::Tool) => before(NativeTransferFailure::Tool),
            Err(TransferError::Cancelled) => TransferResult {
                outcome: NativeTransferOutcome::Cancelled,
                failure: Some(NativeTransferFailure::Cancelled),
                bytes,
                may_have_changed: false,
            },
            Err(_) => TransferResult {
                outcome: if restore {
                    NativeTransferOutcome::Unknown
                } else {
                    NativeTransferOutcome::Failed
                },
                failure: Some(NativeTransferFailure::Cleanup),
                bytes,
                may_have_changed: restore,
            },
            Ok(status) => {
                let failure = if !status.cleanup {
                    Some(NativeTransferFailure::Cleanup)
                } else if status.deadline {
                    Some(NativeTransferFailure::Deadline)
                } else if status.cancelled {
                    Some(NativeTransferFailure::Cancelled)
                } else if status.code != Some(0) {
                    Some(NativeTransferFailure::Process)
                } else {
                    None
                };
                let outcome = if failure.is_none() {
                    NativeTransferOutcome::Succeeded
                } else if restore {
                    if status.cancelled || status.deadline || !status.cleanup {
                        NativeTransferOutcome::Unknown
                    } else {
                        NativeTransferOutcome::PossiblePartial
                    }
                } else if status.cancelled {
                    NativeTransferOutcome::Cancelled
                } else {
                    NativeTransferOutcome::Failed
                };
                TransferResult {
                    outcome,
                    failure,
                    bytes,
                    may_have_changed: restore,
                }
            }
        };
        if let PreparedFile::Output(file) = &mut self.file {
            if result.outcome == NativeTransferOutcome::Succeeded {
                progress(NativeTransferStage::Finalizing, bytes);
                if file.publish().is_err() {
                    result.outcome = if file.was_published() {
                        NativeTransferOutcome::Unknown
                    } else {
                        NativeTransferOutcome::Failed
                    };
                    result.failure = Some(NativeTransferFailure::File);
                }
            }
            if file.cleanup().is_err() {
                result.outcome = NativeTransferOutcome::Unknown;
                result.failure = Some(NativeTransferFailure::File);
            }
        }
        result
    }
}
