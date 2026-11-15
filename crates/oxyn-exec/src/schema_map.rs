//! One owned human map demand, using the same native read queue/publication guards.
#[path = "schema_map/plan.rs"]
mod plan;
#[path = "schema_map/publication.rs"]
mod publication;

use super::{Executor, Outcome};
use crate::catalog::priority::Priority;
use oxyn_catalog::schema_map::{
    MapFacet, MapIssueKind, MapLoadIssue, SchemaMapLoaded, project_schema_map,
};
use oxyn_catalog::{CatalogGenerationId, CatalogPath, CatalogScope, Freshness};
use oxyn_core::{
    CancelToken, Capabilities, CatalogRefreshScope, CommandId, OxynError, Result, SchemaMapRequest,
};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(60);
const MAX_ISSUES: usize = 80;

struct Pass<'a> {
    executor: &'a Executor,
    id: CommandId,
    request: &'a SchemaMapRequest,
    cancel: CancelToken,
    epoch: CatalogGenerationId,
    started: Instant,
    reads: u32,
    issues: Vec<MapLoadIssue>,
    stopped: Option<String>,
}
impl Pass<'_> {
    async fn read(
        &mut self,
        scope: CatalogRefreshScope,
        facet: MapFacet,
        path: &CatalogPath,
    ) -> bool {
        if self.cancel.is_cancelled() {
            self.stopped = Some("cancelled".into());
            return false;
        }
        if self.started.elapsed() >= DEADLINE {
            self.stopped = Some("deadline".into());
            self.cancel.cancel();
            return false;
        }
        self.reads += 1;
        let result = {
            let pending = self.executor.refresh_session_catalog_with_priority(
                self.id,
                self.request.connection,
                self.request.session,
                &scope,
                &self.cancel,
                Priority::DemandAt(self.epoch),
            );
            tokio::pin!(pending);
            tokio::select! {
                result = &mut pending => result,
                _ = tokio::time::sleep(DEADLINE.saturating_sub(self.started.elapsed())) => {
                    self.cancel.cancel(); self.stopped = Some("deadline".into());
                    let _ = pending.await;
                    Err(OxynError::Cancelled)
                }
            }
        };
        match result {
            Ok(Outcome::CatalogRefreshed { generation, .. }) => {
                self.epoch = generation;
                true
            }
            Err(OxynError::Cancelled) => {
                if self.stopped.is_none() {
                    self.stopped = Some(
                        if self.cancel.is_cancelled() {
                            "cancelled"
                        } else {
                            "sourceChanged"
                        }
                        .into(),
                    );
                }
                false
            }
            Err(OxynError::NotSupported { .. }) => {
                self.issue(path, facet, MapIssueKind::Unavailable);
                false
            }
            _ => {
                self.issue(path, facet, MapIssueKind::Failed);
                false
            }
        }
    }
    fn issue(&mut self, path: &CatalogPath, facet: MapFacet, kind: MapIssueKind) {
        if self.issues.len() < MAX_ISSUES {
            self.issues.push(MapLoadIssue {
                address: plan::address(path),
                facet,
                kind,
            });
        }
    }
    async fn object(&mut self, path: &CatalogPath) {
        let Ok((slot, _)) = self
            .executor
            .catalog_target(self.request.connection, self.request.session)
        else {
            self.stopped = Some("sourceClosed".into());
            return;
        };
        for (facet, scope, support) in plan::object_reads(path, slot.capabilities()) {
            if self.stopped.is_some() {
                return;
            }
            let fresh = {
                let cache = slot.catalog.cache.read();
                if cache.generation() != self.epoch {
                    self.stopped = Some("sourceChanged".into());
                    return;
                }
                matches!(cache.freshness(&scope), Freshness::Fetched(_))
            };
            if fresh && !self.request.refresh {
                continue;
            }
            if !support {
                self.issue(path, facet, MapIssueKind::Unavailable);
                continue;
            }
            if let Some(request) = plan::refresh(&scope) {
                self.read(request, facet, path).await;
            }
        }
    }
}

impl Executor {
    pub(in crate::executor) async fn load_schema_map(
        &self,
        id: CommandId,
        request: &SchemaMapRequest,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        request.validate()?;
        let (slot, _) = self.catalog_target(request.connection, request.session)?;
        let mut pass = Pass {
            executor: self,
            id,
            request,
            cancel: cancel.child(),
            epoch: slot.catalog.cache.read().generation(),
            started: Instant::now(),
            reads: 0,
            issues: vec![],
            stopped: None,
        };
        let root = CatalogPath::empty();
        let root_fresh = matches!(
            slot.catalog.cache.read().freshness(&CatalogScope::Server),
            Freshness::Fetched(_)
        );
        if !root_fresh || request.refresh && request.objects.is_empty() {
            pass.read(CatalogRefreshScope::Root, MapFacet::Names, &root)
                .await;
        }
        let parent = CatalogPath::from_levels(
            request.scope.catalog.clone(),
            request.scope.namespace.clone(),
            None,
        )?;
        let choose_namespace = request.objects.is_empty()
            && (slot.capabilities().contains(Capabilities::SCHEMAS)
                && request.scope.namespace.is_none()
                || request.scope.catalog.is_none()
                    && slot.catalog.cache.read().catalogs().next().is_some());
        if choose_namespace && pass.stopped.is_none() {
            let known = request.scope.catalog.as_deref().is_none_or(|name| {
                slot.catalog
                    .cache
                    .read()
                    .catalogs()
                    .any(|catalog| catalog.name() == name)
            });
            if !known {
                return Err(OxynError::Config(
                    "choose an observed native catalog".into(),
                ));
            }
            if let Some(catalog) = &request.scope.catalog {
                pass.read(
                    CatalogRefreshScope::Namespaces {
                        catalog: Some(catalog.clone()),
                    },
                    MapFacet::Names,
                    &parent,
                )
                .await;
            }
        }
        let mut paths = Vec::new();
        let mut omitted = 0usize;
        if !choose_namespace && pass.stopped.is_none() {
            if request.objects.is_empty() {
                if !slot.catalog.cache.read().observes_native_scope(
                    request.scope.catalog.as_deref(),
                    request.scope.namespace.as_deref(),
                    slot.capabilities(),
                ) {
                    return Err(OxynError::Config(
                        "choose an observed native database/schema before loading a map".into(),
                    ));
                }
                let fresh = matches!(
                    slot.catalog
                        .cache
                        .read()
                        .freshness(&CatalogScope::Namespace(parent.clone())),
                    Freshness::Fetched(_)
                );
                if !fresh || request.refresh {
                    pass.read(
                        CatalogRefreshScope::Relations {
                            catalog: request.scope.catalog.clone(),
                            namespace: request.scope.namespace.clone(),
                        },
                        MapFacet::Names,
                        &parent,
                    )
                    .await;
                }
                let cache = slot.catalog.cache.read();
                let mut known: Vec<_> = cache
                    .relations(&parent)
                    .map(|relation| relation.path())
                    .collect();
                known.sort();
                omitted = known
                    .len()
                    .saturating_sub(oxyn_catalog::schema_map::MAX_MAP_OBJECTS);
                known.truncate(oxyn_catalog::schema_map::MAX_MAP_OBJECTS);
                paths = known;
            } else {
                for object in &request.objects {
                    let path = plan::path(object)?;
                    if plan::observed_object(&slot.catalog.cache.read(), &path) {
                        if !paths.contains(&path) {
                            paths.push(path);
                        }
                    } else {
                        pass.issue(&path, MapFacet::Names, MapIssueKind::NotObserved);
                    }
                }
            }
            for path in &paths {
                pass.object(path).await;
                if pass.stopped.is_some() {
                    break;
                }
            }
            if let Some(expand) = &request.expand {
                let target = plan::path(expand)?;
                if paths.contains(&target) && pass.stopped.is_none() {
                    if slot
                        .capabilities()
                        .contains(Capabilities::INCOMING_FOREIGN_KEYS)
                    {
                        let scope = CatalogRefreshScope::IncomingForeignKeys {
                            catalog: target.catalog().map(str::to_owned),
                            namespace: target.namespace().map(str::to_owned),
                            relation: target.relation().unwrap_or_default().into(),
                        };
                        pass.read(scope, MapFacet::IncomingKeys, &target).await;
                    }
                    let neighbours = plan::neighbours(&slot.catalog.cache.read(), &target);
                    for neighbour in neighbours {
                        if paths.contains(&neighbour) {
                            continue;
                        }
                        if paths.len() >= oxyn_catalog::schema_map::MAX_MAP_OBJECTS {
                            omitted += 1;
                            continue;
                        }
                        pass.object(&neighbour).await;
                        paths.push(neighbour);
                        if pass.stopped.is_some() {
                            break;
                        }
                    }
                } else if pass.stopped.is_none() {
                    pass.issue(&target, MapFacet::Names, MapIssueKind::NotObserved);
                }
            }
        }
        let cache = slot.catalog.cache.read();
        if cache.generation() != pass.epoch || slot.closing_token().is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let mut projection = project_schema_map(
            &cache,
            request.connection,
            request.session,
            &paths,
            slot.capabilities(),
            slot.schema_sample_limits(),
        );
        for object in &mut projection.objects {
            for issue in &pass.issues {
                if issue.address == object.address && issue.kind == MapIssueKind::Failed {
                    match issue.facet {
                        MapFacet::Fields
                            if object.fields_state
                                != oxyn_catalog::schema_map::MapFreshness::Unread =>
                        {
                            object.fields_state = oxyn_catalog::schema_map::MapFreshness::Stale
                        }
                        MapFacet::OutgoingKeys
                            if object.keys_state
                                != oxyn_catalog::schema_map::MapFreshness::Unread =>
                        {
                            object.keys_state = oxyn_catalog::schema_map::MapFreshness::Stale
                        }
                        _ => {}
                    }
                }
            }
        }
        for link in &mut projection.links {
            let failed_source = projection
                .objects
                .iter()
                .find(|object| object.key == link.from);
            if failed_source.is_some_and(|object| {
                object.fields_state == oxyn_catalog::schema_map::MapFreshness::Stale
            }) {
                link.source_unique = None;
            }
            if projection.objects.iter().any(|object| {
                object.key == link.to
                    && object.fields_state == oxyn_catalog::schema_map::MapFreshness::Stale
            }) {
                link.target_unique = None;
            }
            if link.kind == oxyn_catalog::schema_map::MapLinkKind::ForeignKey
                && failed_source.is_some_and(|object| {
                    object.keys_state == oxyn_catalog::schema_map::MapFreshness::Stale
                })
            {
                link.freshness = oxyn_catalog::schema_map::MapFreshness::Stale;
            }
        }
        projection.coverage.stale_objects = projection
            .objects
            .iter()
            .filter(|object| {
                object.fields_state == oxyn_catalog::schema_map::MapFreshness::Stale
                    || object.keys_state == oxyn_catalog::schema_map::MapFreshness::Stale
            })
            .count();
        projection.coverage.omitted_objects += omitted;
        projection.coverage.known_objects += omitted;
        let mut map = SchemaMapLoaded {
            projection,
            issues: pass.issues,
            reads: pass.reads,
            choose_namespace,
            stopped: pass.stopped,
        };
        publication::bound(&mut map)?;
        Ok(Outcome::SchemaMapLoaded { map })
    }
}
