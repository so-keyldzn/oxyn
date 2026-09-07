//! Les états d'interaction des contrôles partagés.
//!
//! # Pourquoi un module plutôt qu'un style par bouton
//!
//! Avant ce module, chaque composant réécrivait
//! `.tab_index(0).cursor_pointer().hover(…)` à la main. Trois conséquences,
//! toutes constatées dans le code : aucun bouton n'avait de **focus visible**,
//! aucun n'avait d'état **pressé**, et aucun ne savait se rendre
//! **indisponible** autrement qu'en changeant de couleur de texte — ce qui
//! porte l'information par la seule couleur, que la
//! [liste de contrôle](../../../.claude/checklists/revue-ui.md) refuse.
//!
//! Le focus clavier n'est pas une finition :
//! [ADR-0001](../../../docs/adr/0001-ui-toolkit.md) l'identifie comme un risque
//! **structurel** de GPUI, qui ne l'offre pas gratuitement. Une surface non
//! atteignable au clavier est un défaut bloquant.
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne fixe **ni taille ni contenu**. Un bouton de barre d'état et un bouton
//! de formulaire n'ont pas la même hauteur ; imposer ici celle de la maquette
//! ferait déborder la barre d'état de six pixels. L'appelant pose sa hauteur —
//! `theme.metrics.control_height` pour un contrôle de formulaire — et ce module
//! pose les cinq états.
//!
//! Il n'émet aucun événement et ne connaît aucune donnée : ce sont les vues qui
//! branchent leur `on_click` ([I-01](../../../CLAUDE.md#i-01)).

use gpui::{
    App, BoxShadow, ClickEvent, Div, ElementId, InteractiveElement, Stateful,
    StatefulInteractiveElement, Styled, Window, div, point, px,
};

use crate::theme::Theme;

/// Ce qu'un contrôle laisse faire.
///
/// La distinction entre [`Disabled`](Self::Disabled) et
/// [`ReadOnly`](Self::ReadOnly) n'est pas cosmétique, et c'est le clavier qui la
/// rend visible : un contrôle **désactivé** sort de l'ordre de tabulation, un
/// contrôle en **lecture seule** y reste. Un champ qui affiche une valeur qu'on
/// ne peut pas modifier doit rester atteignable — sinon l'utilisateur au clavier
/// ne peut ni la lire au lecteur d'écran, ni la sélectionner pour la copier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ControlState {
    /// Le contrôle réagit : survol, focus, pression, clic.
    #[default]
    Enabled,
    /// Le contrôle ne réagit pas et **sort du parcours clavier**.
    ///
    /// À réserver à ce qui est indisponible pour une raison que l'écran
    /// explique par ailleurs — une capacité absente du driver
    /// ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)), une action
    /// sans objet. Un contrôle désactivé sans explication est un cul-de-sac.
    Disabled,
    /// Le contrôle montre sa valeur, reste atteignable, et ne se modifie pas.
    ReadOnly,
}

impl ControlState {
    /// Vrai si le contrôle répond au clic et à la touche d'activation.
    #[must_use]
    pub const fn accepts_input(self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Vrai si le contrôle est dans le parcours de tabulation.
    #[must_use]
    pub const fn is_focusable(self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

/// Le poids visuel d'un contrôle.
///
/// Nommé par **rôle** et non par couleur, pour la raison qui vaut déjà dans
/// [`Palette`](crate::theme::Palette) : un `Danger` qui deviendrait orange reste
/// un danger, alors qu'un `Red` qui devient orange ne veut plus rien dire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ControlTone {
    /// Action ordinaire, action secondaire, annulation.
    #[default]
    Neutral,
    /// L'action principale de l'écran.
    Primary,
    /// Une opération destructrice, ou une écriture en production
    /// ([I-02](../../../CLAUDE.md#i-02)).
    Danger,
}

/// Le socle d'un contrôle interactif, avec ses cinq états.
///
/// Pose l'identité, le parcours clavier, le rayon et la bordure de la maquette,
/// puis les styles de survol, de focus, de pression et d'indisponibilité. Ne
/// pose ni taille, ni marge intérieure, ni contenu : l'appelant les ajoute.
///
/// # Pourquoi l'action passe par ici
///
/// `action` n'est branchée **que** si l'état accepte une entrée. C'est la seule
/// façon de le garantir : GPUI accumule les gestionnaires de clic dans une
/// liste, donc un `on_click` accroché après coup par l'appelant se déclenche
/// quoi qu'en dise l'apparence du contrôle. Un bouton d'écriture en production
/// grisé en attendant une confirmation partirait quand même au clic de souris
/// ([I-02](../../../CLAUDE.md#i-02)).
///
/// ```ignore
/// control(
///     "oxyn-approval-reject",
///     ControlState::Enabled,
///     ControlTone::Neutral,
///     theme,
///     cx.listener(|vue, _, _, cx| vue.refuser(cx)),
/// )
/// .h(theme.metrics.control_height)
/// .px(theme.spacing.medium)
/// .child("Refuser")
/// ```
#[must_use]
pub fn control(
    id: impl Into<ElementId>,
    state: ControlState,
    tone: ControlTone,
    theme: &Theme,
    action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let colors = theme.colors;
    let ring = focus_ring(theme);

    // Le fond et le texte au repos.
    //
    // Un contrôle neutre repose sur `surface` et non sur `surface_raised` :
    // c'est ce que fait la maquette pour `SidebarMenuButton`, dont la variante
    // `Hover` **est** `surface_raised`. Un neutre déjà posé sur `surface_raised`
    // n'aurait aucun survol visible, puisque la palette donne la même valeur
    // aux deux rôles.
    let (background, foreground) = match tone {
        ControlTone::Neutral => (colors.surface, colors.text),
        ControlTone::Primary => (colors.accent, colors.text_on_accent),
        ControlTone::Danger => (colors.danger, colors.text_on_accent),
    };
    let border = match tone {
        ControlTone::Neutral => colors.border,
        ControlTone::Primary => colors.accent,
        ControlTone::Danger => colors.danger,
    };

    let base = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap(theme.spacing.tiny)
        .rounded(theme.radii.control)
        .border_1()
        .border_color(border)
        .bg(background)
        // Un contrôle indisponible perd son texte plein : la tonalité seule ne
        // porte pas l'information, le contraste du libellé la double.
        .text_color(if state.accepts_input() {
            foreground
        } else {
            colors.text_faint
        });

    // `tab_stop` porte aussi l'activation : GPUI déclenche `on_click` avec un
    // `ClickEvent::Keyboard` sur Entrée et Espace pour l'élément qui a le
    // focus. Un contrôle désactivé doit donc en sortir explicitement, sinon il
    // reste atteignable **et** activable au clavier tout en paraissant inerte.
    let base = if state.is_focusable() {
        base.tab_index(0).tab_stop(true)
    } else {
        base.tab_stop(false)
    };

    if !state.accepts_input() {
        // Ni survol, ni pression, ni action.
        return match state {
            // La lecture seule garde son anneau de focus — elle est
            // atteignable, donc son focus doit se voir — et garde son plein
            // contraste : la valeur qu'elle montre est faite pour être lue.
            ControlState::ReadOnly => base
                .cursor_default()
                .focus(move |style| style.shadow(vec![ring]).border_color(colors.border_focus)),
            // Le désactivé s'efface. C'est ce qui le distingue de la lecture
            // seule autrement que par la couleur du libellé : l'affaiblissement
            // se voit aussi en niveaux de gris, et il se double du fait que le
            // contrôle ne reçoit plus le focus.
            _ => base.cursor_default().opacity(DISABLED_OPACITY),
        };
    }

    let base = base
        .cursor_pointer()
        // L'anneau, et non une bordure épaissie : épaissir la bordure au focus
        // décale le contenu d'un pixel à chaque tabulation, ce qui se voit sur
        // toute une rangée de boutons.
        .focus(move |style| style.shadow(vec![ring]).border_color(colors.border_focus))
        .on_click(action);

    match tone {
        ControlTone::Neutral => base
            .hover(move |style| style.bg(colors.hover))
            .active(move |style| style.bg(colors.selection)),
        // Un aplat de tonalité s'estompe, il ne se remplace pas. Repeindre le
        // fond avec `hover` ferait virer au gris neutre le bouton « Exécuter en
        // PRODUCTION » à l'instant précis où le curseur l'atteint — le signal
        // de danger disparaîtrait juste avant le clic.
        _ => base
            .hover(|style| style.opacity(HOVER_OPACITY))
            .active(|style| style.opacity(PRESSED_OPACITY)),
    }
}

/// Estompement d'un contrôle désactivé.
///
/// Assez marqué pour se lire comme « indisponible » sans que le libellé cesse
/// d'être déchiffrable : un contrôle désactivé qu'on ne peut plus lire ne dit
/// plus ce qui deviendra disponible.
const DISABLED_OPACITY: f32 = 0.55;

/// Estompement d'un aplat de tonalité sous le curseur.
const HOVER_OPACITY: f32 = 0.90;

/// Estompement d'un aplat de tonalité pendant la pression.
///
/// Plus marqué que le survol : les deux états doivent se distinguer l'un de
/// l'autre, pas seulement du repos.
const PRESSED_OPACITY: f32 = 0.78;

/// L'anneau de focus clavier.
///
/// Une ombre sans flou ni décalage, étalée de
/// [`focus_ring`](crate::theme::Metrics::focus_ring) : c'est l'équivalent exact
/// du jeton `ring` de la maquette, et **rien ne bouge** quand il apparaît. Une
/// bordure qui s'épaissit au focus, elle, repousse le contenu.
#[must_use]
pub fn focus_ring(theme: &Theme) -> BoxShadow {
    BoxShadow {
        color: theme.colors.border_focus,
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(0.0),
        spread_radius: theme.metrics.focus_ring,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seul_letat_actif_accepte_une_entree() {
        assert!(ControlState::Enabled.accepts_input());
        assert!(!ControlState::Disabled.accepts_input());
        assert!(!ControlState::ReadOnly.accepts_input());
    }

    #[test]
    fn la_lecture_seule_reste_atteignable_au_clavier() {
        // C'est toute la différence avec « désactivé » : une valeur affichée et
        // non modifiable doit pouvoir être atteinte, lue et copiée au clavier.
        assert!(ControlState::ReadOnly.is_focusable());
        assert!(!ControlState::Disabled.is_focusable());
        assert!(ControlState::Enabled.is_focusable());
    }

    #[test]
    fn lanneau_ne_decale_rien() {
        // Ni décalage ni flou : l'anneau s'étale autour du contrôle sans
        // déplacer son contenu ni empiéter sur la lisibilité du libellé.
        let anneau = focus_ring(&Theme::dark());
        assert_eq!(anneau.offset, point(px(0.0), px(0.0)));
        assert_eq!(anneau.blur_radius, px(0.0));
        assert_eq!(anneau.spread_radius, Theme::dark().metrics.focus_ring);
    }

    #[test]
    fn lanneau_porte_la_couleur_de_focus_des_deux_variantes() {
        for theme in [Theme::dark(), Theme::light()] {
            assert_eq!(focus_ring(&theme).color, theme.colors.border_focus);
        }
    }
}
