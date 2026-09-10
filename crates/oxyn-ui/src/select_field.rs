//! A labeled-choice input with an explicit keyboard selection and a virtual menu.

use crate::{
    Theme,
    icons::{IconName, icon},
};
use gpui::prelude::*;
use gpui::{
    Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, SharedString,
    UniformListScrollHandle, Window, deferred, div, px, uniform_list,
};

/// The user accepted a choice; moving through the menu does not emit this event.
#[derive(Clone, Copy, Debug)]
pub struct SelectEvent {
    /// Rank in the supplied display labels, never a connection identifier.
    pub index: usize,
}

/// A choice field. Its owner maps ranks to domain values and commands.
pub struct SelectField {
    focus: FocusHandle,
    options: Vec<SharedString>,
    selected: usize,
    highlighted: usize,
    open: bool,
    scroll: UniformListScrollHandle,
    menu_bounds: Option<gpui::Bounds<gpui::Pixels>>,
}
impl std::fmt::Debug for SelectField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectField")
            .field("choices", &self.options.len())
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}
impl EventEmitter<SelectEvent> for SelectField {}
impl Focusable for SelectField {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
impl SelectField {
    /// Builds the input from display labels; an empty list cannot be opened.
    pub fn new(options: Vec<SharedString>, selected: usize, cx: &mut Context<'_, Self>) -> Self {
        let selected = selected.min(options.len().saturating_sub(1));
        Self {
            focus: cx.focus_handle(),
            options,
            selected,
            highlighted: selected,
            open: false,
            scroll: UniformListScrollHandle::new(),
            menu_bounds: None,
        }
    }
    /// Replaces the choices in place, keeping the field itself alive.
    ///
    /// Rebuilding the entity instead would drop the focus handle, so a list
    /// refreshed while someone was navigating it would throw them out of the
    /// menu — and the lists that change are exactly the ones loaded from a
    /// server, which arrive at a moment nobody chooses.
    ///
    /// Announces nothing: like [`Self::set_selected`], the owner is the one
    /// that decided. An open menu closes when the choices themselves change —
    /// someone was picking from a list that no longer exists, and accepting a
    /// rank in the new one would choose something they never saw.
    pub fn set_options(
        &mut self,
        options: Vec<SharedString>,
        selected: usize,
        cx: &mut Context<'_, Self>,
    ) {
        if self.options == options && self.selected == selected {
            return;
        }
        if self.options != options {
            self.open = false;
            self.options = options;
        }
        self.selected = selected.min(self.options.len().saturating_sub(1));
        self.highlighted = self.selected;
        cx.notify();
    }

    /// Shows a selection decided elsewhere, without announcing it again.
    ///
    /// Silent on purpose: the owner already knows — it is the one that changed
    /// the state — and emitting here would send it back a choice it just made.
    pub fn set_selected(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if index >= self.options.len() || self.selected == index {
            return;
        }
        self.selected = index;
        self.highlighted = index;
        cx.notify();
    }

    fn open(&mut self, cx: &mut Context<'_, Self>) {
        self.open = !self.options.is_empty();
        self.highlighted = self.selected;
        self.scroll
            .scroll_to_item(self.highlighted, gpui::ScrollStrategy::Top);
        cx.notify();
    }
    fn accept(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if index >= self.options.len() {
            return;
        }
        self.open = false;
        if self.selected != index {
            self.selected = index;
            cx.emit(SelectEvent { index });
        }
        cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<'_, Self>) {
        match event.keystroke.key.as_str() {
            "enter" | "space" => {
                if self.open {
                    self.accept(self.highlighted, cx);
                } else {
                    self.open(cx);
                }
            }
            "down" | "up" => {
                if !self.open {
                    self.open(cx);
                } else {
                    self.highlighted = if event.keystroke.key == "down" {
                        self.highlighted
                            .saturating_add(1)
                            .min(self.options.len().saturating_sub(1))
                    } else {
                        self.highlighted.saturating_sub(1)
                    };
                }
                self.scroll
                    .scroll_to_item(self.highlighted, gpui::ScrollStrategy::Top);
            }
            "home" if self.open => self.highlighted = 0,
            "end" if self.open => self.highlighted = self.options.len().saturating_sub(1),
            "escape" if self.open => self.open = false,
            "tab" => {
                self.open = false;
                cx.notify();
                return;
            }
            _ => return,
        }
        if self.open {
            self.scroll
                .scroll_to_item(self.highlighted, gpui::ScrollStrategy::Top);
        }
        cx.notify();
        cx.stop_propagation();
    }
}
impl Render for SelectField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        if self.open && !self.focus.is_focused(window) {
            self.open = false;
        }
        let entity = cx.entity();
        let label = self
            .options
            .get(self.selected)
            .cloned()
            .unwrap_or_else(|| "No choices available".into());
        div()
            .id("select-field")
            .relative()
            .w_full()
            .track_focus(&self.focus)
            .tab_index(0)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                if this.open
                    && !this
                        .menu_bounds
                        .is_some_and(|bounds| bounds.contains(&event.position))
                {
                    this.open = false;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .id("select-trigger")
                    .h(theme.metrics.control_height)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .bg(theme.colors.background)
                    .border_1()
                    .rounded(theme.radii.control)
                    .border_color(if self.focus.is_focused(window) {
                        theme.colors.border_focus
                    } else {
                        theme.colors.border
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.focus(&this.focus);
                        if this.open {
                            this.open = false;
                            cx.notify();
                        } else {
                            this.open(cx);
                        }
                    }))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(icon(IconName::Chevron).size(px(14.))),
            )
            .when(self.open, |el| {
                el.child(
                    deferred(
                        div()
                            .absolute()
                            .top(theme.metrics.control_height + px(4.))
                            .left_0()
                            .w_full()
                            .bg(theme.colors.surface_raised)
                            .border_1()
                            .border_color(theme.colors.border)
                            .rounded(theme.radii.control)
                            .occlude()
                            .child(
                                gpui::canvas(
                                    move |bounds, _, cx| {
                                        entity
                                            .update(cx, |view, _| view.menu_bounds = Some(bounds));
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            )
                            .child(
                                uniform_list(
                                    "select-options",
                                    self.options.len(),
                                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                        let theme = Theme::of(cx);
                                        range
                                            .filter_map(|index| {
                                                this.options.get(index).map(|label| {
                                                    div()
                                                        .id(("select-option", index))
                                                        .h(px(32.))
                                                        .px_2()
                                                        .flex()
                                                        .items_center()
                                                        .overflow_hidden()
                                                        .bg(if this.highlighted == index {
                                                            theme.colors.selection
                                                        } else {
                                                            theme.colors.surface_raised
                                                        })
                                                        .hover(|el| el.bg(theme.colors.hover))
                                                        .cursor_pointer()
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.accept(index, cx);
                                                                cx.stop_propagation();
                                                            },
                                                        ))
                                                        .child(
                                                            div().truncate().child(label.clone()),
                                                        )
                                                })
                                            })
                                            .collect::<Vec<_>>()
                                    }),
                                )
                                .h(px((self.options.len().min(8) * 32) as f32))
                                .track_scroll(self.scroll.clone()),
                            ),
                    )
                    .with_priority(1),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn navigation_is_tentative_until_enter_and_escape_preserves_the_choice(
        cx: &mut gpui::TestAppContext,
    ) {
        let selected = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = selected.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = SelectField::new(
                vec!["All".into(), "Success".into(), "Failure".into()],
                0,
                cx,
            );
            window.focus(&view.focus);
            cx.subscribe(&cx.entity(), move |_, _, event: &SelectEvent, _| {
                observed.set(event.index)
            })
            .detach();
            view
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("space");
        cx.simulate_keystrokes("down");
        assert_eq!(selected.get(), 0);
        cx.simulate_keystrokes("escape");
        assert_eq!(view.read_with(cx, |view, _| view.selected), 0);
        cx.simulate_keystrokes("space");
        cx.simulate_keystrokes("end");
        cx.simulate_keystrokes("enter");
        assert_eq!(selected.get(), 2);
        assert!(!view.read_with(cx, |view, _| view.open));
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        let bounds = view.read_with(cx, |view, _| view.menu_bounds.expect("painted menu"));
        cx.simulate_click(
            bounds.origin + gpui::point(px(10.), px(48.)),
            gpui::Modifiers::default(),
        );
        assert_eq!(
            selected.get(),
            1,
            "a menu click outside the trigger must still commit"
        );
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        cx.simulate_click(
            bounds.bottom_left() + gpui::point(px(10.), px(20.)),
            gpui::Modifiers::default(),
        );
        assert!(!view.read_with(cx, |view, _| view.open));
    }

    /// Rafraîchir la liste ne doit pas éjecter celui qui la parcourait.
    #[gpui::test]
    fn refreshing_the_choices_keeps_the_field_and_its_focus(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            SelectField::new(vec!["public".into(), "analytics".into()], 1, cx)
        });
        let focus = view.read_with(cx, |field, cx| field.focus_handle(cx));
        cx.update(|window, _| window.focus(&focus));
        cx.run_until_parked();

        view.update(cx, |field, cx| {
            field.set_options(
                vec!["public".into(), "analytics".into(), "billing".into()],
                2,
                cx,
            );
        });
        cx.run_until_parked();
        view.read_with(cx, |field, _| {
            assert_eq!(field.options.len(), 3);
            assert_eq!(field.selected, 2);
        });
        cx.update(|window, cx| {
            assert!(
                view.read(cx).focus_handle(cx).is_focused(window),
                "le champ est le même : le focus n'a pas sauté"
            );
        });

        // Une liste qui rétrécit sous le rang retenu referme le menu plutôt que
        // de laisser une surbrillance sur ce qui n'existe plus.
        view.update(cx, |field, cx| {
            field.open(cx);
            field.set_options(vec!["public".into()], 0, cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |field, _| {
            assert!(!field.open);
            assert_eq!(field.selected, 0);
        });
    }
}
