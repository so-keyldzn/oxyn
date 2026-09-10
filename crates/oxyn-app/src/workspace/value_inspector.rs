//! Paged full-value inspection, performed by the executor rather than the renderer.

use super::*;
use gpui::{AnyElement, KeyDownEvent, MouseButton, div, point, px};
use oxyn_data::value_page::ValuePage;
use oxyn_ui::Theme;

#[derive(Debug)]
pub(super) struct ValueInspection {
    pub source: ResultSource,
    result: ResultId,
    pub row: usize,
    column: usize,
    title: String,
    offsets: Vec<usize>,
    pub(super) page: Option<ValuePage>,
    error: Option<String>,
    pub(super) active: Option<(CommandId, CancelToken)>,
}

impl ValueInspection {
    pub fn cancel(&self) {
        if let Some((_, cancel)) = &self.active {
            cancel.cancel();
        }
    }
}

#[derive(Clone, Copy)]
enum ValueAction {
    Previous,
    Next,
    Close,
}

impl Workspace {
    pub(super) fn close_value(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(value) = self.value_inspection.take() {
            value.cancel();
            cx.notify();
        }
    }

    pub(super) fn inspect_selected_value(&mut self, cx: &mut Context<'_, Self>) {
        let source = self.active_result_source();
        let Some(result) = self.displayed_result(source, cx) else {
            return;
        };
        let grid = self.result_grid(source);
        let Some(row) = grid.read(cx).selected_row() else {
            return;
        };
        let column = self
            .inspected_column
            .min(grid.read(cx).columns().len().saturating_sub(1));
        let Some(field) = grid.read(cx).columns().get(column) else {
            return;
        };
        let title = format!(
            "{} · {} · row {}",
            self.display.name,
            field.name,
            row.saturating_add(1)
        );
        self.close_value(cx);
        self.value_inspection = Some(ValueInspection {
            source,
            result,
            row,
            column,
            title,
            offsets: vec![0],
            page: None,
            error: None,
            active: None,
        });
        self.load_value_page(cx);
    }

    fn load_value_page(&mut self, cx: &mut Context<'_, Self>) {
        let Some(value) = self.value_inspection.as_mut() else {
            return;
        };
        value.cancel();
        let id = CommandId::new();
        let cancel = CancelToken::new();
        let offset = value.offsets.last().copied().unwrap_or(0);
        let command = Command::InspectResultValue {
            connection: self.connection,
            result: value.result,
            row: value.row,
            column: value.column,
            offset,
        };
        value.active = Some((id, cancel.clone()));
        value.error = None;
        self.value_scroll.set_offset(point(px(0.), px(0.)));
        let response = self.backend.dispatch(id, command, cancel);
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal(
                    "The value inspection worker stopped answering".into(),
                ))
            });
            let _ = this.update(cx, |this, cx| {
                if this
                    .value_inspection
                    .as_ref()
                    .and_then(|value| value.active.as_ref().map(|run| run.0))
                    != Some(id)
                {
                    if let Ok(Outcome::NeedsApproval { command, .. }) = outcome {
                        drop(this.backend.decide(command, false, CancelToken::new()));
                    }
                    return;
                }
                let Some(value) = this.value_inspection.as_mut() else {
                    return;
                };
                value.active = None;
                match outcome {
                    Ok(Outcome::ValueInspected { page }) => value.page = Some(page),
                    Ok(Outcome::Denied { reason, .. }) => value.error = Some(reason),
                    Ok(Outcome::NeedsApproval { command, .. }) => {
                        drop(this.backend.decide(command, false, CancelToken::new()));
                        value.error = Some(
                            "The connection policy does not allow automatic value inspection."
                                .into(),
                        );
                    }
                    Err(error) => value.error = Some(error.to_string()),
                    _ => value.error = Some("Unexpected value inspection response".into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn value_action(
        &mut self,
        action: ValueAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if matches!(action, ValueAction::Close) {
            self.close_value(cx);
            self.focus_result_area(window, cx);
            return;
        }
        let Some(value) = self
            .value_inspection
            .as_mut()
            .filter(|value| value.active.is_none())
        else {
            return;
        };
        match action {
            ValueAction::Previous if value.offsets.len() > 1 => {
                value.offsets.pop();
            }
            ValueAction::Next => {
                let Some(next) = value.page.as_ref().and_then(|page| page.next_offset) else {
                    return;
                };
                value.offsets.push(next);
            }
            _ => return,
        }
        self.load_value_page(cx);
    }

    fn value_button(
        &self,
        index: usize,
        id: &'static str,
        label: &'static str,
        action: ValueAction,
        enabled: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let focus = self.value_buttons.get(index).unwrap_or(&self.value_focus);
        div()
            .id(id)
            .track_focus(focus)
            .when(enabled, |el| el.tab_index(0).cursor_pointer())
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .border_1()
            .border_color(theme.colors.border)
            .rounded(theme.radii.control)
            .focus(|style| style.border_color(theme.colors.border_focus))
            .when(!enabled, |el| el.opacity(0.5))
            .child(label)
            .when(enabled, |el| {
                el.on_click(
                    cx.listener(move |this, _, window, cx| this.value_action(action, window, cx)),
                )
                .on_key_down(cx.listener(
                    move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.value_action(action, window, cx);
                            cx.stop_propagation();
                        }
                    },
                ))
            })
            .into_any_element()
    }

    pub(super) fn render_value_inspection(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let value = self.value_inspection.as_ref()?;
        let theme = Theme::of(cx);
        let previous = value.active.is_none() && value.offsets.len() > 1;
        let next = value.active.is_none()
            && value
                .page
                .as_ref()
                .is_some_and(|page| page.next_offset.is_some());
        Some(
            div()
                .id("value-inspection-dialog")
                .track_focus(&self.value_focus)
                .absolute()
                .inset_0()
                .bg(theme.colors.scrim)
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            this.close_value(cx);
                            this.focus_result_area(window, cx);
                            cx.stop_propagation();
                        }
                        "tab" => {
                            let indices = [(0, previous), (1, next), (2, true)]
                                .into_iter()
                                .filter(|(_, enabled)| *enabled)
                                .map(|(index, _)| index)
                                .collect::<Vec<_>>();
                            let current = indices.iter().position(|index| {
                                this.value_buttons
                                    .get(*index)
                                    .is_some_and(|focus| focus.is_focused(window))
                            });
                            let selected = if event.keystroke.modifiers.shift {
                                current
                                    .unwrap_or(0)
                                    .checked_sub(1)
                                    .unwrap_or(indices.len().saturating_sub(1))
                            } else {
                                current.map_or(0, |current| (current + 1) % indices.len())
                            };
                            if let Some(focus) = indices
                                .get(selected)
                                .and_then(|index| this.value_buttons.get(*index))
                            {
                                window.focus(focus);
                            }
                            cx.stop_propagation();
                        }
                        "up" | "down" | "pageup" | "pagedown" | "home" | "end" => {
                            let mut offset = this.value_scroll.offset();
                            offset.y = match event.keystroke.key.as_str() {
                                "up" => offset.y + px(24.),
                                "down" => offset.y - px(24.),
                                "pageup" => offset.y + px(200.),
                                "pagedown" => offset.y - px(200.),
                                "home" => px(0.),
                                _ => -this.value_scroll.max_offset().height,
                            };
                            this.value_scroll.set_offset(offset);
                            cx.notify();
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                }))
                .child(
                    div()
                        .w(px(800.))
                        .max_w_full()
                        .h(px(520.))
                        .max_h_full()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .bg(theme.colors.surface)
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(theme.radii.surface)
                        .child("Full value · Read only")
                        .child(
                            div()
                                .text_color(theme.colors.text_muted)
                                .child(value.title.clone()),
                        )
                        .when(value.active.is_some(), |el| {
                            el.child("Loading existing value…")
                        })
                        .when_some(value.error.as_ref(), |el, error| {
                            el.child(div().text_color(theme.colors.danger).child(format!(
                                "{error} Close this view and inspect the value again."
                            )))
                        })
                        .child(
                            div()
                                .id("full-value-text")
                                .flex_1()
                                .min_h_0()
                                .overflow_scroll()
                                .track_scroll(&self.value_scroll)
                                .p_3()
                                .bg(theme.colors.background)
                                .font_family(theme.typography.mono_family.clone())
                                .child(value.page.as_ref().map_or_else(String::new, |page| {
                                    if page.is_null {
                                        "∅ NULL".into()
                                    } else if page.total_bytes == 0 {
                                        "Empty value (0 bytes)".into()
                                    } else {
                                        page.text.clone()
                                    }
                                })),
                        )
                        .when_some(value.page.as_ref(), |el, page| {
                            el.child(
                                div()
                                    .text_size(theme.typography.small_size)
                                    .text_color(theme.colors.text_muted)
                                    .child(format!(
                                        "Rendered bytes {}–{} of {} · No SQL executed",
                                        page.offset,
                                        page.offset.saturating_add(page.text.len()),
                                        page.total_bytes
                                    )),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(self.value_button(
                                    0,
                                    "value-previous",
                                    "Previous",
                                    ValueAction::Previous,
                                    previous,
                                    cx,
                                ))
                                .child(self.value_button(
                                    1,
                                    "value-next",
                                    "Next",
                                    ValueAction::Next,
                                    next,
                                    cx,
                                ))
                                .child(div().flex_1())
                                .child(self.value_button(
                                    2,
                                    "value-close",
                                    if value.active.is_some() {
                                        "Cancel inspection"
                                    } else {
                                        "Close"
                                    },
                                    ValueAction::Close,
                                    true,
                                    cx,
                                )),
                        ),
                )
                .into_any_element(),
        )
    }
}
