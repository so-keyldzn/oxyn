//! The 48 px bar of Figma `190:1543`: where you are, and where a question goes.
//!
//! Split out of `layout.rs` when the AI entry joined it. The bar now carries six
//! things that answer different questions — the sidebar toggle, the location,
//! `READ ONLY`, the environment, the privacy tier and `Ask AI` — and the rules
//! that decide whether the last two exist at all are worth reading on their own.
//!
//! # The rule that governs this file
//!
//! With no provider declared there is **no** `Ask AI` and **no** privacy badge:
//! not greyed, not hinted at, absent. Oxyn is a complete client without AI, and
//! an entry that invites the user to go and configure something is an
//! advertisement rather than a feature
//! ([UX-SPEC](../../../docs/UX-SPEC.md#le-workspace-ia-nexiste-que-sil-a-été-configuré),
//! [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)).
//!
//! The badge and the entry are neighbours because they are read together, and
//! the badge announces the tier of the **current connection** — never an
//! application setting. Switching consoles switches connection, and the badge
//! follows, conversation open or not.

use super::assistant::{AskAi, provider_for};
use super::controls::WorkspaceTooltip;
use super::layout::Control;
use super::*;
use crate::backend::ClassifiedProvider;
use gpui::{AnyElement, SharedString, div, px};
use oxyn_core::PrivacyTier;
use oxyn_ui::Theme;

/// What the bar says about where the user is.
///
/// Only the object panel adds a path: on a console, the location is the
/// connection and nothing else, and appending a stale catalog selection there
/// would name an object the user is not looking at.
pub(super) fn connection_context(
    panel: WorkspacePanel,
    selected: Option<&CatalogPath>,
    name: &str,
) -> String {
    match (panel, selected) {
        (WorkspacePanel::Object, Some(path)) => format!("{name} / {path}"),
        _ => name.to_owned(),
    }
}

/// Met une majuscule initiale, sans toucher au reste.
///
/// `PrivacyTier::as_str` rend un jeton stable en minuscules — c'est ce qui est
/// écrit dans les préférences et l'audit, et il ne doit pas changer pour un
/// besoin d'affichage.
fn capitalise(mot: &str) -> String {
    let mut lettres = mot.chars();
    match lettres.next() {
        Some(premiere) => premiere.to_uppercase().collect::<String>() + lettres.as_str(),
        None => String::new(),
    }
}

/// Ce que le badge de confidentialité écrit.
///
/// [UX-SPEC](../../../docs/UX-SPEC.md#repères-permanents) prescrit la forme
/// `Metadata · Cloud` : le niveau **et** la destination. Le badge n'écrivait que
/// le niveau, et c'est la moitié qui manquait qui compte — sous `Metadata` avec
/// un Ollama local et sous `Metadata` avec un fournisseur distant, l'utilisateur
/// voyait exactement la même chose, alors que
/// [AI-PROVIDERS](../../../docs/AI-PROVIDERS.md#local-et-distant-ne-se-distinguent-pas-par-lapi)
/// pose que les deux « se distinguent par un seul fait : les données quittent la
/// machine, ou non ».
///
/// La destination décrit **ce qui servira**, c'est-à-dire la déclaration que
/// `provider_for` retiendrait. Sous `Local`, aucun fournisseur distant n'est
/// éligible : le badge dit alors `Local` sans destination, parce qu'il n'y a
/// rien qui parte.
fn privacy_badge(tier: PrivacyTier, providers: Option<&[ClassifiedProvider]>) -> String {
    // Casse normale, pas de capitales : le relevé `190:1163` écrit
    // « Metadata · Cloud ». Les capitales sont réservées au marquage
    // d'environnement, où elles portent l'alerte — `PRODUCTION` doit se voir
    // d'un coup d'œil, le niveau de confidentialité se lit.
    let niveau = capitalise(tier.as_str());
    let Some(providers) = providers else {
        // La liste n'est pas encore lue : annoncer une destination serait la
        // deviner.
        return niveau;
    };
    match provider_for(providers, tier) {
        Some(retenu) if retenu.is_local() => format!("{niveau} · Local"),
        Some(_) => format!("{niveau} · Cloud"),
        None => niveau,
    }
}

impl Workspace {
    /// The connection bar, drawn once per frame from state alone.
    pub(super) fn connection_bar(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let context =
            connection_context(self.panel, self.selected_path.as_ref(), &self.display.name);
        let tier = self.display.privacy_tier;
        let entry = self.assistant.entry(tier, self.capabilities);
        div()
            // La métrique, pas un nombre recopié : `Metrics::toolbar_height`
            // existait, valait 48 nulle part et 52 dans le thème, et n'avait
            // aucun lecteur. Deux sources pour une même mesure, dont une fausse.
            .h(theme.metrics.toolbar_height)
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .child(self.control(
                "sidebar-toggle",
                "Toggle sidebar · ⌘B",
                Control::Sidebar,
                true,
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.colors.text_muted)
                    .child(context),
            )
            // The reason a disabled entry is disabled, next to the entry
            // itself. It is only ever drawn when `Ask AI` is inert, so it never
            // adds width to a bar whose last control has to stay clickable.
            .when_some(disabled_reason(entry), |el, reason| {
                el.child(
                    div()
                        .flex_none()
                        .max_w(px(320.))
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child(reason),
                )
            })
            .when(self.read_only, |el| {
                el.child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child("READ ONLY"),
                )
            })
            .child(
                div()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .rounded(theme.radii.full)
                    .border_1()
                    .border_color(theme.colors.environment(self.environment))
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.environment(self.environment))
                    .child(self.environment.as_str().to_uppercase()),
            )
            // Figma `190:1547`: 136 px, immediately left of `Ask AI`.
            .when(entry != AskAi::Absent, |el| {
                el.child(
                    div()
                        .id("privacy-tier")
                        .debug_selector(|| "privacy-tier".into())
                        .flex_none()
                        .min_w(px(136.))
                        .justify_center()
                        .flex()
                        .items_center()
                        .px_2()
                        .py_1()
                        .rounded(theme.radii.full)
                        .border_1()
                        .border_color(theme.colors.border)
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child(privacy_badge(tier, self.assistant.providers.as_deref()))
                        .tooltip(move |_, cx| {
                            cx.new(|_| WorkspaceTooltip(SharedString::from(tier.describe())))
                                .into()
                        }),
                )
            })
            // Figma `190:1549`: 96 × 32 at the right end.
            .when(entry != AskAi::Absent, |el| {
                el.child(self.control("ask-ai", "Ask AI", Control::AskAi, false, cx))
            })
            .into_any_element()
    }
}

/// The sentence that accompanies an inert `Ask AI`, if it is inert.
///
/// Split out so the wording is one value a test can hold, rather than a literal
/// buried in a `when`.
pub(super) fn disabled_reason(entry: AskAi) -> Option<&'static str> {
    match entry {
        AskAi::Disabled(reason) => Some(reason),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_barre_ne_nomme_l_objet_que_dans_le_panneau_objet() {
        let path = CatalogPath::for_relation(Some("shop"), Some("public"), "orders")
            .expect("chemin de catalogue valide");
        assert_eq!(
            connection_context(WorkspacePanel::Object, Some(&path), "commerce-prod"),
            format!("commerce-prod / {path}")
        );
        // Le piège fermé ici : une sélection de catalogue laissée derrière soi
        // et réaffichée au-dessus d'une console qui ne la regarde pas.
        assert_eq!(
            connection_context(WorkspacePanel::Sql, Some(&path), "commerce-prod"),
            "commerce-prod"
        );
        assert_eq!(
            connection_context(WorkspacePanel::Object, None, "commerce-prod"),
            "commerce-prod"
        );
    }

    #[test]
    fn une_entree_absente_ou_active_n_explique_rien() {
        // Une raison affichée à côté d'un bouton actif serait un message sans
        // objet ; à côté d'une entrée absente, elle trahirait l'absence même.
        assert_eq!(disabled_reason(AskAi::Absent), None);
        assert_eq!(disabled_reason(AskAi::Enabled), None);
        assert!(disabled_reason(AskAi::Disabled("parce que")).is_some());
    }

    /// Le badge dit **où** part la donnée, pas seulement combien il en part.
    ///
    /// Trouvé par la relecture des divergences : le badge écrivait
    /// `AI · METADATA` alors qu'UX-SPEC prescrit `Metadata · Cloud`. Sous
    /// `Metadata` avec un Ollama local et sous `Metadata` avec un fournisseur
    /// distant, l'utilisateur voyait la même chose — or c'est exactement la
    /// distinction qu'AI-PROVIDERS pose comme la seule qui compte.
    #[test]
    fn le_badge_de_confidentialite_nomme_la_destination() {
        use crate::backend::ClassifiedProvider;
        use oxyn_core::{AiProviderConfig, AiProviderKind, ProviderId};

        let declaration = |base_url: &str, reach| ClassifiedProvider {
            config: AiProviderConfig::new(
                ProviderId::for_new_declaration(AiProviderKind::OpenAiCompatible),
                AiProviderKind::OpenAiCompatible,
                "essai",
                base_url,
                "un-modele",
            ),
            reach,
            measured_at: chrono::Utc::now(),
        };

        let local = [declaration(
            "http://127.0.0.1:11434",
            oxyn_llm::Reach::Local,
        )];
        let distant = [declaration(
            "https://api.example.com",
            oxyn_llm::Reach::Remote,
        )];

        assert_eq!(
            privacy_badge(PrivacyTier::Metadata, Some(&distant)),
            "Metadata · Cloud",
            "un fournisseur distant doit se voir"
        );
        assert_eq!(
            privacy_badge(PrivacyTier::Metadata, Some(&local)),
            "Metadata · Local",
            "et un fournisseur local aussi — c'est toute la distinction"
        );

        // Sous `Local`, aucun distant n'est éligible : rien ne part, donc aucune
        // destination à annoncer.
        assert_eq!(privacy_badge(PrivacyTier::Local, Some(&distant)), "Local");

        // Liste pas encore lue : annoncer une destination serait la deviner.
        assert_eq!(privacy_badge(PrivacyTier::Metadata, None), "Metadata");
    }
}
