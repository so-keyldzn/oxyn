//! A MongoDB session: one client, one logical session per execution.
//!
//! # What the session owns, and declares
//!
//! A `mongodb::Client` is a pool. The Oxyn session keeps one, bounded
//! (`options.rs`), so that a `killSessions` can leave while a command runs.
//! What would be session state elsewhere is fixed and declared
//! ([ADR-0063](../../../docs/adr/0063-mongodb-driver-database-commands-in-extended-json.md)):
//!
//! * the **target database** is the context (`set_context`), the form's
//!   database otherwise, `admin` when the form names none — or the `$db` a
//!   command carries. It is client-side only: no server state changes;
//! * read preference `primary`, the server's default read and write concerns,
//!   **no causal consistency**: each execution starts its own logical session
//!   with causal consistency off, so a command never waits on the cluster time
//!   of another;
//! * **no transaction**: `TRANSACTIONS` is not declared. A multi-document
//!   transaction needs a replica set and a session kept across executions;
//!   until the console offers one, `begin` says it cannot rather than open
//!   nothing ([DRIVER-CONTRACT §5](../../../docs/DRIVER-CONTRACT.md)).
//!
//! # Read-only is client-side, and the server's when the user is
//!
//! MongoDB has no read-only session mode, and `readPreference` is not a write
//! guard. A bounded execution (`limits.read_only`) is refused here unless
//! `oxyn-query` reads its command as a read — the executor refuses it first,
//! this is the floor under it. What a client-side reading cannot see —
//! server-side JavaScript, which is read as mutating anyway — is what a
//! read-only *user* stops: the session declares `READ_ONLY_SESSION` without
//! `DML`/`DDL` only when the server says the account holds no write privilege
//! (`connectionStatus`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bson::{Document, RawDocumentBuf, doc};
use mongodb::Client;
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    CancelToken, Capabilities, DriverId, ExecRequest, OxynError, PreviewShape, QueryLanguage,
    Result, StatementHandle, StatementIntent,
};
use oxyn_driver::{Cursor, Session, SessionContext};
use parking_lot::Mutex;

use crate::cancel::{self, Registry, Running};
use crate::catalog::MongoCatalog;
use crate::command::{self, BsonCommand};
use crate::cursor::{Feed, MongoCursor};
use crate::decode::Plan;
use crate::error::{map_error, stopped};
use crate::preview;

/// The commands whose reply is a cursor: their results stream through
/// `getMore`. Every other command's reply is one document, the whole result.
pub(crate) const CURSOR_COMMANDS: &[&str] =
    &["find", "aggregate", "listCollections", "listIndexes"];

/// The database a session targets when neither the context nor the form
/// names one.
pub const DEFAULT_DATABASE: &str = "admin";

/// What the session shares with its catalog.
pub(crate) struct Shared {
    pub(crate) client: Client,
    pub(crate) driver: DriverId,
    pub(crate) default_database: Option<String>,
    pub(crate) capabilities: Capabilities,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("driver", &self.driver)
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

/// An open MongoDB session.
#[derive(Debug)]
pub struct MongoSession {
    shared: Arc<Shared>,
    catalog: MongoCatalog,
    registry: Arc<Registry>,
    context: Mutex<Option<SessionContext>>,
}

impl MongoSession {
    pub(crate) fn new(shared: Arc<Shared>, version: String) -> Self {
        Self {
            catalog: MongoCatalog::new(Arc::clone(&shared), version),
            shared,
            registry: Arc::new(Registry::default()),
            context: Mutex::new(None),
        }
    }

    /// Where a command without `$db` goes.
    fn target_database(&self) -> String {
        self.context
            .lock()
            .as_ref()
            .and_then(SessionContext::namespace)
            .map(str::to_owned)
            .or_else(|| self.shared.default_database.clone())
            .unwrap_or_else(|| DEFAULT_DATABASE.to_owned())
    }

    /// Runs one command and reads its sample, racing `cancel`.
    async fn run(
        &self,
        command: BsonCommand,
        intent: StatementIntent,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Cursor>> {
        let started = Instant::now();
        let driver = &self.shared.driver;
        let database = command
            .database
            .clone()
            .unwrap_or_else(|| self.target_database());
        let raw = RawDocumentBuf::try_from(&command.document).map_err(|error| {
            OxynError::Query(format!("the command cannot be encoded as BSON: {error}"))
        })?;
        let mut session = self
            .shared
            .client
            .start_session()
            .causal_consistency(false)
            .await
            .map_err(|error| map_error(driver, intent, &error, false))?;
        let running = Arc::new(Running::new(session.id().clone()));
        let handle = StatementHandle::new();
        self.registry.insert(handle, Arc::clone(&running));
        let outcome = async {
            let db = self.shared.client.database(&database);
            let mut feed = if CURSOR_COMMANDS.contains(&command.name.as_str()) {
                let cursor = cancel::race(&self.shared.client, &running, cancel, async {
                    db.run_raw_cursor_command(raw)
                        .session(&mut session)
                        .batch()
                        .await
                })
                .await
                .map_err(|error| map_error(driver, intent, &error, running.was_killed()))?;
                Feed::server(
                    cursor,
                    session,
                    driver.clone(),
                    intent,
                    Arc::clone(&running),
                )
            } else {
                let reply: Document = cancel::race(&self.shared.client, &running, cancel, async {
                    db.run_raw_command(raw).session(&mut session).await
                })
                .await
                .map_err(|error| map_error(driver, intent, &error, running.was_killed()))?;
                let reply = RawDocumentBuf::try_from(&reply).map_err(|error| {
                    OxynError::Internal(format!("the server's reply cannot be re-encoded: {error}"))
                })?;
                Feed::reply(reply, driver.clone(), intent, Arc::clone(&running))
            };
            self.race_oxyn(&running, intent, cancel, feed.fill_sample())
                .await?;
            let plan = Plan::infer(feed.sample()).map_err(|error| {
                OxynError::driver(driver.clone(), oxyn_core::ErrorClass::Permanent, error)
            })?;
            Ok(MongoCursor::new(
                handle,
                plan,
                feed,
                Arc::clone(&self.registry),
                driver.clone(),
                started,
            ))
        }
        .await;
        match outcome {
            Ok(cursor) => Ok(Box::new(cursor)),
            Err(error) => {
                self.registry.remove(handle);
                Err(error)
            }
        }
    }

    /// [`cancel::race`] for the sample read, whose errors are already mapped:
    /// any failure after the stop is the stop's verdict.
    async fn race_oxyn<T>(
        &self,
        running: &Running,
        intent: StatementIntent,
        cancel: &CancelToken,
        call: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let mut call = std::pin::pin!(call);
        tokio::select! {
            outcome = &mut call => outcome,
            () = cancel.cancelled() => {
                let _ = cancel::kill(&self.shared.client, running).await;
                match tokio::time::timeout(cancel::KILL_GRACE, call).await {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(_)) | Err(_) => Err(stopped(intent)),
                }
            }
        }
    }
}

#[async_trait]
impl Session for MongoSession {
    fn capabilities(&self) -> Capabilities {
        self.shared.capabilities
    }

    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities().require_language(request.language)?;
        if !request.params.is_empty() {
            return Err(OxynError::NotSupported {
                capability: "bound parameters: a MongoDB command carries its values in its \
                             own document"
                    .to_owned(),
            });
        }
        let classification = oxyn_query::classify_mongo(&request.text);
        let intent = classification.intent;
        if request.limits.read_only && !classification.is_read_only() {
            return Err(OxynError::PolicyDenied {
                reason: "this execution is bounded to read-only, and the command is not one \
                         Oxyn can prove only reads"
                    .to_owned(),
            });
        }
        let command = command::parse(&request.text)?;
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        self.run(command, intent, cancel).await
    }

    async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        let relation = if shape.needs_total_order() || shape.columns.is_some() {
            Some(self.catalog.describe_relation(path, cancel).await?)
        } else {
            None
        };
        preview::request(path, limit, shape, relation.as_ref())
    }

    async fn set_context(&self, context: &SessionContext, _cancel: &CancelToken) -> Result<()> {
        if context.catalog().is_some() {
            return Err(OxynError::Config(
                "MongoDB has no catalog level: a context names a database only".to_owned(),
            ));
        }
        if let Some(database) = context.namespace() {
            command::check_database_name(database)
                .map_err(|error| OxynError::Config(error.to_string()))?;
        }
        *self.context.lock() = Some(context.clone());
        Ok(())
    }

    fn context(&self) -> Option<SessionContext> {
        self.context.lock().clone()
    }

    /// Sends `killSessions` for the execution's logical session.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] when the session cannot stop commands on
    /// the server; the library's error, mapped, when the server refuses.
    async fn cancel(&self, statement: StatementHandle) -> Result<()> {
        self.capabilities()
            .require(Capabilities::SERVER_SIDE_CANCEL)?;
        let Some(running) = self.registry.get(statement) else {
            return Ok(());
        };
        cancel::kill(&self.shared.client, &running)
            .await
            .map_err(|error| map_error(&self.shared.driver, StatementIntent::Read, &error, false))
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// Measures a `ping` round trip on `admin`.
    ///
    /// # Errors
    /// Any transport error: a session whose ping fails is considered lost.
    async fn ping(&self) -> Result<Duration> {
        let started = Instant::now();
        self.shared
            .client
            .database("admin")
            .run_command(doc! { "ping": 1 })
            .await
            .map_err(|error| {
                map_error(&self.shared.driver, StatementIntent::Read, &error, false)
            })?;
        Ok(started.elapsed())
    }

    /// Closes the pool without waiting for cursors still held elsewhere:
    /// the server reclaims what it held when the connections close.
    async fn close(self: Box<Self>) -> Result<()> {
        self.shared.client.clone().shutdown().immediate(true).await;
        Ok(())
    }
}

/// The language this driver speaks.
pub const LANGUAGE: QueryLanguage = QueryLanguage::MongoQuery;
