//! Console-local metadata never enters another session's catalog cache.

use super::*;

impl Executor {
    /// Whether this live session was opened with the immutable catalog role.
    #[must_use]
    pub fn is_catalog_session(&self, connection: ConnectionId, session: SessionId) -> bool {
        self.sessions.get(session).is_some_and(|slot| {
            slot.connection() == connection && slot.is_catalog() && slot.is_open()
        })
    }

    /// Whether a separate execution session can safely address this source's objects.
    /// Arbitrary user SQL may have installed a temporary homonym or private settings.
    #[must_use]
    pub fn is_reproducible_catalog_source(
        &self,
        connection: ConnectionId,
        session: SessionId,
    ) -> bool {
        self.sessions.get(session).is_some_and(|slot| {
            slot.connection() == connection
                && slot.is_catalog()
                && slot.is_open()
                && !slot.has_private_context()
        })
    }

    /// Whether a displayed result still owns its retained read, independently
    /// of catalog-cache eviction. Checks the original session and object.
    pub fn retains_preview_result(
        &self,
        connection: ConnectionId,
        session: SessionId,
        result: ResultId,
        path: &oxyn_catalog::CatalogPath,
    ) -> Result<bool> {
        self.previews
            .retains_result(connection, session, result, path)
    }

    /// Reads only the chosen live session's cache, without server I/O.
    /// A closed or mismatched owner never falls back to another session.
    pub fn session_catalog(
        &self,
        connection: ConnectionId,
        session: SessionId,
    ) -> Result<SharedCatalog> {
        Ok(Arc::clone(
            &self.catalog_target(connection, session)?.0.catalog.cache,
        ))
    }

    pub(super) fn catalog_target(
        &self,
        connection: ConnectionId,
        session: SessionId,
    ) -> Result<(Arc<SessionSlot>, Arc<crate::catalog::ConnectionCatalog>)> {
        let slot = self
            .sessions
            .get(session)
            .filter(|slot| slot.connection() == connection && slot.is_open() && !slot.is_preview())
            .ok_or_else(|| {
                OxynError::Connection("the catalog's owning session is no longer open".into())
            })?;
        let state = self
            .catalogs
            .read()
            .get(&connection)
            .cloned()
            .ok_or_else(|| OxynError::Connection("the connection is no longer open".into()))?;
        Ok((slot, state))
    }

    pub(super) async fn refresh_session_catalog(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        scope: &CatalogRefreshScope,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        self.refresh_session_catalog_with_priority(
            id,
            connection,
            session,
            scope,
            cancel,
            crate::catalog::priority::Priority::Demand,
        )
        .await
    }

    pub(super) async fn refresh_session_catalog_with_priority(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        scope: &CatalogRefreshScope,
        cancel: &CancelToken,
        priority: crate::catalog::priority::Priority,
    ) -> Result<Outcome> {
        let (slot, state) = self.catalog_target(connection, session)?;
        let _turn = state
            .queue
            .enter(id, priority, cancel, slot.closing_token())
            .await?;
        let mut budget = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(OxynError::Cancelled),
            _ = slot.closing_token().cancelled() => return Err(OxynError::Cancelled),
            guard = state.refresh.lock() => guard,
        };
        let generation = slot.catalog.cache.read().generation();
        if let crate::catalog::priority::Priority::Prefetch(expected)
        | crate::catalog::priority::Priority::DemandAt(expected) = priority
            && generation != expected
        {
            return Err(OxynError::Cancelled);
        }
        let patch = slot.read_catalog(scope, cancel).await?;
        if cancel.is_cancelled() || slot.closing_token().is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let metadata_bytes = u64::try_from(patch.encoded_bytes()?)
            .map_err(|_| OxynError::Config("metadata byte count exceeds report range".into()))?;
        let generation =
            self.publish_catalog_patch(&slot, &mut budget, scope, patch, generation, cancel)?;
        self.events
            .publish(id, Some(connection), Event::CatalogUpdated);
        Ok(Outcome::CatalogRefreshed {
            connection,
            scope: scope.clone(),
            metadata_bytes,
            generation,
        })
    }
    pub(super) fn publish_catalog_patch(
        &self,
        slot: &SessionSlot,
        budget: &mut crate::catalog::CatalogBudget,
        scope: &CatalogRefreshScope,
        patch: crate::catalog::CatalogPatch,
        generation: oxyn_catalog::CatalogGenerationId,
        cancel: &CancelToken,
    ) -> Result<oxyn_catalog::CatalogGenerationId> {
        slot.catalog.publish_checked(generation, |cache| {
            if cancel.is_cancelled() || slot.closing_token().is_cancelled() {
                return Err(OxynError::Cancelled);
            }
            let evicted = budget.reserve_for(slot.id(), scope, &patch)?;
            for (owner, evicted_scope) in evicted {
                if owner == slot.id() {
                    cache.evict(&evicted_scope);
                } else if let Some(other) = self.sessions.get(owner) {
                    other.catalog.cache.write().evict(&evicted_scope);
                }
            }
            if let Err(error) = patch.apply(cache) {
                budget.release_owner(slot.id());
                return Err(error);
            }
            Ok(cache.generation())
        })
    }
}
