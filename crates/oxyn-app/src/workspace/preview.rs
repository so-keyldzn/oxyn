//! Read-only table previews with a separate result buffer and request identity.

use super::*;

pub(super) mod filter;

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

/// Ce que le pied d'un aperçu annonce.
///
/// La mention qui compte est la dernière : **le total n'a pas été demandé**. Le
/// relevé `190:1163` l'écrit, et le code ne le disait pas — il annonçait « Up
/// to 200 rows », ce qui borne la lecture sans dire que le nombre affiché n'est
/// pas celui de la table.
///
/// L'écart n'est pas cosmétique. Un aperçu de 200 lignes sur une table qui en
/// contient cinquante millions ressemble en tout point à un aperçu de 200 lignes
/// sur une table qui en contient 200 : rien à l'écran ne les distingue. C'est le
/// même piège que celui de l'export d'un tampon tronqué
/// ([UX-SPEC](../../../docs/UX-SPEC.md#ce-qui-est-exporté-est-ce-qui-est-affiché)),
/// et Oxyn ne compte **jamais** les lignes d'une table pour l'afficher — un
/// `COUNT(*)` sur cinquante millions de lignes est une requête que personne n'a
/// demandée.
///
/// # Face à la maquette
///
/// `190:1860` écrit « 200 rows loaded · 284 ms · Total count not requested »
/// (relevé au serveur le 2026-09-15). **La durée en venait et manquait** : elle
/// n'est pas décorative, c'est ce qui distingue une base lente d'un aperçu qui
/// n'a rien trouvé. Deux écarts restent, assumés et consignés dans
/// [FIGMA-HANDOFF](../../../docs/FIGMA-HANDOFF.md) : « shown » plutôt que
/// « loaded », pour la raison ci-dessus, et pas de « Read only » ici — la barre
/// Data le porte déjà quarante pixels plus haut, et la maquette ne l'écrit
/// qu'une fois.
pub(super) fn preview_notice(rows: u64, duree: std::time::Duration) -> String {
    format!(
        "{rows} rows shown · {} ms · Total count not requested",
        duree.as_millis()
    )
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
            // Another relation has other columns, another key and no reason to
            // inherit the predicate written for the previous one.
            self.reset_preview_controls(cx);
        }
        if self.preview_available() && self.preview_path.is_none() {
            self.load_preview(cx);
        }
        // Choosing an object is what the next session restores; the chosen
        // object is now a fact of the catalog, so the location is settled.
        self.location_unconfirmed = false;
        self.persist_location(cx);
        cx.notify();
    }

    /// Reads the shape in force again, from the top.
    ///
    /// Automatic browsing: it yields to a read already on the wire rather than
    /// superseding it, because nobody asked for this one.
    pub(super) fn load_preview(&mut self, cx: &mut Context<'_, Self>) {
        if self.preview_active.is_some() {
            return;
        }
        let shape = self.preview_controls.applied.clone();
        self.start_preview(shape, cx);
    }

    /// Reads a shape the user composed.
    ///
    /// Unlike [`Self::load_preview`], this supersedes a read on the wire and
    /// stops it at the server: the answer to the previous shape is no longer
    /// what anyone is waiting for, and leaving it running keeps a connection
    /// busy for rows nobody will see.
    pub(in crate::workspace) fn reshape_preview(
        &mut self,
        shape: oxyn_core::PreviewShape,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some((_, cancel)) = self.preview_active.take() {
            cancel.cancel();
        }
        self.start_preview(shape, cx);
    }

    fn start_preview(&mut self, shape: oxyn_core::PreviewShape, cx: &mut Context<'_, Self>) {
        if !self.preview_available() {
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
            shape: shape.clone(),
        };
        let id = CommandId::new();
        let cancel = CancelToken::new();
        // What was asked for, kept apart from what is on screen: the display
        // moves on the answer and never before it
        // ([UX-SPEC](../../../docs/UX-SPEC.md#ce-qui-nest-jamais-optimiste)).
        self.preview_controls.requested = shape;
        // This read leaves now, so it will see every change already applied:
        // it settles whatever automatic re-read was owed before it.
        self.preview_refresh_owed = false;
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
        // The debt is settled either way, but it is only paid to a read that
        // brought rows back: re-reading after a failure, a cancellation or a
        // refusal would replace the message with rows and hide what happened
        // ([UX-SPEC](../../../docs/UX-SPEC.md#données-dune-table-sélectionnée)).
        let owed = std::mem::take(&mut self.preview_refresh_owed)
            && matches!(&result, Ok(Outcome::Executed { sink, .. }) if !matches!(sink, oxyn_data::SinkOutcome::Cancelled));
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
                    preview_notice(stats.rows, stats.total_time)
                };
                // The shape becomes the one in force exactly when its rows reach
                // the grid — including a cancelled read that produced some, since
                // those rows did come from it.
                let showed_rows = !cancelled || buffer.row_count() > 0;
                self.preview_grid.update(cx, |grid, cx| {
                    if showed_rows {
                        grid.set_buffer(buffer, cx);
                        grid.on_batch(cx);
                    } else {
                        grid.cancelled(cx);
                    }
                });
                if showed_rows {
                    self.preview_controls.applied = self.preview_controls.requested.clone();
                }
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
        // The menu committed the rank the moment the user accepted it. Whatever
        // this answer was, it must now name the order the rows on screen have —
        // which, for a refused read, is the one from before.
        self.resync_preview_sort(cx);
        // Whatever changed while these rows were on the wire is answered by one
        // read, not one per change: this is where the coalescing of ADR-0022
        // actually happens. It carries the shape in force, so the filter, the
        // sort and the page the user chose survive the refresh.
        if owed {
            self.load_preview(cx);
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
