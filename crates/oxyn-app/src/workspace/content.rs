//! Workspace content rendering using real session state.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, FontWeight, div, px};
use oxyn_ui::Theme;

impl Workspace {
    pub(super) fn body(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        match self.panel {
            WorkspacePanel::Sql if self.capabilities.contains(Capabilities::SQL) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_4()
                .p_6()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .flex_none()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(24.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("SQL editor"),
                                )
                                .child(div().text_color(theme.colors.text_muted).child(format!(
                                    "{} · {}",
                                    self.display.driver,
                                    if self.read_only {
                                        "Read-only session"
                                    } else {
                                        "Current session"
                                    }
                                ))),
                        )
                        .child(self.control(
                            "run-query",
                            if self.active.is_some() {
                                "Cancel execution"
                            } else {
                                "Run query · ⌘Enter"
                            },
                            Control::Run,
                            false,
                            cx,
                        )),
                )
                .child(
                    div()
                        .h(px(220.))
                        .min_h(px(80.))
                        .flex_shrink()
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(px(8.))
                        .overflow_hidden()
                        .child(self.editor.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.colors.text_muted)
                        .text_size(px(11.))
                        .child("RESULTS"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(px(8.))
                        .overflow_hidden()
                        .child(self.grid.clone()),
                )
                .into_any_element(),
            WorkspacePanel::Object => self.object_details(cx),
            WorkspacePanel::Help => div()
                .flex_1()
                .p_6()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .text_size(px(24.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Workspace guide"),
                )
                .children(
                    [
                        "⌘B — Expand or collapse the sidebar",
                        "⌘J — Focus the SQL editor",
                        "⌘1 — Focus the catalog",
                        "⌘⇧L — Switch light / dark appearance",
                        "⌘Enter — Run the current statement",
                        "Escape — Cancel a running query from the editor",
                        "Tab / Shift+Tab — Move between controls; Tab in the editor indents",
                        "Catalog: select a table to preview its first 200 rows",
                        "⌘2 — Focus the table preview; Escape cancels its loading",
                        "History and saved queries are not available in this version.",
                    ]
                    .map(|text| div().child(text)),
                )
                .into_any_element(),
            _ => div()
                .flex_1()
                .p_6()
                .child("SQL is not supported by this session.")
                .into_any_element(),
        }
    }
}
