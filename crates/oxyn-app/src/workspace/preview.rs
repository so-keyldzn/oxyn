//! Read-only table previews with a separate result buffer and request identity.

use super::*;

pub(super) const PREVIEW_ROWS: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ObjectTab {
    Data,
    Structure,
    Indexes,
    Constraints,
    Relations,
    IncomingRelations,
    Ddl,
}

impl Workspace {
    pub(super) fn preview_available(&self) -> bool {
        self.capabilities.contains(Capabilities::SQL)
            && self.selected_path.as_ref().is_some_and(|path| {
                self.catalog_cache.try_read().is_some_and(|cache| {
                    cache
                        .relation_summary(path)
                        .is_some_and(|relation| relation.kind.holds_records())
                })
            })
    }

    pub(super) fn select_object(&mut self, path: CatalogPath, cx: &mut Context<'_, Self>) {
        self.catalog_overlay = false;
        let changed = self.selected_path.as_ref() != Some(&path);
        self.selected_path = Some(path);
        self.panel = WorkspacePanel::Object;
        self.object_tab = ObjectTab::Data;
        self.cancel_definition(cx);
        if changed {
            self.reset_definition(cx);
            self.close_value(cx);
            self.inspected_column = 0;
            if let Some((_, cancel)) = self.preview_active.take() {
                cancel.cancel();
            }
            self.preview_path = None;
            self.preview_result = None;
            self.displayed_results.remove(&ResultSource::Preview);
            self.preview_export_open = self
                .export_active
                .as_ref()
                .is_some_and(|run| run.2 == ResultSource::Preview);
            self.preview_export.update(cx, |export, cx| {
                export.reset(cx);
                export.set_result_ready(Some(NotExportable::NoResult), cx);
            });
            self.preview_notice.clear();
            self.preview_grid.update(cx, DataGrid::reset);
        }
        if self.preview_available() && self.preview_path.is_none() {
            self.load_preview(cx);
        }
        cx.notify();
    }

    pub(super) fn load_preview(&mut self, cx: &mut Context<'_, Self>) {
        if !self.preview_available() || self.preview_active.is_some() {
            return;
        }
        let Some(path) = self.selected_path.clone() else {
            return;
        };
        let Some(relation) = path.relation() else {
            return;
        };
        let command = Command::PreviewRelation {
            connection: self.connection,
            session: self.session,
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: relation.to_owned(),
            limit: PREVIEW_ROWS,
        };
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.preview_path = Some(path);
        self.preview_active = Some((id, cancel.clone()));
        self.close_value(cx);
        self.preview_notice = "Loading rows…".into();
        self.preview_result = None;
        self.displayed_results.remove(&ResultSource::Preview);
        self.preview_export.update(cx, |export, cx| {
            export.set_result_ready(Some(NotExportable::NoResult), cx);
        });
        self.preview_grid.update(cx, DataGrid::start);
        let response = self.backend.dispatch(id, command, cancel);
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal("The executor stopped answering".into()))
            });
            let _ = this.update(cx, |this, cx| this.complete_preview(id, result, cx));
        })
        .detach();
        cx.notify();
    }

    pub(super) fn complete_preview(
        &mut self,
        id: CommandId,
        result: Result<Outcome, OxynError>,
        cx: &mut Context<'_, Self>,
    ) {
        // A late reply must never replace another table's rows or errors.
        if self.preview_active.as_ref().map(|run| run.0) != Some(id) {
            if let Ok(Outcome::NeedsApproval { command, .. }) = result {
                drop(self.backend.decide(command, false, CancelToken::new()));
            }
            return;
        }
        self.preview_active = None;
        match result {
            Ok(Outcome::Executed {
                result,
                buffer,
                stats,
                sink,
                ..
            }) => {
                self.displayed_results.insert(ResultSource::Preview, result);
                let cancelled = matches!(sink, oxyn_data::SinkOutcome::Cancelled);
                let blocked = export::exportability(
                    cancelled,
                    buffer.is_complete(),
                    buffer.stats().truncated,
                );
                self.preview_result = blocked.is_none().then_some(result);
                self.preview_export.update(cx, |export, cx| {
                    export.reset(cx);
                    export.set_preview_rows(buffer.row_count(), cx);
                    export.set_result_ready(blocked, cx);
                });
                self.preview_notice = if cancelled {
                    "Preview cancelled".into()
                } else {
                    format!(
                        "{} rows shown · Up to {PREVIEW_ROWS} rows · Read only",
                        stats.rows
                    )
                };
                self.preview_grid.update(cx, |grid, cx| {
                    if cancelled && buffer.row_count() == 0 {
                        grid.cancelled(cx);
                    } else {
                        grid.set_buffer(buffer, cx);
                        grid.on_batch(cx);
                    }
                });
            }
            Err(OxynError::Cancelled) => {
                self.preview_notice = "Preview cancelled · Refresh to load again".into();
                self.preview_grid.update(cx, DataGrid::cancelled);
            }
            Err(error) => {
                self.preview_notice = "Preview failed".into();
                self.preview_grid.update(cx, |grid, cx| {
                    grid.fail(error.to_string(), error.is_retryable(), cx)
                });
            }
            Ok(Outcome::NeedsApproval { command, .. }) => {
                // Automatic browsing never keeps a hidden approval pending.
                drop(self.backend.decide(command, false, CancelToken::new()));
                self.preview_notice = "Preview requires approval · Use the SQL editor".into();
                self.preview_grid.update(cx, |grid, cx| {
                    grid.fail(
                        "This connection policy requires an explicit query approval",
                        false,
                        cx,
                    )
                });
            }
            Ok(Outcome::Denied { reason, .. }) => {
                self.preview_notice = "Preview not permitted".into();
                self.preview_grid
                    .update(cx, |grid, cx| grid.fail(reason, false, cx));
            }
            Ok(_) => {
                self.preview_notice = "Preview failed".into();
                self.preview_grid.update(cx, |grid, cx| {
                    grid.fail("Unexpected preview response", false, cx)
                });
            }
        }
        cx.notify();
    }

    pub(super) fn cancel_preview(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((_, cancel)) = &self.preview_active {
            cancel.cancel();
            self.preview_notice = "Cancelling preview…".into();
            cx.notify();
        }
    }
}
