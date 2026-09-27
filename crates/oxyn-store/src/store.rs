//! Opening the local state and serialized access to SQLite.
//!
//! # A single connection, behind a lock
//!
//! [`Store`] wraps **one** `rusqlite::Connection` in a
//! [`parking_lot::Mutex`]. It is not a poor man's pool: Oxyn's local state is
//! written in small touches — a history row, a journal row, a catalog
//! snapshot — and a pool would only add concurrent writers to a database
//! SQLite serializes on write anyway. The lock makes that serialization
//! visible in the types rather than in an intermittent `SQLITE_BUSY`.
//!
//! **Consequence for the caller: no method of this crate may be called from
//! the UI thread** (I-05). They are synchronous and take a lock; it is up to
//! `oxyn-exec` to carry them onto the blocking pool.
//!
//! # The opening settings, and what they buy
//!
//! | PRAGMA | Value | Why |
//! |---|---|---|
//! | `journal_mode` | `WAL` | a reader no longer blocks a writer: the grid can read the history back while a command is being logged |
//! | `synchronous` | `NORMAL` | WAL's usual companion: durable across a process crash, a recent transaction can be lost on power loss |
//! | `foreign_keys` | on | SQLite ignores them **by default**; without this setting the schema's cascades do not apply |
//! | `busy_timeout` | 5 s | a second Oxyn instance waits rather than failing |
//!
//! `foreign_keys` is the classic trap: the schema declares `REFERENCES` that
//! do strictly nothing until this pragma is set, **per connection**.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use rusqlite::Connection;

use crate::connections::Connections;
use crate::documents::Documents;
use crate::error::{Result, StoreError};
use crate::history::History;
use crate::journal::Journal;
use crate::schema;
use crate::workspaces::Workspaces;

/// Name of the database file under the OS data directory.
pub const DATABASE_FILE_NAME: &str = "oxyn.sqlite3";

/// Maximum wait on a database busy with another process.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Components of the data path, in the `directories` sense.
const APP_QUALIFIER: &str = "dev";
/// Organization, in the `directories` sense.
const APP_ORGANIZATION: &str = "keyldzn";
/// Application, in the `directories` sense.
const APP_NAME: &str = "oxyn";

/// The persistent local state: workspaces, connections, history, audit
/// journal, documents.
///
/// The type is `Send + Sync`: it is shared through `Arc` between the execution
/// bus and background tasks.
///
/// # Example
///
/// ```
/// use oxyn_store::Store;
///
/// let store = Store::open_in_memory()?;
/// let atelier = store.workspaces().create("atelier")?;
/// assert_eq!(store.workspaces().list()?.len(), 1);
/// assert_eq!(atelier.name, "atelier");
/// # Ok::<(), oxyn_store::StoreError>(())
/// ```
pub struct Store {
    /// `None` for an in-memory database.
    path: Option<PathBuf>,
    conn: Mutex<Connection>,
}

impl Store {
    /// Opens the local state at its standard location under the user's data
    /// directory, creating the tree if needed.
    ///
    /// # Errors
    /// * [`StoreError::DataDirUnavailable`] if the system exposes no data
    ///   directory;
    /// * [`StoreError::Io`] if the tree cannot be created;
    /// * the errors of [`Store::open_at`].
    pub fn open_default() -> Result<Self> {
        Self::open_at(Self::default_path()?)
    }

    /// Opens — or creates — the local state at a given path.
    ///
    /// The parent directory is created if needed. Missing migrations are
    /// applied before the method returns: an existing `Store` always has an
    /// up-to-date schema.
    ///
    /// # Errors
    /// [`StoreError::Io`], [`StoreError::Sqlite`],
    /// [`StoreError::SchemaTooRecent`] or [`StoreError::Migration`].
    pub fn open_at(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        Self::from_connection(conn, Some(path))
    }

    /// Opens an in-memory local state, migrated and empty.
    ///
    /// Meant for tests — its own as well as those of the crates depending on
    /// it. Nothing is written to disk, and `journal_mode` stays `memory`: WAL
    /// makes no sense without a file.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] or [`StoreError::Migration`].
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn, None)
    }

    /// Standard path of the local state file for this user.
    ///
    /// # Errors
    /// [`StoreError::DataDirUnavailable`] if the system exposes no data
    /// directory — the case of an environment without a `HOME` variable.
    pub fn default_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
            .ok_or(StoreError::DataDirUnavailable)?;
        Ok(dirs.data_dir().join(DATABASE_FILE_NAME))
    }

    /// Path of the file, or `None` for an in-memory database.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Schema version recorded in the file.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] if the tracking table is unreadable.
    pub fn schema_version(&self) -> Result<u32> {
        self.with_connection(schema::current_version)
    }

    /// The workspaces.
    #[must_use]
    pub fn workspaces(&self) -> Workspaces<'_> {
        Workspaces::new(self)
    }

    /// The configured connections.
    #[must_use]
    pub fn connections(&self) -> Connections<'_> {
        Connections::new(self)
    }

    /// The execution history.
    #[must_use]
    pub fn history(&self) -> History<'_> {
        History::new(self)
    }

    /// The audit trail, append-only.
    #[must_use]
    pub fn journal(&self) -> Journal<'_> {
        Journal::new(self)
    }

    /// Display preferences stored as versioned JSON. Never call on the UI thread.
    #[must_use]
    pub fn preferences(&self) -> crate::preferences::Preferences<'_> {
        crate::preferences::Preferences::new(self)
    }

    /// Application sessions: what tells a clean shutdown from a crash. Never
    /// call from the UI thread.
    #[must_use]
    pub fn sessions(&self) -> crate::sessions::Sessions<'_> {
        crate::sessions::Sessions::new(self)
    }

    /// The window layout ([ADR-0043](../../../docs/adr/0043-multi-fenetre.md)).
    /// Never call from the UI thread.
    #[must_use]
    pub fn windows(&self) -> crate::windows::Windows<'_> {
        crate::windows::Windows::new(self)
    }

    /// The workspace's documents.
    #[must_use]
    pub fn documents(&self) -> Documents<'_> {
        Documents::new(self)
    }

    /// The log of data egress to an AI recipient, append-only. Never call from
    /// the UI thread.
    #[must_use]
    pub fn egress(&self) -> crate::egress::Egress<'_> {
        crate::egress::Egress::new(self)
    }

    /// The assistant's conversations and their transcript. Never call from the
    /// UI thread.
    #[must_use]
    pub fn conversations(&self) -> crate::conversations::Conversations<'_> {
        crate::conversations::Conversations::new(self)
    }

    /// The declared model providers — **per machine**, not per workspace
    /// (ADR-0023). Never call from the UI thread.
    #[must_use]
    pub fn providers(&self) -> crate::providers::Providers<'_> {
        crate::providers::Providers::new(self)
    }

    /// The declared external agents — **per machine**, and **without a
    /// secret** ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
    /// Never call from the UI thread.
    #[must_use]
    pub fn external_agents(&self) -> crate::agents::ExternalAgents<'_> {
        crate::agents::ExternalAgents::new(self)
    }

    /// Configures then migrates a fresh connection.
    fn from_connection(mut conn: Connection, path: Option<PathBuf>) -> Result<Self> {
        Self::configure(&conn)?;
        schema::migrate(&mut conn)?;
        Ok(Self {
            path,
            conn: Mutex::new(conn),
        })
    }

    /// Sets the four opening settings described at the top of the module.
    fn configure(conn: &Connection) -> Result<()> {
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // On an in-memory database, SQLite answers `memory` and ignores the
        // request: that is expected, and not an error.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Boolean — hence the integer 1 — rather than the string `ON`: the
        // pragma accepts both, the integer does not depend on quoting.
        conn.pragma_update(None, "foreign_keys", true)?;
        // Without it, a deleted row stays readable in SQLite's free pages: an
        // answer erased because its exchange received a sample, a purged
        // history, a deleted conversation would all still be in the file for
        // anyone with a hex editor. The cost is a write of zeros per freed
        // page, on a store written in small touches.
        conn.pragma_update(None, "secure_delete", true)?;
        Ok(())
    }

    /// Runs one operation under the lock. Multi-statement writes use a transaction
    /// inside this closure so the connection cannot change between statements.
    pub(crate) fn with_connection<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.conn.lock();
        f(&guard)
    }

    /// Installs cancellation only while this operation owns the connection.
    pub(crate) fn with_connection_cancellable<T>(
        &self,
        cancel: &oxyn_core::CancelToken,
        f: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        let connection = self.conn.lock();
        if cancel.is_cancelled() {
            return Err(StoreError::Cancelled);
        }
        struct ResetProgress<'a>(&'a Connection);
        impl Drop for ResetProgress<'_> {
            fn drop(&mut self) {
                self.0.progress_handler(0, None::<fn() -> bool>);
            }
        }
        let token = cancel.clone();
        connection.progress_handler(1000, Some(move || token.is_cancelled()));
        let _reset = ResetProgress(&connection);
        let result = f(&connection);
        // A successful commit remains successful even if cancellation arrives later.
        if result.is_err() && cancel.is_cancelled() {
            Err(StoreError::Cancelled)
        } else {
            result
        }
    }
}

impl fmt::Debug for Store {
    /// Only shows the path: the content of the local state — statements,
    /// journal, catalog — has no business in a `{store:?}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field(
                "path",
                &self
                    .path
                    .as_deref()
                    .map_or("<in memory>", |p| p.to_str().unwrap_or("<non-UTF-8 path>")),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_in_memory_database_is_migrated_on_open() {
        let store = Store::open_in_memory().expect("open in memory");
        assert_eq!(
            store.schema_version().expect("readable version"),
            schema::latest_version()
        );
        assert!(store.path().is_none());
    }

    #[test]
    fn the_store_is_shared_between_threads() {
        // The type's documentation claims it; the compiler checks it.
        // `rusqlite::Connection` is `Send` but not `Sync`: the `Mutex` is what
        // makes `Store` shareable, and removing it would break this test.
        fn exige_send_sync<T: Send + Sync>() {}
        exige_send_sync::<Store>();
    }

    #[test]
    fn foreign_keys_are_enabled() {
        // SQLite ignores them by default: without this setting, the schema's
        // cascades would be decorative.
        let store = Store::open_in_memory().expect("open");
        let actif: i64 = store
            .with_connection(
                |conn| Ok(conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?),
            )
            .expect("read the pragma");
        assert_eq!(actif, 1);
    }

    #[test]
    fn a_file_is_created_with_its_directory_and_reopens() {
        let racine = tempfile::tempdir().expect("temporary directory");
        let chemin = racine
            .path()
            .join("profond")
            .join("etat")
            .join("oxyn.sqlite3");

        {
            let store = Store::open_at(&chemin).expect("first open");
            store
                .workspaces()
                .create("atelier")
                .expect("workspace creation");
            assert_eq!(store.path(), Some(chemin.as_path()));
        }

        let store = Store::open_at(&chemin).expect("reopen");
        assert_eq!(
            store.schema_version().expect("version"),
            schema::latest_version(),
            "reopening must not replay the migrations"
        );
        assert_eq!(
            store.workspaces().list().expect("list").len(),
            1,
            "the written state must survive closing"
        );
    }

    #[test]
    fn a_file_is_in_wal_mode() {
        let racine = tempfile::tempdir().expect("temporary directory");
        let store = Store::open_at(racine.path().join("oxyn.sqlite3")).expect("open");
        let mode: String = store
            .with_connection(
                |conn| Ok(conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?),
            )
            .expect("read the pragma");
        assert_eq!(mode, "wal");
    }

    #[test]
    fn debug_does_not_show_the_content() {
        let store = Store::open_in_memory().expect("open");
        let rendu = format!("{store:?}");
        assert!(rendu.contains("Store"));
        assert!(
            !rendu.contains("audit_journal"),
            "Debug must say nothing about the content: {rendu}"
        );
    }

    #[test]
    fn a_failing_migration_does_not_leave_the_schema_half_applied() {
        // The migration applies in a transaction; SQLite can roll back DDL,
        // so a refused batch leaves no table behind.
        // The collision is on `documents`, created last: the five previous
        // tables are therefore in place before the failure, and their
        // disappearance is what proves the rollback.
        let mut conn = Connection::open_in_memory().expect("in-memory database");
        conn.execute_batch("CREATE TABLE documents (bloquante INTEGER);")
            .expect("table that will collide");

        let erreur = schema::migrate(&mut conn).expect_err("`documents` already exists");
        assert!(matches!(erreur, StoreError::Migration { version: 1, .. }));
        assert_eq!(
            schema::current_version(&conn).expect("version"),
            0,
            "no migration must be recorded"
        );

        let audit: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'audit_journal'",
                [],
                |row| row.get(0),
            )
            .expect("query the schema");
        assert_eq!(audit, 0, "the batch had to be rolled back in full");
    }

    #[test]
    fn a_new_database_has_no_workspace() {
        let store = Store::open_in_memory().expect("open");
        assert!(
            store.workspaces().list().expect("list").is_empty(),
            "migrating creates the schema, not data"
        );
        assert_eq!(store.journal().count().expect("count"), 0);
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use oxyn_core::CancelToken;

    #[test]
    fn sqlite_scan_is_interrupted_and_the_next_operation_has_no_stale_handler() {
        let store = Store::open_in_memory().expect("store");
        let cancel = CancelToken::new();
        let result = store.with_connection_cancellable(&cancel, |connection| {
            cancel.cancel();
            let _: i64 = connection.query_row(
                "WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<10000000) SELECT sum(n) FROM numbers",
                [], |row| row.get(0),
            )?;
            Ok(())
        });
        assert!(matches!(result, Err(StoreError::Cancelled)));
        let sum = store.with_connection(|connection| {
            Ok(connection.query_row("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<2000) SELECT sum(n) FROM numbers", [], |row| row.get::<_, i64>(0))?)
        }).expect("next scan");
        assert_eq!(sum, 2_001_000);
    }

    #[test]
    fn cancelled_write_rolls_back_but_a_completed_commit_is_not_relabelled() {
        let store = Store::open_in_memory().expect("store");
        store
            .with_connection(|connection| {
                connection.execute_batch("CREATE TABLE cancellation_probe(n INTEGER)")?;
                Ok(())
            })
            .expect("fixture");
        let cancel = CancelToken::new();
        let result = store.with_connection_cancellable(&cancel, |connection| {
            let transaction = connection.unchecked_transaction()?;
            transaction.execute("INSERT INTO cancellation_probe VALUES(0)", [])?;
            cancel.cancel();
            transaction.execute("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<10000000) INSERT INTO cancellation_probe SELECT n FROM numbers", [])?;
            transaction.commit()?;
            Ok(())
        });
        assert!(matches!(result, Err(StoreError::Cancelled)));
        let count = store
            .with_connection(|connection| {
                Ok(
                    connection.query_row("SELECT count(*) FROM cancellation_probe", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                )
            })
            .expect("rolled back");
        assert_eq!(count, 0);
        let cancel = CancelToken::new();
        store
            .with_connection_cancellable(&cancel, |connection| {
                connection.execute("INSERT INTO cancellation_probe VALUES(1)", [])?;
                cancel.cancel();
                Ok(())
            })
            .expect("committed before cancellation");
        let cancelled = store.with_connection_cancellable(&cancel, |_| -> Result<()> {
            panic!("pre-cancelled operation must not start");
        });
        assert!(matches!(cancelled, Err(StoreError::Cancelled)));
    }
}
