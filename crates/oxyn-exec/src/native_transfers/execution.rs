//! A reviewed native execution owns its files, task and possible-change invalidation.
use super::*;

impl Executor {
    /// Exact host-side review; a webview never supplies the destination dialog text.
    pub fn native_review(
        &self,
        connection: ConnectionId,
        owner: WindowId,
        transfer: NativeTransferId,
    ) -> Result<NativeTransferReview> {
        let plans = self.native.prepared.lock();
        let ready = plans
            .get(&transfer)
            .filter(|ready| {
                ready.job.connection == connection
                    && ready.job.owner == owner
                    && ready.since.elapsed() < PREPARED_TTL
            })
            .ok_or_else(|| {
                OxynError::Config("native preparation expired; prepare the transfer again".into())
            })?;
        Ok(NativeTransferReview {
            job: ready.job.clone(),
            destination: ready.plan.destination(),
        })
    }
    pub(in crate::executor) async fn run_native(
        &self,
        id: CommandId,
        connection: ConnectionId,
        owner: WindowId,
        transfer: NativeTransferId,
        restore: bool,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let review = self.native_review(connection, owner, transfer)?;
        if restore != (review.job.operation == NativeTransferOperation::Restore) {
            return Err(OxynError::Config(
                "native operation differs from its reviewed preparation".into(),
            ));
        }
        // Native writes share ordinary mutation admission and release retained
        // previews before touching the target. Move the guard into the owned
        // worker so dropping the caller cannot admit another mutation early.
        let mutation = if restore {
            let (slot, _) = self.catalog_target(connection, review.job.session)?;
            let guard = self
                .previews
                .mutation(connection, cancel, slot.closing_token(), None)
                .await?;
            self.previews.close(connection, None);
            self.previews.join_sessions(Some(connection), None).await;
            Some(guard)
        } else {
            None
        };
        let (source, current, slot) = self
            .native_source(id, connection, review.job.session, cancel)
            .await?;
        if restore && source.read_only {
            return Err(OxynError::PolicyDenied {
                reason: "native restoration to a read-only target is refused".into(),
            });
        }
        {
            let plans = self.native.prepared.lock();
            let ready = plans.get(&transfer).ok_or_else(|| {
                OxynError::Config("native preparation is no longer available".into())
            })?;
            if ready.config.driver != current.driver
                || ready.config.params != current.params
                || !slot.opening_matches(&current)
            {
                return Err(OxynError::Config(
                    "native destination changed; reconnect and prepare again".into(),
                ));
            }
        }
        let plans = Arc::clone(&self.native.prepared);
        let store = Arc::clone(&self.store);
        let workspace = self.workspace;
        let events = self.events.clone();
        let records = Arc::clone(&self.native.records);
        let sessions = Arc::clone(&self.sessions);
        let session = review.job.session;
        let closed = slot.closing_token().clone();
        let worker_closed = closed.clone();
        let job = self
            .native
            .workers
            .run(
                workers::Scope {
                    id,
                    connection: Some(connection),
                    session: Some(session),
                    owner,
                },
                cancel,
                &closed,
                move |cancel| {
                    let _mutation = mutation;
                    let mut ready = plans.lock().remove(&transfer).ok_or_else(|| {
                        OxynError::Config("native preparation already consumed".into())
                    })?;
                    if cancel.is_cancelled() || worker_closed.is_cancelled() {
                        ready.job.stage = NativeTransferStage::Finished;
                        ready.job.outcome = Some(NativeTransferOutcome::FailedBeforeStart);
                        ready.job.failure = Some(NativeTransferFailure::Cancelled);
                        ready.job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                        store.native_transfers().save(workspace, &ready.job)?;
                        return Ok(ready.job);
                    }
                    ready.job.command = Some(id);
                    ready.job.stage = NativeTransferStage::Running;
                    ready.job.may_have_changed = restore;
                    store.native_transfers().save(workspace, &ready.job)?;
                    let record = Arc::new(Mutex::new(ready.job));
                    records.lock().insert(transfer, Arc::clone(&record));
                    let mut last = Instant::now();
                    let result =
                        ready
                            .plan
                            .run(id, &cancel, &worker_closed, &mut |stage, bytes| {
                                if last.elapsed() < Duration::from_millis(250)
                                    && stage == NativeTransferStage::Running
                                {
                                    return;
                                }
                                last = Instant::now();
                                let mut job = record.lock();
                                job.stage = stage;
                                job.bytes = bytes;
                            });
                    if result.may_have_changed {
                        for slot in sessions.for_connection(connection) {
                            slot.catalog.cache.write().invalidate(&CatalogScope::Server);
                        }
                        events.publish(id, Some(connection), Event::DatabaseMayHaveChanged);
                    }
                    let mut job = record.lock();
                    job.stage = NativeTransferStage::Finished;
                    job.outcome = Some(result.outcome);
                    job.failure = result.failure;
                    job.bytes = result.bytes;
                    job.may_have_changed = result.may_have_changed;
                    job.command = None;
                    job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                    store.native_transfers().save(workspace, &job)?;
                    let final_job = job.clone();
                    drop(job);
                    records.lock().remove(&transfer);
                    Ok(final_job)
                },
            )
            .await?;
        Ok(Outcome::NativeTransferUpdated { job: Box::new(job) })
    }
}
