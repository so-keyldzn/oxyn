//! The local workspace shell and keyboard navigation; data stays in its entities.

use super::*;
use gpui::{KeyDownEvent, div, px};
use oxyn_ui::{Theme, ThemeMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkspacePanel {
    Sql,
    Object,
    Help,
    Preferences,
    Library,
    /// The conversation panel. Only reachable when a provider is declared.
    Assistant,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Control {
    Sidebar,
    Library,
    NewConsole,
    CloseConsole,
    SaveConsole,
    SaveConsoleCopy,
    CancelSave,
    CancelDocumentClose,
    CancelNewConsole,
    Sql,
    Catalog,
    Theme,
    Help,
    NewConnection,
    Run,
    Stop,
    Explain,
    Parameters,
    Refresh,
    CancelCatalog,
    Describe,
    Data,
    Preview,
    FormatSettings,
    Object,
    PreviewExport,
    ObjectHelp,
    Indexes,
    Constraints,
    Relations,
    IncomingRelations,
    OutgoingRelations,
    OpenRelated,
    /// Ouvre le modèle de requête liée dans une console, sans l'exécuter.
    ReviewRelatedQuery,
    Definition,
    RefreshDefinition,
    CancelDefinition,
    CopyDefinition,
    OpenDefinition,
    ProposeChange,
    PreviousMatch,
    NextMatch,
    MoreMetadata,
    RefreshMetadata,
    Columns,
    ResultActions,
    Inspector,
    InspectValue,
    Preferences,
    RetryPreferences,
    ToggleReading,
    CancelSessionContext,
    PreviewApply,
    PreviewSort,
    PreviewColumns,
    PreviewPreviousPage,
    PreviewNextPage,
    AskAi,
}

impl Workspace {
    pub(super) fn activate_control(
        &mut self,
        control: Control,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.approval.read(cx).is_open()
            || self.console_close.is_some()
            || self.value_inspection.is_some()
            || !self.control_enabled(control, cx)
        {
            return;
        }
        match control {
            Control::SaveConsoleCopy => self
                .console
                .update(cx, |console, cx| console.save_as_new_query(cx)),
            Control::SaveConsole => {
                if !self.consoles.is_empty() {
                    self.console
                        .update(cx, |console, cx| console.save_document(cx));
                }
            }
            Control::CancelDocumentClose => self
                .console
                .update(cx, |console, cx| console.cancel_document_close(cx)),
            Control::CancelSave => self
                .console
                .update(cx, |console, cx| console.cancel_save(cx)),
            Control::NewConsole => self.new_console(cx),
            Control::CloseConsole => self.request_close_console(self.console.clone(), window, cx),
            Control::CancelNewConsole => self.cancel_new_console(cx),
            Control::Library => {
                self.panel = WorkspacePanel::Library;
                self.library.update(cx, |library, cx| library.reload(cx));
                window.focus(&self.library.read(cx).focus_handle(cx));
            }
            Control::Preferences => {
                self.panel = WorkspacePanel::Preferences;
                window.focus(&self.preferences_focus);
            }
            Control::RetryPreferences => self.retry_preferences(cx),
            Control::ToggleReading => {
                let density = if Theme::of(cx).reading_density == oxyn_core::ReadingDensity::Compact
                {
                    oxyn_core::ReadingDensity::Comfortable
                } else {
                    oxyn_core::ReadingDensity::Compact
                };
                self.set_reading_density(density, cx);
            }
            Control::Columns => {
                self.columns_open = !self.columns_open;
                self.result_actions_open = false;
            }
            Control::ResultActions => self.result_actions_open = !self.result_actions_open,
            Control::Inspector => {
                if self.compact_layout {
                    self.inspector_overlay = !self.inspector_overlay;
                } else {
                    self.inspector_open = !self.inspector_open;
                }
                self.focus_result_area(window, cx);
                if !self.compact_layout {
                    self.persist_preferences(cx);
                }
            }
            Control::InspectValue => {
                self.inspect_selected_value(cx);
                if self.value_inspection.is_some() {
                    window.focus(&self.value_focus);
                }
            }
            Control::Constraints => self.select_metadata_tab(ObjectTab::Constraints, cx),
            Control::Indexes => self.select_metadata_tab(ObjectTab::Indexes, cx),
            Control::Relations => {
                let tab = if self
                    .capabilities
                    .contains(Capabilities::INCOMING_FOREIGN_KEYS)
                {
                    ObjectTab::IncomingRelations
                } else {
                    ObjectTab::Relations
                };
                self.select_metadata_tab(tab, cx);
            }
            Control::IncomingRelations => {
                self.select_metadata_tab(ObjectTab::IncomingRelations, cx)
            }
            Control::OutgoingRelations => self.select_metadata_tab(ObjectTab::Relations, cx),
            Control::OpenRelated => self.open_related_object(cx),
            Control::Definition => self.select_metadata_tab(ObjectTab::Ddl, cx),
            Control::RefreshDefinition => self.load_definition(cx),
            Control::CancelDefinition => self.cancel_definition(cx),
            Control::CopyDefinition => self.copy_definition(cx),
            Control::OpenDefinition => self.open_definition_console(cx),
            Control::ProposeChange => self.open_proposed_change(cx),
            Control::PreviousMatch => self.reveal_find(false, cx),
            Control::NextMatch => self.reveal_find(true, cx),
            Control::ReviewRelatedQuery => self.open_related_row_query(cx),
            Control::MoreMetadata => self.metadata_menu_open = !self.metadata_menu_open,
            Control::RefreshMetadata => {
                self.refresh_object_metadata(cx);
                self.load_definition(cx);
            }
            Control::Object => self.panel = WorkspacePanel::Object,
            Control::PreviewExport => {
                self.result_actions_open = false;
                self.preview_export_open = !self.preview_export_open;
                if self.preview_export_open {
                    self.preview_export.update(cx, ResultExport::show_formats);
                }
            }
            Control::ObjectHelp => self.object_help_open = !self.object_help_open,
            Control::Sidebar => {
                if self.compact_layout {
                    self.catalog_overlay = !self.catalog_overlay;
                    window.focus(&self.shell_focus);
                } else {
                    self.sidebar_collapsed = !self.sidebar_collapsed;
                }
                if !self.compact_layout {
                    self.focus_current_panel(window, cx);
                }
                if !self.compact_layout {
                    self.persist_preferences(cx);
                }
            }
            Control::Sql => self.focus_sql(window, cx),
            Control::Catalog => {
                if self.compact_layout {
                    self.catalog_overlay = true;
                } else if self.sidebar_collapsed {
                    self.sidebar_collapsed = false;
                    self.persist_preferences(cx);
                }
                self.catalog_focus_pending = true;
                if matches!(self.catalog_state, CatalogState::Initial) && self.catalog_supported() {
                    self.refresh_catalog(CatalogScope::Server, cx);
                }
            }
            Control::Theme => {
                let next = if Theme::of(cx).mode == ThemeMode::Dark {
                    ThemeMode::Light
                } else {
                    ThemeMode::Dark
                };
                Theme::init(next, cx);
                self.persist_preferences(cx);
                window.refresh();
            }
            Control::Help => self.panel = WorkspacePanel::Help,
            // The focus lands in the question field rather than on the panel:
            // the panel exists to be typed into, and a keyboard user who has to
            // tab across the whole transcript to reach it would give up.
            Control::AskAi => {
                self.panel = WorkspacePanel::Assistant;
                window.focus(&self.assistant.question.read(cx).focus_handle(cx));
            }
            Control::NewConnection => cx.emit(WorkspaceEvent::NewConnectionRequested),
            Control::Run => self.execute(cx),
            Control::Stop => self.cancel(cx),
            Control::Explain => self
                .console
                .update(cx, |console, cx| console.execute_explain(cx)),
            Control::Parameters => self
                .console
                .update(cx, |console, cx| console.toggle_parameters(cx)),
            Control::CancelSessionContext => self
                .console
                .update(cx, |console, cx| console.cancel_context(cx)),
            Control::PreviewApply => self.apply_preview_predicate(cx),
            Control::PreviewSort => self.toggle_preview_sort(cx),
            // Only the columns: the DDL of `RefreshMetadata` is a second read
            // that nobody asked for from the sort menu.
            Control::PreviewColumns => self.refresh_object_metadata(cx),
            Control::PreviewPreviousPage => self.page_preview(false, cx),
            Control::PreviewNextPage => self.page_preview(true, cx),
            Control::Refresh => self.refresh_catalog(self.catalog_scope.clone(), cx),
            Control::CancelCatalog => {
                self.cancel_catalog(cx);
                self.cancel_definition(cx);
            }
            Control::Describe => self.select_metadata_tab(ObjectTab::Structure, cx),
            Control::Data => {
                self.object_tab = ObjectTab::Data;
                self.cancel_definition(cx);
                if self.preview_path.is_none() {
                    self.load_preview(cx);
                }
            }
            Control::FormatSettings => {
                self.settings_open = !self.settings_open;
                cx.notify();
            }
            Control::Preview => {
                if self.preview_active.is_some() {
                    self.cancel_preview(cx);
                } else {
                    self.load_preview(cx);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn focus_current_panel(&self, window: &mut Window, cx: &gpui::App) {
        let focus = match self.panel {
            WorkspacePanel::Sql
                if self.capabilities.contains(Capabilities::SQL) && !self.consoles.is_empty() =>
            {
                self.editor.read(cx).focus_handle(cx)
            }
            WorkspacePanel::Object
                if self.object_tab == ObjectTab::Data && self.preview_available() =>
            {
                self.preview_grid.read(cx).focus_handle(cx)
            }
            WorkspacePanel::Preferences => self.preferences_focus.clone(),
            WorkspacePanel::Library => self.library.read(cx).focus_handle(cx),
            WorkspacePanel::Assistant => self.assistant.question.read(cx).focus_handle(cx),
            _ => self.shell_focus.clone(),
        };
        window.focus(&focus);
    }

    fn focus_sql(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.catalog_overlay = false;
        if self.capabilities.contains(Capabilities::SQL) {
            self.panel = WorkspacePanel::Sql;
            if self.consoles.is_empty() {
                window.focus(&self.shell_focus);
            } else {
                window.focus(&self.editor.read(cx).focus_handle(cx));
            }
        } else {
            window.focus(&self.shell_focus);
        }
    }

    fn on_workspace_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.console_close.is_some() {
            self.close_console_key(event, window, cx);
            return;
        }
        if self.value_inspection.is_some() {
            return;
        }
        if self.approval.read(cx).is_open() {
            return;
        }
        let key = &event.keystroke;
        if key.key == "tab" && key.modifiers.control {
            self.cycle_console(key.modifiers.shift, window, cx);
            cx.stop_propagation();
            return;
        }
        if key.key == "escape" && self.inspector_overlay {
            self.inspector_overlay = false;
            window.focus(&self.shell_focus);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if key.key == "escape" && self.catalog_overlay {
            self.catalog_overlay = false;
            window.focus(&self.shell_focus);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if key.key == "escape"
            && self.panel == WorkspacePanel::Object
            && self.preview_active.is_some()
        {
            self.cancel_preview(cx);
            cx.stop_propagation();
            return;
        }
        if key.modifiers.secondary() && key.key == "2" && self.panel == WorkspacePanel::Object {
            self.object_tab = ObjectTab::Data;
            window.focus(&self.preview_grid.read(cx).focus_handle(cx));
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let control = if key.modifiers.secondary() && !key.modifiers.alt {
            match (key.key.as_str(), key.modifiers.shift) {
                ("b", false) => Some(Control::Sidebar),
                ("j", false) => Some(Control::Sql),
                ("l", true) => Some(Control::Theme),
                ("1", false) => Some(Control::Catalog),
                (",", false) => Some(Control::Preferences),
                ("h", true) => Some(Control::Library),
                ("t", false) => Some(Control::NewConsole),
                ("w", false) if self.panel == WorkspacePanel::Sql => Some(Control::CloseConsole),
                ("s", false) if self.panel == WorkspacePanel::Sql => Some(Control::SaveConsole),
                _ => None,
            }
        } else {
            None
        };
        if let Some(control) = control {
            self.activate_control(control, window, cx);
            cx.stop_propagation();
        } else if key.key == "tab"
            && !key.modifiers.secondary()
            && !key.modifiers.alt
            && !self.editor.read(cx).focus_handle(cx).is_focused(window)
        {
            if key.modifiers.shift {
                window.focus_prev();
            } else {
                window.focus_next();
            }
            cx.stop_propagation();
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(1200.);
        if compact != self.compact_layout {
            if let Some((_, initial)) = self.inspector_drag.take() {
                self.inspector_width = initial;
            }
            self.inspector_overlay = false;
            self.result_actions_open = false;
        }
        self.compact_layout = compact;
        self.ensure_inspector_page(cx);
        if !self.compact_layout {
            self.catalog_overlay = false;
        }
        let theme = Theme::of(cx).clone();
        if self.initial_focus {
            self.focus_sql(window, cx);
            self.initial_focus = false;
        }
        if self.catalog_focus_pending
            && ((!self.sidebar_collapsed && !self.compact_layout) || self.catalog_overlay)
            && matches!(
                self.catalog_state,
                CatalogState::Ready
                    | CatalogState::Empty
                    | CatalogState::Cancelled
                    | CatalogState::Error(_)
            )
        {
            if let Some(tree) = &self.catalog {
                window.focus(&tree.read(cx).focus_handle(cx));
            }
            self.catalog_focus_pending = false;
        }
        if self.console_close.is_none()
            && !self.approval.read(cx).is_open()
            && let Some(console) = self.console_to_focus.take()
        {
            self.select_console(console, window, cx);
        }
        if let Some(request) = self.console.update(cx, |console, _| console.pending.take()) {
            self.approval
                .update(cx, |dialog, cx| dialog.present(request, window, cx));
        }
        div()
            .relative()
            .size_full()
            .track_focus(&self.shell_focus)
            .tab_group()
            .flex()
            .gap_2()
            .p_2()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.ui_size)
            .capture_key_down(cx.listener(Self::on_workspace_key))
            .on_mouse_move(cx.listener(|this, event, window, cx| {
                this.on_inspector_drag(event, window, cx);
                this.on_definition_drag(event, window, cx);
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, event, window, cx| {
                    this.finish_inspector_drag(event, window, cx);
                    this.finish_definition_drag(event, window, cx);
                }),
            )
            .child(self.sidebar(self.sidebar_collapsed || self.compact_layout, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .bg(theme.colors.background)
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(px(8.))
                    .child(self.connection_bar(cx))
                    .child(self.document_tabs(cx))
                    .when_some(self.console_notice.clone(), |el, notice| {
                        el.child(
                            div()
                                .px_3()
                                .py_1()
                                .text_color(theme.colors.text_muted)
                                .child(notice),
                        )
                    })
                    .child(self.body(cx))
                    .when_some(self.preference_error_banner(cx), |el, banner| {
                        el.child(banner)
                    })
                    .child(if self.panel == WorkspacePanel::Object {
                        div()
                            .h(theme.metrics.status_bar_height)
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .px_3()
                            .bg(theme.colors.surface_raised)
                            .text_size(theme.typography.small_size)
                            .text_color(theme.colors.text_muted)
                            .child(self.display.name.clone())
                            .child(div().flex_1().min_w_0().truncate().child(
                                if self.object_tab == ObjectTab::Data && self.preview_available() {
                                    self.preview_notice.clone()
                                } else {
                                    "Read-only catalog browsing".into()
                                },
                            ))
                            .into_any_element()
                    } else if self.consoles.is_empty() {
                        div()
                            .px_3()
                            .py_1()
                            .child("No console open")
                            .into_any_element()
                    } else {
                        self.status.clone().into_any_element()
                    }),
            )
            .when(self.catalog_overlay, |el| {
                el.child(
                    div()
                        .id("catalog-overlay")
                        .absolute()
                        .left(px(72.))
                        .top(px(8.))
                        .bottom(px(8.))
                        .w(theme.metrics.sidebar_width)
                        .bg(theme.colors.surface)
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(theme.radii.surface)
                        .child(self.sidebar(false, cx)),
                )
            })
            .when_some(self.render_value_inspection(cx), |el, value| {
                el.child(value)
            })
            .child(self.approval.clone())
            .when_some(self.render_close_console(cx), |el, dialog| el.child(dialog))
    }
}
