//! A preview never issues a separate document sample while composing its query.
use super::*;
use oxyn_catalog::Relation;
use oxyn_driver::{PreparedPreview, PreviewMode};

impl MongoSession {
    pub(super) async fn compose_preview(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        known: Option<&Relation>,
        retained: bool,
        cancel: &CancelToken,
    ) -> Result<PreparedPreview> {
        let needs_fields = shape.needs_total_order() || shape.columns.is_some();
        let relation = if needs_fields {
            Some(known.ok_or_else(|| OxynError::NotSupported {
                capability: "read field structure explicitly in the Structure tab before sorting, projecting or independently paging this documentary source".into(),
            })?)
        } else {
            None
        };
        let ordered = if let Some(relation) = relation {
            relation
                .primary_key()
                .iter()
                .any(|field| field.name == "_id")
        } else {
            // Object kind is declared metadata; no document values are read.
            self.catalog.previews_in_id_order(path, cancel).await?
        };
        let request = if retained {
            crate::preview::stream_request(path, limit, shape, relation, ordered)?
        } else {
            crate::preview::request(path, limit, shape, relation, ordered)?
        };
        Ok(PreparedPreview {
            request,
            mode: if retained {
                PreviewMode::Retained
            } else {
                PreviewMode::Bounded
            },
        })
    }
}
