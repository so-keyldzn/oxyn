//! Table data and structure share object identity while keeping query drafts separate.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, FontWeight, div, px, uniform_list};
use oxyn_ui::Theme;

impl Workspace {
    pub(super) fn object_details(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let path = self.selected_path.as_ref();
        let title = path
            .and_then(|p| p.relation().or(p.namespace()).or(p.catalog()))
            .unwrap_or("Catalog");
        let is_relation = path.is_some_and(|p| p.relation().is_some());
        let metadata = path.and_then(|path| {
            let cache = self.catalog_cache.try_read()?;
            let summary = cache.relation_summary(path)?;
            let mut detail = format!("{} · {}", summary.kind.as_str(), self.display.driver);
            if let Some(relation) = cache.relation(path) {
                detail.push_str(&format!(" · {} columns", relation.fields.len()));
            }
            Some(detail)
        });
        div().flex_1().min_h_0().flex().flex_col().gap_4().p_6()
            .child(div().flex().items_center().justify_between().gap_4().flex_none()
                .child(div().flex_1().min_w_0().flex().flex_col().gap_1()
                    .child(div().text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).truncate().child(title.to_owned()))
                    .child(div().text_color(theme.colors.text_muted).truncate().child(path.map(|p|p.to_string()).unwrap_or_default()))
                    .when_some(metadata, |el, detail| el.child(div().text_color(theme.colors.text_muted).child(detail))))
                .when(self.capabilities.contains(Capabilities::SQL), |el| el.child(self.control("object-sql", "SQL editor · ⌘J", Control::Sql, false, cx))))
            .when(is_relation, |el| el.child(div().flex().gap_4().flex_none()
                .child(self.control("object-data", "Data", Control::Data, false, cx))
                .child(self.control("object-structure", "Structure", Control::Describe, false, cx))))
            .child(if !is_relation {
                div().p_4().text_color(theme.colors.text_muted).child("Select a table in the sidebar to see its data.").into_any_element()
            } else if self.object_tab == ObjectTab::Structure {
                self.object_structure(cx)
            } else if self.preview_available() {
                div().flex_1().min_h_0().flex().flex_col().gap_3()
                    .child(div().flex().items_center().justify_between().gap_3().flex_none()
                        .child(div().text_size(px(11.)).text_color(theme.colors.text_muted).child(self.preview_notice.clone()))
                        .child(self.control("refresh-preview", if self.preview_active.is_some() { "Cancel · Esc" } else { "Refresh data" }, Control::Preview, false, cx)))
                    .child(div().flex_1().min_h_0().border_1().border_color(theme.colors.border).rounded(px(8.)).overflow_hidden().child(self.preview_grid.clone()))
                    .child(div().text_size(px(11.)).text_color(theme.colors.text_muted).child("First 200 rows · Read only · No guaranteed order · ⌘2 to focus the grid"))
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
        let mut body = div().flex_1().min_h_0().flex().flex_col().gap_3();
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
