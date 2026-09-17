//! The drawing of the conversation panel.
//!
//! Kept apart from [`super`] for the reason the preview bar is: that file owns
//! the transcript and the rules that build it, this one owns pixels, and one
//! file carrying both would carry two subjects.
//!
//! Every element here reads state the workspace already holds. Nothing is
//! computed at draw time that a test could not otherwise reach — the five
//! states, the proposals, the target of a command are all decided by free
//! functions next door.

use super::transcript::{PanelState, ending_line, last_answer, panel_state, report_lines};
use super::*;
use gpui::{AnyElement, FontWeight, div};
use oxyn_ui::{ControlState, ControlTone, Theme, activable, control};

impl Workspace {
    /// The whole panel: header, transcript, proposals, question.
    pub(in crate::workspace) fn assistant_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let running = self.assistant.active.is_some();
        let state = panel_state(&self.assistant.entries, running);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(self.assistant_header(cx))
            .child(
                div()
                    .id("assistant-transcript")
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.assistant.scroll)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(theme.radii.surface)
                    .focus(|style| style.border_color(theme.colors.border_focus))
                    .when(state == PanelState::Initial, |el| {
                        el.child(div().text_color(theme.colors.text_muted).child(
                            "Ask a question about this connection. \
                                 Anything the assistant proposes arrives in a console as text, \
                                 and you run it yourself.",
                        ))
                    })
                    .when(state == PanelState::Empty, |el| {
                        el.child(
                            div()
                                .text_color(theme.colors.text_muted)
                                .child("The model answered nothing. This is not a failure."),
                        )
                    })
                    .children(
                        self.assistant
                            .entries
                            .iter()
                            .enumerate()
                            .map(|(index, entry)| self.assistant_entry(index, entry, cx)),
                    ),
            )
            .children(self.assistant_proposals(cx))
            .child(self.assistant_question(state, cx))
            .into_any_element()
    }

    /// The tier of this connection, its provider, and what a failure to read
    /// the declarations was.
    fn assistant_header(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let tier = self.display.privacy_tier;
        let provider = self
            .assistant
            .providers
            .as_deref()
            .and_then(|providers| provider_for(providers, tier))
            .map(|provider| {
                format!(
                    "{} · {} · {}",
                    provider.config.label,
                    provider.config.model,
                    // Trois mots et non deux : « unresolved » dit qu'Oxyn n'a
                    // pas su classer, ce qui n'est pas la même chose que
                    // « distant ». Il compte comme distant pour décider.
                    match provider.reach {
                        oxyn_llm::Reach::Local => "endpoint resolves to this machine",
                        oxyn_llm::Reach::Remote => "endpoint leaves this machine",
                        oxyn_llm::Reach::Unresolved => {
                            "endpoint could not be resolved; treated as remote"
                        }
                    }
                )
            });
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Assistant · {}", self.display.name)),
            )
            // The tier is shown in words, permanently, and it is the
            // connection's: a user who cannot tell at a glance where their
            // question goes is not giving informed consent (AI-PROVIDERS).
            .child(
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .child(format!("Privacy tier {tier} — {}", tier.describe())),
            )
            .children(provider.map(|provider| {
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .child(provider)
            }))
            .children(self.assistant.providers_error.clone().map(|erreur| {
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.danger)
                    .child(format!(
                        "The declared providers could not be read: {erreur}"
                    ))
            }))
            .into_any_element()
    }

    /// One transcript line.
    fn assistant_entry(&self, index: usize, entry: &Entry, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let row = div()
            .id(("assistant-entry", index))
            .debug_selector(move || format!("assistant-entry-{index}"))
            .flex()
            .flex_col()
            .gap_1()
            .py_1();
        match entry {
            Entry::Question(text) => row
                .child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child("You"),
                )
                .child(div().child(text.clone()))
                .into_any_element(),
            Entry::Turn { turn, max_turns } => row
                .child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child(format!("Turn {turn} / {max_turns}")),
                )
                .into_any_element(),
            Entry::Answer(text) => row
                .child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child("Assistant"),
                )
                .child(
                    div()
                        .font_family(theme.typography.mono_family.clone())
                        .child(text.clone()),
                )
                .into_any_element(),
            // Shown before its result, because it arrived before it: an agent
            // working in silence for eight turns is indistinguishable from an
            // agent that is stuck (UX-SPEC).
            Entry::Command {
                tool,
                command,
                target,
                mutating,
            } => row
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(theme.typography.small_size)
                                .text_color(theme.colors.text_muted)
                                .child(format!("Submitted · {tool} · {command}")),
                        )
                        // The word, not only the colour: information carried by
                        // colour alone does not exist for everyone (revue-ui).
                        .child(
                            div()
                                .text_size(theme.typography.small_size)
                                .text_color(if *mutating {
                                    theme.colors.warning
                                } else {
                                    theme.colors.text_muted
                                })
                                .child(if *mutating {
                                    "may change data"
                                } else {
                                    "read only"
                                }),
                        ),
                )
                .child(div().child(format!("on {target}")))
                .into_any_element(),
            Entry::Report {
                tool,
                outcome,
                withheld,
            } => {
                let (heading, detail) = report_lines(outcome);
                row.child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child(format!("{tool} · {heading}")),
                )
                .child(
                    div()
                        .font_family(theme.typography.mono_family.clone())
                        .child(detail),
                )
                // Hiding the gap would make a badly informed answer look like a
                // wrong one (UX-SPEC).
                .when(*withheld, |el| {
                    el.child(
                        div()
                            .text_size(theme.typography.small_size)
                            .text_color(theme.colors.warning)
                            .child(
                                "The model received less than this: \
                                 the connection's privacy tier withheld part of it.",
                            ),
                    )
                })
                .into_any_element()
            }
            Entry::Rejected { tool, error } => row
                .child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child(format!("{tool} · Refused before the bus — nothing ran")),
                )
                .child(div().child(error.clone()))
                .into_any_element(),
            Entry::Ended(ending) => row
                .child(
                    div()
                        .text_color(theme.colors.text_muted)
                        .child(ending_line(*ending)),
                )
                .into_any_element(),
            // Un refus expliqué, pas une panne : couleur d'avertissement et
            // non de danger, et le texte dit ce qui n'a pas eu lieu.
            Entry::Notice(message) => row
                .child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.warning)
                        .child(gpui::SharedString::from(message.clone())),
                )
                .into_any_element(),
            Entry::Failed(message) => row
                .child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.danger)
                        .child("The conversation stopped"),
                )
                .child(
                    div()
                        .font_family(theme.typography.mono_family.clone())
                        .child(message.clone()),
                )
                .into_any_element(),
        }
    }

    /// The statements the last answer proposes, each with a way to open it.
    fn assistant_proposals(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let theme = Theme::of(cx);
        let blocks = sql_proposals(last_answer(&self.assistant.entries)?);
        if blocks.is_empty() {
            return None;
        }
        let sql = self.capabilities.contains(Capabilities::SQL);
        Some(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child(
                            "Opening a proposal puts its text in a new console. \
                             Nothing runs until you run it.",
                        ),
                )
                .children(blocks.into_iter().enumerate().map(|(index, block)| {
                    let text = block.clone();
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .p_2()
                                .rounded(theme.radii.control)
                                .border_1()
                                .border_color(theme.colors.border)
                                .font_family(theme.typography.mono_family.clone())
                                .text_size(theme.typography.mono_size)
                                .child(block),
                        )
                        .child(
                            activable(
                                control(
                                    ("assistant-proposal", index),
                                    if sql {
                                        ControlState::Enabled
                                    } else {
                                        ControlState::Disabled
                                    },
                                    ControlTone::Neutral,
                                    theme,
                                    cx.listener({
                                        let text = text.clone();
                                        move |this, _, _, cx| this.open_proposal(text.clone(), cx)
                                    }),
                                ),
                                move |this, _, cx| this.open_proposal(text.clone(), cx),
                                cx,
                            )
                            .debug_selector(move || format!("assistant-proposal-{index}"))
                            .flex_none()
                            .h(theme.metrics.control_height)
                            .px(theme.spacing.medium)
                            .child("Open in a console"),
                        )
                }))
                .into_any_element(),
        )
    }

    /// The question field, its send control, and the cancellation.
    fn assistant_question(&self, state: PanelState, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let running = state == PanelState::Running;
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(theme.metrics.control_height)
                            .child(self.assistant.question.clone()),
                    )
                    .child(
                        activable(
                            control(
                                "assistant-ask",
                                if running {
                                    ControlState::Disabled
                                } else {
                                    ControlState::Enabled
                                },
                                ControlTone::Primary,
                                theme,
                                cx.listener(|this, _, _, cx| this.ask_assistant(cx)),
                            ),
                            |this, _, cx| this.ask_assistant(cx),
                            cx,
                        )
                        .debug_selector(|| "assistant-ask".into())
                        .flex_none()
                        .h(theme.metrics.control_height)
                        .px(theme.spacing.medium)
                        .child("Ask"),
                    ),
            )
            // Offered for the whole time the conversation runs, not between two
            // turns. It stays enabled after being pressed: the request is taken
            // immediately, its effect is the provider's (UX-SPEC).
            .when(running, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            activable(
                                control(
                                    "assistant-cancel",
                                    ControlState::Enabled,
                                    ControlTone::Neutral,
                                    theme,
                                    cx.listener(|this, _, _, cx| this.cancel_conversation(cx)),
                                ),
                                |this, _, cx| this.cancel_conversation(cx),
                                cx,
                            )
                            .debug_selector(|| "assistant-cancel".into())
                            .flex_none()
                            .h(theme.metrics.control_height)
                            .px(theme.spacing.medium)
                            .child("Stop · Esc"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(theme.typography.small_size)
                                .text_color(theme.colors.text_muted)
                                .child(if self.assistant.cancelling {
                                    "Stop requested. The conversation ends as soon as the \
                                     provider speaks again."
                                } else {
                                    "The assistant is working. You can stop it at any moment."
                                }),
                        ),
                )
            })
            .into_any_element()
    }
}
