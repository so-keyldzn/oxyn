//! The updater's errors, sorted into the four families the view knows.
//!
//! Read from the variants of `tauri-plugin-updater` 2.13.1, never from their
//! message: a message changes between releases, a variant does not without a
//! compile error — except for new ones, which the enum's `non_exhaustive`
//! sends to `server`.

use tauri_plugin_updater::Error;

use crate::ipc::updates::UpdateErrorKind;

/// A failure as the state machine records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) kind: UpdateErrorKind,
    pub(crate) message: String,
    pub(crate) retryable: bool,
}

impl Failure {
    /// `error`, classified. Only the network's and the server's failures
    /// are worth trying again; a bad signature stays bad.
    pub(crate) fn of(error: &Error) -> Self {
        let kind = classify(error);
        Self {
            kind,
            message: message(kind, error),
            retryable: matches!(kind, UpdateErrorKind::Offline | UpdateErrorKind::Server),
        }
    }

    /// The manifest points the archive somewhere else than github.com, or
    /// over plain HTTP: refused before any byte is asked for, and not
    /// retried, since the manifest will say the same.
    pub(crate) fn foreign_asset() -> Self {
        Self {
            kind: UpdateErrorKind::Server,
            message:
                "The update manifest points outside Oxyn's GitHub releases. Nothing was downloaded."
                    .into(),
            retryable: false,
        }
    }
}

/// The family of `error`.
pub(crate) fn classify(error: &Error) -> UpdateErrorKind {
    match error {
        Error::Reqwest(request)
            if request.is_connect()
                || request.is_timeout()
                || request.is_request()
                || request.is_body() =>
        {
            UpdateErrorKind::Offline
        }
        Error::Minisign(_)
        | Error::Base64(_)
        | Error::SignatureUtf8(_)
        | Error::SignedVersionMismatch { .. }
        | Error::MissingSignedVersion => UpdateErrorKind::Signature,
        Error::Io(_)
        | Error::TempDirNotOnSameMountPoint
        | Error::BinaryNotFoundInArchive
        | Error::TempDirNotFound
        | Error::AuthenticationFailed
        | Error::DebInstallFailed
        | Error::PackageInstallFailed
        | Error::InvalidUpdaterFormat
        | Error::FailedToDetermineExtractPath
        | Error::Tauri(_) => UpdateErrorKind::Install,
        // A manifest missing, malformed or without this platform, a refused
        // status: github.com answered, not with an update.
        _ => UpdateErrorKind::Server,
    }
}

/// What the view says. The plugin's own message is kept for `install`,
/// whose cause — a permission, a full disk — the user can act on; the
/// others are the journal's.
pub(crate) fn message(kind: UpdateErrorKind, error: &Error) -> String {
    match kind {
        UpdateErrorKind::Offline => "Oxyn could not reach github.com to check for updates.".into(),
        UpdateErrorKind::Server => {
            "github.com did not answer with an update Oxyn could read.".into()
        }
        UpdateErrorKind::Signature => {
            "The downloaded update is not signed by Oxyn's key. It was discarded.".into()
        }
        UpdateErrorKind::Install => format!("The update could not be installed: {error}"),
    }
}
