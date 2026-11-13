//! Preview composition and isolated readers preserve the live session's target.

use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{CancelToken, OxynError, PreviewShape, Result};
use oxyn_driver::{PreparedPreview, PreviewMode, Session};

use super::SqliteSession;
use crate::{preview, preview_context, worker};

impl SqliteSession {
    pub(super) async fn prepare_data_preview(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
        allow_retained: bool,
    ) -> Result<PreparedPreview> {
        let described = CatalogPath::for_relation(
            None,
            Some(preview::database_of(path)?),
            path.relation().ok_or_else(|| {
                OxynError::CatalogUnavailable("preview path must name a relation".into())
            })?,
        )?;
        let (facts, retained) = match self.catalog.describe_relation(&described, cancel).await {
            Ok(relation) => {
                let mut facts = preview::RelationFacts::of(&relation);
                if relation.has_preview_key() {
                    facts =
                        facts.with_key_index(&self.catalog.key_index(&described, cancel).await?);
                }
                (facts, allow_retained && relation.retained_preview)
            }
            Err(error)
                if error.missing_module().is_some()
                    && !shape.needs_total_order()
                    && shape.columns.is_none() =>
            {
                (preview::RelationFacts::default(), false)
            }
            Err(error) => return Err(error),
        };
        let mode = if retained {
            let shares_memory = crate::preview_uri::shares_memory(&self.spec);
            let reproducible = self
                .worker
                .call(cancel, move |connection| {
                    Ok(preview_context::Context::read(connection, shares_memory)?.reproducible())
                })
                .await?;
            if preview::database_of(path)? == "temp" || !reproducible {
                PreviewMode::Original
            } else {
                PreviewMode::Dedicated
            }
        } else {
            PreviewMode::Bounded
        };
        Ok(PreparedPreview {
            request: if retained {
                preview::stream_request(path, limit, shape, &facts)?
            } else {
                preview::request(path, limit, shape, &facts)?
            },
            mode,
        })
    }

    pub(super) async fn fork_preview(
        &self,
        path: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let shares_memory = crate::preview_uri::shares_memory(&self.spec);
        let context = self
            .worker
            .call(cancel, move |connection| {
                preview_context::Context::read(connection, shares_memory)
            })
            .await?;
        if preview::database_of(path)? == "temp" || !context.reproducible() {
            return Err(OxynError::NotSupported {
                capability:
                    "retained previews of session-local temporary or private in-memory data".into(),
            });
        }
        let spec = crate::preview_uri::read_only(self.spec.clone());
        let (worker, thread) = worker::spawn(spec.clone(), cancel).await?;
        let session = Box::new(Self::new(
            worker,
            thread,
            crate::SqliteDriver::session_capabilities(true),
            self.limits,
            spec,
        ));
        if let Err(error) = context.apply(&session.worker, cancel).await {
            let _ = session.close().await;
            return Err(error);
        }
        Ok(session)
    }
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
