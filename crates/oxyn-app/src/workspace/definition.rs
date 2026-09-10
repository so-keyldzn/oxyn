//! Independently cancellable DDL inspection and explicit preparation in a new console.

use super::layout::Control;
use super::*;
use gpui::{
    AnyElement, ClipboardItem, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent, div, px,
};
use oxyn_catalog::{DefinitionSource, RelationDefinition};
use oxyn_core::CatalogRefreshScope;
use oxyn_ui::Theme;

pub(super) struct DefinitionPreview {
    pub path: Option<CatalogPath>,
    pub editor: Entity<QueryEditor>,
    pub source: Option<DefinitionSource>,
    pub notes: Vec<String>,
    pub active: Option<(CommandId, CancelToken)>,
    pub error: Option<String>,
    pub cancelled: bool,
    pub width: u16,
    pub drag: Option<(gpui::Pixels, u16)>,
    pub resize_focus: FocusHandle,
}

impl DefinitionPreview {
    pub fn new(cx: &mut Context<'_, Workspace>) -> Self {
        Self {
            path: None,
            editor: readonly_editor("", cx),
            source: None,
            notes: Vec::new(),
            active: None,
            error: None,
            cancelled: false,
            width: 424,
            drag: None,
            resize_focus: cx.focus_handle(),
        }
    }
}

fn readonly_editor(text: &str, cx: &mut Context<'_, Workspace>) -> Entity<QueryEditor> {
    cx.new(|cx| {
        let mut editor = QueryEditor::with_text(text, cx);
        editor.set_read_only(true, cx);
        editor
    })
}

impl Workspace {
    pub(super) fn reset_definition(&mut self, cx: &mut Context<'_, Self>) {
        self.cancel_definition(cx);
        self.definition.path = None;
        self.definition.source = None;
        self.definition.notes.clear();
        self.definition.error = None;
        self.definition.cancelled = false;
        self.definition.drag = None;
        self.definition.editor = readonly_editor("", cx);
    }

    pub(super) fn cancel_definition(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((_, cancel)) = self.definition.active.take() {
            cancel.cancel();
            self.definition.cancelled = true;
            cx.notify();
        }
    }

    pub(super) fn ensure_definition(&mut self, cx: &mut Context<'_, Self>) {
        if self.object_tab == ObjectTab::Data {
            self.cancel_definition(cx);
            return;
        }
        if self.definition.path == self.selected_path {
            let stale = self.selected_path.as_ref().is_some_and(|path| {
                self.catalog_cache.try_read().is_some_and(|cache| {
                    matches!(
                        cache.freshness(&CatalogScope::Definition(path.clone())),
                        oxyn_catalog::Freshness::Invalidated
                    )
                })
            });
            if !stale {
                return;
            }
        }
        self.load_definition(cx);
    }

    pub(super) fn load_definition(&mut self, cx: &mut Context<'_, Self>) {
        if self.definition.active.is_some()
            || !self.capabilities.contains(Capabilities::OBJECT_DEFINITION)
        {
            return;
        }
        let Some(path) = self
            .selected_path
            .clone()
            .filter(|path| path.relation().is_some())
        else {
            return;
        };
        if self.definition.path.as_ref() != Some(&path) {
            self.reset_definition(cx);
        }
        self.definition.path = Some(path.clone());
        self.definition.cancelled = false;
        self.definition.error = None;
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.definition.active = Some((id, cancel.clone()));
        let response = self.backend.dispatch(
            id,
            Command::RefreshCatalogScope {
                connection: self.connection,
                scope: CatalogRefreshScope::Definition {
                    catalog: path.catalog().map(str::to_owned),
                    namespace: path.namespace().map(str::to_owned),
                    relation: path.relation().unwrap_or_default().to_owned(),
                },
            },
            cancel,
        );
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| Err(OxynError::Internal("The executor stopped answering".into())));
            let _ = this.update(cx, |this, cx| {
                if this.definition.active.as_ref().map(|run| run.0) != Some(id) || this.selected_path.as_ref() != Some(&path) { return; }
                this.definition.active = None;
                match result {
                    Ok(Outcome::CatalogRefreshed { connection, .. }) if connection == this.connection => {
                        let definition = this.catalog_cache.try_read().and_then(|cache| cache.definition(&path).cloned());
                        match definition {
                            Some(RelationDefinition { sql, source, notes }) => {
                                // Replacing the readonly entity prevents retained undo snapshots
                                // from accumulating whole definitions as objects are visited.
                                this.definition.editor = readonly_editor(&sql, cx);
                                this.definition.source = Some(source);
                                this.definition.notes = notes;
                            }
                            None => this.definition.error = Some("The definition could not be read from the cache. Refresh to retry.".into()),
                        }
                    }
                    Err(OxynError::Cancelled) => this.definition.cancelled = true,
                    Err(error) => this.definition.error = Some(error.to_string()),
                    Ok(Outcome::Denied { reason, .. }) => this.definition.error = Some(reason),
                    _ => this.definition.error = Some("Unexpected definition response".into()),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn definition_ready(&self) -> bool {
        self.definition.path == self.selected_path && self.definition.source.is_some()
    }

    pub(super) fn copy_definition(&self, cx: &mut Context<'_, Self>) {
        if self.definition_ready() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.definition.editor.read(cx).text(),
            ));
        }
    }

    pub(super) fn open_definition_console(&mut self, cx: &mut Context<'_, Self>) {
        if !self.definition_ready() {
            return;
        }
        let text = self.definition.editor.read(cx).text();
        self.open_library_query(
            library::OpenQuery::Copy {
                text,
                title: "Object DDL".into(),
                origin: if self.definition.error.is_some() || self.definition.cancelled {
                    "previous object definition (refresh did not complete)"
                } else {
                    "object definition"
                }
                .into(),
            },
            cx,
        );
    }

    pub(super) fn with_definition_panel(
        &self,
        content: AnyElement,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(content),
            )
            .when(
                !self.compact_layout && self.capabilities.contains(Capabilities::OBJECT_DEFINITION),
                |el| {
                    el.child(self.definition_handle(cx)).child(
                        div()
                            .w(px(f32::from(self.definition.width)))
                            .flex_none()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(self.definition_panel(cx)),
                    )
                },
            )
            .into_any_element()
    }

    pub(super) fn structure_toolbar(&self, id: &'static str, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .flex()
            .gap_2()
            .flex_none()
            .child(self.control(id, "Refresh structure", Control::RefreshMetadata, false, cx))
            .when(!self.compact_layout && self.definition_ready(), |row| {
                row.child(self.control(
                    "copy-structure-ddl",
                    "Copy DDL",
                    Control::CopyDefinition,
                    false,
                    cx,
                ))
            })
            .into_any_element()
    }

    pub(super) fn definition_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let mut panel = div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .bg(theme.colors.surface)
            .border_1()
            .border_color(theme.colors.border)
            .child(div().text_size(px(13.)).child("DDL · Read only"));
        if !self.capabilities.contains(Capabilities::OBJECT_DEFINITION) {
            return panel
                .child("This session does not provide object definitions.")
                .into_any_element();
        }
        if self.object_tab == ObjectTab::Ddl
            || self.definition.error.is_some()
            || self.definition.cancelled
        {
            panel = panel.child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.control(
                        "refresh-definition",
                        "Refresh DDL",
                        Control::RefreshDefinition,
                        false,
                        cx,
                    ))
                    .when(self.definition_ready(), |el| {
                        el.child(self.control(
                            "copy-definition",
                            "Copy DDL",
                            Control::CopyDefinition,
                            false,
                            cx,
                        ))
                    }),
            );
        }
        if self.definition.active.is_some() {
            panel = panel
                .child(
                    div()
                        .text_color(theme.colors.text_muted)
                        .child("Loading definition… Previous text, if any, is retained."),
                )
                .child(self.control(
                    "cancel-definition",
                    "Cancel DDL loading",
                    Control::CancelDefinition,
                    false,
                    cx,
                ));
        }
        if self.definition.cancelled {
            panel = panel.child(
                div()
                    .text_color(theme.colors.text_muted)
                    .child("Definition loading cancelled. Refresh DDL to retry."),
            );
        }
        if let Some(error) = &self.definition.error {
            panel = panel.child(div().text_color(theme.colors.danger).child(error.clone()));
        }
        if self.definition_ready() && (self.definition.error.is_some() || self.definition.cancelled)
        {
            panel = panel.child(
                div()
                    .text_color(theme.colors.text_muted)
                    .child("Showing the last successful definition; it may be outdated."),
            );
        }
        if self.definition_ready() {
            let source = match self.definition.source {
                Some(DefinitionSource::Stored) => "Stored definition",
                _ => "Reconstructed definition",
            };
            panel = panel.child(div().text_size(px(12.)).text_color(theme.colors.text_muted).child(source))
                .child(div().flex_1().min_h_0().min_w_0().child(self.definition.editor.clone()))
                .child(div().id("definition-notes").max_h(px(100.)).overflow_y_scroll().text_size(px(12.)).text_color(theme.colors.text_muted).children(self.definition.notes.iter().cloned().map(|note| div().child(note))))
                .child(div().text_size(px(13.)).text_color(theme.colors.text_muted).child(format!("Review changes for {} before execution. Opening a console does not run SQL.", self.display.name)))
                .child(self.control("open-definition-console", "Open DDL in console", Control::OpenDefinition, false, cx));
        } else if self.definition.active.is_none()
            && self.definition.error.is_none()
            && !self.definition.cancelled
        {
            panel = panel.child("Refresh DDL to load this object's creation statements.");
        }
        panel.into_any_element()
    }

    pub(super) fn on_definition_drag(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((origin, initial)) = self.definition.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.definition.drag = None;
            return;
        }
        self.definition.width = super::inspector_resize::resized_panel_width(
            origin,
            initial,
            event.position.x,
            320,
            640,
        );
        cx.notify();
        cx.stop_propagation();
    }

    pub(super) fn finish_definition_drag(
        &mut self,
        _: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.definition.drag.take().is_some() {
            cx.notify();
        }
    }

    fn definition_handle(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .id("definition-resize")
            .debug_selector(|| "definition-resize".into())
            .track_focus(&self.definition.resize_focus)
            .tab_index(0)
            .w(px(8.))
            .h_full()
            .flex_none()
            .cursor_col_resize()
            .border_1()
            .border_color(theme.colors.border)
            .focus(|style| style.border_color(theme.colors.border_focus))
            .hover(|style| style.bg(theme.colors.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    this.definition.drag = Some((event.position.x, this.definition.width));
                    this.inspector_drag = None;
                    window.focus(&this.definition.resize_focus);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.definition.width = match event.keystroke.key.as_str() {
                    "left" => this.definition.width.saturating_add(10).min(640),
                    "right" => this.definition.width.saturating_sub(10).max(320),
                    "home" => 424,
                    _ => return,
                };
                cx.notify();
                cx.stop_propagation();
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
