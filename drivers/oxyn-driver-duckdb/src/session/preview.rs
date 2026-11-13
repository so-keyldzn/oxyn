//! Retained previews use the same engine and access policy, on another worker.

use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{CancelToken, OxynError, PreviewShape, Result};
use oxyn_driver::{PreparedPreview, PreviewMode, Session};

use super::DuckDbSession;

impl DuckDbSession {
    pub(super) async fn prepare_data_preview(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
        allow_retained: bool,
    ) -> Result<PreparedPreview> {
        let relation = self.catalog.describe_relation(path, cancel).await?;
        let retained = allow_retained && relation.retained_preview;
        let facts = crate::preview::RelationFacts::of(&relation);
        Ok(PreparedPreview {
            request: if retained {
                crate::preview::stream_request(path, limit, shape, &facts)?
            } else {
                crate::preview::request(path, limit, shape, &facts)?
            },
            mode: if retained {
                if self
                    .worker
                    .call(cancel, crate::preview_context::reproducible)
                    .await?
                {
                    PreviewMode::Dedicated
                } else {
                    PreviewMode::Original
                }
            } else {
                PreviewMode::Bounded
            },
        })
    }

    pub(super) async fn fork_preview(
        &self,
        _path: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let context = self
            .worker
            .call(cancel, |connection| {
                if !crate::preview_context::reproducible(connection)? {
                    return Err(OxynError::NotSupported {
                        capability:
                            "retained previews while the source session has temporary objects"
                                .into(),
                    });
                }
                super::current_context(connection)
            })
            .await?;
        // Opening through the original spec reuses the same engine anchor,
        // including :memory: and ATTACH. Changing read_only/file_access here
        // would either reopen a different engine or violate its access policy.
        let session =
            Box::new(Self::open(self.spec.clone(), self.capabilities, self.limits, cancel).await?);
        if let Err(error) = session.set_context(&context, cancel).await {
            let _ = session.close().await;
            return Err(error);
        }
        Ok(session)
    }
}
