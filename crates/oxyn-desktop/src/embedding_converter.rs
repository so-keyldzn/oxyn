//! The embedding model's conversion, in a child process
//! ([ADR-0056](../../../docs/adr/0056-local-cpu-embeddings-for-context-selection.md)).
//!
//! # Why a process of its own
//!
//! Widening the downloaded bf16 weights to the f32 `model.bpk` peaks at about
//! 1.2 GB. The system allocator keeps most of that memory in the process once
//! it is freed: run in Oxyn, the conversion would leave it more than a
//! gigabyte heavier for the rest of the session, for a step that runs once.
//! A child gives it all back by exiting.
//!
//! # The child is Oxyn itself
//!
//! `oxyn-desktop`, relaunched from [`std::env::current_exe`] with
//! [`CONVERT_ARGUMENT`] and the models directory: the same binary in
//! `make desktop-dev` and once installed, on every platform, and no second
//! executable to ship. `main` recognises the argument **before anything
//! else** — no journal, no store, no window — converts, and exits with
//! [`EmbedError::exit_code`]. The parent holds the model directory's lock for
//! the whole download: the child does not take it.
//!
//! # Not a primitive to write anywhere
//!
//! Any process can launch Oxyn with arguments. The child therefore accepts
//! only a directory Oxyn computes itself — [`MODELS_DIRECTORY`] or
//! [`TEMPORARY_MODELS_DIRECTORY`] under the data directory — compared
//! exactly, and refuses anything else before touching the disk. What it then
//! does there is what the download already does: read a file checked on its
//! pinned checksum, write `model.bpk.part`, rename it once verified.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use oxyn_core::CancelToken;
use oxyn_embed::{EmbedError, ModelStore};

/// The directory, next to the local store's file, that holds the models.
pub(crate) const MODELS_DIRECTORY: &str = "models";

/// The temporary workspace's own models directory, beside
/// [`MODELS_DIRECTORY`]: turning the option off in a development session
/// must not delete the installed Oxyn's model.
pub(crate) const TEMPORARY_MODELS_DIRECTORY: &str = "models-temporary-workspace";

/// The internal argument that makes `oxyn-desktop` a conversion child.
pub(crate) const CONVERT_ARGUMENT: &str = "--convert-embedding-model";

/// Variables the child gets from Oxyn's environment, and nothing else
/// (I-03): the ones that decide where the data directory is, so the child
/// computes the same directories as the parent and recognises the one it is
/// given. Paths, never secrets. `SystemRoot` is what a Windows process
/// needs to load system libraries at all.
const PASSED_VARIABLES: &[&str] = &["HOME", "XDG_DATA_HOME", "SystemRoot"];

/// The models directories Oxyn computes from the data directory: the one
/// the saved workspace uses, and the temporary workspace's.
pub(crate) fn models_directories() -> Option<[PathBuf; 2]> {
    let data = oxyn_store::Store::default_path().ok()?;
    let data = data.parent()?;
    Some([
        data.join(MODELS_DIRECTORY),
        data.join(TEMPORARY_MODELS_DIRECTORY),
    ])
}

/// When this process was launched as a conversion child, converts and
/// returns the exit code; `None` otherwise, and `main` goes on.
///
/// Reads the arguments only: called first thing in `main`.
pub(crate) fn run_if_child() -> Option<u8> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next()? != CONVERT_ARGUMENT {
        return None;
    }
    let root = arguments.next();
    let extra = arguments.next().is_some();
    Some(child(root, extra, models_directories().as_ref()))
}

/// The child's work: refuse what is not one of `allowed`, convert there.
fn child(root: Option<OsString>, extra: bool, allowed: Option<&[PathBuf; 2]>) -> u8 {
    match accepted_root(root, extra, allowed) {
        Some(root) => match ModelStore::new(root).convert_in_place() {
            Ok(()) => 0,
            Err(error) => error.exit_code(),
        },
        None => refused().exit_code(),
    }
}

/// The root, if it is exactly one of the directories Oxyn computes and the
/// only argument after [`CONVERT_ARGUMENT`].
fn accepted_root(
    root: Option<OsString>,
    extra: bool,
    allowed: Option<&[PathBuf; 2]>,
) -> Option<PathBuf> {
    if extra {
        return None;
    }
    let root = PathBuf::from(root?);
    allowed?
        .iter()
        .any(|directory| directory == &root)
        .then_some(root)
}

/// Why the child did nothing, as the parent reads it: a conversion failure.
fn refused() -> EmbedError {
    EmbedError::Conversion("the conversion was asked for another directory".to_owned())
}

/// Converts the model under `root` in a child process, and waits for it
/// without blocking a thread.
///
/// `cancel` kills the child; what it left is a `.part` file, removed by the
/// next download or by turning the option off, as for an interrupted
/// download.
///
/// # Errors
/// The child could not start, was killed by a signal, or exited with a code
/// [`EmbedError::from_exit_code`] reads; [`EmbedError::Cancelled`].
pub(crate) async fn convert_in_child(root: PathBuf, cancel: CancelToken) -> Result<(), EmbedError> {
    let executable = std::env::current_exe().map_err(|_| {
        EmbedError::Conversion("Oxyn could not locate its own executable".to_owned())
    })?;
    let arguments = [
        OsString::from(CONVERT_ARGUMENT),
        root.clone().into_os_string(),
    ];
    let child = command(&executable, &arguments, &root)
        .spawn()
        .map_err(|_| EmbedError::Conversion("the conversion process did not start".to_owned()))?;
    wait_or_kill(child, &cancel).await
}

/// Waits for `child`, or kills it when `cancel` fires.
async fn wait_or_kill(
    mut child: tokio::process::Child,
    cancel: &CancelToken,
) -> Result<(), EmbedError> {
    tokio::select! {
        status = child.wait() => {
            let status = status.map_err(|_| {
                EmbedError::Conversion("the conversion process could not be awaited".to_owned())
            })?;
            match status.code() {
                Some(0) => Ok(()),
                // A signal has no code: read as a conversion failure.
                code => Err(EmbedError::from_exit_code(code.unwrap_or(-1))),
            }
        }
        () = cancel.cancelled() => {
            // Killed and reaped: no zombie, no process left converting.
            let _ignored = child.kill().await;
            Err(EmbedError::Cancelled)
        }
    }
}

/// The child's command: `executable` with `arguments`, an environment Oxyn
/// chose, no standard stream, `directory` as its working directory — the
/// models directory.
fn command(executable: &Path, arguments: &[OsString], directory: &Path) -> tokio::process::Command {
    // The repository forbids this call because a child inherits the parent's
    // environment, secrets included (I-03): `env_clear` right below is what
    // makes this site acceptable, as in `oxyn-ai`'s `external/spawn.rs`.
    #[expect(
        clippy::disallowed_methods,
        reason = "the conversion child's single spawn site, which clears the environment it would otherwise leak"
    )]
    let mut command = std::process::Command::new(executable);
    command.args(arguments);
    command.env_clear();
    for name in PASSED_VARIABLES {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    // Not the workspace's directory: nothing there concerns the child.
    command.current_dir(directory);
    // Nothing to read, nothing said: an error is its exit code, and a
    // dependency's message never reaches a journal through it.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing open in a development build.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut command = tokio::process::Command::from(command);
    // A parent that drops the wait — a cancelled task — does not leave the
    // child converting.
    command.kill_on_drop(true);
    command
}

#[cfg(test)]
mod tests;
