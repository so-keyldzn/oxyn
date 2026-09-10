//! The local workspace shell and keyboard navigation; data stays in its entities.

use super::*;
use gpui::{AnyElement, KeyDownEvent, SharedString, div, px};
use oxyn_ui::icons::{IconName, icon};
use oxyn_ui::{Theme, ThemeMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkspacePanel {
    Sql,
    Object,
    Help,
    Preferences,
    Library,
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
    Definition,
    RefreshDefinition,
    CancelDefinition,
    CopyDefinition,
    OpenDefinition,
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
}

impl Workspace {
    fn activate_control(
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

    /// Whether a control can act right now.
    ///
    /// Figma `273:37036` draws Run and Stop as two segments of one group, with
    /// the one that has nothing to do greyed out. A single decision point serves
    /// both the drawing and the activation, so a greyed control is genuinely
    /// inert rather than merely looking it.
    fn control_enabled(&self, control: Control, cx: &gpui::App) -> bool {
        match control {
            Control::Run => !self.is_executing(cx),
            Control::Stop => self.is_executing(cx),
            _ => true,
        }
    }

    pub(super) fn control(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        control: Control,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let enabled = self.control_enabled(control, cx);
        let label = label.into();
        let tooltip = label.clone();
        let glyph = match control {
            Control::Sidebar | Control::CancelCatalog => IconName::Panel,
            Control::Sql
            | Control::Run
            | Control::Stop
            | Control::Explain
            | Control::Parameters => IconName::Terminal,
            Control::Catalog | Control::Refresh => IconName::Database,
            Control::Help => IconName::Book,
            Control::Library => IconName::History,
            Control::Theme
            | Control::FormatSettings
            | Control::Preferences
            | Control::RetryPreferences
            | Control::ToggleReading => IconName::Settings,
            Control::NewConnection | Control::NewConsole => IconName::Plus,
            Control::CancelNewConsole | Control::CloseConsole | Control::CancelSessionContext => {
                IconName::History
            }
            Control::SaveConsole
            | Control::SaveConsoleCopy
            | Control::CancelSave
            | Control::CancelDocumentClose => IconName::Folder,
            Control::Describe | Control::Data | Control::Object => IconName::Table,
            Control::Preview | Control::RefreshMetadata => IconName::History,
            Control::Constraints
            | Control::Indexes
            | Control::Relations
            | Control::IncomingRelations
            | Control::OutgoingRelations
            | Control::OpenRelated
            | Control::Definition
            | Control::RefreshDefinition
            | Control::CancelDefinition
            | Control::CopyDefinition
            | Control::OpenDefinition
            | Control::MoreMetadata => IconName::Table,
            Control::PreviewExport => IconName::Folder,
            Control::ObjectHelp => IconName::Book,
            Control::Columns
            | Control::ResultActions
            | Control::Inspector
            | Control::InspectValue => IconName::Table,
        };
        let selected = matches!(
            (control, self.panel),
            (Control::Sql, WorkspacePanel::Sql)
                | (Control::Catalog, WorkspacePanel::Object)
                | (Control::Help, WorkspacePanel::Help)
                | (Control::Object, WorkspacePanel::Object)
                | (Control::Preferences, WorkspacePanel::Preferences)
                | (Control::Library, WorkspacePanel::Library)
        ) || (self.panel == WorkspacePanel::Object
            && matches!(
                (control, self.object_tab),
                (Control::Data, ObjectTab::Data)
                    | (Control::Describe, ObjectTab::Structure)
                    | (Control::Indexes, ObjectTab::Indexes)
                    | (Control::Constraints, ObjectTab::Constraints)
                    | (Control::Definition, ObjectTab::Ddl)
                    | (Control::Relations, ObjectTab::Relations)
                    | (Control::Relations, ObjectTab::IncomingRelations)
                    | (Control::IncomingRelations, ObjectTab::IncomingRelations)
                    | (Control::OutgoingRelations, ObjectTab::Relations)
            ));
        div()
            .id(id)
            .debug_selector(move || id.into())
            .tab_index(0)
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .rounded(px(6.))
            .border_1()
            .border_color(theme.colors.surface)
            .when(compact, |el| el.w(px(32.)).px(px(7.)).justify_center())
            .when(
                matches!(control, Control::Inspector | Control::InspectValue),
                |el| el.justify_center().px_3(),
            )
            .when(matches!(control, Control::Data), |el| {
                el.min_w(px(64.)).justify_center().px_3()
            })
            .when(
                matches!(control, Control::Describe | Control::Relations),
                |el| el.min_w(px(99.)).justify_center().px_3(),
            )
            .when(matches!(control, Control::Constraints), |el| {
                el.min_w(px(113.)).justify_center().px_3()
            })
            .when(matches!(control, Control::Definition), |el| {
                el.min_w(px(57.)).justify_center().px_3()
            })
            .when(matches!(control, Control::Indexes), |el| {
                el.min_w(px(85.)).justify_center().px_3()
            })
            // Figma `191:1993`: the parameters control is 144 px wide, right of
            // the 286 px Run/Stop/Explain group.
            .when(matches!(control, Control::Parameters), |el| {
                el.min_w(px(144.)).justify_center().px_3()
            })
            .when(selected, |el| {
                el.bg(theme.colors.background)
                    .border_color(theme.colors.border)
            })
            .when(enabled, |el| {
                el.hover(|style| style.bg(theme.colors.hover))
                    .cursor_pointer()
            })
            .when(!enabled, |el| el.text_color(theme.colors.text_muted))
            .focus(|style| style.border_color(theme.colors.border_focus))
            .tooltip(move |window, cx| {
                let _ = window;
                cx.new(|_| WorkspaceTooltip(tooltip.clone())).into()
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.activate_control(control, window, cx)),
            )
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.activate_control(control, window, cx);
                    cx.stop_propagation();
                }
            }))
            .when(
                !matches!(
                    control,
                    Control::CancelCatalog
                        | Control::RetryPreferences
                        | Control::ToggleReading
                        | Control::Columns
                        | Control::ResultActions
                        | Control::Inspector
                        | Control::InspectValue
                        | Control::Indexes
                        | Control::Constraints
                        | Control::Relations
                        | Control::IncomingRelations
                        | Control::OutgoingRelations
                        | Control::Definition
                        | Control::MoreMetadata
                        | Control::Data
                        | Control::Describe
                        | Control::Object
                        | Control::PreviewExport
                        | Control::ObjectHelp
                ),
                |el| {
                    el.child(
                        icon(glyph)
                            .size(px(16.))
                            .text_color(theme.colors.text_muted),
                    )
                },
            )
            .when(!compact, |el| el.child(label))
            .into_any_element()
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
        let context = if self.panel == WorkspacePanel::Object {
            self.selected_path
                .as_ref()
                .map(|path| format!("{} / {path}", self.display.name))
                .unwrap_or_else(|| self.display.name.clone())
        } else {
            self.display.name.clone()
        };
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
                    .child(
                        div()
                            .h(px(48.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .child(self.control(
                                "sidebar-toggle",
                                "Toggle sidebar · ⌘B",
                                Control::Sidebar,
                                true,
                                cx,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.colors.text_muted)
                                    .child(context),
                            )
                            .when(self.read_only, |el| {
                                el.child(
                                    div()
                                        .text_size(theme.typography.small_size)
                                        .text_color(theme.colors.text_muted)
                                        .child("READ ONLY"),
                                )
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .px_2()
                                    .py_1()
                                    .rounded(theme.radii.full)
                                    .border_1()
                                    .border_color(theme.colors.environment(self.environment))
                                    .text_size(theme.typography.small_size)
                                    .text_color(theme.colors.environment(self.environment))
                                    .child(self.environment.as_str().to_uppercase()),
                            ),
                    )
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

#[derive(Debug)]
struct WorkspaceTooltip(SharedString);
impl Render for WorkspaceTooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .bg(theme.colors.surface_raised)
            .border_1()
            .border_color(theme.colors.border)
            .text_color(theme.colors.text)
            .text_size(theme.typography.small_size)
            .child(self.0.clone())
    }
}
