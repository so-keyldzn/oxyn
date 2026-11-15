//! No native stdout/stderr, filesystem path or connection string enters an error.
use thiserror::Error;

/// Sanitized native-boundary failures; every message has a recovery category.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TransferError {
    /// A capability or authentication/platform mode has no proven adapter.
    #[error("{0}")]
    Unavailable(&'static str),
    /// A token was missing, consumed, replaced or belonged to another owner.
    #[error(
        "The native file/tool selection is no longer available for this window; select it again"
    )]
    Grant,
    /// A native file could not be safely opened, published or cleaned.
    #[error(
        "Native file access or no-clobber finalization failed; choose an unused destination on a supported filesystem"
    )]
    File,
    /// Inspection never accepts an arbitrary program's raw diagnostic.
    #[error(
        "The selected native tool could not be verified or started; select a supported corrected client"
    )]
    Tool,
    /// Version checks are separate from an archive's untrusted contents.
    #[error(
        "The selected tool version is unsupported or older than the required security baseline"
    )]
    Version,
    /// A non-executing format preflight failed.
    #[error(
        "The selected file is not an archive this native tool can read; SQL scripts are not an archive fallback"
    )]
    Archive,
    /// Cancellation of preparation proves no destination process started.
    #[error("The native operation was cancelled")]
    Cancelled,
    /// No rollback or automatic retry follows a deadline.
    #[error(
        "The native operation reached its deadline; inspect its outcome before starting another transfer"
    )]
    Deadline,
    /// Ownership is retained until cleanup settles.
    #[error(
        "Native process cleanup could not be confirmed; the operation cannot be reported successful"
    )]
    Cleanup,
}
/// Native-boundary results never expose raw diagnostics.
pub type Result<T> = std::result::Result<T, TransferError>;
