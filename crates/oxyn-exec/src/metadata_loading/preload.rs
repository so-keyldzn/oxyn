//! One reviewed namespace page, bounded and cancellable on its original session.

use super::jobs::Job;
use super::{Executor, Outcome};
use crate::catalog::priority::Priority;
use crate::sessions::SessionSlot;
use oxyn_catalog::{CatalogPath, CatalogScope, Freshness};
use oxyn_core::metadata_loading::MAX_PRELOAD_RELATIONS;
use oxyn_core::{
    CancelToken, Capabilities, CatalogRefreshScope, CommandId, MetadataFacet,
    MetadataPreloadCursor, MetadataPreloadIssue, MetadataPreloadProgress, MetadataPreloadRequest,
    MetadataPreloadState, OxynError, Result,
};
use std::sync::Arc;
use std::time::Duration;

#[path = "preload/scopes.rs"]
mod scopes;
use scopes::{cache_scope, can_prefetch, observed_scope, read_scope};

const DEADLINE: Duration = Duration::from_secs(60);
const MAX_FAILURE_DETAILS: usize = 16;

impl Executor {
    /// Live facts only; the desktop separately checks the calling window.
    #[must_use]
    pub fn metadata_preload_progress(&self, id: CommandId) -> Option<MetadataPreloadProgress> {
        self.metadata_jobs.progress(id)
    }

    pub(in crate::executor) async fn preload_metadata(
        &self,
        id: CommandId,
        request: &MetadataPreloadRequest,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        request.validate()?;
        let (slot, _) = self.catalog_target(request.connection, request.session)?;
        let store = self.store.clone();
        let workspace = self.workspace;
        let connection = request.connection;
        let snapshot = self
            .local_worker(cancel, move |cancel| {
                if cancel.is_cancelled() {
                    return Err(oxyn_store::StoreError::Cancelled);
                }
                store.metadata_policies().load(workspace, connection)
            })
            .await?;
        self.metadata_jobs.observe(connection, snapshot.revision);
        if snapshot.revision != request.expected_revision {
            return Err(OxynError::Config(
                "metadata policy changed; review it before preloading".into(),
            ));
        }
        let parent = CatalogPath::from_levels(
            request.scope.catalog.clone(),
            request.scope.namespace.clone(),
            None,
        )?;
        let generation = {
            let cache = slot.catalog.cache.read();
            observed_scope(&cache, &slot, request)?;
            if request
                .after
                .as_ref()
                .is_some_and(|cursor| cursor.generation != cache.generation().into_uuid())
            {
                return Err(OxynError::Config(
                    "metadata scope changed; start a new preload page".into(),
                ));
            }
            cache.generation()
        };
        let level = snapshot.policy.effective(&request.scope);
        let progress = MetadataPreloadProgress {
            command: id,
            connection,
            session: request.session,
            scope: request.scope.clone(),
            level,
            policy_revision: snapshot.revision,
            state: MetadataPreloadState::Running,
            planned: 1,
            reads: 0,
            loaded: 0,
            cached: 0,
            failed: 0,
            unavailable: 0,
            sampling_required: 0,
            known_relations: None,
            unread_relations: 0,
            available_facets: None,
            stale_facets: None,
            unread_facets: None,
            metadata_bytes: 0,
            elapsed_ms: 0,
            stopped: None,
            next: None,
            failures: Vec::new(),
            omitted_failures: 0,
        };
        let lease = self.metadata_jobs.register(progress, cancel, generation)?;
        let job = &lease.job;
        let names = CatalogRefreshScope::Relations {
            catalog: request.scope.catalog.clone(),
            namespace: request.scope.namespace.clone(),
        };
        let names_known = matches!(
            slot.catalog
                .cache
                .read()
                .freshness(&CatalogScope::Namespace(parent.clone())),
            Freshness::Fetched(_)
        );
        let names_ok = if names_known && (!request.refresh || request.after.is_some()) {
            job.progress.lock().cached += 1;
            true
        } else {
            self.preload_read(job, &names, MetadataFacet::Names, None)
                .await
        };
        if !names_ok {
            return Ok(Outcome::MetadataPreloaded {
                progress: finish(job, &slot, &[], None),
            });
        }
        let mut paths: Vec<_> = slot
            .catalog
            .cache
            .read()
            .relations(&parent)
            .map(|relation| relation.path())
            .collect();
        paths.sort();
        let known = u32::try_from(paths.len()).map_err(|_| {
            OxynError::Config("metadata relation count exceeds its report range".into())
        })?;
        if let Some(cursor) = &request.after {
            paths.retain(|path| {
                path.relation()
                    .is_some_and(|name| name > cursor.after.as_str())
            });
        }
        let remaining = paths.len().saturating_sub(MAX_PRELOAD_RELATIONS);
        paths.truncate(MAX_PRELOAD_RELATIONS);
        {
            let mut progress = job.progress.lock();
            progress.known_relations = Some(known);
            progress.unread_relations = if level.preload_facets().is_empty() {
                0
            } else {
                u32::try_from(remaining).unwrap_or(u32::MAX)
            };
            progress.planned =
                1 + u32::try_from(paths.len().saturating_mul(level.preload_facets().len()))
                    .unwrap_or(u32::MAX);
        }
        let mut last = None;
        'relations: for path in &paths {
            for facet in level.preload_facets() {
                if slot.catalog.cache.read().generation() != *job.generation.lock() {
                    job.progress.lock().stopped = Some("sourceChanged".into());
                    break 'relations;
                }
                if job.cancel.is_cancelled() || slot.closing_token().is_cancelled() {
                    break 'relations;
                }
                let cache_scope = cache_scope(*facet, path);
                let fresh = matches!(
                    slot.catalog.cache.read().freshness(&cache_scope),
                    Freshness::Fetched(_)
                );
                if fresh && !request.refresh {
                    job.progress.lock().cached += 1;
                    continue;
                }
                if !can_prefetch(*facet, slot.capabilities()) {
                    let mut progress = job.progress.lock();
                    progress.unavailable += 1;
                    if *facet == MetadataFacet::Description
                        && slot.capabilities().contains(Capabilities::INFERRED_SCHEMA)
                    {
                        progress.sampling_required += 1;
                    }
                    continue;
                }
                let scope = read_scope(*facet, path)?;
                if !self
                    .preload_read(job, &scope, *facet, path.relation())
                    .await
                    && job.progress.lock().stopped.is_some()
                {
                    break 'relations;
                }
            }
            last = path.relation().map(str::to_owned);
        }
        let next = if remaining > 0 && !level.preload_facets().is_empty() {
            last.and_then(|after| {
                if after.len() > oxyn_core::metadata_loading::MAX_METADATA_SCOPE_BYTES {
                    job.progress.lock().stopped = Some("continuationUnavailable".into());
                    None
                } else {
                    Some(MetadataPreloadCursor {
                        generation: job.generation.lock().into_uuid(),
                        scope: request.scope.clone(),
                        policy_revision: snapshot.revision,
                        after,
                    })
                }
            })
        } else {
            None
        };
        Ok(Outcome::MetadataPreloaded {
            progress: finish(job, &slot, &paths, next),
        })
    }

    async fn preload_read(
        &self,
        job: &Arc<Job>,
        scope: &CatalogRefreshScope,
        facet: MetadataFacet,
        relation: Option<&str>,
    ) -> bool {
        if job.started.elapsed() >= DEADLINE {
            job.progress.lock().stopped = Some("deadline".into());
            job.cancel.cancel();
            return false;
        }
        let (id, connection, session) = {
            let mut progress = job.progress.lock();
            progress.reads += 1;
            (progress.command, progress.connection, progress.session)
        };
        let remaining = DEADLINE.saturating_sub(job.started.elapsed());
        let read = self.refresh_session_catalog_with_priority(
            id,
            connection,
            session,
            scope,
            &job.cancel,
            Priority::Prefetch(*job.generation.lock()),
        );
        tokio::pin!(read);
        let result = tokio::select! {
            result = &mut read => result,
            _ = tokio::time::sleep(remaining) => {
                job.progress.lock().stopped = Some("deadline".into()); job.cancel.cancel();
                let _ = read.await;
                Err(OxynError::Cancelled)
            }
        };
        let mut progress = job.progress.lock();
        match result {
            Ok(Outcome::CatalogRefreshed {
                metadata_bytes,
                generation,
                ..
            }) => {
                *job.generation.lock() = generation;
                progress.loaded += 1;
                progress.metadata_bytes = progress.metadata_bytes.saturating_add(metadata_bytes);
                true
            }
            Ok(_) => {
                progress.failed += 1;
                false
            }
            Err(OxynError::Cancelled) => {
                if progress.stopped.is_none() {
                    progress.stopped = Some(
                        if job.cancel.is_cancelled() {
                            "cancelled"
                        } else {
                            "sourceChanged"
                        }
                        .into(),
                    );
                }
                false
            }
            Err(_) => {
                progress.failed += 1;
                if progress.failures.len() < MAX_FAILURE_DETAILS {
                    progress.failures.push(MetadataPreloadIssue {
                        relation: relation
                            .filter(|name| {
                                name.len() <= oxyn_core::metadata_loading::MAX_METADATA_SCOPE_BYTES
                            })
                            .map(str::to_owned),
                        facet,
                    });
                } else {
                    progress.omitted_failures += 1;
                }
                false
            }
        }
    }
}

fn finish(
    job: &Job,
    slot: &SessionSlot,
    paths: &[CatalogPath],
    next: Option<MetadataPreloadCursor>,
) -> MetadataPreloadProgress {
    let mut progress = job.progress.lock().clone();
    let cache = slot.catalog.cache.read();
    if cache.generation() != *job.generation.lock() {
        progress.stopped = Some("sourceChanged".into());
    }
    if progress.known_relations.is_some() {
        let (mut fresh, mut stale, mut unread) = (0, 0, 0);
        for path in paths {
            for facet in progress.level.preload_facets() {
                match cache.freshness(&cache_scope(*facet, path)) {
                    Freshness::Fetched(_) => fresh += 1,
                    Freshness::Invalidated => stale += 1,
                    _ if can_prefetch(*facet, slot.capabilities()) => unread += 1,
                    _ => {}
                }
            }
        }
        progress.available_facets = Some(fresh);
        progress.stale_facets = Some(stale);
        progress.unread_facets = Some(unread);
    }
    progress.elapsed_ms = u64::try_from(job.started.elapsed().as_millis()).unwrap_or(u64::MAX);
    progress.next = next;
    if slot.closing_token().is_cancelled() {
        progress.stopped = Some("sourceClosed".into());
    }
    progress.state = if progress.stopped.as_deref() == Some("continuationUnavailable") {
        MetadataPreloadState::Partial
    } else if job.cancel.is_cancelled() || progress.stopped.is_some() {
        MetadataPreloadState::Cancelled
    } else if progress.failed > 0 && progress.loaded == 0 && progress.cached == 0 {
        MetadataPreloadState::Failed
    } else if progress.failed > 0
        || progress.sampling_required > 0
        || progress.unread_relations > 0
        || progress.unread_facets.is_some_and(|count| count > 0)
        || progress.stale_facets.is_some_and(|count| count > 0)
    {
        MetadataPreloadState::Partial
    } else {
        MetadataPreloadState::Complete
    };
    progress
}
