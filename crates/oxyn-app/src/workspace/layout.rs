//! The local workspace shell and keyboard navigation; data stays in its entities.

use super::*;
use gpui::{AnyElement, KeyDownEvent, SharedString, div, px};
use oxyn_ui::icons::{IconName, icon};
use oxyn_ui::{Theme, ThemeMode, environment_label};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkspacePanel {
    Sql,
    Object,
    Help,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Control {
    Sidebar,
    Sql,
    Catalog,
    Theme,
    Help,
    NewConnection,
    Run,
    Refresh,
    CancelCatalog,
    Describe,
    Data,
    Preview,
    FormatSettings,
}

impl Workspace {
    fn activate_control(
        &mut self,
        control: Control,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match control {
            Control::Sidebar => {
                self.sidebar_collapsed = !self.sidebar_collapsed;
                if self.sidebar_collapsed {
                    self.focus_sql(window, cx);
                }
            }
            Control::Sql => self.focus_sql(window, cx),
            Control::Catalog => {
                self.sidebar_collapsed = false;
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
                window.refresh();
            }
            Control::Help => self.panel = WorkspacePanel::Help,
            Control::NewConnection => {
                if self.active.is_none()
                    && self.catalog_active.is_none()
                    && self.preview_active.is_none()
                {
                    cx.emit(WorkspaceEvent::NewConnectionRequested);
                } else {
                    self.status.update(cx, |bar, cx| bar.set_notice(Some("Wait for the current operation or cancel it before opening another connection."), cx));
                }
            }
            Control::Run => {
                if self.active.is_some() {
                    self.cancel(cx);
                } else {
                    self.execute(cx);
                }
            }
            Control::Refresh => self.refresh_catalog(self.catalog_scope.clone(), cx),
            Control::CancelCatalog => self.cancel_catalog(cx),
            Control::Describe => {
                self.object_tab = ObjectTab::Structure;
                self.cancel_preview(cx);
                if let Some(path) = self
                    .selected_path
                    .clone()
                    .filter(|p| p.relation().is_some())
                {
                    self.refresh_catalog(CatalogScope::Relation(path), cx);
                }
            }
            Control::Data => {
                self.object_tab = ObjectTab::Data;
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

    fn focus_sql(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.capabilities.contains(Capabilities::SQL) {
            self.panel = WorkspacePanel::Sql;
            window.focus(&self.editor.read(cx).focus_handle(cx));
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
        if self.approval.read(cx).is_open() {
            return;
        }
        let key = &event.keystroke;
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

    pub(super) fn control(
        &self,
        id: &'static str,
        label: &'static str,
        control: Control,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let tooltip = SharedString::from(label);
        let glyph = match control {
            Control::Sidebar | Control::CancelCatalog => IconName::Panel,
            Control::Sql | Control::Run => IconName::Terminal,
            Control::Catalog | Control::Refresh => IconName::Database,
            Control::Help => IconName::Book,
            Control::Theme | Control::FormatSettings => IconName::Settings,
            Control::NewConnection => IconName::Plus,
            Control::Describe | Control::Data | Control::Preview => IconName::Table,
        };
        let selected = matches!(
            (control, self.panel),
            (Control::Sql, WorkspacePanel::Sql)
                | (Control::Catalog, WorkspacePanel::Object)
                | (Control::Help, WorkspacePanel::Help)
        ) || (self.panel == WorkspacePanel::Object
            && matches!(
                (control, self.object_tab),
                (Control::Data, ObjectTab::Data) | (Control::Describe, ObjectTab::Structure)
            ));
        div()
            .id(id)
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
            .when(selected, |el| el.bg(theme.colors.surface_raised))
            .hover(|style| style.bg(theme.colors.hover))
            .focus(|style| style.border_color(theme.colors.border_focus))
            .cursor_pointer()
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
            .when(!matches!(control, Control::CancelCatalog), |el| {
                el.child(
                    icon(glyph)
                        .size(px(16.))
                        .text_color(theme.colors.text_muted),
                )
            })
            .when(!compact, |el| el.child(label))
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        if self.initial_focus {
            self.focus_sql(window, cx);
            self.initial_focus = false;
        }
        if self.catalog_focus_pending
            && !self.sidebar_collapsed
            && matches!(
                self.catalog_state,
                CatalogState::Ready | CatalogState::Empty | CatalogState::Error(_)
            )
        {
            if let Some(tree) = &self.catalog {
                window.focus(&tree.read(cx).focus_handle(cx));
            }
            self.catalog_focus_pending = false;
        }
        if let Some(request) = self.pending.take() {
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
            .py_2()
            .pr_2()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.ui_size)
            .capture_key_down(cx.listener(Self::on_workspace_key))
            .child(self.sidebar(cx))
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
                            .h(px(56.))
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
                                        .text_size(px(11.))
                                        .text_color(theme.colors.text_muted)
                                        .child("READ ONLY"),
                                )
                            })
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.colors.environment(self.environment))
                                    .child(environment_label(self.environment)),
                            ),
                    )
                    .child(self.body(cx))
                    .child(if self.panel == WorkspacePanel::Object {
                        div()
                            .h(px(28.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .px_3()
                            .bg(theme.colors.surface_raised)
                            .text_size(px(11.))
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
                    } else {
                        self.status.clone().into_any_element()
                    }),
            )
            .child(self.approval.clone())
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
            .text_size(px(11.))
            .child(self.0.clone())
    }
}
