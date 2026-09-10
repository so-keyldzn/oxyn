//! Local column visibility and read-only inspection over the existing Arrow result.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, ElementId, KeyDownEvent, ScrollStrategy, div, px, uniform_list};
use oxyn_ui::Theme;

impl Workspace {
    pub(super) fn active_result_source(&self) -> ResultSource {
        if self.panel == WorkspacePanel::Object {
            ResultSource::Preview
        } else {
            ResultSource::Query
        }
    }

    pub(super) fn focus_result_area(&self, window: &mut Window, cx: &gpui::App) {
        if self.inspector_open && !self.compact_layout
            || self.inspector_overlay && self.compact_layout
        {
            window.focus(&self.record_focus);
        } else {
            window.focus(
                &self
                    .result_grid(self.active_result_source())
                    .read(cx)
                    .focus_handle(cx),
            );
        }
    }

    pub(super) fn on_result_ui_event(
        &mut self,
        source: ResultSource,
        event: &GridEvent,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            GridEvent::RowSelected(row) => {
                if self
                    .value_inspection
                    .as_ref()
                    .is_some_and(|value| value.source == source && value.row != *row)
                {
                    self.close_value(cx);
                }
                cx.notify();
            }
            GridEvent::ColumnsChanged => cx.notify(),
            _ => {}
        }
    }

    pub(super) fn ensure_inspector_page(&mut self, cx: &mut Context<'_, Self>) {
        if !(self.inspector_open && !self.compact_layout
            || self.inspector_overlay && self.compact_layout)
            || matches!(
                self.panel,
                WorkspacePanel::Help | WorkspacePanel::Preferences
            )
            || self.panel == WorkspacePanel::Object && self.object_tab != ObjectTab::Data
        {
            return;
        }
        let grid = self.result_grid(self.active_result_source());
        let row = grid.read(cx).selected_row();
        if let Some(row) = row {
            grid.update(cx, |grid, cx| grid.request_row_page(row, cx));
        }
    }

    pub(super) fn columns_label(&self, source: ResultSource, cx: &gpui::App) -> String {
        let grid = self.result_grid(source);
        let columns = grid.read(cx).columns();
        format!(
            "Columns · {}/{}",
            columns.iter().filter(|column| column.visible).count(),
            columns.len()
        )
    }

    pub(super) fn column_manager(
        &self,
        source: ResultSource,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let grid = self.result_grid(source);
        let count = grid.read(cx).columns().len();
        div()
            .flex_none()
            .h(px(200.))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(theme.colors.border)
            .rounded(theme.radii.surface)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child("Visible columns · Display only; export keeps all columns")
                    .child(
                        div()
                            .id("show-all-columns")
                            .tab_index(0)
                            .px_2()
                            .py_1()
                            .border_1()
                            .border_color(theme.colors.border)
                            .focus(|style| style.border_color(theme.colors.border_focus))
                            .cursor_pointer()
                            .child("Show all")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.result_grid(source)
                                    .update(cx, DataGrid::show_all_columns)
                            }))
                            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.result_grid(source)
                                        .update(cx, DataGrid::show_all_columns);
                                    cx.stop_propagation();
                                }
                            })),
                    ),
            )
            .when(count == 0, |el| {
                el.child("No result columns are available yet.")
            })
            .when(count > 0, |el| {
                el.child(
                    uniform_list(
                        "column-visibility",
                        count,
                        cx.processor(
                            move |this: &mut Self, range: std::ops::Range<usize>, _, cx| {
                                let theme = Theme::of(cx);
                                let grid = this.result_grid(source);
                                range
                                    .filter_map(|index| {
                                        grid.read(cx).columns().get(index).map(|column| {
                                            (index, column.name.clone(), column.visible)
                                        })
                                    })
                                    .map(|(index, name, visible)| {
                                        div()
                                            .id(ElementId::named_usize(
                                                "column-visibility-item",
                                                index,
                                            ))
                                            .tab_index(0)
                                            .h(px(32.))
                                            .w_full()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .px_2()
                                            .border_1()
                                            .border_color(theme.colors.border)
                                            .focus(|style| {
                                                style.border_color(theme.colors.border_focus)
                                            })
                                            .hover(|style| style.bg(theme.colors.hover))
                                            .cursor_pointer()
                                            .child(div().w(px(68.)).flex_none().child(if visible {
                                                "Visible"
                                            } else {
                                                "Hidden"
                                            }))
                                            .child(div().flex_1().min_w_0().truncate().child(name))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.result_grid(source).update(cx, |grid, cx| {
                                                    grid.set_column_visible(index, !visible, cx)
                                                })
                                            }))
                                            .on_key_down(cx.listener(
                                                move |this, event: &KeyDownEvent, _, cx| {
                                                    if matches!(
                                                        event.keystroke.key.as_str(),
                                                        "enter" | "space"
                                                    ) {
                                                        this.result_grid(source).update(
                                                            cx,
                                                            |grid, cx| {
                                                                grid.set_column_visible(
                                                                    index, !visible, cx,
                                                                )
                                                            },
                                                        );
                                                        cx.stop_propagation();
                                                    }
                                                },
                                            ))
                                    })
                                    .collect::<Vec<_>>()
                            },
                        ),
                    )
                    .flex_1(),
                )
            })
            .into_any_element()
    }

    pub(super) fn result_area(&self, source: ResultSource, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .gap_0()
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(theme.radii.surface)
                    .overflow_hidden()
                    .child(self.result_grid(source)),
            )
            .when(!self.compact_layout && self.inspector_open, |el| {
                el.child(self.inspector_handle(cx))
                    .child(self.record_inspector(source, cx))
            })
            .when(self.compact_layout && self.inspector_overlay, |el| {
                el.child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .w(px(280.))
                        .child(self.record_inspector(source, cx)),
                )
            })
            .into_any_element()
    }

    fn record_inspector(&self, source: ResultSource, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let grid = self.result_grid(source);
        let row = grid.read(cx).selected_row();
        let count = grid.read(cx).columns().len();
        div()
            .id("record-inspector")
            .track_focus(&self.record_focus)
            .tab_index(0)
            .w(self.inspector_display_width())
            .h_full()
            .flex_none()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .bg(theme.colors.surface)
            .border_1()
            .border_color(theme.colors.border)
            .focus(|style| style.border_color(theme.colors.border_focus))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let next = match event.keystroke.key.as_str() {
                    "up" => this.inspected_column.saturating_sub(1),
                    "down" => this
                        .inspected_column
                        .saturating_add(1)
                        .min(count.saturating_sub(1)),
                    "home" => 0,
                    "end" => count.saturating_sub(1),
                    "enter" => {
                        this.inspect_selected_value(cx);
                        if this.value_inspection.is_some() {
                            window.focus(&this.value_focus);
                        }
                        cx.stop_propagation();
                        return;
                    }
                    _ => return,
                };
                this.inspected_column = next;
                this.record_scroll
                    .scroll_to_item(next, ScrollStrategy::Center);
                cx.notify();
                cx.stop_propagation();
            }))
            .child(row.map_or_else(
                || "Record inspector".into(),
                |row| format!("Record · row {}", row.saturating_add(1)),
            ))
            .child(
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .child("Selected row · Read only"),
            )
            .when(row.is_none(), |el| {
                el.child("Select a row in the result grid.")
            })
            .when_some(row, |el, row| {
                el.child(
                    uniform_list(
                        "record-fields",
                        count,
                        cx.processor(
                            move |this: &mut Self, range: std::ops::Range<usize>, _, cx| {
                                let theme = Theme::of(cx);
                                let grid = this.result_grid(source);
                                let grid = grid.read(cx);
                                let batch = grid
                                    .state()
                                    .buffer()
                                    .and_then(|buffer| oxyn_ui::data_grid::row_batch(buffer, row));
                                range
                                    .filter_map(|index| {
                                        grid.columns().get(index).map(|column| (index, column))
                                    })
                                    .map(|(index, column)| {
                                        let (text, absent) = if let Some((batch, offset)) = &batch {
                                            let value = oxyn_data::format_cell(
                                                batch,
                                                *offset,
                                                index,
                                                grid.format_options(),
                                            );
                                            (
                                                format!(
                                                    "{}{}",
                                                    value.display_with(grid.format_options()),
                                                    if value.is_truncated() { "…" } else { "" }
                                                ),
                                                value.is_null(),
                                            )
                                        } else {
                                            ("Loading existing row page…".into(), false)
                                        };
                                        div()
                                            .id(ElementId::named_usize("record-field", index))
                                            .h(px(60.))
                                            .w_full()
                                            .flex()
                                            .flex_col()
                                            .gap_3()
                                            .when(index == this.inspected_column, |el| {
                                                el.bg(theme.colors.selection)
                                            })
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.inspected_column = index;
                                                window.focus(&this.record_focus);
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .text_size(theme.typography.small_size)
                                                    .text_color(theme.colors.text_muted)
                                                    .truncate()
                                                    .child(format!(
                                                        "{} · {}",
                                                        column.name, column.type_name
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_color(if absent {
                                                        theme.colors.null
                                                    } else {
                                                        theme.colors.text
                                                    })
                                                    .child(text),
                                            )
                                    })
                                    .collect::<Vec<_>>()
                            },
                        ),
                    )
                    .track_scroll(self.record_scroll.clone())
                    .flex_1()
                    .min_h_0(),
                )
            })
            .when(row.is_some() && count > 0, |el| {
                el.child(self.control(
                    "inspect-full-value",
                    "Inspect full value",
                    Control::InspectValue,
                    false,
                    cx,
                ))
            })
            .child(self.control(
                "hide-record-inspector",
                "Hide inspector",
                Control::Inspector,
                false,
                cx,
            ))
            .into_any_element()
    }
}
