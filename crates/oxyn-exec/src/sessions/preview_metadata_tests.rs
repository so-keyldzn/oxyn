//! Metadata hints cannot cross owner caches or turn stale structure into permission to sample.
use super::*;
use async_trait::async_trait;
use oxyn_catalog::{
    CatalogPath, CatalogProvider, CatalogScope, Relation, RelationKind, ServerInfo,
};
use oxyn_core::PreviewShape;
use oxyn_driver::{PreparedPreview, Session};
use parking_lot::Mutex;

struct HintProbe(Arc<Mutex<Vec<Option<String>>>>);
#[async_trait]
impl CatalogProvider for HintProbe {
    async fn list_relations(
        &self,
        _: &CatalogPath,
        _: &CancelToken,
    ) -> Result<Vec<oxyn_catalog::RelationRef>> {
        panic!("cached preview planning must not discover metadata")
    }
    async fn describe_relation(&self, _: &CatalogPath, _: &CancelToken) -> Result<Relation> {
        panic!("cached preview planning must not sample structure")
    }
    async fn server_info(&self, _: &CancelToken) -> Result<ServerInfo> {
        Ok(ServerInfo::new(
            "fixture",
            "",
            Capabilities::INFERRED_SCHEMA,
        ))
    }
}
#[async_trait]
impl Session for HintProbe {
    fn capabilities(&self) -> Capabilities {
        Capabilities::INFERRED_SCHEMA
    }
    fn catalog(&self) -> &dyn CatalogProvider {
        self
    }
    async fn execute(
        &self,
        _: ExecRequest,
        _: &CancelToken,
    ) -> Result<Box<dyn oxyn_driver::Cursor>> {
        panic!("planning reads no rows")
    }
    async fn cancel(&self, _: StatementHandle) -> Result<()> {
        Ok(())
    }
    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::ZERO)
    }
    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
    async fn plan_preview(
        &self,
        _: &CatalogPath,
        _: u32,
        _: &PreviewShape,
        known: Option<&Relation>,
        _: &CancelToken,
    ) -> Result<PreparedPreview> {
        self.0
            .lock()
            .push(known.map(|relation| relation.name.clone()));
        Err(OxynError::NotSupported {
            capability: "fixture records planning only".into(),
        })
    }
}
#[tokio::test]
async fn preview_planning_uses_only_fresh_structure_from_the_original_owner() {
    let connection = ConnectionId::new();
    let hints = Arc::new(Mutex::new(Vec::new()));
    let other = SessionSlot::new(connection, Box::new(HintProbe(hints.clone())));
    let owner = SessionSlot::new(connection, Box::new(HintProbe(hints.clone())));
    let path = CatalogPath::for_relation(None, Some("db"), "items").expect("path");
    other
        .catalog
        .cache
        .write()
        .set_relation(&path, Relation::new("items", RelationKind::Collection))
        .expect("other owner");
    let shape = PreviewShape::unordered();
    let cancel = CancelToken::new();
    assert!(
        owner
            .preview_request(&path, 10, &shape, &cancel)
            .await
            .is_err()
    );
    owner
        .catalog
        .cache
        .write()
        .set_relation(&path, Relation::new("items", RelationKind::Collection))
        .expect("explicitly read on owner");
    assert!(
        owner
            .preview_request(&path, 10, &shape, &cancel)
            .await
            .is_err()
    );
    owner
        .catalog
        .cache
        .write()
        .invalidate(&CatalogScope::Description(path.clone()));
    assert!(
        owner
            .preview_request(&path, 10, &shape, &cancel)
            .await
            .is_err()
    );
    assert_eq!(*hints.lock(), [None, Some("items".into()), None]);
}
