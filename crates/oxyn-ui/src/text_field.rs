//! Single-line native text input shared by connection fields.
use crate::Theme;
use crate::controls::focus_ring;
use gpui::prelude::*;
use gpui::{
    Bounds, ClipboardItem, ElementInputHandler, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, KeyDownEvent, MouseButton, Pixels, Point, ShapedLine, SharedString, TextRun,
    UTF16Selection, Window, canvas, div, fill, point, px, size,
};
use std::ops::Range;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum FieldEvent {
    Changed,
    /// The replacement exceeded the field's configured UTF-8 byte limit.
    LimitReached,
    Focused,
    Next(bool),
    Submit,
    Escape,
    Browse(bool),
}

pub struct TextField {
    focus: FocusHandle,
    text: String,
    cursor: usize,
    anchor: usize,
    marked: Option<Range<usize>>,
    secret: bool,
    clipboard_export_allowed: bool,
    read_only: bool,
    byte_limit: Option<usize>,
    managed_tab_order: bool,
    selecting: bool,
    layout: Option<(ShapedLine, Bounds<Pixels>, Pixels)>,
}

impl std::fmt::Debug for TextField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextField")
            .field("secret", &self.secret)
            .finish_non_exhaustive()
    }
}
impl EventEmitter<FieldEvent> for TextField {}
impl Focusable for TextField {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

// All offsets stored by this component are character indices, not UTF-8 bytes.
fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(i, _)| i)
}
fn from_utf16(text: &str, offset: usize) -> usize {
    let mut units = 0;
    text.chars()
        .take_while(|ch| {
            if units >= offset {
                false
            } else {
                units += ch.len_utf16();
                true
            }
        })
        .count()
}
fn to_utf16(text: &str, offset: usize) -> usize {
    text.chars().take(offset).map(char::len_utf16).sum()
}

impl TextField {
    pub fn new(text: String, secret: bool, cx: &mut Context<'_, Self>) -> Self {
        let end = text.chars().count();
        Self {
            focus: cx.focus_handle(),
            text,
            cursor: end,
            anchor: end,
            marked: None,
            secret,
            clipboard_export_allowed: true,
            read_only: false,
            byte_limit: None,
            managed_tab_order: false,
            selecting: false,
            layout: None,
        }
    }
    /// Prevents copying/cutting the field or exposing its value through native
    /// text extraction, while keeping normal display, editing and paste available.
    pub fn set_clipboard_export_allowed(&mut self, allowed: bool) {
        self.clipboard_export_allowed = allowed;
    }

    /// Lets a composite form route Tab through its own field model.
    pub fn with_managed_tab_order(mut self) -> Self {
        self.managed_tab_order = true;
        self
    }

    /// Refuses oversized replacements without cutting the user's input into a different value.
    pub fn with_byte_limit(mut self, limit: usize) -> Self {
        self.byte_limit = Some(limit);
        self
    }

    /// Keeps selection and copy available while refusing native and keyboard edits.
    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<'_, Self>) {
        self.read_only = read_only;
        self.marked = None;
        cx.notify();
    }

    /// Whether the field currently refuses edits.
    ///
    /// Readable so that an owner can assert what it made read-only, instead of
    /// tracking the same flag a second time on its side.
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn set_text(&mut self, text: String, cx: &mut Context<'_, Self>) {
        self.text = text;
        self.cursor = self.text.chars().count();
        self.anchor = self.cursor;
        self.marked = None;
        cx.notify();
    }
    fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }
    fn utf16_range(&self, range: Range<usize>) -> Range<usize> {
        to_utf16(&self.text, range.start)..to_utf16(&self.text, range.end)
    }
    fn character_range(&self, range: Range<usize>) -> Range<usize> {
        let a = from_utf16(&self.text, range.start);
        let b = from_utf16(&self.text, range.end);
        a.min(b)..a.max(b)
    }
    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<'_, Self>) -> bool {
        if self.read_only {
            return false;
        }
        if self.byte_limit.is_some_and(|limit| text.len() > limit) {
            cx.emit(FieldEvent::LimitReached);
            return false;
        }
        let text: String = text.chars().filter(|ch| !ch.is_control()).collect();
        let start = byte_at(&self.text, range.start);
        let end = byte_at(&self.text, range.end);
        if self
            .byte_limit
            .is_some_and(|limit| self.text.len() - (end - start) + text.len() > limit)
        {
            cx.emit(FieldEvent::LimitReached);
            return false;
        }
        self.text.replace_range(start..end, &text);
        self.cursor = range.start + text.chars().count();
        self.anchor = self.cursor;
        self.marked = None;
        cx.emit(FieldEvent::Changed);
        cx.notify();
        true
    }
    fn index_at(&self, position: Point<Pixels>) -> usize {
        let Some((line, bounds, offset)) = &self.layout else {
            return self.cursor;
        };
        let byte = line.closest_index_for_x(position.x - bounds.left() + *offset);
        line.text
            .get(..byte)
            .unwrap_or_default()
            .chars()
            .count()
            .min(self.text.chars().count())
    }
    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<'_, Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let command = modifiers.secondary();
        let length = self.text.chars().count();
        match key {
            "a" if command => {
                self.anchor = 0;
                self.cursor = length;
            }
            "c" | "x" if command => {
                if !self.secret && self.clipboard_export_allowed {
                    let range = self.selection();
                    let text = self
                        .text
                        .get(byte_at(&self.text, range.start)..byte_at(&self.text, range.end))
                        .unwrap_or_default();
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
                    if key == "x" {
                        self.replace(range, "", cx);
                    }
                }
            }
            "v" if command => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(self.selection(), &text, cx);
                }
            }
            "o" | "n" if command => cx.emit(FieldEvent::Browse(key == "n")),
            "left" | "right" | "home" | "end" => {
                self.cursor = match key {
                    "home" => 0,
                    "end" => length,
                    "left" if command => 0,
                    "right" if command => length,
                    "left" if !modifiers.shift && self.cursor != self.anchor => {
                        self.selection().start
                    }
                    "right" if !modifiers.shift && self.cursor != self.anchor => {
                        self.selection().end
                    }
                    "left" => self.cursor.saturating_sub(1),
                    _ => (self.cursor + 1).min(length),
                };
                if !modifiers.shift {
                    self.anchor = self.cursor;
                }
            }
            "backspace" | "delete" => {
                let mut range = self.selection();
                if range.is_empty() {
                    if key == "backspace" {
                        range.start = range.start.saturating_sub(1);
                    } else {
                        range.end = (range.end + 1).min(length);
                    }
                }
                self.replace(range, "", cx);
            }
            "tab" if !self.managed_tab_order => return,
            "tab" => cx.emit(FieldEvent::Next(modifiers.shift)),
            "enter" => cx.emit(FieldEvent::Submit),
            "escape" => cx.emit(FieldEvent::Escape),
            _ => return, // Printable text and composition go through the native input handler.
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<String> {
        let range = self.character_range(range);
        *actual = Some(self.utf16_range(range.clone()));
        if self.secret || !self.clipboard_export_allowed {
            return Some("•".repeat(
                to_utf16(&self.text, range.end).saturating_sub(to_utf16(&self.text, range.start)),
            ));
        }
        self.text
            .get(byte_at(&self.text, range.start)..byte_at(&self.text, range.end))
            .map(str::to_owned)
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.utf16_range(self.selection()),
            reversed: self.cursor < self.anchor,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<'_, Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|range| self.utf16_range(range))
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<'_, Self>) {
        self.marked = None;
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = range
            .map(|r| self.character_range(r))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.selection());
        self.replace(range, text, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = range
            .map(|r| self.character_range(r))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let start = range.start;
        if !self.replace(range, text, cx) {
            return;
        }
        let end = self.cursor;
        self.marked = (end > start).then_some(start..end);
        if let Some(selection) = selection {
            self.anchor = (start + from_utf16(text, selection.start)).min(end);
            self.cursor = (start + from_utf16(text, selection.end)).min(end);
        }
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.character_range(range);
        let (line, bounds, offset) = self.layout.as_ref()?;
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(byte_at(&line.text, range.start)) - *offset,
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(byte_at(&line.text, range.end)) - *offset,
                bounds.bottom(),
            ),
        ))
    }
    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<usize> {
        Some(to_utf16(&self.text, self.index_at(position)))
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let focused = self.focus.is_focused(window);
        let entity = cx.entity();
        let paint_entity = entity.clone();
        let color = theme.colors;
        div()
            .id("text-field")
            .track_focus(&self.focus)
            .tab_index(0)
            .cursor_text()
            .w_full()
            // La hauteur de contrôle de la maquette, et non un `px(30.)` que
            // rien ne rattachait à une source.
            .h(theme.metrics.control_height)
            .px(theme.spacing.small)
            .py(theme.spacing.tiny)
            .bg(color.surface)
            .border_1()
            .rounded(theme.radii.control)
            .border_color(if focused {
                color.border_focus
            } else {
                color.grid_line
            })
            // Le même anneau que les autres contrôles : un champ dont le focus
            // se signale autrement que le reste de l'interface se cherche à
            // chaque tabulation.
            .when(focused, |element| element.shadow(vec![focus_ring(&theme)]))
            .overflow_hidden()
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    let index = this.index_at(event.position);
                    if event.click_count >= 2 {
                        this.anchor = 0;
                        this.cursor = this.text.chars().count();
                    } else {
                        if !event.modifiers.shift {
                            this.anchor = index;
                        }
                        this.cursor = index;
                    }
                    this.selecting = true;
                    cx.emit(FieldEvent::Focused);
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if this.selecting {
                    this.cursor = this.index_at(event.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let input = entity.read(cx);
                        let text: SharedString = if input.secret {
                            "•".repeat(input.text.chars().count()).into()
                        } else {
                            input.text.clone().into()
                        };
                        let style = window.text_style();
                        let run = TextRun {
                            len: text.len(),
                            font: style.font(),
                            color: color.text,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let line = window.text_system().shape_line(
                            text,
                            style.font_size.to_pixels(window.rem_size()),
                            &[run],
                            None,
                        );
                        let cursor = line.x_for_index(byte_at(&line.text, input.cursor));
                        let offset = (cursor - bounds.size.width + px(3.)).max(px(0.));
                        (line, input.selection(), input.cursor, offset)
                    },
                    move |bounds, (line, selection, cursor, offset), window, cx| {
                        let focus = paint_entity.read(cx).focus.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, paint_entity.clone()),
                            cx,
                        );
                        let origin = point(bounds.left() - offset, bounds.top());
                        if focused && !selection.is_empty() {
                            window.paint_quad(fill(
                                Bounds::from_corners(
                                    point(
                                        origin.x
                                            + line
                                                .x_for_index(byte_at(&line.text, selection.start)),
                                        bounds.top(),
                                    ),
                                    point(
                                        origin.x
                                            + line.x_for_index(byte_at(&line.text, selection.end)),
                                        bounds.bottom(),
                                    ),
                                ),
                                color.selection,
                            ));
                        }
                        let _ = line.paint(origin, bounds.size.height, window, cx);
                        if focused {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(
                                        origin.x + line.x_for_index(byte_at(&line.text, cursor)),
                                        bounds.top(),
                                    ),
                                    size(px(1.), bounds.size.height),
                                ),
                                color.accent,
                            ));
                        }
                        paint_entity
                            .update(cx, |input, _| input.layout = Some((line, bounds, offset)));
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf16_offsets_preserve_unicode_boundaries() {
        let text = "a😀é";
        assert_eq!(byte_at(text, 2), 5);
        assert_eq!(to_utf16(text, 2), 3);
        assert_eq!(from_utf16(text, 3), 2);
        assert_eq!(byte_at(text, usize::MAX), text.len());
        assert_eq!(from_utf16(text, usize::MAX), 3);
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    #[gpui::test]
    fn protected_visible_fields_never_export_their_value(cx: &mut gpui::TestAppContext) {
        let (field, cx) = cx.add_window_view(|_, cx| {
            let mut field = TextField::new("private-value".into(), false, cx);
            field.set_clipboard_export_allowed(false);
            field
        });
        cx.update(|window, cx| {
            window.focus(&field.read(cx).focus_handle(cx));
            cx.write_to_clipboard(ClipboardItem::new_string("clipboard-marker".into()));
        });
        cx.simulate_keystrokes("cmd-a cmd-c cmd-x");
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("clipboard-marker")
        );
        field.read_with(cx, |field, _| {
            assert_eq!(field.text, "private-value");
            assert!(
                !field.secret,
                "display remains unmasked for deliberate parameter editing"
            );
        });
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                let mut actual = None;
                let exported =
                    EntityInputHandler::text_for_range(field, 0..13, &mut actual, window, cx)
                        .expect("native range");
                assert!(!exported.contains("private-value"));
                assert_eq!(exported.chars().count(), 13);
            })
        });
        cx.simulate_keystrokes("cmd-v");
        assert_eq!(
            field.read_with(cx, |field, _| field.text.clone()),
            "clipboard-marker"
        );
    }

    #[gpui::test]
    fn rejected_composition_keeps_the_previous_text_and_read_only_blocks_edits(
        cx: &mut gpui::TestAppContext,
    ) {
        let (field, cx) = cx.add_window_view(|window, cx| {
            let field = TextField::new("ok".into(), false, cx).with_byte_limit(4);
            window.focus(&field.focus);
            field
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                EntityInputHandler::replace_and_mark_text_in_range(
                    field,
                    Some(0..2),
                    "ééé",
                    None,
                    window,
                    cx,
                );
                assert_eq!(field.text, "ok");
                assert!(field.marked.is_none());
            })
        });
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("éé");
        assert_eq!(field.read_with(cx, |field, _| field.text.clone()), "éé");
        field.update(cx, |field, cx| field.set_read_only(true, cx));
        cx.simulate_keystrokes("backspace");
        cx.simulate_input("x");
        assert_eq!(field.read_with(cx, |field, _| field.text.clone()), "éé");
    }
}
