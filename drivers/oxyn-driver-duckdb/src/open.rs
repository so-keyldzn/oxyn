//! Opening a database: one engine per file, and the access policy.
//!
//! # One engine per file in the process
//!
//! A DuckDB database file is held by **one** engine instance: DuckDB locks it
//! against other processes, but the lock is a POSIX record lock, which a
//! second instance opened by the same process does not see. Oxyn opens several
//! sessions on one connection — the catalog's, one per console — and two
//! engines writing the same file from one process would corrupt it with no
//! error anywhere.
//!
//! Every session of a file therefore shares one engine: the first opens it,
//! the others take a new connection to it ([`duckdb::Connection::try_clone`]),
//! exactly what DuckDB documents for concurrency inside a process. The
//! registry keeps the engine alive through an idle *anchor* connection held
//! while a session lives; the last session to close drops it, and the file is
//! released. A file already open under another access mode or policy is
//! refused rather than opened twice.
//!
//! `:memory:` names one private database per configured connection, shared by
//! its sessions — the console's tables are visible in the catalog tree — and
//! gone when the last one closes.
//!
//! # The access policy, set before any statement
//!
//! DuckDB statements reach the file system and the network: `read_csv`,
//! `read_parquet`, `COPY … TO`, `EXPORT DATABASE`, `ATTACH`, `INSTALL`,
//! `LOAD`. The engine is opened with
//!
//! * `autoinstall_known_extensions` and `autoload_known_extensions` off: no
//!   statement fetches or loads an extension on its own;
//! * then, before the session exists, `enable_external_access = false`: no
//!   file outside the database, no network, no extension loaded from disk.
//!   DuckDB refuses to turn it back on while the database runs
//!   (`EnableExternalAccessSetting::OnSet`, v1.5.6), so a statement typed later
//!   cannot undo it.
//!
//! The one opening a connection may grant is a directory
//! ([`crate::DuckDbDriver::FILE_ACCESS`]): it goes to `allowed_directories`
//! before external access is turned off, and SQL may then read and write files
//! under it — and only there. Parquet and JSON are linked into the binary, so
//! reading them needs no extension download.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};

use duckdb::{AccessMode, Config, Connection};
use oxyn_core::{ConnectionId, ErrorClass, OxynError, Result, SqlDialect};
use parking_lot::Mutex;

use crate::error::{self, DuckDbError};

/// What to open.
///
/// The `Debug` is written by hand: a file path is a connection parameter
/// value, which has no place in a log ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct OpenSpec {
    /// The database.
    pub target: OpenTarget,
    /// Opened read-only by the engine.
    pub read_only: bool,
    /// The one directory SQL may read and write files in.
    pub file_access: Option<PathBuf>,
}

impl OpenSpec {
    /// Hides this connection's database path and granted folder from every
    /// engine message the calling thread converts ([`error::redact_on_this_thread`]).
    pub(crate) fn redact_on_this_thread(&self) {
        error::redact_on_this_thread(
            path_text(&self.target),
            self.file_access.as_deref().and_then(Path::to_str),
        );
    }
}

impl std::fmt::Debug for OpenSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSpec")
            .field("target", &self.target)
            .field("read_only", &self.read_only)
            .field("file_access", &self.file_access.is_some())
            .finish()
    }
}

/// The database a session opens.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum OpenTarget {
    /// One in-memory database per configured connection.
    Memory(ConnectionId),
    /// A file, its path as configured.
    File(PathBuf),
}

impl std::fmt::Debug for OpenTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(_) => f.write_str("Memory"),
            Self::File(_) => f.write_str("File(<redacted path>)"),
        }
    }
}

/// The idle connection that keeps a shared engine alive.
pub(crate) struct Anchor {
    connection: Mutex<Connection>,
    spec: OpenSpec,
}

impl std::fmt::Debug for Anchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Anchor").field("spec", &self.spec).finish()
    }
}

/// The engines open in this process, by file.
static ENGINES: LazyLock<Mutex<HashMap<EngineKey, Weak<Anchor>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// What identifies an engine: the file once resolved, or the connection of an
/// in-memory database.
#[derive(Clone, PartialEq, Eq, Hash)]
enum EngineKey {
    Memory(ConnectionId),
    File(PathBuf),
}

/// Opens a connection to the database `spec` names, sharing the engine
/// another session of this process already holds on it.
///
/// Blocking: called on the session's carrier thread, never on the runtime.
///
/// # Errors
/// [`DuckDbError::OpenElsewhere`] if the file is open here under another mode
/// or policy, and those of [`error::open`].
pub(crate) fn connect(spec: &OpenSpec) -> Result<(Connection, Arc<Anchor>)> {
    let key = engine_key(&spec.target);
    let mut engines = ENGINES.lock();
    engines.retain(|_, anchor| anchor.strong_count() > 0);
    if let Some(anchor) = engines.get(&key).and_then(Weak::upgrade) {
        if anchor.spec.read_only != spec.read_only || anchor.spec.file_access != spec.file_access {
            return Err(error::driver(
                DuckDbError::OpenElsewhere,
                ErrorClass::Permanent,
            ));
        }
        let connection = anchor
            .connection
            .lock()
            .try_clone()
            .map_err(|err| error::open(&err, path_text(&spec.target)))?;
        return Ok((connection, anchor));
    }
    let connection = open(spec)?;
    let anchor_connection = connection
        .try_clone()
        .map_err(|err| error::open(&err, path_text(&spec.target)))?;
    let anchor = Arc::new(Anchor {
        connection: Mutex::new(anchor_connection),
        spec: spec.clone(),
    });
    engines.insert(key, Arc::downgrade(&anchor));
    Ok((connection, anchor))
}

/// The configured path as text, to redact it from engine messages.
fn path_text(target: &OpenTarget) -> Option<&str> {
    match target {
        OpenTarget::File(path) => path.to_str(),
        OpenTarget::Memory(_) => None,
    }
}

/// Resolves a file to the name the operating system gives it, so that two
/// spellings of one file share one engine.
///
/// A file that does not exist yet — DuckDB creates it — is resolved through its
/// directory.
fn engine_key(target: &OpenTarget) -> EngineKey {
    match target {
        OpenTarget::Memory(id) => EngineKey::Memory(*id),
        OpenTarget::File(path) => EngineKey::File(resolve(path)),
    }
}

fn resolve(path: &Path) -> PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            let parent = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            std::fs::canonicalize(parent)
                .map_or_else(|_| path.to_path_buf(), |resolved| resolved.join(name))
        }
        _ => path.to_path_buf(),
    }
}

/// Opens a new engine and applies the access policy.
fn open(spec: &OpenSpec) -> Result<Connection> {
    // Before the engine exists: a wrong folder must not leave a new, empty
    // database file behind.
    if spec
        .file_access
        .as_deref()
        .is_some_and(|directory| !directory.is_dir())
    {
        return Err(OxynError::Config(format!(
            "driver `duckdb`: parameter `{}` must name an existing folder",
            crate::DuckDbDriver::FILE_ACCESS
        )));
    }
    let path = path_text(&spec.target);
    let refused = |err: duckdb::Error| error::open(&err, path);
    let mode = if spec.read_only {
        AccessMode::ReadOnly
    } else {
        AccessMode::ReadWrite
    };
    let config = Config::default()
        .access_mode(mode)
        .and_then(|config| config.with("autoinstall_known_extensions", "false"))
        .and_then(|config| config.with("autoload_known_extensions", "false"))
        .map_err(refused)?;
    let connection = match &spec.target {
        OpenTarget::Memory(_) => Connection::open_in_memory_with_flags(config),
        OpenTarget::File(file) => Connection::open_with_flags(file, config),
    }
    .map_err(refused)?;
    restrict(&connection, spec.file_access.as_deref())?;
    Ok(connection)
}

/// Grants the one directory, then turns external access off for good.
fn restrict(connection: &Connection, file_access: Option<&Path>) -> Result<()> {
    let policy = |err: duckdb::Error| error::engine(&err, error::Effect::Mutating);
    if let Some(directory) = file_access {
        let Some(directory) = directory.to_str() else {
            return Err(OxynError::Config(
                "driver `duckdb`: the file access directory is not valid UTF-8".to_owned(),
            ));
        };
        let mut statement = String::from("SET allowed_directories = [");
        oxyn_catalog::literal::push_string_literal(&mut statement, directory, SqlDialect::DuckDb)
            .map_err(|err| OxynError::Config(format!("driver `duckdb`: {err}")))?;
        statement.push(']');
        connection.execute_batch(&statement).map_err(policy)?;
    }
    connection
        .execute_batch("SET enable_external_access = false")
        .map_err(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_never_appears_in_a_debug_rendering() {
        let spec = OpenSpec {
            target: OpenTarget::File(PathBuf::from("/home/someone/clients/acme.duckdb")),
            read_only: true,
            file_access: Some(PathBuf::from("/home/someone/clients")),
        };
        let rendered = format!("{spec:?}");
        assert!(!rendered.contains("someone"), "{rendered}");
    }

    #[test]
    fn two_spellings_of_one_file_share_one_key() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let direct = dir.path().join("shared.duckdb");
        let indirect = dir.path().join(".").join("shared.duckdb");
        assert!(
            engine_key(&OpenTarget::File(direct)) == engine_key(&OpenTarget::File(indirect)),
            "a file not created yet resolves through its directory"
        );
    }
}
