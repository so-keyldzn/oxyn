//! Connection screens, styled with the shared workspace tokens.

use super::*;
use crate::icons::{IconName, icon};
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
            .child(
                div()
                    .h(px(56.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .child(icon(IconName::Logo).text_color(theme.colors.text))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Oxyn"))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.colors.text_muted)
                                    .child("Personal workspace"),
                            ),
                    ),
            )
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
    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &Context<'_, Self>,
        action: impl Fn(&mut Self, &mut Context<'_, Self>) + 'static,
    ) -> AnyElement {
        let colors = Theme::of(cx).colors;
        div()
            .id(id)
            .tab_index(0)
            .px_3()
            .py_2()
            .rounded(px(6.))
            .border_1()
            .border_color(colors.border)
            .cursor_pointer()
            .hover(move |style| style.bg(colors.hover))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus);
                action(this, cx);
                cx.notify();
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

    fn render_form(&self, theme: &Theme, cx: &Context<'_, Self>) -> AnyElement {
        let Some(driver) = self.model.current() else {
            return div().into_any_element();
        };
        let mut form = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .mb_2()
                    .child(icon(IconName::Database).text_color(theme.colors.text_muted))
                    .child(
                        div()
                            .text_size(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(driver.display_name.clone()),
                    ),
            )
            .child(
                div()
                    .mb_3()
                    .text_color(theme.colors.text_muted)
                    .child("Configure your connection. Fields marked * are required."),
            )
            .child(self.input_row("Connection name *", CHAMP_NOM, theme))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(180.)).child("Environment"))
                    .child(
                        div()
                            .id("environment")
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .gap_1()
                            .p_1()
                            .border_1()
                            .rounded(px(6.))
                            .border_color(if self.model.focused == CHAMP_ENVIRONNEMENT {
                                theme.colors.border_focus
                            } else {
                                theme.colors.border
                            })
                            .children(ENVIRONNEMENTS.iter().enumerate().map(
                                |(rank, environment)| {
                                    let selected = self.model.environment == rank;
                                    div()
                                        .id(("environment-option", rank))
                                        .flex_1()
                                        .flex()
                                        .justify_center()
                                        .py_1()
                                        .rounded(px(4.))
                                        .text_size(px(11.))
                                        .bg(if selected {
                                            theme.colors.surface_raised
                                        } else {
                                            theme.colors.background
                                        })
                                        .text_color(if selected {
                                            theme.colors.environment(*environment)
                                        } else {
                                            theme.colors.text_muted
                                        })
                                        .font_weight(if selected {
                                            FontWeight::SEMIBOLD
                                        } else {
                                            FontWeight::NORMAL
                                        })
                                        .cursor_pointer()
                                        .hover(|style| style.bg(theme.colors.hover))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            window.focus(&this.focus);
                                            this.model.focused = CHAMP_ENVIRONNEMENT;
                                            this.model.environment = rank;
                                            cx.notify();
                                        }))
                                        .child(environment_choice_label(*environment))
                                },
                            )),
                    ),
            );
        for (rank, field) in driver.fields.iter().enumerate() {
            let index = rank + CHAMPS_COMMUNS;
            let label = format!("{}{}", field.label, if field.required { " *" } else { "" });
            if self.inputs.contains_key(&index) {
                form = form.child(self.input_row(&label, index, theme));
            } else {
                let value = self
                    .model
                    .values
                    .get(field.key.as_ref())
                    .cloned()
                    .unwrap_or_default();
                form = form.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(div().w(px(180.)).child(label))
                        .child(
                            div()
                                .id(("choice", index))
                                .flex_1()
                                .px_2()
                                .py_1()
                                .border_1()
                                .rounded(px(6.))
                                .border_color(if self.model.focused == index {
                                    theme.colors.accent
                                } else {
                                    theme.colors.border
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    window.focus(&this.focus);
                                    this.model.focused = index;
                                    this.model.cycle();
                                    cx.notify();
                                }))
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(value)
                                .child(icon(IconName::Down).text_color(theme.colors.text_muted)),
                        ),
                );
            }
            if matches!(field.kind, FormFieldKind::Path) {
                let key = field.key.clone();
                let create_key = key.clone();
                form = form.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(self.button(
                            "browse-file",
                            "Browse… · ⌘O",
                            cx,
                            move |this, cx| this.parcourir(key.clone(), cx),
                        ))
                        .child(self.button(
                            "create-file",
                            "New file… · ⌘N",
                            cx,
                            move |this, cx| this.nommer_un_fichier(create_key.clone(), cx),
                        )),
                );
            }
            if let Some(help) = &field.help {
                form = form.child(
                    div()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child(help.clone()),
                );
            }
        }
        let complete = self.model.is_complete();
        form.child(
            div()
                .flex()
                .gap_2()
                .pt_2()
                .child(self.button("back-to-drivers", "Back", cx, |this, cx| {
                    this.back_to_drivers(cx)
                }))
                .child(
                    div()
                        .id("connect")
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(if complete {
                            theme.colors.accent
                        } else {
                            theme.colors.surface
                        })
                        .text_color(if complete {
                            theme.colors.text_on_accent
                        } else {
                            theme.colors.text_muted
                        })
                        .when(complete, |button| button.cursor_pointer())
                        .on_click(cx.listener(|this, _, _, cx| this.submit(cx)))
                        .child("Connect"),
                ),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(if complete {
                    "Tab: next field · Enter: connect · Esc: back"
                } else {
                    "Complete the required fields to connect. Tab: next field · Esc: back"
                }),
        )
        .into_any_element()
    }

    fn input_row(&self, label: &str, index: usize, theme: &Theme) -> AnyElement {
        div()
            .flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .w(px(180.))
                    .flex_none()
                    .text_color(theme.colors.text_muted)
                    .child(label.to_owned()),
            )
            .children(
                self.inputs
                    .get(&index)
                    .map(|input| div().flex_1().min_w_0().child(input.clone())),
            )
            .into_any_element()
    }
}
