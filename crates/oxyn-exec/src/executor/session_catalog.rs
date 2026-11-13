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

    fn catalog_target(
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
        let (slot, state) = self.catalog_target(connection, session)?;
        let mut budget = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(OxynError::Cancelled),
            _ = slot.closing_token().cancelled() => return Err(OxynError::Cancelled),
            guard = state.refresh.lock() => guard,
        };
        let patch = slot.read_catalog(scope, cancel).await?;
        if cancel.is_cancelled() || slot.closing_token().is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let evicted = budget.reserve_for(session, scope, &patch)?;
        self.evict_session_catalogs(evicted);
        if let Err(error) = slot.catalog.publish(patch) {
            budget.release_owner(session);
            return Err(error);
        }
        self.events
            .publish(id, Some(connection), Event::CatalogUpdated);
        Ok(Outcome::CatalogRefreshed {
            connection,
            scope: scope.clone(),
        })
    }
    pub(super) fn evict_session_catalogs(
        &self,
        evicted: Vec<(SessionId, oxyn_catalog::CatalogScope)>,
    ) {
        for (owner, scope) in evicted {
            if let Some(slot) = self.sessions.get(owner) {
                slot.catalog.cache.write().evict(&scope);
            }
        }
    }
}
