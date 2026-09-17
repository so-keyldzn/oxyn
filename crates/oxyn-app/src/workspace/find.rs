//! `Find in loaded results…` (`273:37024`) : le champ, et ce qu'il avoue.
//!
//! # Pourquoi ce lot ne touche pas à l'export
//!
//! La maquette écrit **`Find`**, et sa grille montre les dix lignes du résultat
//! sous le champ : rien n'y est caché, aucun compteur de correspondances ne s'y
//! affiche. Une recherche qui **révèle** ne retranche aucune ligne, donc la
//! règle « [ce qui est exporté est ce qui est affiché](../../../../docs/UX-SPEC.md) »
//! reste vraie sans rien arbitrer. Un filtre, lui, l'aurait rendue fausse — et
//! c'est cet arbitrage-là qui reste ouvert, pour les colonnes masquées.
//!
//! # Le parcours ne bloque pas
//!
//! [`oxyn_data::find_rows`] coûte en proportion des cellules résidentes : il
//! part sur l'exécuteur de fond ([I-05](../../../CLAUDE.md#i-05)). Le champ
//! cherche à la validation plutôt qu'à chaque touche — un parcours par frappe
//! serait du travail jeté à la frappe suivante.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, App, SharedString, div, px};
use oxyn_ui::{FieldEvent, Theme};
use std::sync::Arc;

impl Workspace {
    /// Raccorde le champ de recherche à la grille.
    pub(super) fn wire_find_field(&mut self, cx: &mut Context<'_, Self>) {
        cx.subscribe(&self.find_field.clone(), |this, champ, event, cx| {
            match event {
                // Entrée : chercher si l'aiguille a changé, sinon aller à la
                // correspondance suivante. C'est le geste d'un champ de
                // recherche partout ailleurs.
                FieldEvent::Submit => {
                    let aiguille = champ.read(cx).text().to_owned();
                    if aiguille == this.find_needle {
                        this.reveal_find(true, cx);
                    } else {
                        this.run_find(aiguille, cx);
                    }
                }
                FieldEvent::Escape => this.clear_find(cx),
                _ => {}
            }
        })
        .detach();
    }

    /// Oublie la recherche en cours.
    ///
    /// Appelée aussi quand un nouveau résultat arrive : des correspondances
    /// calculées sur le résultat précédent désigneraient des lignes qui ne sont
    /// plus les mêmes.
    pub(super) fn clear_find(&mut self, cx: &mut Context<'_, Self>) {
        self.find_needle.clear();
        self.find_generation = self.find_generation.wrapping_add(1);
        self.find_field
            .update(cx, |champ, cx| champ.set_text(String::new(), cx));
        self.grid.update(cx, |grille, cx| {
            grille.set_find(None, cx);
        });
    }

    /// Va à la correspondance suivante ou précédente.
    ///
    /// Appelée par les deux contrôles de la barre, jamais par un événement du
    /// champ : `TextField` n'émet `Next` que sur **Tab**, et seulement s'il a été
    /// construit avec `with_managed_tab_order()` — ce qui n'est pas le cas ici,
    /// pour que Tab continue de sortir du champ. Un bras `FieldEvent::Next` a
    /// existé ici ; il était mort, et « correspondance précédente » n'était donc
    /// atteignable par aucun geste.
    pub(super) fn reveal_find(&mut self, forward: bool, cx: &mut Context<'_, Self>) {
        self.grid.update(cx, |grille, cx| {
            grille.reveal_match(forward, cx);
        });
    }

    /// Parcourt les lots résidents **hors du fil d'interface**.
    fn run_find(&mut self, needle: String, cx: &mut Context<'_, Self>) {
        self.find_needle = needle.clone();
        self.find_generation = self.find_generation.wrapping_add(1);
        let generation = self.find_generation;

        let (Some(tampon), format) = self
            .grid
            .read_with(cx, |grille, _| (grille.buffer(), grille.format().clone()))
        else {
            return;
        };

        // L'identité du tampon parcouru, pas seulement son contenu : la
        // génération ne capte qu'une **nouvelle recherche**, jamais un
        // **nouveau résultat**. Sans cette comparaison, un parcours parti sur le
        // résultat A et revenu après l'arrivée de B souligne dans B des lignes
        // trouvées dans A — et un « No match » calculé sur A se lit comme une
        // affirmation sur B. Pour quelqu'un qui cherche un identifiant précis,
        // c'est le mensonge exact que ce module dit vouloir éviter.
        let parcouru = Arc::as_ptr(&tampon);

        cx.spawn(async move |this, cx| {
            let trouve = cx
                .background_executor()
                .spawn(async move { oxyn_data::find_rows(&tampon, &needle, &format) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.find_generation != generation {
                    return;
                }
                let trouvee = trouve.rows.first().copied();
                this.grid.update(cx, |grille, cx| {
                    if grille.buffer().map(|actuel| Arc::as_ptr(&actuel)) != Some(parcouru) {
                        return;
                    }
                    grille.set_find(Some(trouve), cx);
                    if let Some(ligne) = trouvee {
                        grille.select_row(ligne, cx);
                    }
                });
            });
        })
        .detach();
    }

    /// La barre d'actions de résultat de `191:2058`.
    ///
    /// Le libellé précède le champ plutôt que de vivre dedans : `TextField` n'a
    /// pas de texte d'invite, et lui en ajouter un pour un seul appelant serait
    /// une API de confort qu'aucun autre écran ne demande. Un libellé visible
    /// vaut de toute façon mieux qu'une invite qui disparaît à la première
    /// frappe — c'est ce que l'accessibilité demande d'un champ.
    pub(super) fn find_bar(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .debug_selector(|| "find-bar".into())
            .flex_none()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_2()
            .pb_2()
            .text_size(theme.typography.small_size)
            .text_color(theme.colors.text_muted)
            .child("Find in loaded results…")
            // `273:37024` donne 300 px au champ. `max_w` plutôt que `w` : à
            // 300 px fixes, le champ pousse le résumé hors du cadre dès que la
            // fenêtre est étroite, et un avertissement invisible ne vaut pas
            // mieux qu'un avertissement absent. `flex_wrap` sur la rangée fait
            // le reste.
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.))
                    .max_w(px(300.))
                    .child(self.find_field.clone()),
            )
            // `273:37076` : le groupe de boutons que la maquette place juste
            // après le champ. C'est le seul geste qui atteint « correspondance
            // précédente » — voir `reveal_find`.
            .children(self.grid.read(cx).find().is_some().then(|| {
                div()
                    .flex_none()
                    .flex()
                    .gap_1()
                    .child(self.control(
                        "find-previous",
                        "Previous match",
                        Control::PreviousMatch,
                        false,
                        cx,
                    ))
                    .child(self.control("find-next", "Next match", Control::NextMatch, false, cx))
            }))
            .children(self.find_summary(cx).map(|resume| {
                div()
                    .text_color(
                        if self
                            .grid
                            .read(cx)
                            .find()
                            .is_some_and(|t| t.skipped_batches > 0 || t.capped)
                        {
                            // Ce qui n'a pas été parcouru est un avertissement, pas
                            // une précision : c'est ce qui décide si « aucune
                            // correspondance » veut dire quelque chose.
                            theme.colors.warning
                        } else {
                            theme.colors.text_muted
                        },
                    )
                    .child(SharedString::from(resume))
            }))
            .into_any_element()
    }

    /// Ce que la barre dit de la dernière recherche.
    ///
    /// Trois phrases, et la troisième est celle qui compte : un tampon qui a
    /// débordé n'a pas été parcouru en entier, et le taire ferait lire « aucune
    /// correspondance » comme « cette valeur n'est pas dans votre résultat ».
    pub(super) fn find_summary(&self, cx: &App) -> Option<String> {
        if self.find_needle.trim().is_empty() {
            return None;
        }
        let trouve = self.grid.read(cx).find()?;
        let mut phrase = match trouve.rows.len() {
            0 => "No match".to_owned(),
            1 => "1 matching row".to_owned(),
            nombre => format!("{nombre} matching rows"),
        };
        // Un compte plafonné annoncé comme exact se lit comme un total. Même
        // raison que pour les lots sautés : ce qui n'a pas été fait se dit.
        if trouve.capped {
            phrase.push_str(" or more · the search stopped counting");
        }
        if trouve.skipped_batches > 0 {
            phrase.push_str(&format!(
                " · {} batch{} not searched: they spilled to disk",
                trouve.skipped_batches,
                if trouve.skipped_batches == 1 {
                    ""
                } else {
                    "es"
                }
            ));
        }
        Some(phrase)
    }
}
