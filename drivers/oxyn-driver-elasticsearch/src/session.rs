//! An Elasticsearch/OpenSearch session with conservative local guards.

use std::time::Instant;

use async_trait::async_trait;
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    CancelToken, Capabilities, ExecRequest, OxynError, PreviewShape, Result, StatementHandle,
};
use oxyn_driver::{Cursor, Session};
use serde_json::json;

use crate::api::{Api, encode_segment};
use crate::catalog::ElasticsearchCatalog;
use crate::cursor::JsonCursor;
use crate::request::{parse_request, statement_intent};

/// An open HTTP session. Elasticsearch and OpenSearch have no transaction or
/// server-enforced read-only session mode, so neither capability is declared.
pub(crate) struct ElasticsearchSession {
    api: Api,
    capabilities: Capabilities,
    catalog: ElasticsearchCatalog,
}

impl ElasticsearchSession {
    pub(crate) fn new(
        api: Api,
        product: String,
        version: String,
        capabilities: Capabilities,
    ) -> Self {
        let catalog = ElasticsearchCatalog::new(api.clone(), product, version, capabilities);
        Self {
            api,
            capabilities,
            catalog,
        }
    }
}

#[async_trait]
impl Session for ElasticsearchSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        if !request.params.is_empty() {
            return Err(OxynError::NotSupported {
                capability: "Search DSL bound parameters".to_owned(),
            });
        }
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let parsed = parse_request(&request.text).map_err(OxynError::Query)?;
        let intent = statement_intent(&parsed);
        if request.limits.read_only && intent.is_mutating() {
            return Err(OxynError::Query(
                "this execution is bounded to read-only and the request may modify the cluster"
                    .to_owned(),
            ));
        }
        let response = self
            .api
            .json(
                &parsed.method,
                &parsed.path,
                parsed.body.as_ref(),
                intent,
                request.limits.timeout,
            )
            .await?;
        if parsed
            .path
            .split('?')
            .next()
            .is_some_and(|path| path.ends_with("/_bulk") || path == "/_bulk")
            && response.get("errors").and_then(serde_json::Value::as_bool) == Some(true)
        {
            return Err(OxynError::Query(
                "the bulk response contains one or more item failures".to_owned(),
            ));
        }
        Ok(Box::new(JsonCursor::from_response(
            StatementHandle::new(),
            response,
        )?))
    }

    async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        if !(1..=1_000).contains(&limit) {
            return Err(OxynError::Config(
                "an Elasticsearch preview limit is in 1..=1000".to_owned(),
            ));
        }
        if !shape.is_plain() {
            return Err(OxynError::NotSupported {
                capability: "Elasticsearch preview sorting, filtering, and pages beyond the first"
                    .to_owned(),
            });
        }
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let Some(index) = path.relation() else {
            return Err(OxynError::Query(
                "an Elasticsearch preview needs an index".to_owned(),
            ));
        };
        let source = match shape.projection()? {
            Some(columns) => json!({"includes": columns}),
            None => json!(true),
        };
        let body = json!({
            "size": limit,
            "query": {"match_all": {}},
            "sort": ["_shard_doc"],
            "_source": source,
        });
        let text = format!("POST /{}/_search\n{}", encode_segment(index), body);
        Ok(ExecRequest::new(oxyn_core::QueryLanguage::SearchDsl, text)
            .with_intent(oxyn_core::StatementIntent::Read)
            .with_limits(oxyn_core::ExecLimits::default().with_max_rows(
                usize::try_from(limit).map_err(|_| {
                    OxynError::Config("the preview limit does not fit this platform".to_owned())
                })?,
            )))
    }

    async fn cancel(&self, _statement: StatementHandle) -> Result<()> {
        Err(OxynError::NotSupported {
            capability: "SERVER_SIDE_CANCEL".to_owned(),
        })
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    async fn ping(&self) -> Result<std::time::Duration> {
        let start = Instant::now();
        let _ = self
            .api
            .json("GET", "/", None, oxyn_core::StatementIntent::Read, None)
            .await?;
        Ok(start.elapsed())
    }

    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}
