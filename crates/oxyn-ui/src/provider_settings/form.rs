//! Le formulaire de déclaration d'un fournisseur.
//!
//! Séparé de [`view`](super::view) pour la raison qui vaut dans
//! `connection_form/` : la liste et le formulaire sont deux sujets, et le
//! fichier qui les portait tous les deux passait le seuil de vigilance de
//! `CLAUDE.md`.

use super::view::hint;
use super::*;
use crate::controls::{ControlState, ControlTone, control};
use crate::theme::Theme;
use gpui::{AnyElement, ClickEvent, FontWeight, Pixels, px};

/// Largeur maximale d'un champ de cet écran.
///
/// Une constante locale et **non** `metrics.sidebar_width` : emprunter la
/// largeur du panneau latéral coupleraient ces champs à un panneau qui n'a rien
/// à voir avec eux, et le jour où le panneau change, les champs suivraient sans
/// raison — c'est la mise en garde que porte déjà
/// [`format_settings`](crate::format_settings). La valeur est celle de l'écran
/// de préférences (Figma `47:8461`), pour que deux écrans de réglages ne se
/// présentent pas avec deux largeurs de champ différentes ; il n'existe pas de
/// planche de celui-ci.
const FIELD_WIDTH: Pixels = px(408.0);

impl ProviderSettings {
    /// Le formulaire de déclaration.
    pub(super) fn render_form(&self, theme: &Theme, cx: &Context<'_, Self>) -> AnyElement {
        let occupe = !self.state.accepts_edits();
        let enregistrable = self.is_submittable();
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.medium)
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(if self.declares_agent() {
                        "Declare an external agent"
                    } else {
                        "Declare a provider"
                    }),
            )
            .child(field_row(
                "Kind",
                None,
                self.kind.clone().into_any_element(),
                theme,
            ))
            .child(field_row(
                "Name",
                Some("the one you will read in the interface"),
                self.label.clone().into_any_element(),
                theme,
            ))
            // Les champs des deux sortes ne coexistent jamais : montrer un
            // « Endpoint » grisé à côté d'une commande laisserait croire qu'un
            // agent en a un.
            .children((!self.declares_agent()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.spacing.medium)
                    .child(field_row(
                        "Endpoint",
                        Some("no credentials in the URL: the key goes to the keychain"),
                        self.base_url.clone().into_any_element(),
                        theme,
                    ))
                    .child(field_row(
                        "Model",
                        None,
                        self.model.clone().into_any_element(),
                        theme,
                    ))
                    .child(field_row(
                        "Key",
                        Some("optional; it goes to the system keychain and is never shown again"),
                        self.key.clone().into_any_element(),
                        theme,
                    ))
            }))
            .children(self.declares_agent().then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.spacing.medium)
                    .child(field_row(
                        "Command",
                        Some("the program to run; Oxyn never passes it through a shell"),
                        self.command.clone().into_any_element(),
                        theme,
                    ))
                    .child(field_row(
                        "Arguments",
                        Some("one per line, spaces included — no quoting rules to learn"),
                        self.args.clone().into_any_element(),
                        theme,
                    ))
                    .child(hint(
                        "No key is asked for: an external agent carries its own \
                         authentication. Oxyn cannot see where it sends your schema, so a \
                         local-only connection will refuse it.",
                        theme,
                    ))
            }))
            .children(self.notice.clone().map(|message| {
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.danger)
                    .child(message)
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.spacing.small)
                    .child(
                        control(
                            "oxyn-provider-save",
                            if enregistrable {
                                ControlState::Enabled
                            } else {
                                ControlState::Disabled
                            },
                            ControlTone::Primary,
                            theme,
                            cx.listener(|ecran, _: &ClickEvent, _, cx| ecran.submit(cx)),
                        )
                        .h(theme.metrics.control_height)
                        .px(theme.spacing.medium)
                        .track_focus(&self.save_focus)
                        .child("Save"),
                    )
                    .children(self.render_progress(theme, cx)),
            )
            .child(hint(
                if occupe {
                    "The system keychain may ask for your permission."
                } else {
                    "Tab: next field · Enter: save"
                },
                theme,
            ))
            .into_any_element()
    }

    /// Ce qui se passe pendant l'attente, et de quoi y renoncer.
    ///
    /// L'abandon est **demandé**, pas exécuté ici : c'est `oxyn-app` qui porte
    /// le jeton d'annulation. Un bouton qui prétendrait interrompre le
    /// trousseau lui-même mentirait.
    fn render_progress(&self, theme: &Theme, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let ProviderSettingsState::Working { operation } = &self.state else {
            return None;
        };
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.small)
                .child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child(SharedString::new_static(operation.label())),
                )
                .child(
                    control(
                        "oxyn-provider-cancel",
                        ControlState::Enabled,
                        ControlTone::Neutral,
                        theme,
                        cx.listener(|ecran, _: &ClickEvent, _, cx| ecran.cancel(cx)),
                    )
                    .h(theme.metrics.control_height)
                    .px(theme.spacing.medium)
                    .track_focus(&self.abandon_focus)
                    .child("Cancel · Esc"),
                )
                .into_any_element(),
        )
    }
}

/// Un champ, son libellé et sa précision.
fn field_row(
    libelle: &'static str,
    precision: Option<&'static str>,
    champ: AnyElement,
    theme: &Theme,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.tiny)
        .child(SharedString::new_static(libelle))
        .children(precision.map(|texte| hint(texte, theme)))
        .child(div().max_w(FIELD_WIDTH).child(champ))
        .into_any_element()
}
