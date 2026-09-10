//! Workspace content rendering using real session state.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, FontWeight, div, px};
use oxyn_ui::{NEVER_EMULATED, Theme, surfaces};

impl Workspace {
    pub(super) fn document_tabs(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .id("document-tabs")
            .overflow_x_scroll()
            .track_scroll(&self.console_tabs_scroll)
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .bg(theme.colors.surface)
            .when_some(self.selected_path.as_ref(), |el, path| {
                el.child(
                    self.control(
                        "document-object",
                        path.relation()
                            .or(path.namespace())
                            .or(path.catalog())
                            .unwrap_or("Catalog")
                            .to_owned(),
                        Control::Object,
                        false,
                        cx,
                    ),
                )
            })
            .when(self.capabilities.contains(Capabilities::SQL), |el| {
                el.children(self.consoles.iter().map(|console| {
                    let selected = self.console == *console && self.panel == WorkspacePanel::Sql;
                    let view = console.read(cx);
                    let title = if view.title.trim().is_empty() {
                        "Untitled query"
                    } else {
                        &view.title
                    };
                    let label = if view.document_closing {
                        format!("{} · closing", title)
                    } else if view.save_active.is_some() {
                        format!("{} · saving", title)
                    } else if view.awaiting_approval {
                        format!("{} · approval", title)
                    } else if view.active.is_some() {
                        format!("{} · running", title)
                    } else {
                        if view.dirty {
                            format!("{} · unsaved", title)
                        } else {
                            title.to_owned()
                        }
                    };
                    let click_target = console.clone();
                    let key_target = console.clone();
                    let close_target = console.clone();
                    div()
                        .id(gpui::SharedString::from(format!("console-tab-{}", view.id)))
                        .tab_index(0)
                        .h(px(32.))
                        .min_w(px(164.))
                        .max_w(px(300.))
                        .rounded(px(6.))
                        .flex_none()
                        .px_3()
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(if selected {
                            theme.colors.border
                        } else {
                            theme.colors.surface
                        })
                        .bg(if selected {
                            theme.colors.background
                        } else {
                            theme.colors.surface
                        })
                        .text_color(if selected {
                            theme.colors.text
                        } else {
                            theme.colors.text_muted
                        })
                        .font_weight(FontWeight::MEDIUM)
                        .focus(|el| el.border_color(theme.colors.border_focus))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_console(click_target.clone(), window, cx)
                        }))
                        .on_key_down(cx.listener(
                            move |this, event: &gpui::KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.select_console(key_target.clone(), window, cx);
                                    cx.stop_propagation();
                                }
                            },
                        ))
                        .child(div().min_w_0().truncate().child(label))
                        .child(
                            div()
                                .id(("close-console", console.entity_id()))
                                .ml_2()
                                .px_1()
                                .cursor_pointer()
                                .text_size(theme.typography.small_size)
                                .child("Close")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.request_close_console(close_target.clone(), window, cx);
                                    cx.stop_propagation();
                                })),
                        )
                }))
                .child(if self.console_attempt.is_some() {
                    self.control(
                        "cancel-new-console",
                        "Cancel new console",
                        Control::CancelNewConsole,
                        false,
                        cx,
                    )
                } else {
                    self.control("new-console", "New console", Control::NewConsole, false, cx)
                })
            })
            .into_any_element()
    }

    /// The toolbar label of the bound-value editor, with what the next run sends.
    pub(super) fn parameters_label(&self, cx: &gpui::App) -> String {
        format!("Parameters · {}", self.console.read(cx).parameter_count(cx))
    }

    /// The bound-value editor of the active console, when it is open.
    ///
    /// It sits below the toolbar rather than over the editor: the values must
    /// stay readable next to the statement that uses them.
    fn parameters_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .id("query-parameters-panel")
            .max_h(px(300.))
            .overflow_y_scroll()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.colors.border)
            .child(
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .child(
                        "Values are bound by the driver. They are never inserted into the SQL \
                         text, saved with the query, or written to the history.",
                    ),
            )
            .child(self.console.read(cx).parameters.clone())
            .into_any_element()
    }

    pub(super) fn body(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        match self.panel {
            WorkspacePanel::Sql if self.consoles.is_empty() => div().flex_1().flex().flex_col().items_center().justify_center().gap_3()
                .child("Open a console to write a query.")
                .child(self.control("empty-new-console", "New console · ⌘T", Control::NewConsole, false, cx)).into_any_element(),
            WorkspacePanel::Sql if self.capabilities.contains(Capabilities::SQL) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .flex_none()
                        // Figma `273:37036`: Run, Stop and Explain form one
                        // 286 px group of three 96 px segments. Stop is its own
                        // control rather than a Run that changes meaning: a
                        // button whose label swaps under the pointer is one
                        // mis-click away from starting what you meant to stop.
                        .child(self.control(
                            "run-query",
                            "Run · ⌘Enter",
                            Control::Run,
                            false,
                            cx,
                        ))
                        .child(self.control("stop-query", "Stop", Control::Stop, false, cx))
                        .child(self.control(
                            "explain-query",
                            "Explain query",
                            Control::Explain,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "query-parameters",
                            self.parameters_label(cx),
                            Control::Parameters,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "query-columns",
                            self.columns_label(ResultSource::Query, cx),
                            Control::Columns,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "query-inspector",
                            "Inspect row",
                            Control::Inspector,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "toggle-format-settings",
                            if self.settings_open {
                                "Hide display settings"
                            } else {
                                "Display settings"
                            },
                            Control::FormatSettings,
                            false,
                            cx,
                        ))
                        // Figma `191:2003`: at the right end of the 1272 px bar.
                        // Absent, never greyed, when the session cannot declare
                        // where it resolves names (ADR-0003, ADR-0019).
                        .children(self.session_context_control(cx)),
                )
                .children(self.session_context_notice(cx))
                .child(div().flex_none().flex().flex_wrap().items_center().gap_2()
                    .child("Query name")
                    .child(div().w(px(240.)).child(self.console.read(cx).name.clone()))
                    .child(self.control("save-query", if self.console.read(cx).document_closing { "Cancel closing" } else if self.console.read(cx).save_active.is_some() { "Cancel save" } else if self.console.read(cx).save_conflict { "Save as new query" } else { "Save query · ⌘S" },
                        if self.console.read(cx).document_closing { Control::CancelDocumentClose } else if self.console.read(cx).save_active.is_some() { Control::CancelSave } else if self.console.read(cx).save_conflict { Control::SaveConsoleCopy } else { Control::SaveConsole }, false, cx))
                    .child(div().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child(self.console.read(cx).save_notice.clone())))
                .child(div().flex_none().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child(self.console.read(cx).draft_notice.clone()))
                .when(self.console.read(cx).parameters_open, |el| {
                    el.child(self.parameters_panel(cx))
                })
                .when(self.columns_open, |el| {
                    el.child(self.column_manager(ResultSource::Query, cx))
                })
                .when(self.settings_open, |element| {
                    element.child(
                        div()
                            .id("query-display-settings")
                            .max_h(px(300.))
                            .overflow_y_scroll()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(self.reading_settings(cx))
                            .child(self.settings.clone()),
                    )
                })
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
                        .text_size(theme.typography.small_size)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child("RESULTS")
                        .child(self.control(
                            "query-text-size",
                            self.reading_label(cx),
                            Control::ToggleReading,
                            false,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(px(8.))
                        .overflow_hidden()
                        .child(self.result_area(ResultSource::Query, cx))
                        .child(self.export.clone()),
                )
                .into_any_element(),
            WorkspacePanel::Object => self.object_details(cx),
            WorkspacePanel::Preferences => self.preferences_panel(cx),
            WorkspacePanel::Library => div().flex_1().min_h_0().flex().child(self.library.clone()).into_any_element(),
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
                        "⌘T — Open a console with its own session",
                        "Ctrl+Tab / Ctrl+Shift+Tab — Switch consoles",
                        "⌘W — Close the current console; unsaved SQL requires a choice",
                        "⌘⇧L — Switch light / dark appearance",
                        "⌘Enter — Run the current statement",
                        "Escape — Cancel a running query from the editor",
                        "Tab / Shift+Tab — Move between controls; Tab in the editor indents",
                        "Catalog: select a table to preview its first 200 rows",
                        "⌘2 — Focus the table preview; Escape cancels its loading",
                        "⌘⇧H — Browse local history and saved queries without replacing the current console.",
                    ]
                    .map(|text| div().child(text)),
                )
                .child(self.session_support(cx))
                .into_any_element(),
            _ => div()
                .flex_1()
                .p_6()
                .child("SQL is not supported by this session.")
                .into_any_element(),
        }
    }

    /// what a source cannot do is part of what a professional needs to know
    /// before writing, not after ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
    fn session_support(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .border_1()
            .border_color(theme.colors.border)
            .rounded(px(8.))
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Session capabilities"),
            )
            .child(
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .child(NEVER_EMULATED),
            )
            .children(surfaces(self.capabilities).into_iter().map(|surface| {
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(div().w(px(160.)).flex_none().child(surface.surface))
                    .child(
                        div()
                            .w(px(96.))
                            .flex_none()
                            // Le mot, pas seulement la couleur : une information
                            // portée par la seule couleur n'existe pas pour tout
                            // le monde (revue-ui, accessibilité).
                            .text_color(if surface.supported {
                                theme.colors.success
                            } else {
                                theme.colors.warning
                            })
                            .child(if surface.supported {
                                "Supported"
                            } else {
                                "Unsupported"
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(theme.colors.text_muted)
                            .child(surface.detail),
                    )
            }))
            .into_any_element()
    }
}
