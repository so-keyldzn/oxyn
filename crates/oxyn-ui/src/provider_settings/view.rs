//! Le rendu de l'écran de configuration des fournisseurs.
//!
//! Aucune géométrie « relevée » ici : il n'existe pas de planche de cet écran
//! (`docs/FIGMA-HANDOFF.md`, § ce que la maquette ne montre pas). Tout vient des
//! jetons de [`Theme`] et des primitives déjà écrites — [`control`],
//! [`TextField`], [`SelectField`] — pour que cet écran suive la maquette là où
//! elle existe, c'est-à-dire dans ses composants.

use super::*;
use crate::controls::{ControlState, ControlTone, control};
use crate::theme::Theme;
use gpui::{AnyElement, ClickEvent, FontWeight};

impl Render for ProviderSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let corps = match &self.state {
            ProviderSettingsState::Loading => self.render_loading(&theme),
            _ => div()
                .flex()
                .flex_col()
                .gap(theme.spacing.huge)
                .children(self.render_banner(&theme, cx))
                .child(self.render_list(&theme, window, cx))
                .child(self.render_form(&theme, cx))
                .into_any_element(),
        };

        div()
            .id("oxyn-provider-settings")
            .track_focus(&self.focus)
            .key_context("ProviderSettings")
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .gap(theme.spacing.large)
            .p(theme.spacing.large)
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.ui_size)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.spacing.tiny)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Model providers"),
                    )
                    .child(hint(
                        "They are shared by every workspace. The privacy tier, in contrast, \
                         stays attached to each connection.",
                        &theme,
                    )),
            )
            .child(corps)
    }
}

impl ProviderSettings {
    /// L'état initial : la liste n'a pas encore été lue.
    fn render_loading(&self, theme: &Theme) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.tiny)
            .child("Reading declared providers…")
            .child(hint(
                "Nothing is contacted: this reads from this machine only.",
                theme,
            ))
            .into_any_element()
    }

    /// Le bandeau d'échec, au-dessus de ce qui reste utilisable.
    ///
    /// Il ne remplace ni la liste ni le formulaire : l'action suivante est de
    /// corriger la saisie, et un écran d'erreur plein la ferait disparaître.
    fn render_banner(&self, theme: &Theme, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let ProviderSettingsState::Failed {
            message,
            retryable,
            key_must_be_retyped,
        } = &self.state
        else {
            return None;
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.tiny)
                .p(theme.spacing.small)
                .rounded(theme.radii.surface)
                .border_1()
                .border_color(theme.colors.danger)
                // Pas de glyphe : aucune des icônes embarquées ne dit
                // « échec », et en emprunter une qui dit autre chose serait
                // pire que rien. Le titre porte l'information, la couleur ne
                // fait que la doubler.
                .child(
                    div()
                        .text_color(theme.colors.danger)
                        .child("The operation failed"),
                )
                // Le message tel qu'il est venu, à chasse fixe : le public de
                // ce produit lit les messages de ses outils.
                .child(
                    div()
                        .font_family(theme.typography.mono_family.clone())
                        .text_size(theme.typography.mono_size)
                        .child(message.clone()),
                )
                .child(hint(
                    if *retryable {
                        "This operation can be retried as is."
                    } else {
                        "This operation must not be retried as is."
                    },
                    theme,
                ))
                .when(*key_must_be_retyped, |bandeau| {
                    bandeau.child(hint("The key was not kept: type it again.", theme))
                })
                .child(
                    div().child(
                        control(
                            "oxyn-provider-dismiss-error",
                            ControlState::Enabled,
                            ControlTone::Neutral,
                            theme,
                            cx.listener(|ecran, _: &ClickEvent, _, cx| ecran.dismiss(cx)),
                        )
                        .h(theme.metrics.control_height)
                        .px(theme.spacing.medium)
                        .track_focus(&self.dismiss_focus)
                        .child("Close · Esc"),
                    ),
                )
                .into_any_element(),
        )
    }

    /// Les déclarations existantes, ou l'état vide.
    fn render_list(&self, theme: &Theme, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let liste = div()
            .flex()
            .flex_col()
            .gap(theme.spacing.small)
            .child(div().font_weight(FontWeight::MEDIUM).child("Declared"));

        if self.declaration_count() == 0 {
            // L'état vide, et pas un appel à l'action : une installation sans
            // fournisseur est une installation complète (ADR-0006).
            return liste
                .child(
                    div()
                        .p(theme.spacing.medium)
                        .rounded(theme.radii.surface)
                        .border_1()
                        .border_color(theme.colors.border)
                        .flex()
                        .flex_col()
                        .gap(theme.spacing.tiny)
                        .child("Nothing declared")
                        .child(hint(
                            "Oxyn works without either: AI features appear only once you \
                             declare a provider or an external agent.",
                            theme,
                        )),
                )
                .into_any_element();
        }

        // La liste entière est une étape de tabulation, et les flèches y
        // choisissent la ligne : une étape par bouton ferait traverser autant
        // de tabulations qu'il y a de fournisseurs pour atteindre le
        // formulaire.
        let mut lignes = div()
            .id("oxyn-provider-list")
            .track_focus(&self.list_focus)
            .tab_index(0)
            .flex()
            .flex_col()
            .gap(theme.spacing.small);
        let liste_active = self.list_focus.is_focused(window);
        for (rang, fournisseur) in self.providers.iter().enumerate() {
            let ligne = crate::provider_settings::row::row_display(
                crate::provider_settings::row::Declaration::Provider(fournisseur),
            );
            lignes = lignes.child(self.render_row(
                rang,
                &ligne,
                liste_active && self.focused_row() == rang,
                theme,
                cx,
            ));
        }
        // Les agents suivent, dans la **même** liste et avec la même ligne : le
        // calcul les a rendus interchangeables, et un second parcours de
        // clavier n'aurait servi qu'à en doubler les défauts.
        let apres = self.providers.len();
        for (rang, agent) in self.agents().iter().enumerate() {
            let ligne = crate::provider_settings::row::row_display(
                crate::provider_settings::row::Declaration::Agent {
                    label: &agent.label,
                    command: &agent.command,
                    args: agent.args,
                },
            );
            let global = apres + rang;
            lignes = lignes.child(self.render_row(
                global,
                &ligne,
                liste_active && self.focused_row() == global,
                theme,
                cx,
            ));
        }
        liste
            .child(lignes)
            .child(hint(
                "↑ ↓ to choose a declaration · Enter to remove it",
                theme,
            ))
            .into_any_element()
    }

    /// Une déclaration : ce qu'elle vise, l'état de sa clé, son classement daté.
    fn render_row(
        &self,
        rang: usize,
        ligne: &crate::provider_settings::row::RowDisplay,
        curseur: bool,
        theme: &Theme,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let confirme = self.confirming == Some(rang);
        let etat = if self.state.accepts_edits() {
            ControlState::Enabled
        } else {
            ControlState::Disabled
        };
        div()
            .id(("oxyn-provider-row", rang))
            .flex()
            .flex_col()
            .gap(theme.spacing.tiny)
            .p(theme.spacing.small)
            .rounded(theme.radii.surface)
            .border_1()
            // Le curseur de la liste se voit : sans cela, les flèches
            // déplaceraient un choix invisible.
            .border_color(if curseur {
                theme.colors.border_focus
            } else {
                theme.colors.border
            })
            .when(curseur, |ligne| ligne.bg(theme.colors.selection))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.spacing.small)
                    // Le nom donné par l'utilisateur, jamais un identifiant.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(SharedString::from(ligne.label.clone())),
                    )
                    .child(
                        div()
                            .text_size(theme.typography.small_size)
                            .text_color(theme.colors.text_muted)
                            .child(SharedString::from(ligne.family.clone())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .child(SharedString::from(ligne.primary.clone()))
                    .child(SharedString::from(ligne.secondary.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.spacing.small)
                    .text_size(theme.typography.small_size)
                    // « configurée » ou « absente », jamais la valeur, et le
                    // mot plutôt qu'une pastille : une information portée par
                    // la seule couleur n'existe pas pour tout le monde.
                    .child(
                        div()
                            .text_color(theme.colors.text_muted)
                            .child(SharedString::from(ligne.key_note.clone())),
                    )
                    .child(
                        div()
                            .text_color(if ligne.reach_warns {
                                theme.colors.warning
                            } else {
                                theme.colors.text_muted
                            })
                            .child(SharedString::from(ligne.reach_note.clone())),
                    ),
            )
            .child(if confirme {
                self.render_confirmation(rang, &ligne.label, theme, cx)
            } else {
                div()
                    .child(
                        control(
                            ("oxyn-provider-remove", rang),
                            etat,
                            ControlTone::Neutral,
                            theme,
                            cx.listener(move |ecran, _: &ClickEvent, window, cx| {
                                ecran.ask_removal(rang, window, cx);
                            }),
                        )
                        .h(theme.metrics.control_height)
                        .px(theme.spacing.medium)
                        .child("Remove…"),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    /// La confirmation d'un retrait. Elle **nomme** le fournisseur, et le focus
    /// s'ouvre sur le refus.
    fn render_confirmation(
        &self,
        rang: usize,
        label: &str,
        theme: &Theme,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.tiny)
            .child(SharedString::from(removal_question(label)))
            .child(hint(
                "Documents already written keep the provenance they carry.",
                theme,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(theme.spacing.small)
                    // Le refus d'abord, dans l'ordre de tabulation comme à
                    // l'écran : c'est lui qui porte le focus initial.
                    .child(
                        control(
                            ("oxyn-provider-remove-cancel", rang),
                            ControlState::Enabled,
                            ControlTone::Neutral,
                            theme,
                            cx.listener(|ecran, _: &ClickEvent, _, cx| ecran.dismiss(cx)),
                        )
                        .h(theme.metrics.control_height)
                        .px(theme.spacing.medium)
                        .track_focus(&self.cancel_focus)
                        .child("Cancel · Esc"),
                    )
                    .child(
                        control(
                            ("oxyn-provider-remove-confirm", rang),
                            ControlState::Enabled,
                            ControlTone::Danger,
                            theme,
                            cx.listener(|ecran, _: &ClickEvent, _, cx| ecran.confirm_removal(cx)),
                        )
                        .h(theme.metrics.control_height)
                        .px(theme.spacing.medium)
                        .track_focus(&self.confirm_focus)
                        .child("Remove"),
                    ),
            )
            .into_any_element()
    }
}

/// Une précision secondaire, en petit.
pub(super) fn hint(texte: &'static str, theme: &Theme) -> AnyElement {
    div()
        .text_size(theme.typography.small_size)
        .text_color(theme.colors.text_muted)
        .child(SharedString::new_static(texte))
        .into_any_element()
}
