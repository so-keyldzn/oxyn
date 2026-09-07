//! Driver-specific fields and explicit environment selection.

use super::*;
use crate::icons::{IconName, icon};
use gpui::FontWeight;

impl ConnectionForm {
    pub(super) fn render_form(&self, theme: &Theme, cx: &Context<'_, Self>) -> AnyElement {
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
