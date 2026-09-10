//! Inspector geometry and its mouse/keyboard resizing, independent of database work.

use super::*;
use gpui::{AnyElement, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent, div, px};
use oxyn_ui::Theme;

pub(super) fn resized_width(origin: gpui::Pixels, initial: u16, pointer: gpui::Pixels) -> u16 {
    resized_panel_width(origin, initial, pointer, 240, 480)
}

pub(super) fn resized_panel_width(
    origin: gpui::Pixels,
    initial: u16,
    pointer: gpui::Pixels,
    min: u16,
    max: u16,
) -> u16 {
    let width = f32::from(initial) + f32::from(origin - pointer);
    if !width.is_finite() {
        return initial;
    }
    // Pointer input is clamped before conversion; every value fits in u16.
    width.round().clamp(f32::from(min), f32::from(max)) as u16
}

impl Workspace {
    pub(super) fn inspector_display_width(&self) -> gpui::Pixels {
        px(f32::from(if self.compact_layout {
            280
        } else {
            self.inspector_width
        }))
    }

    pub(super) fn on_inspector_drag(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((origin, initial)) = self.inspector_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.inspector_drag = None;
            return;
        }
        self.inspector_width = resized_width(origin, initial, event.position.x);
        cx.notify();
        cx.stop_propagation();
    }

    pub(super) fn finish_inspector_drag(
        &mut self,
        _: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some((_, initial)) = self.inspector_drag.take() {
            if initial != self.inspector_width {
                self.persist_preferences(cx);
            }
            cx.notify();
        }
    }

    pub(super) fn inspector_handle(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .id("inspector-resize")
            .debug_selector(|| "inspector-resize".into())
            .track_focus(&self.inspector_resize_focus)
            .tab_index(0)
            .relative()
            .w(px(8.))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_col_resize()
            .border_1()
            .border_color(theme.colors.background)
            .focus(|style| style.border_color(theme.colors.border_focus))
            .hover(|style| style.bg(theme.colors.hover))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .w(px(1.))
                    .bg(theme.colors.border),
            )
            .child(
                div()
                    .w(px(4.))
                    .h(px(24.))
                    .rounded(theme.radii.control)
                    .bg(theme.colors.text_muted),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    this.inspector_drag = Some((event.position.x, this.inspector_width));
                    window.focus(&this.inspector_resize_focus);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.inspector_width = match event.keystroke.key.as_str() {
                    "left" => this.inspector_width.saturating_add(10).min(480),
                    "right" => this.inspector_width.saturating_sub(10).max(240),
                    "home" => 280,
                    _ => return,
                };
                this.persist_preferences(cx);
                cx.stop_propagation();
            }))
            .into_any_element()
    }
}
