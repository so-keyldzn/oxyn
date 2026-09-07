//! Les réglages d'affichage des cellules de la grille.
//!
//! # Ce que règle cet écran, et ce qu'il ne règle pas
//!
//! Il produit un [`FormatOptions`], que `oxyn-app` porte jusqu'à
//! [`DataGrid::set_format_options`](crate::data_grid::DataGrid::set_format_options).
//! Ces réglages ne changent **que l'affichage** : ni la requête, ni ce que le
//! serveur renvoie, ni ce que l'export écrit — celui-ci a ses propres options
//! (`oxyn_data::ExportOptions`). Un utilisateur qui active le groupement des
//! milliers voit `4 823 917` à l'écran et retrouve `4823917` dans son CSV.
//!
//! # Pourquoi il n'y a pas de commande du bus, ni cinq états
//!
//! Rien ici n'atteint un driver ni un serveur : c'est du réglage local, que
//! [UX-SPEC](../../../docs/UX-SPEC.md#ce-qui-nest-jamais-optimiste) autorise
//! explicitement à s'appliquer immédiatement. Les cinq états d'une vue valent
//! pour *ce qui dépend d'une opération distante* ; il n'y en a aucune ici, et
//! fabriquer un état « erreur » qui ne peut pas survenir n'apprendrait rien à
//! personne.
//!
//! Il n'y a pas non plus de condition de capacité
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)) : ces réglages
//! portent sur le rendu d'un `RecordBatch`, que tout driver produit
//! ([ADR-0002](../../../docs/adr/0002-arrow-result-model.md)). Ils ne supposent
//! ni table, ni schéma, ni SQL.
//!
//! # Ce qui n'est pas fait
//!
//! **Rien n'est enregistré.** Les réglages vivent le temps de la session. Les
//! persister demande un format ouvert et documenté
//! ([I-11](../../../CLAUDE.md#i-11)) et une écriture de workspace, donc une
//! commande du bus : c'est un autre lot.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels, SharedString,
    Window, div, px,
};
use oxyn_data::cell::{FormatOptions, NumberGrouping};

use crate::controls::{ControlState, ControlTone, control};
use crate::text_field::{FieldEvent, TextField};
use crate::theme::Theme;

/// Ce que l'écran de réglages demande.
///
/// Un événement, pas un appel : la vue ne connaît pas la grille
/// ([I-01](../../../CLAUDE.md#i-01)).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FormatSettingsEvent {
    /// Les réglages ont changé ; voici leur état complet.
    Changed(FormatOptions),
}

/// Largeur maximale d'un champ de cet écran.
///
/// Figma `47:8461` : les champs de l'écran de préférences font 408 px de large.
/// Une largeur propre à cet écran, pas un jeton du thème — rien d'autre ne s'y
/// aligne, et un jeton partagé créerait un couplage entre écrans sans rapport.
const FIELD_WIDTH: Pixels = px(408.0);

/// Un groupement offert par l'écran.
struct Segment {
    mode: NumberGrouping,
    /// L'identité stable du contrôle.
    ///
    /// Stable et non dérivée d'un index : GPUI retrouve le focus par cet
    /// identifiant, et un identifiant qui change au réordonnancement ferait
    /// sauter le focus d'un segment à l'autre.
    id: &'static str,
    label: &'static str,
    /// Ce que donne ce groupement, sur un nombre assez long pour le montrer.
    ///
    /// Un aperçu plutôt qu'une explication : « 4 823 917 » dit en un coup d'œil
    /// ce qu'un paragraphe dirait mal. Écrit ici plutôt que calculé, pour que
    /// l'écran n'ait pas à formater un nombre fictif à chaque trame.
    preview: &'static str,
}

/// Les groupements offerts, dans l'ordre où ils s'affichent.
///
/// Une table et non deux `match` : [`NumberGrouping`] est `#[non_exhaustive]`,
/// donc une variante ajoutée dans `oxyn-data` n'aurait pas de branche ici. Avec
/// une table, elle n'a simplement **pas de segment** tant que personne ne l'a
/// ajoutée — un réglage manquant se voit à l'écran, alors qu'une branche `_`
/// attrape-tout aurait affiché un libellé faux.
const GROUPEMENTS: [Segment; 2] = [
    Segment {
        mode: NumberGrouping::None,
        id: "oxyn-format-grouping-none",
        label: "Aucun",
        preview: "4823917",
    },
    Segment {
        mode: NumberGrouping::Thousands,
        id: "oxyn-format-grouping-thousands",
        label: "Milliers",
        preview: "4\u{a0}823\u{a0}917",
    },
];

/// L'aperçu d'un groupement, s'il en a un.
///
/// `None` pour une variante que cet écran ne sait pas encore présenter.
#[must_use]
pub fn apercu(mode: NumberGrouping) -> Option<&'static str> {
    GROUPEMENTS
        .iter()
        .find(|segment| segment.mode == mode)
        .map(|segment| segment.preview)
}

/// L'écran de réglages d'affichage des cellules.
pub struct FormatSettings {
    focus: FocusHandle,
    options: FormatOptions,
    null_text: Entity<TextField>,
    /// Vrai quand les contrôles sont montrés sans pouvoir être modifiés.
    read_only: bool,
}

impl std::fmt::Debug for FormatSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FormatSettings")
            .field("options", &self.options)
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<FormatSettingsEvent> for FormatSettings {}

impl Focusable for FormatSettings {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FormatSettings {
    /// L'écran, sur des réglages de départ.
    pub fn new(options: FormatOptions, cx: &mut Context<'_, Self>) -> Self {
        let null_text = cx.new(|cx| TextField::new(options.null_text.to_string(), false, cx));
        cx.subscribe(&null_text, |this, champ, event, cx| {
            // Seul `Changed` compte : la saisie de l'indicateur de NULL ne
            // valide ni n'annule rien, il n'y a pas de formulaire à soumettre.
            if matches!(event, FieldEvent::Changed) {
                let texte = champ.read(cx).text().to_owned();
                this.options.null_text = texte.into();
                this.announce(cx);
            }
        })
        .detach();

        Self {
            focus: cx.focus_handle(),
            options,
            null_text,
            read_only: false,
        }
    }

    /// Les réglages courants.
    #[must_use]
    pub const fn options(&self) -> &FormatOptions {
        &self.options
    }

    /// Montre les réglages sans permettre de les changer.
    ///
    /// Les contrôles restent atteignables au clavier : une valeur qu'on ne peut
    /// pas modifier doit rester lisible et copiable
    /// ([`ControlState::ReadOnly`]).
    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<'_, Self>) {
        self.read_only = read_only;
        cx.notify();
    }

    /// Choisit un groupement de chiffres.
    pub fn choose_grouping(&mut self, grouping: NumberGrouping, cx: &mut Context<'_, Self>) {
        if self.read_only || self.options.number_grouping == grouping {
            return;
        }
        self.options.number_grouping = grouping;
        self.announce(cx);
    }

    /// Publie l'état complet et redessine.
    fn announce(&mut self, cx: &mut Context<'_, Self>) {
        cx.emit(FormatSettingsEvent::Changed(self.options.clone()));
        cx.notify();
    }

    /// L'état à donner aux contrôles, selon que l'écran est modifiable.
    const fn control_state(&self) -> ControlState {
        if self.read_only {
            ControlState::ReadOnly
        } else {
            ControlState::Enabled
        }
    }

    /// Le réglage de l'indicateur de valeur absente.
    fn render_null(&self, theme: &Theme) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.tiny)
            .child(label("Indicateur de valeur absente", theme).child(
                // La règle du module `oxyn_data::cell` en une phrase : une
                // colonne texte peut contenir littéralement « NULL », et
                // l'écran doit rester capable de distinguer les deux.
                hint("dessiné en italique, distinct d'une chaîne", theme),
            ))
            // Le champ porte sa propre hauteur ; la largeur est celle du
            // conteneur, bornée. Emprunter ici `metrics.sidebar_width` aurait
            // couplé la largeur d'un champ de formulaire à celle du panneau
            // latéral : le jour où le panneau change, le champ suit sans raison.
            .child(div().max_w(FIELD_WIDTH).child(self.null_text.clone()))
            .into_any_element()
    }

    /// Le réglage de groupement des chiffres.
    fn render_grouping(&self, theme: &Theme, cx: &Context<'_, Self>) -> AnyElement {
        let state = self.control_state();
        let actif = self.options.number_grouping;
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.tiny)
            .child(
                label("Format des nombres", theme)
                    .child(hint("n'affecte pas les fichiers exportés", theme)),
            )
            .child(div().flex().flex_row().gap(theme.spacing.small).children(
                GROUPEMENTS.iter().map(|segment| {
                    let mode = segment.mode;
                    let retenu = mode == actif;
                    control(
                        segment.id,
                        state,
                        if retenu {
                            ControlTone::Primary
                        } else {
                            ControlTone::Neutral
                        },
                        theme,
                        cx.listener(move |ecran, _, _, cx| {
                            ecran.choose_grouping(mode, cx);
                        }),
                    )
                    .h(theme.metrics.control_height)
                    .px(theme.spacing.medium)
                    // La pastille, et pas seulement la tonalité : une
                    // information portée par la seule couleur n'existe pas pour
                    // qui ne la distingue pas, et la liste de contrôle
                    // d'interface le refuse. Elle dit aussi ce que la tonalité
                    // ne dit pas — que le choix est exclusif.
                    .child(SharedString::new_static(if retenu { "●" } else { "○" }))
                    .child(SharedString::new_static(segment.label))
                }),
            ))
            .children(apercu(actif).map(|texte| hint(texte, theme)))
            .into_any_element()
    }
}

impl Render for FormatSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        div()
            .id("oxyn-format-settings")
            .track_focus(&self.focus)
            .key_context("FormatSettings")
            .flex()
            .flex_col()
            .gap(theme.spacing.huge)
            .p(theme.spacing.large)
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .text_size(theme.typography.ui_size)
            .child(self.render_null(&theme))
            .child(self.render_grouping(&theme, cx))
    }
}

/// Le libellé d'un réglage.
fn label(texte: &'static str, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(theme.spacing.small)
        .child(SharedString::new_static(texte))
}

/// Une précision secondaire, en petit.
fn hint(texte: &'static str, theme: &Theme) -> AnyElement {
    div()
        .text_size(theme.typography.small_size)
        .text_color(theme.colors.text_muted)
        .child(SharedString::new_static(texte))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_segment_a_son_identite_son_libelle_et_son_apercu() {
        // Deux segments qui partageraient un identifiant se voleraient le
        // focus ; deux aperçus identiques ne montreraient rien.
        for (rang, segment) in GROUPEMENTS.iter().enumerate() {
            for autre in GROUPEMENTS.iter().skip(rang + 1) {
                assert_ne!(segment.id, autre.id, "deux segments se confondent");
                assert_ne!(segment.label, autre.label);
                assert_ne!(segment.preview, autre.preview);
            }
        }
    }

    #[test]
    fn lapercu_montre_le_separateur_que_la_grille_emploie() {
        // Si l'aperçu et le rendu divergeaient, l'écran de réglages mentirait
        // sur ce que la grille va faire. Le séparateur vient de `oxyn-data`,
        // et c'est le seul endroit où il est décidé.
        use oxyn_data::cell::GROUP_SEPARATOR;
        assert!(
            apercu(NumberGrouping::Thousands)
                .expect("le groupement par milliers a un segment dans cet écran")
                .contains(GROUP_SEPARATOR)
        );
        assert!(
            !apercu(NumberGrouping::None)
                .expect("l'absence de groupement a un segment dans cet écran")
                .contains(GROUP_SEPARATOR)
        );
    }

    #[test]
    fn lapercu_reproduit_exactement_ce_que_rend_la_grille() {
        // Le seul test qui empêche l'aperçu de dériver : il passe par le
        // formateur réel de `oxyn-data` plutôt que par une chaîne recopiée.
        use arrow::array::Int64Array;
        use oxyn_data::cell::{CellValue, format_value};

        let array = Int64Array::from(vec![Some(4_823_917)]);
        for segment in &GROUPEMENTS {
            let options = FormatOptions::default().with_number_grouping(segment.mode);
            let rendu = format_value(&array, 0, &options);
            assert_eq!(
                rendu,
                CellValue::Text(segment.preview.into()),
                "l'aperçu de « {} » ne correspond pas au rendu",
                segment.label
            );
        }
    }
}
