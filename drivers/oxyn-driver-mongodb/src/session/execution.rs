//! Command execution and bounded batches, shared by consoles and previews.

use super::{BULK_WRITE, CURSOR_COMMANDS, MongoSession};
use crate::cancel::{self, Leased};
use crate::command::{self, BsonCommand};
use crate::cursor::{Feed, MongoCursor};
use crate::decode::Plan;
use crate::error::{map_error, map_statement_error, write_concern_failure};
use bson::{Document, RawDocumentBuf};
use oxyn_core::{CancelToken, ExecRequest, OxynError, Result, StatementHandle, StatementIntent};
use oxyn_driver::{Cursor, Session};
use std::sync::Arc;
use std::time::Instant;

impl MongoSession {
    pub(super) async fn execute_request(
        &self,
        request: ExecRequest,
        cancel: &CancelToken,
        preview: bool,
    ) -> Result<Box<dyn Cursor>> {
        self.capabilities().require_language(request.language)?;
        if !request.params.is_empty() {
            return Err(OxynError::NotSupported {
                capability: "bound parameters: a MongoDB command carries its values in its \
                             own document"
                    .to_owned(),
            });
        }
        let classification = oxyn_query::classify_mongo(&request.text);
        if classification.transaction_control {
            return Err(OxynError::NotSupported {
                capability: "transactions: each execution runs in its own logical session, so \
                             there is no transaction for this command to end"
                    .to_owned(),
            });
        }
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
        let row_limit = if preview {
            let size = match command.document.get("batchSize") {
                Some(bson::Bson::Int32(size)) => i64::from(*size),
                Some(bson::Bson::Int64(size)) => *size,
                _ => {
                    return Err(OxynError::Config(
                        "a retained MongoDB preview needs a batch size".into(),
                    ));
                }
            };
            usize::try_from(size)
                .ok()
                .filter(|size| (1..=1000).contains(size))
                .ok_or_else(|| {
                    OxynError::Config("a preview batch size must be in 1..=1000".into())
                })?
        } else {
            crate::cursor::BATCH_ROW_CEILING
        };
        self.run(command, intent, cancel, row_limit).await
    }

    /// Runs one command and reads its sample, racing `cancel`.
    async fn run(
        &self,
        command: BsonCommand,
        intent: StatementIntent,
        cancel: &CancelToken,
        row_limit: usize,
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
        let session = self
            .shared
            .client
            .start_session()
            .causal_consistency(false)
            .await
            .map_err(|error| map_error(driver, intent, &error, false))?;
        let mut leased = Leased::new(session, &self.shared.quarantine);
        let running = Arc::clone(leased.running());
        let handle = StatementHandle::new();
        self.registry.insert(handle, Arc::clone(&running));
        let withheld = command.carries_credential;
        let fail = |error: mongodb::error::Error| {
            map_statement_error(driver, intent, &error, running.was_killed(), withheld)
        };
        let outcome = async {
            let db = self.shared.client.database(&database);
            let session = leased.session()?;
            let mut feed = if CURSOR_COMMANDS.contains(&command.name.as_str()) {
                let cursor = cancel::race(&self.shared.stopper, &running, cancel, async {
                    db.run_raw_cursor_command(raw)
                        .session(session)
                        .batch()
                        .await
                })
                .await
                .map_err(fail)?;
                let summarized = command.name == BULK_WRITE;
                Feed::server(cursor, leased, driver.clone(), intent, withheld, summarized)
            } else {
                let reply: Document = cancel::race(&self.shared.stopper, &running, cancel, async {
                    db.run_raw_command(raw).session(session).await
                })
                .await
                .map_err(fail)?;
                let reply = RawDocumentBuf::try_from(&reply).map_err(|error| {
                    OxynError::Internal(format!("the server's reply cannot be re-encoded: {error}"))
                })?;
                if !intent.is_read_only()
                    && let Some(failure) = write_concern_failure(&reply, withheld)
                {
                    return Err(failure);
                }
                Feed::reply(reply, driver.clone(), intent, Arc::clone(&running))
            };
            self.race_oxyn(&running, intent, cancel, feed.fill_sample(row_limit))
                .await?;
            // The command already ran: after a write, a reply that cannot be
            // read leaves its outcome unknown.
            let plan = Plan::infer(feed.sample()).map_err(|error| {
                let class = if intent.is_read_only() {
                    oxyn_core::ErrorClass::Permanent
                } else {
                    oxyn_core::ErrorClass::Ambiguous
                };
                OxynError::driver(driver.clone(), class, error)
            })?;
            Ok(MongoCursor::new(
                handle,
                plan,
                feed,
                Arc::clone(&self.registry),
                driver.clone(),
                started,
            )
            .with_batch_rows(row_limit))
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
}
