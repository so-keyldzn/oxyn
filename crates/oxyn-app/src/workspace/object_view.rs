//! Table data and structure share object identity while keeping query drafts separate.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, div, px, uniform_list};
use oxyn_ui::Theme;

impl Workspace {
    pub(super) fn object_details(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let path = self.selected_path.as_ref();
        let is_relation = path.is_some_and(|p| p.relation().is_some());
        div().flex_1().min_h_0().flex().flex_col().gap_2().p_3()
            .when(is_relation, |el| el.child(self.object_tabs(cx)))
            .child(if !is_relation {
                div().p_4().text_color(theme.colors.text_muted).child("Select a table in the sidebar to see its data.").into_any_element()
            } else if self.object_tab == ObjectTab::Ddl {
                self.definition_panel(cx)
            } else if self.object_tab == ObjectTab::Structure {
                self.with_definition_panel(self.object_structure(cx), cx)
            } else if matches!(self.object_tab, ObjectTab::Indexes | ObjectTab::Relations | ObjectTab::IncomingRelations | ObjectTab::Constraints) {
                self.with_definition_panel(self.object_metadata(cx), cx)
            } else if self.preview_available() {
                div().flex_1().min_h_0().flex().flex_col().gap_2()
                    .child(div().flex().items_center().gap_2().flex_none()
                        .child(self.control("refresh-preview", if self.preview_active.is_some() { "Cancel · Esc" } else { "Refresh data" }, Control::Preview, false, cx))
                        .when(!self.compact_layout, |el| el
                            .child(self.control("preview-columns", self.columns_label(ResultSource::Preview, cx), Control::Columns, false, cx))
                            .child(self.control("preview-export-toggle", "Export preview…", Control::PreviewExport, false, cx)))
                        .when(self.compact_layout, |el| el.child(self.control("preview-actions", "Actions", Control::ResultActions, false, cx)))
                        .when(self.compact_layout || !self.inspector_open, |el| el.child(self.control("preview-inspect-row", "Inspect row", Control::Inspector, false, cx)))
                        .child(div().flex_1().whitespace_nowrap().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child("Read-only preview"))
                        .child(div().h(px(32.)).px_3().flex().items_center().border_1().border_color(theme.colors.border).rounded(theme.radii.control).opacity(0.5).child("Edit rows…"))
                        .child(self.control("preview-edit-help", "Why unavailable?", Control::ObjectHelp, false, cx)))
                    .when(self.compact_layout && self.result_actions_open, |el| el.child(div().flex().gap_2().flex_none()
                        .child(self.control("preview-columns-menu", self.columns_label(ResultSource::Preview, cx), Control::Columns, false, cx))
                        .child(self.control("preview-export-menu", "Export preview…", Control::PreviewExport, false, cx))))
                    .when(self.columns_open, |el| el.child(self.column_manager(ResultSource::Preview, cx)))
                    .when(self.object_help_open, |el| el.child(div().p_3().border_1().border_color(theme.colors.border).rounded(theme.radii.control).child("Editing requires an editable view, a writable session and appropriate driver capabilities. This preview is read only. Production writes require a review naming the connection and showing the exact SQL.")))
                    .when(self.preview_export_open, |el| el.child(self.preview_export.clone()))
                    .child(self.result_area(ResultSource::Preview, cx))
                    .child(div().flex().items_center().justify_between().text_size(theme.typography.small_size).text_color(theme.colors.text_muted)
                        .child(self.preview_notice.clone())
                        .child(self.control("preview-text-size", self.reading_label(cx), Control::ToggleReading, false, cx)))
                    .child(div().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child("Preview is limited to 200 rows. Refresh data starts a new read. Export includes only these preview rows."))
                    .into_any_element()
            } else {
                div().p_4().text_color(theme.colors.text_muted).child("This object does not support a SQL data preview. Its metadata is available in Structure.").into_any_element()
            })
            .into_any_element()
    }

    fn object_structure(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let count = self.selected_path.as_ref().and_then(|path| {
            self.catalog_cache
                .try_read()
                .and_then(|cache| cache.relation(path).map(|relation| relation.fields.len()))
        });
        let heading = div()
            .flex()
            .gap_3()
            .px_3()
            .py_2()
            .bg(theme.colors.surface_raised)
            .child(div().flex_1().child("Column"))
            .child(div().w(px(180.)).child("Type"))
            .child(div().w(px(100.)).child("Nullable"))
            .child(div().w(px(100.)).child("Key"));
        let mut body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.structure_toolbar("refresh-structure", cx));
        if self.catalog_active.is_some() {
            body = body.child(self.control(
                "cancel-structure",
                "Cancel structure loading",
                Control::CancelCatalog,
                false,
                cx,
            ));
        }
        if let CatalogState::Error(message) = &self.catalog_state {
            body = body.child(div().text_color(theme.colors.danger).child(message.clone()));
        }
        match count {
            Some(0) => body
                .child("No columns reported for this object.")
                .into_any_element(),
            Some(count) => body
                .child(
                    div()
                        .id("object-structure-list")
                        .track_focus(&self.metadata_focus)
                        .tab_index(0)
                        .on_key_down(cx.listener(Self::on_metadata_key))
                        .focus(|style| style.border_color(theme.colors.border_focus))
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(px(8.))
                        .child(heading)
                        .child(
                            uniform_list(
                                "object-fields",
                                count,
                                cx.processor(
                                    |this: &mut Self, range: std::ops::Range<usize>, _, cx| {
                                        let theme = Theme::of(cx);
                                        let Some(cache) = this.catalog_cache.try_read() else {
                                            return Vec::new();
                                        };
                                        let Some(relation) = this
                                            .selected_path
                                            .as_ref()
                                            .and_then(|path| cache.relation(path))
                                        else {
                                            return Vec::new();
                                        };
                                        range
                                            .filter_map(|index| relation.fields.get(index))
                                            .map(|field| {
                                                div()
                                                    .h(px(32.))
                                                    .w_full()
                                                    .flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .px_3()
                                                    .border_b_1()
                                                    .border_color(theme.colors.border)
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .truncate()
                                                            .child(field.name.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .w(px(180.))
                                                            .truncate()
                                                            .child(field.raw_type.clone()),
                                                    )
                                                    .child(div().w(px(100.)).child(
                                                        if field.nullable { "Yes" } else { "No" },
                                                    ))
                                                    .child(div().w(px(100.)).child(
                                                        if field.is_primary_key {
                                                            "Primary key"
                                                        } else {
                                                            "—"
                                                        },
                                                    ))
                                            })
                                            .collect::<Vec<_>>()
                                    },
                                ),
                            )
                            .track_scroll(self.metadata_scroll.clone())
                            .flex_1(),
                        ),
                )
                .into_any_element(),
            None => body
                .child(div().p_4().text_color(theme.colors.text_muted).child(
                    if self.catalog_active.is_some() {
                        "Loading columns…"
                    } else {
                        "Select Structure to load the columns."
                    },
                ))
                .into_any_element(),
        }
    }
}
