//! What kind of installation runs, and whether it can replace itself.
//!
//! Pure: the build, the platform, the executable's path and `$APPIMAGE` are
//! passed in, and so is the test of a folder's write access — the only
//! reading of the disk, done once at launch.

use std::ffi::OsStr;
use std::path::{Component, Path};

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
    /// A macOS `.app` bundle. `writable` when its folder lets Oxyn swap it
    /// without an administrator: only then does quitting install.
    MacBundle { writable: bool },
    /// A Linux AppImage, replaced in place.
    AppImage { writable: bool },
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

    pub(crate) const fn installs_on_quit(&self) -> bool {
        match self {
            Self::MacBundle { writable } | Self::AppImage { writable } => *writable,
            Self::Unsupported(_) => false,
        }
    }
}

/// Classifies the running installation.
///
/// A development build never updates: it would replace a bundle built from
/// another tree. On Linux only an AppImage replaces itself; a deb or an rpm
/// belongs to the package manager. On macOS, an app run from a disk image
/// (`/Volumes`) or translocated by Gatekeeper is never written to on quit.
pub(crate) fn classify(probe: Probe<'_>, writable: impl Fn(&Path) -> bool) -> Installation {
    if probe.debug_build {
        return Installation::Unsupported(DisabledReason::Dev);
    }
    match probe.platform {
        Platform::Linux => match probe.appimage.filter(|path| !path.is_empty()) {
            Some(image) => Installation::AppImage {
                writable: Path::new(image).parent().is_some_and(&writable),
            },
            None => Installation::Unsupported(DisabledReason::PackageManager),
        },
        Platform::MacOs => match bundle_of(probe.executable) {
            Some(bundle) => Installation::MacBundle {
                writable: !read_only_location(bundle) && bundle.parent().is_some_and(&writable),
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
