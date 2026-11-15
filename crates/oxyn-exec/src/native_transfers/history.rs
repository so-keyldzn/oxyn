//! Native history, explicit reconciliation and owner release.
use super::*;

impl Executor {
    /// Memory-only live progress; a token or row from another window grants nothing.
    #[must_use]
    pub fn native_transfer_progress(
        &self,
        connection: ConnectionId,
        owner: WindowId,
        transfer: NativeTransferId,
    ) -> Option<NativeTransferJob> {
        let record = self.native.records.lock().get(&transfer).cloned()?;
        let job = record.lock();
        (job.connection == connection && job.owner == owner).then(|| job.clone())
    }

    pub(in crate::executor) async fn discard_native(
        &self,
        connection: ConnectionId,
        owner: WindowId,
        transfer: NativeTransferId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let plans = Arc::clone(&self.native.prepared);
        let store = Arc::clone(&self.store);
        let workspace = self.workspace;
        let job = self
            .local_worker(cancel, move |_| {
                let mut plans = plans.lock();
                if !plans.contains_key(&transfer) {
                    return store
                        .native_transfers()
                        .get(workspace, transfer)?
                        .filter(|job| {
                            job.connection == connection
                                && job.owner == owner
                                && job.stage == NativeTransferStage::Finished
                        })
                        .ok_or_else(|| oxyn_store::StoreError::Corrupted {
                            field: "native_transfers",
                            detail:
                                "only this owner's prepared or finished transfer may be discarded"
                                    .into(),
                        });
                }
                let existing = plans
                    .get(&transfer)
                    .filter(|plan| plan.job.connection == connection && plan.job.owner == owner)
                    .ok_or_else(|| oxyn_store::StoreError::Corrupted {
                        field: "native_transfers",
                        detail: "prepared transfer is unavailable for this owner".into(),
                    })?;
                let mut job = existing.job.clone();
                plans.remove(&transfer);
                job.stage = NativeTransferStage::Finished;
                job.outcome = Some(NativeTransferOutcome::Refused);
                job.failure = None;
                job.command = None;
                job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                store.native_transfers().save(workspace, &job)?;
                Ok(job)
            })
            .await?;
        Ok(Outcome::NativeTransferUpdated { job: Box::new(job) })
    }
    pub(in crate::executor) async fn read_native(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let store = Arc::clone(&self.store);
        let workspace = self.workspace;
        let jobs = self
            .local_worker(cancel, move |_| {
                store.native_transfers().list(workspace, connection)
            })
            .await?;
        let records = self.native.records.lock();
        let jobs = jobs
            .into_iter()
            .map(|job| {
                records
                    .get(&job.id)
                    .map_or(job.clone(), |live| live.lock().clone())
            })
            .collect();
        Ok(Outcome::NativeTransfers { jobs })
    }
    pub(in crate::executor) async fn reconcile_native(
        &self,
        connection: ConnectionId,
        transfer: NativeTransferId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let store = Arc::clone(&self.store);
        let workspace = self.workspace;
        let job = self
            .local_worker(cancel, move |_| {
                let mut job = store
                    .native_transfers()
                    .get(workspace, transfer)?
                    .filter(|job| {
                        job.connection == connection && job.stage == NativeTransferStage::Finished
                    })
                    .ok_or_else(|| oxyn_store::StoreError::Corrupted {
                        field: "native_transfers",
                        detail: "only a finished transfer can be inspected".into(),
                    })?;
                job.reconciled = true;
                store.native_transfers().save(workspace, &job)?;
                Ok(job)
            })
            .await?;
        Ok(Outcome::NativeTransferUpdated { job: Box::new(job) })
    }
    async fn cleanup_native_plans(
        &self,
        selected: impl Fn(&Prepared) -> bool + Send + 'static,
    ) -> Result<()> {
        let plans = Arc::clone(&self.native.prepared);
        let workspace = self.workspace;
        // Native release is part of recorded shutdown, including headless
        // callers outside a runtime. Reuse the audited local writer's fallback;
        // GUI callers still perform every blocking drop/write on its pool.
        self.write_audit(move |store| -> oxyn_store::Result<()> {
            let removed = {
                let mut plans = plans.lock();
                let ids: Vec<_> = plans
                    .iter()
                    .filter(|(_, plan)| selected(plan))
                    .map(|(id, _)| *id)
                    .collect();
                ids.into_iter()
                    .filter_map(|id| plans.remove(&id))
                    .collect::<Vec<_>>()
            };
            for mut plan in removed {
                plan.job.stage = NativeTransferStage::Finished;
                plan.job.outcome = Some(NativeTransferOutcome::FailedBeforeStart);
                plan.job.failure = Some(NativeTransferFailure::Cancelled);
                plan.job.command = None;
                plan.job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                store.native_transfers().save(workspace, &plan.job)?;
            }
            Ok(())
        })
        .await?
        .map_err(Into::into)
    }
    pub(in crate::executor) async fn stop_native_source(
        &self,
        connection: ConnectionId,
        session: Option<SessionId>,
    ) -> Result<()> {
        self.native.workers.stop_source(connection, session).await?;
        self.cleanup_native_plans(move |plan| {
            plan.job.connection == connection
                && session.is_none_or(|session| plan.job.session == session)
        })
        .await?;
        let runner = Arc::clone(&self.native.runner);
        let sources = self
            .sessions
            .for_connection(connection)
            .into_iter()
            .filter(|slot| session.is_none_or(|session| slot.id() == session))
            .map(|slot| oxyn_transfer::SourceBinding {
                connection,
                session: slot.id(),
            })
            .collect::<Vec<_>>();
        self.write_audit(move |_| {
            for source in sources {
                runner.revoke_source(source);
            }
        })
        .await
    }
    pub(in crate::executor) async fn stop_all_native(&self) -> Result<()> {
        self.native.workers.stop_all().await?;
        let cleanup = self.cleanup_native_plans(|_| true).await;
        let runner = Arc::clone(&self.native.runner);
        self.write_audit(move |_| {
            runner.revoke_all();
        })
        .await?;
        cleanup
    }
    /// Cancel and join native work before a host window releases its capabilities.
    pub async fn stop_native_owner(&self, owner: WindowId) -> Result<()> {
        self.native.workers.stop_owner(owner).await?;
        let cleanup = self
            .cleanup_native_plans(move |plan| plan.job.owner == owner)
            .await;
        let runner = Arc::clone(&self.native.runner);
        self.write_audit(move |_| {
            runner.revoke_owner(owner);
        })
        .await?;
        cleanup
    }
}
