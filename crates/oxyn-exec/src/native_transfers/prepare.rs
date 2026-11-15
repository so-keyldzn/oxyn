//! Source authority and fixed preparation; no target writes.
use super::*;

impl Executor {
    pub(in crate::executor) async fn native_source(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        cancel: &CancelToken,
    ) -> Result<(
        TransferSource,
        ConnectionConfig,
        Arc<crate::sessions::SessionSlot>,
    )> {
        let config = self
            .connections
            .read()
            .get(&connection)
            .cloned()
            .ok_or_else(|| {
                OxynError::Connection(
                    "native transfer requires an opened original connection".into(),
                )
            })?;
        let (slot, _) = self.catalog_target(connection, session)?;
        if !slot.opening_matches(&config) || slot.closing_token().is_cancelled() {
            return Err(OxynError::Config(
                "reconnect the source after changing its destination before using native tools"
                    .into(),
            ));
        }
        // An unavailable adapter needs no native metadata read (not even a root scan).
        if config.driver != DriverId::postgres() {
            return Ok((
                TransferSource::new(
                    &config,
                    session,
                    String::new(),
                    String::new(),
                    config.read_only,
                ),
                config,
                slot,
            ));
        }
        if slot.catalog.cache.read().server_info().is_none() {
            self.refresh_session_catalog(
                id,
                connection,
                session,
                &CatalogRefreshScope::Root,
                cancel,
            )
            .await?;
        }
        let info = slot
            .catalog
            .cache
            .read()
            .server_info()
            .cloned()
            .ok_or_else(|| {
                OxynError::Config(
                    "read the original source's server metadata before using native tools".into(),
                )
            })?;
        let retained = self.policy.retained_read_only(
            &Command::ReadNativeTransferSupport {
                connection,
                session,
            },
            config.read_only,
        );
        let read_only = retained
            || !slot
                .capabilities()
                .intersects(Capabilities::DDL | Capabilities::DML);
        let source = TransferSource::new(&config, session, info.product, info.version, read_only);
        Ok((source, config, slot))
    }
    pub(in crate::executor) async fn native_support(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let (source, _, _) = self.native_source(id, connection, session, cancel).await?;
        Ok(Outcome::NativeTransferSupport {
            support: source.support(),
        })
    }
    pub(in crate::executor) async fn inspect_native_tool(
        &self,
        id: CommandId,
        owner: WindowId,
        grant: NativeGrantId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let runner = Arc::clone(&self.native.runner);
        let tool = self
            .native
            .workers
            .run(
                workers::Scope {
                    id,
                    connection: None,
                    session: None,
                    owner,
                },
                cancel,
                &CancelToken::new(),
                move |cancel| runner.inspect_tool(owner, grant, &cancel).map_err(failure),
            )
            .await?;
        Ok(Outcome::NativeToolInspected { tool })
    }
    pub(in crate::executor) async fn prepare_native(
        &self,
        id: CommandId,
        request: NativeTransferRequest,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        request.validate()?;
        let (source, config, slot) = self
            .native_source(id, request.connection, request.session, cancel)
            .await?;
        let runner = Arc::clone(&self.native.runner);
        let plans = Arc::clone(&self.native.prepared);
        let store = Arc::clone(&self.store);
        let workspace = self.workspace;
        let owner = request.owner;
        let connection = request.connection;
        let session = request.session;
        let closed = slot.closing_token().clone();
        let worker_closed = closed.clone();
        let review = self
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
                    let plan = runner
                        .prepare(request.clone(), &source, &cancel, &worker_closed)
                        .map_err(failure)?;
                    if cancel.is_cancelled() || worker_closed.is_cancelled() {
                        return Err(OxynError::Cancelled);
                    }
                    let mut prepared = plans.lock();
                    // Expired plans carry no executable authority and are never replayed.
                    let expired: Vec<_> = prepared
                        .iter()
                        .filter(|(_, plan)| plan.since.elapsed() >= PREPARED_TTL)
                        .map(|(id, _)| *id)
                        .collect();
                    for id in expired {
                        if let Some(mut old) = prepared.remove(&id) {
                            old.job.stage = NativeTransferStage::Finished;
                            old.job.outcome = Some(NativeTransferOutcome::FailedBeforeStart);
                            old.job.failure = Some(NativeTransferFailure::Deadline);
                            old.job.command = None;
                            old.job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                            store.native_transfers().save(workspace, &old.job)?;
                        }
                    }
                    if prepared.len() >= 8 {
                        return Err(OxynError::Config(
                            "too many prepared native transfers; discard one first".into(),
                        ));
                    }
                    let job = NativeTransferJob {
                        version: 1,
                        id: NativeTransferId::new(),
                        connection,
                        session,
                        owner,
                        command: None,
                        connection_name: source.name.clone(),
                        database: plan.database().into(),
                        engine: source.driver.clone(),
                        operation: request.operation,
                        format: request.format,
                        no_owner: request.no_owner,
                        no_privileges: request.no_privileges,
                        tool: plan.tool().identity.clone(),
                        file_name: plan.file_name().into(),
                        stage: NativeTransferStage::Ready,
                        outcome: None,
                        failure: None,
                        bytes: 0,
                        may_have_changed: false,
                        reconciled: false,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        finished_at: None,
                    };
                    store.native_transfers().save(workspace, &job)?;
                    let review = NativeTransferReview {
                        job: job.clone(),
                        destination: plan.destination(),
                    };
                    prepared.insert(
                        job.id,
                        Prepared {
                            plan,
                            config,
                            job,
                            since: Instant::now(),
                        },
                    );
                    Ok(review)
                },
            )
            .await?;
        Ok(Outcome::NativeTransferPrepared {
            review: Box::new(review),
        })
    }
}
