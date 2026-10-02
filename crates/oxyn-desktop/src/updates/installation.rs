//! What kind of installation runs, and whether it can replace itself.
//!
//! Pure: the build, the platform, the executable's path and `$APPIMAGE` are
//! passed in. The one reading of the disk — whether the installation's
//! folder can be written — is left to the caller, on the blocking pool once
//! an update is downloaded: at launch, on the main thread, a slow or
//! unreachable network home would hold the first window back (I-05).

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::ipc::updates::DisabledReason;

/// The platforms Oxyn ships an update for, and the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    MacOs,
    Linux,
    Other,
}

impl Platform {
    pub(crate) const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// What the launch observed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Probe<'a> {
    pub(crate) debug_build: bool,
    pub(crate) platform: Platform,
    pub(crate) executable: &'a Path,
    /// `$APPIMAGE`, which the AppImage runtime sets to the image's own path.
    pub(crate) appimage: Option<&'a OsStr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Installation {
    /// A macOS `.app` bundle. Quitting installs only where `quit_folder`, its
    /// folder, lets Oxyn swap it without an administrator; `None` where it
    /// never can.
    MacBundle { quit_folder: Option<PathBuf> },
    /// A Linux AppImage, replaced in place, on the same terms.
    AppImage { quit_folder: Option<PathBuf> },
    /// Nothing Oxyn replaces itself.
    Unsupported(DisabledReason),
}

impl Installation {
    pub(crate) const fn blocked(&self) -> Option<DisabledReason> {
        match self {
            Self::Unsupported(reason) => Some(*reason),
            Self::MacBundle { .. } | Self::AppImage { .. } => None,
        }
    }

    /// The folder whose write access decides whether quitting installs,
    /// tried by [`folder_is_writable`]. `None`: quitting never installs.
    pub(crate) fn quit_folder(&self) -> Option<&Path> {
        match self {
            Self::MacBundle { quit_folder } | Self::AppImage { quit_folder } => {
                quit_folder.as_deref()
            }
            Self::Unsupported(_) => None,
        }
    }
}

/// Classifies the running installation.
///
/// A development build never updates: it would replace a bundle built from
/// another tree. On Linux only an AppImage replaces itself; a deb or an rpm
/// belongs to the package manager. On macOS, an app run from a disk image
/// (`/Volumes`) or translocated by Gatekeeper is never written to on quit.
pub(crate) fn classify(probe: Probe<'_>) -> Installation {
    if probe.debug_build {
        return Installation::Unsupported(DisabledReason::Dev);
    }
    match probe.platform {
        Platform::Linux => match probe.appimage.filter(|path| !path.is_empty()) {
            Some(image) => Installation::AppImage {
                quit_folder: Path::new(image).parent().map(Path::to_path_buf),
            },
            None => Installation::Unsupported(DisabledReason::PackageManager),
        },
        Platform::MacOs => match bundle_of(probe.executable) {
            Some(bundle) => Installation::MacBundle {
                quit_folder: bundle
                    .parent()
                    .filter(|_| !read_only_location(bundle))
                    .map(Path::to_path_buf),
            },
            // A binary outside a bundle: `cargo run` of a release build.
            None => Installation::Unsupported(DisabledReason::Dev),
        },
        // Windows updates come later (ADR-0051): until then, whoever
        // installed Oxyn there updates it.
        Platform::Other => Installation::Unsupported(DisabledReason::PackageManager),
    }
}

/// `X.app` for `X.app/Contents/MacOS/<binary>`.
fn bundle_of(executable: &Path) -> Option<&Path> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let named = macos.file_name() == Some(OsStr::new("MacOS"))
        && contents.file_name() == Some(OsStr::new("Contents"))
        && bundle.extension() == Some(OsStr::new("app"));
    named.then_some(bundle)
}

/// A mounted disk image, or a copy Gatekeeper runs from a random read-only
/// path until the user moves the app.
fn read_only_location(bundle: &Path) -> bool {
    bundle.starts_with("/Volumes")
        || bundle
            .components()
            .any(|component| component == Component::Normal(OsStr::new("AppTranslocation")))
}

/// Whether Oxyn may create a file in `folder`: tried rather than read from
/// the mode bits, which say nothing of ACLs nor of a read-only mount.
pub(crate) fn folder_is_writable(folder: &Path) -> bool {
    let probe = folder.join(format!(".oxyn-update-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
