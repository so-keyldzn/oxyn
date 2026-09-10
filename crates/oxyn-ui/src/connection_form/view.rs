//! Connection screens, styled with the shared workspace tokens.

use super::*;
use crate::icons::{IconName, icon, logo};
use gpui::FontWeight;

impl Render for ConnectionForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        if self.pending_focus {
            let focus = if matches!(self.model.state, FormState::Filling { .. }) {
                self.inputs
                    .get(&self.model.focused)
                    .map(|input| input.read(cx).focus_handle(cx))
                    .unwrap_or_else(|| self.focus.clone())
            } else {
                self.focus.clone()
            };
            window.focus(&focus);
            self.pending_focus = false;
        }
        let body = match &self.model.state {
            FormState::ChoosingDriver => self.render_driver_list(&theme, cx),
            FormState::Filling { .. } => self.render_form(&theme, cx),
            FormState::Connecting => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().text_size(px(24.)).child("Opening connection…"))
                .child(
                    div()
                        .text_color(theme.colors.text_muted)
                        .child("You can cancel while the database responds."),
                )
                .child(
                    self.button("cancel-connection", "Cancel · Esc", cx, |_, cx| {
                        cx.emit(ConnectionFormEvent::CancelRequested)
                    }),
                )
                .into_any_element(),
            FormState::Failed { message, .. } => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_color(theme.colors.danger)
                        .text_size(px(24.))
                        .child("Connection failed"),
                )
                .child(
                    div()
                        .p_3()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme.colors.border)
                        .font_family(theme.typography.mono_family.clone())
                        .child(message.clone()),
                )
                .child(self.button(
                    "correct-connection",
                    "Edit connection · Enter",
                    cx,
                    |this, cx| this.correct(cx),
                ))
                .child(self.button(
                    "back-failed",
                    "Back to connections · Esc",
                    cx,
                    |this, cx| this.back_to_drivers(cx),
                ))
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus)
            .key_context("ConnectionForm")
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .p_2()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.ui_size)
            .child(self.render_title_bar(&theme, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .p_6()
                    .bg(theme.colors.background)
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(px(8.))
                    .child(
                        div()
                            .id("connection-panel")
                            .w(px(680.))
                            .max_w_full()
                            .max_h_full()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .p_4()
                            .child(body),
                    ),
            )
            .child(
                div()
                    .h(px(32.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px_4()
                    .text_size(px(11.))
                    .text_color(theme.colors.text_muted)
                    .child("Local workspace · Credentials stored in your system keychain"),
            )
    }
}

impl ConnectionForm {
    /// The home title bar: the Oxyn mark on the left, window actions on the right.
    ///
    /// Both sides share one row and neither overlaps the other. The previous
    /// layout drew these actions as an absolutely positioned overlay in
    /// `oxyn-app`, which landed exactly on top of the mark and its two title
    /// lines. Sizes come from Figma `191:1958`: a 32 px control inside a
    /// fixed-height bar.
    fn render_title_bar(&self, theme: &Theme, cx: &Context<'_, Self>) -> AnyElement {
        let mut actions = div().flex_none().flex().items_center().gap_2();
        if self.header_actions().return_to_workspace {
            actions = actions.child(self.button(
                "return-to-workspace",
                "Return to workspace · Esc",
                cx,
                |_, cx| cx.emit(ConnectionFormEvent::ReturnRequested),
            ));
        }
        if self.header_actions().saved_copies {
            actions = actions.child(self.button(
                "open-recovery",
                "Saved working copies",
                cx,
                |_, cx| cx.emit(ConnectionFormEvent::RecoveryRequested),
            ));
        }
        div()
            .h(px(56.))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_4()
            .child(
                div()
                    .id("home-brand")
                    .debug_selector(|| "home-brand".into())
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_w_0()
                    .child(logo(theme.mode))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child("Oxyn"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.colors.text_muted)
                                    .truncate()
                                    .child("Personal workspace"),
                            ),
                    ),
            )
            .child(actions)
            .into_any_element()
    }

    pub(super) fn button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &Context<'_, Self>,
        action: impl Fn(&mut Self, &mut Context<'_, Self>) + 'static,
    ) -> AnyElement {
        let colors = Theme::of(cx).colors;
        // GPUI gives no keyboard activation for free: a control reachable by Tab
        // that only answers the mouse is unusable, and retrofitting that is a
        // rewrite (ADR-0001). Enter and Space act, and the focus ring is visible.
        let activate = std::rc::Rc::new(action);
        let by_key = std::rc::Rc::clone(&activate);
        div()
            .id(id)
            .debug_selector(move || id.into())
            .tab_index(0)
            .px_3()
            .py_2()
            .rounded(px(6.))
            .border_1()
            .border_color(colors.border)
            .cursor_pointer()
            .hover(move |style| style.bg(colors.hover))
            .focus(move |style| style.border_color(colors.border_focus))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus);
                activate(this, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    by_key(this, cx);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(label)
            .into_any_element()
    }

    fn render_driver_list(&self, theme: &Theme, cx: &Context<'_, Self>) -> AnyElement {
        let mut list = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Connect to your data"),
            )
            .child(
                div()
                    .text_color(theme.colors.text_muted)
                    .mb_4()
                    .child("Open a local database or connect to a server to start exploring."),
            );
        if self.model.drivers.is_empty() {
            list = list.child("No database drivers are available.");
        }
        for (rank, driver) in self.model.drivers.iter().enumerate() {
            list = list.child(
                div()
                    .id(("driver", rank))
                    .px_3()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_1()
                    .border_color(if self.model.focused == rank {
                        theme.colors.border_focus
                    } else {
                        theme.colors.border
                    })
                    .rounded(px(8.))
                    .cursor_pointer()
                    .bg(if self.model.focused == rank {
                        theme.colors.selection
                    } else {
                        theme.colors.surface
                    })
                    .hover(|style| style.bg(theme.colors.hover))
                    .on_click(cx.listener(move |this, _, _, cx| this.choose_driver(rank, cx)))
                    .child(
                        div()
                            .size(px(36.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .bg(theme.colors.surface_raised)
                            .child(icon(IconName::Database).text_color(theme.colors.text_muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(driver.display_name.clone())
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.colors.text_muted)
                                    .child(driver.family.clone()),
                            ),
                    )
                    .child(icon(IconName::Chevron).text_color(theme.colors.text_muted)),
            );
        }
        list = list.child(
            div()
                .pt_6()
                .pb_2()
                .font_weight(FontWeight::MEDIUM)
                .child("Saved connections"),
        );
        if self.model.saved.is_empty() {
            list = list.child(
                div()
                    .p_4()
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(px(8.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("Your first connection starts here")
                    .child(div().text_color(theme.colors.text_muted).child(
                        "Choose a database above. Saved connections will appear here next time.",
                    )),
            );
        }
        for (rank, saved) in self.model.saved.iter().enumerate() {
            list = list.child(
                div()
                    .id(("saved", rank))
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_1()
                    .border_color(if self.model.focused == rank + self.model.drivers.len() {
                        theme.colors.border_focus
                    } else {
                        theme.colors.background
                    })
                    .cursor_pointer()
                    .bg(if self.model.focused == rank + self.model.drivers.len() {
                        theme.colors.selection
                    } else {
                        theme.colors.surface
                    })
                    .hover(|style| style.bg(theme.colors.hover))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(ConnectionFormEvent::SavedChosen(rank))
                    }))
                    .child(icon(IconName::Database).text_color(theme.colors.text_muted))
                    .child(div().flex_1().child(saved.name.clone()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.colors.environment(saved.environment))
                            .child(format!(
                                "{} · {}",
                                saved.driver,
                                environment_choice_label(saved.environment)
                            )),
                    ),
            );
        }
        list.child(
            div()
                .pt_4()
                .text_size(px(11.))
                .text_color(theme.colors.text_muted)
                .child("↑ ↓ to choose · Enter to connect"),
        )
        .into_any_element()
    }
}
