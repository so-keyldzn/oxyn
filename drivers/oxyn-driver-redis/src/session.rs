//! A session: one console connection, one for the catalog, and what the
//! server said about itself when it opened.
//!
//! # Two connections, two states
//!
//! `SELECT` is connection state. The console's connection is on the database
//! the session context names; the catalog lists every database on a
//! connection of its own, so browsing the tree never moves the database the
//! user's next command addresses. A preview reads on a short-lived connection
//! opened on the previewed key's database, for the same reason.
//!
//! # What a read-only execution means here
//!
//! Redis has no read-only session: nothing on the server refuses a write on
//! a connection the way PostgreSQL's `READ ONLY` transaction does, and the
//! session does not declare [`Capabilities::READ_ONLY_SESSION`]. When an
//! execution must not write — a production read, a connection marked
//! read-only —, the driver sends only a command the server's own table flags
//! `readonly` and keeps out of `@dangerous`; `EVAL_RO` and `FCALL_RO` are the
//! scripts that qualify, and inside them the server does refuse writes. Every
//! other command is refused before it is sent.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    CancelToken, Capabilities, DriverId, ExecRequest, OxynError, PreviewShape, Result, ScalarValue,
    StatementHandle, StatementIntent,
};
use oxyn_driver::{Cursor, Session, SessionContext};
use parking_lot::Mutex;
use tokio::sync::Mutex as AsyncMutex;
use tokio_rustls::rustls::ClientConfig;

use crate::argv;
use crate::cancel::{Executions, Running, Stopper};
use crate::catalog::RedisCatalog;
use crate::commands::{self, CommandTable};
use crate::connection::{Connection, Hello, INTERNAL_REPLY_BUDGET};
use crate::cursor::{Lease, Plan, RedisCursor};
use crate::driver::{driver_id, require_standalone};
use crate::error::during_connect;
use crate::options::ConnectSpec;

/// The capabilities every Redis session has, before the server is asked.
#[must_use]
pub fn base_capabilities() -> Capabilities {
    Capabilities::REDIS_COMMAND
        | Capabilities::KEY_VALUE
        | Capabilities::SCHEMALESS
        | Capabilities::STREAMING
        | Capabilities::DML
        | Capabilities::GRANT_REVOKE
        | Capabilities::SESSION_CONTEXT
        | Capabilities::ROW_COUNT_ESTIMATE
}

/// What the cursors, the catalog and the session share.
pub struct Shared {
    pub(crate) driver: DriverId,
    pub(crate) spec: Arc<ConnectSpec>,
    pub(crate) tls: Option<Arc<ClientConfig>>,
    pub(crate) hello: Hello,
    pub(crate) commands: CommandTable,
    pub(crate) stopper: Stopper,
    pub(crate) can_stop: bool,
    pub(crate) executions: Executions,
    console: Arc<AsyncMutex<Option<Connection>>>,
    pub(crate) catalog: Arc<AsyncMutex<Option<Connection>>>,
    /// The database the session context selected; `None` is the form's.
    context: Mutex<Option<u32>>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("spec", &self.spec)
            .field("hello", &self.hello)
            .field("commands", &self.commands.len())
            .field("can_stop", &self.can_stop)
            .finish_non_exhaustive()
    }
}

impl Shared {
    /// Opens a connection of the session's account, on `database`.
    ///
    /// # Errors
    /// Those of [`Connection::open`], and the server's refusal of the
    /// database.
    pub(crate) async fn open(&self, database: u32, cancel: &CancelToken) -> Result<Connection> {
        let mut connection = Connection::open(&self.spec, self.tls.as_ref(), cancel).await?;
        if connection.database() != database {
            connection
                .select(database)
                .await
                .map_err(|error| during_connect(&error))?;
        }
        Ok(connection)
    }

    /// Makes `slot` hold a connection on `database`, opening or moving it.
    ///
    /// # Errors
    /// As [`Shared::open`].
    pub(crate) async fn ready(
        &self,
        slot: &mut Option<Connection>,
        database: u32,
        cancel: &CancelToken,
    ) -> Result<()> {
        // A reply left half read — its reader's future dropped by a deadline
        // or a closed tab — would answer the next command: that connection is
        // closed, never reused.
        if slot
            .as_ref()
            .is_some_and(|connection| !connection.in_step())
        {
            *slot = None;
        }
        match slot {
            Some(connection) if connection.database() == database => Ok(()),
            Some(connection) => {
                if let Err(error) = connection.select(database).await {
                    *slot = None;
                    return Err(OxynError::Query(error.to_string()));
                }
                Ok(())
            }
            None => {
                *slot = Some(self.open(database, cancel).await?);
                Ok(())
            }
        }
    }

    /// The database the console addresses.
    pub(crate) fn console_database(&self) -> u32 {
        self.context.lock().unwrap_or(self.spec.database())
    }
}

/// An open Redis session.
#[derive(Debug)]
pub struct RedisSession {
    shared: Arc<Shared>,
    catalog: RedisCatalog,
    capabilities: Capabilities,
}

impl RedisSession {
    /// Opens the console connection, reads the server's commands, and checks
    /// whether this account may stop a blocked command.
    ///
    /// # Errors
    /// Those of [`Connection::open`]; [`OxynError::NotSupported`] for a
    /// cluster or a sentinel.
    pub async fn open(
        spec: Arc<ConnectSpec>,
        read_only: bool,
        cancel: &CancelToken,
    ) -> Result<Self> {
        let tls =
            crate::tls::client_config(spec.tls(), spec.ca_file().map(std::path::PathBuf::as_path))
                .await?;
        let mut console = Connection::open(&spec, tls.as_ref(), cancel).await?;
        let hello = console.hello().clone();
        require_standalone(&hello.mode)?;
        let commands = match console
            .call(&[b"COMMAND".to_vec()], INTERNAL_REPLY_BUDGET)
            .await
        {
            Ok(reply) => CommandTable::from_reply(&reply),
            Err(error) => {
                // An account without `COMMAND` still works: every command is
                // then unknown, hence treated as a write.
                tracing::warn!(target: "oxyn::driver::redis", prefix = ?error.prefix(), "COMMAND refused: every command is classified unknown");
                CommandTable::default()
            }
        };
        let stopper = Stopper::new(Arc::clone(&spec), tls.clone());
        let can_stop = stopper.probe(cancel).await;
        let mut capabilities = base_capabilities();
        if can_stop {
            capabilities |= Capabilities::SERVER_SIDE_CANCEL;
        }
        if read_only {
            capabilities.remove(Capabilities::DML | Capabilities::GRANT_REVOKE);
        }
        tracing::debug!(
            target: "oxyn::driver::redis",
            server = %hello.server,
            version = %hello.version,
            commands = commands.len(),
            server_side_cancel = can_stop,
            "session opened"
        );
        let shared = Arc::new(Shared {
            driver: driver_id(),
            spec,
            tls,
            hello,
            commands,
            stopper,
            can_stop,
            executions: Executions::default(),
            console: Arc::new(AsyncMutex::new(Some(console))),
            catalog: Arc::new(AsyncMutex::new(None)),
            context: Mutex::new(None),
        });
        Ok(Self {
            catalog: RedisCatalog::new(Arc::clone(&shared), capabilities),
            shared,
            capabilities,
        })
    }

    /// The intent the server's command table proves for a request, or `None`
    /// when it proves nothing — `Unknown`, hence mutating.
    ///
    /// No I/O: the table was read when the session opened.
    #[must_use]
    pub fn intent_of(&self, request: &ExecRequest) -> Option<StatementIntent> {
        if !request.language.eq(&oxyn_core::QueryLanguage::RedisCommand) {
            return None;
        }
        let argv = argv::parse(&request.text).ok()?;
        commands::intent(&self.shared.commands, &argv)
    }

    /// What the server said when the session opened.
    #[must_use]
    pub fn hello(&self) -> &Hello {
        &self.shared.hello
    }
}

/// How the session can stop a command, from the server's table.
fn running_of(table: &CommandTable, argv: &[Vec<u8>]) -> Running {
    let name = argv
        .first()
        .map(|word| word.to_ascii_lowercase())
        .unwrap_or_default();
    match name.as_slice() {
        b"eval" | b"evalsha" | b"eval_ro" | b"evalsha_ro" => Running::Script,
        b"fcall" | b"fcall_ro" => Running::Function,
        _ if table
            .find(argv)
            .is_some_and(|found| found.spec.has_flag("blocking")) =>
        {
            Running::Blocking
        }
        _ => Running::Plain,
    }
}

/// The database a request addresses when it carries one: a preview's.
fn bound_database(params: &[ScalarValue]) -> Result<Option<u32>> {
    match params {
        [] => Ok(None),
        [ScalarValue::Int64(database)] => u32::try_from(*database).map(Some).map_err(|_| {
            OxynError::Config("a database number is a non-negative integer".to_owned())
        }),
        _ => Err(OxynError::Config(
            "a Redis command takes no bound value: its arguments are written in the command"
                .to_owned(),
        )),
    }
}

#[async_trait]
impl Session for RedisSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        let argv =
            argv::parse(&request.text).map_err(|error| OxynError::Query(error.to_string()))?;
        if let Some(reason) = commands::refusal(&argv) {
            return Err(OxynError::NotSupported {
                capability: reason.to_owned(),
            });
        }
        let table = &self.shared.commands;
        let intent = commands::intent(table, &argv);
        let found = table.find(&argv);
        if request.limits.read_only && intent != Some(StatementIntent::Read) {
            return Err(OxynError::Query(format!(
                "this execution is read-only, and the server does not mark `{}` as a read \
                 (`readonly` flag, outside @dangerous): Redis has no read-only session, so Oxyn \
                 does not send it",
                String::from_utf8_lossy(argv.first().map_or(&[][..], Vec::as_slice))
            )));
        }
        let running = running_of(table, &argv);
        let plan = Plan {
            running,
            // By the server's category, or by name when the account could not
            // read the table: a script's error may follow its writes.
            scripted: found.is_some_and(|found| found.spec.in_category("scripting"))
                || matches!(running, Running::Script | Running::Function),
            intent,
            argv,
            max_rows: request.limits.max_rows,
            timeout: request.limits.timeout,
        };
        let lease = match bound_database(&request.params)? {
            Some(database) => Lease::Own(Some(self.shared.open(database, cancel).await?)),
            None => {
                let mut guard = tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(OxynError::Cancelled),
                    guard = Arc::clone(&self.shared.console).lock_owned() => guard,
                };
                self.shared
                    .ready(&mut guard, self.shared.console_database(), cancel)
                    .await?;
                Lease::Console(guard)
            }
        };
        let cursor =
            RedisCursor::start(lease, plan, Arc::clone(&self.shared), cancel.child()).await?;
        Ok(Box::new(cursor))
    }

    async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        crate::preview::request(&self.shared, path, limit, shape, cancel).await
    }

    /// Selects the database the console addresses.
    ///
    /// A context names a database as the tree does, `db3`; a namespace level
    /// does not exist. Going back to the server default selects the form's
    /// database.
    ///
    /// # Errors
    /// [`OxynError::Config`] for a context that names no database;
    /// [`OxynError::Cancelled`]; the server's refusal of the number.
    async fn set_context(&self, context: &SessionContext, cancel: &CancelToken) -> Result<()> {
        if context.namespace().is_some() {
            return Err(OxynError::Config(
                "a Redis context names a database only: there is no level below it".to_owned(),
            ));
        }
        let database = match context.catalog() {
            Some(name) => crate::catalog::database_number(name).ok_or_else(|| {
                OxynError::Config(format!(
                    "`{name}` is not a database: they are named db0, db1…"
                ))
            })?,
            None => self.shared.spec.database(),
        };
        let mut guard = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(OxynError::Cancelled),
            guard = self.shared.console.lock() => guard,
        };
        self.shared.ready(&mut guard, database, cancel).await?;
        *self.shared.context.lock() = context.catalog().map(|_| database);
        Ok(())
    }

    fn context(&self) -> Option<SessionContext> {
        self.shared.context.lock().map(|database| {
            SessionContext::new(Some(crate::catalog::database_name(database)), None)
        })
    }

    async fn cancel(&self, statement: StatementHandle) -> Result<()> {
        self.shared.executions.cancel(statement);
        Ok(())
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// Pings on the catalog's connection: the console's may be held by a
    /// blocked command for minutes, and the session is no less alive.
    async fn ping(&self) -> Result<Duration> {
        let started = Instant::now();
        let mut guard = self.shared.catalog.lock().await;
        let cancel = CancelToken::new();
        self.shared
            .ready(&mut guard, self.shared.spec.database(), &cancel)
            .await?;
        let Some(connection) = guard.as_mut() else {
            return Err(OxynError::Connection(
                "the catalog connection is closed".to_owned(),
            ));
        };
        if let Err(error) = connection.call(&[b"PING".to_vec()], 1 << 10).await {
            *guard = None;
            return Err(OxynError::Connection(error.to_string()));
        }
        Ok(started.elapsed())
    }

    async fn close(self: Box<Self>) -> Result<()> {
        *self.shared.console.lock().await = None;
        *self.shared.catalog.lock().await = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tests::redis_8_10;

    fn argv(line: &str) -> Vec<Vec<u8>> {
        argv::parse(line).expect("valid")
    }

    #[test]
    fn a_command_is_stopped_by_its_family() {
        let table = redis_8_10();
        assert_eq!(running_of(&table, &argv("BLPOP k 0")), Running::Blocking);
        assert_eq!(
            running_of(&table, &argv("XREAD BLOCK 0 STREAMS s $")),
            Running::Blocking
        );
        assert_eq!(running_of(&table, &argv("eval \"x\" 0")), Running::Script);
        assert_eq!(running_of(&table, &argv("FCALL f 0")), Running::Function);
        assert_eq!(running_of(&table, &argv("KEYS *")), Running::Plain);
    }

    #[test]
    fn only_a_database_number_may_be_bound() {
        assert_eq!(bound_database(&[]).expect("none"), None);
        assert_eq!(
            bound_database(&[ScalarValue::Int64(3)]).expect("one"),
            Some(3)
        );
        assert!(bound_database(&[ScalarValue::Int64(-1)]).is_err());
        assert!(bound_database(&[ScalarValue::Text("3".to_owned())]).is_err());
        assert!(bound_database(&[ScalarValue::Int64(1), ScalarValue::Int64(2)]).is_err());
    }

    #[test]
    fn a_read_only_connection_declares_no_write() {
        let mut capabilities = base_capabilities();
        capabilities.remove(Capabilities::DML | Capabilities::GRANT_REVOKE);
        assert!(!capabilities.intersects(Capabilities::DML | Capabilities::DDL));
        assert!(!base_capabilities().contains(Capabilities::READ_ONLY_SESSION));
        assert!(!base_capabilities().contains(Capabilities::TRANSACTIONS));
    }
}
