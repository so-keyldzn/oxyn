//! Explicit session selection never falls back to connection-level metadata.

use super::*;

impl Backend {
    pub(super) fn catalog_for(
        &self,
        connection: ConnectionId,
        source: Option<SessionId>,
    ) -> Result<oxyn_catalog::SharedCatalog, IpcError> {
        match source {
            Some(session) => self
                .inner
                .executor
                .session_catalog(connection, session)
                .map_err(IpcError::from),
            None => self.catalog_cache(connection),
        }
    }
}

#[cfg(test)]
impl Backend {
    pub fn preview_pagination(
        &self,
        connection: ConnectionId,
        address: &CatalogAddress,
        applied: &PreviewShapeDraft,
        rows: u64,
    ) -> Result<Pagination, IpcError> {
        self.preview_pagination_for_session(connection, address, applied, rows, None, None)
    }

    pub fn catalog_tree(&self, connection: ConnectionId) -> Result<Vec<CatalogNode>, IpcError> {
        self.catalog_tree_for_session(connection, None)
    }

    pub fn relation_facets(
        &self,
        connection: ConnectionId,
        address: &CatalogAddress,
    ) -> Result<RelationFacets, IpcError> {
        self.relation_facets_for_session(connection, address, None)
    }

    pub fn search_catalog(
        &self,
        connection: ConnectionId,
        query: &str,
    ) -> Result<Vec<CatalogSearchHit>, IpcError> {
        self.search_catalog_for_session(connection, query, None)
    }

    pub fn related_rows_template(
        &self,
        connection: ConnectionId,
        address: &CatalogAddress,
        incoming: bool,
        index: usize,
        source: Option<RelatedRowsSource>,
    ) -> Result<Option<RelatedRowsQuery>, IpcError> {
        self.related_rows_template_for_session(connection, address, incoming, index, source, None)
    }

    pub fn compose_object_sql(
        &self,
        connection: ConnectionId,
        address: &CatalogAddress,
        form: ObjectSqlForm,
    ) -> Result<String, IpcError> {
        self.compose_object_sql_for_session(connection, address, form, None)
    }
}
