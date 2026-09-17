//! La boîte d'approbation du Policy gate.
//!
//! # Ce que cet écran protège
//!
//! [ADR-0004](../../../docs/adr/0004-command-bus.md) donne au `PolicyGate` trois
//! réponses possibles ; celle-ci est la mise en scène de
//! [`Decision::RequireApproval`]. Sans elle, un agent qui produit un `UPDATE`
//! sans `WHERE` écrit dans la base de l'utilisateur — le risque classé
//! **critique** d'[ARCHITECTURE §12](../../../docs/ARCHITECTURE.md).
//!
//! # Les quatre règles de cet écran
//!
//! 1. **L'acteur est nommé.** « Un agent demande » et « vous demandez » ne se
//!    lisent pas de la même façon, et c'est précisément la distinction qui
//!    justifie l'écran.
//! 2. **L'instruction est montrée exacte**, telle qu'elle a été écrite, à chasse
//!    fixe, sans reformulation. Une confirmation qui résume ne protège de rien.
//!    Les valeurs liées **n'y sont pas** : [`Preview::statement`] ne les porte
//!    pas ([I-03](../../../CLAUDE.md#i-03)).
//! 3. **La connexion est nommée**, par son nom d'utilisateur et jamais par son
//!    identifiant ([I-02](../../../CLAUDE.md#i-02),
//!    [I-03](../../../CLAUDE.md#i-03)). C'est [`Preview::connection`], qui porte
//!    déjà cette contrainte dans `oxyn-core`.
//! 4. **Le bouton par défaut n'est jamais l'action destructrice**
//!    ([UX-SPEC](../../../docs/UX-SPEC.md#les-opérations-destructrices)). Ici :
//!    `Échap` refuse, `Entrée` **ne fait rien**, et le focus initial est sur le
//!    refus. Approuver demande un clic ou `Cmd+Entrée` — un geste qu'on ne fait
//!    pas par réflexe.
//!
//! # Ce que cet écran ne fait pas
//!
//! Il ne décide rien. La décision est déjà prise par le `PolicyGate` ; cet écran
//! en présente une, [`Decision::RequireApproval`], et rend la réponse de
//! l'utilisateur. Un refus (`Deny`) ne passe jamais par ici : aucune
//! confirmation ne le débloque, et lui donner un écran à deux boutons
//! suggérerait le contraire.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, Hsla, KeyDownEvent,
    ScrollHandle, SharedString, Window, div, point, px,
};
use oxyn_core::{Actor, Decision, Environment, Preview};

use crate::controls::{ControlState, ControlTone, control};
use crate::theme::Theme;

/// Ce que l'utilisateur répond.
///
/// Énumération **fermée**, contrairement à l'usage de cette base de code : une
/// troisième réponse — « toujours autoriser », « autoriser pendant une heure » —
/// serait une décision de politique, donc un ADR, et pas un ajout de variante
/// glissé dans une vue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// L'exécution est autorisée, cette fois.
    ///
    /// **Cette fois seulement** : il n'y a pas de « ne plus me demander ». Une
    /// approbation permanente rendrait le journal d'audit inutile — on ne
    /// saurait plus à quel moment l'utilisateur a réellement regardé.
    Approved,
    /// L'exécution est refusée.
    Rejected,
}

/// Ce que l'écran d'approbation émet.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApprovalEvent {
    /// L'utilisateur a tranché.
    Decided {
        /// Le jeton de la demande, pour que `oxyn-exec` retrouve la commande en
        /// attente. Deux demandes peuvent être ouvertes ; répondre « oui » sans
        /// dire à quoi serait une faille.
        request: ApprovalId,
        /// La réponse.
        outcome: ApprovalOutcome,
    },
}

/// Identifiant d'une demande d'approbation.
///
/// Opaque : la vue ne le construit pas et n'en déduit rien ; elle le reçoit avec
/// la demande et le rend avec la réponse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApprovalId(u64);

impl ApprovalId {
    /// Construit un identifiant à partir d'un compteur tenu par `oxyn-exec`.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// La valeur portée.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Une demande d'approbation, telle que la vue la reçoit.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ApprovalRequest {
    /// Le jeton à renvoyer avec la réponse.
    pub id: ApprovalId,
    /// Qui demande.
    pub actor: Actor,
    /// Pourquoi le `PolicyGate` s'arrête, rédigé pour être lu.
    pub reason: SharedString,
    /// L'instruction, la connexion, l'estimation. Absente quand la commande
    /// n'est pas une exécution — un export, par exemple.
    pub preview: Option<Preview>,
    /// L'environnement de la connexion visée.
    pub environment: Environment,
}

impl ApprovalRequest {
    /// Construit une demande à partir d'une décision du `PolicyGate`.
    ///
    /// Rend `None` pour [`Decision::Allow`] et [`Decision::Deny`] : la première
    /// n'a rien à montrer, la seconde ne se débloque par aucun écran, et les
    /// deux passeraient ici par erreur de câblage plutôt que par intention.
    #[must_use]
    pub fn from_decision(
        id: ApprovalId,
        actor: Actor,
        environment: Environment,
        decision: &Decision,
    ) -> Option<Self> {
        match decision {
            Decision::RequireApproval { reason, preview } => Some(Self {
                id,
                actor,
                reason: SharedString::from(reason.clone()),
                preview: preview.clone(),
                environment,
            }),
            Decision::Allow | Decision::Deny { .. } => None,
        }
    }

    /// La demande vient-elle d'un agent ?
    #[must_use]
    pub fn is_from_agent(&self) -> bool {
        self.actor.is_agent()
    }
}

/// Comment l'acteur est présenté.
///
/// Le libellé d'un agent ne porte **pas** son identifiant de session : il
/// n'apprendrait rien à l'utilisateur et allongerait une phrase qui doit se lire
/// d'un coup.
#[must_use]
pub fn actor_label(actor: &Actor) -> &'static str {
    match actor {
        Actor::Human => "You are asking",
        Actor::Agent { .. } => "An agent is asking",
    }
}

/// L'écran d'approbation.
#[derive(Debug)]
pub struct ApprovalDialog {
    focus: FocusHandle,
    cancel_focus: FocusHandle,
    approve_focus: FocusHandle,
    request: Option<ApprovalRequest>,
    preview_scroll: ScrollHandle,
}

impl EventEmitter<ApprovalEvent> for ApprovalDialog {}

impl Focusable for ApprovalDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ApprovalDialog {
    /// Un écran sans demande en cours.
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            approve_focus: cx.focus_handle(),
            request: None,
            preview_scroll: ScrollHandle::new(),
        }
    }

    /// La demande affichée.
    #[must_use]
    pub fn request(&self) -> Option<&ApprovalRequest> {
        self.request.as_ref()
    }

    /// Une demande est-elle en attente de réponse ?
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.request.is_some()
    }

    /// Présente une demande et prend le focus.
    pub fn present(
        &mut self,
        request: ApprovalRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.request = Some(request);
        self.preview_scroll.set_offset(point(px(0.), px(0.)));
        // Cancel is the safe default; a continuing keystroke cannot approve.
        window.focus(&self.cancel_focus);
        cx.notify();
    }

    /// Répond à la demande en cours.
    ///
    /// Sans effet s'il n'y en a pas : un double clic ne doit pas produire deux
    /// réponses pour une seule demande.
    pub fn decide(&mut self, outcome: ApprovalOutcome, cx: &mut Context<'_, Self>) {
        let Some(demande) = self.request.take() else {
            return;
        };
        cx.emit(ApprovalEvent::Decided {
            request: demande.id,
            outcome,
        });
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let touche = event.keystroke.key.as_str();
        let commande = event.keystroke.modifiers.secondary();
        match touche {
            "tab" => {
                let next = if self.cancel_focus.is_focused(window) {
                    &self.approve_focus
                } else {
                    &self.cancel_focus
                };
                window.focus(next);
            }
            "space" if self.cancel_focus.is_focused(window) => {
                self.decide(ApprovalOutcome::Rejected, cx)
            }
            "space" if self.approve_focus.is_focused(window) => {
                self.decide(ApprovalOutcome::Approved, cx)
            }
            // Échap refuse : la sortie par réflexe est la sortie sûre.
            "escape" => self.decide(ApprovalOutcome::Rejected, cx),
            // Approuver demande un raccourci composé. `Entrée` seul ne fait
            // rien, exprès : c'est la touche qu'on presse sans lire.
            "enter" if commande => self.decide(ApprovalOutcome::Approved, cx),
            "up" | "down" | "pageup" | "pagedown" | "home" | "end" => {
                let mut offset = self.preview_scroll.offset();
                offset.y = match touche {
                    "up" => offset.y + px(24.),
                    "down" => offset.y - px(24.),
                    "pageup" => offset.y + px(200.),
                    "pagedown" => offset.y - px(200.),
                    "home" => px(0.),
                    _ => -self.preview_scroll.max_offset().height,
                };
                self.preview_scroll.set_offset(offset);
                cx.notify();
            }
            _ => {}
        }
        cx.stop_propagation();
    }
}

impl Render for ApprovalDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let Some(demande) = self.request.clone() else {
            // Rien à approuver : l'écran n'occupe pas de place. Un conteneur
            // vide mais présent intercepterait les clics de la vue en dessous.
            return div().into_any_element();
        };

        let production = demande.environment.is_production();
        let couleur_environnement = theme.colors.environment(demande.environment);

        div()
            .key_context("ApprovalDialog")
            .track_focus(&self.focus)
            .id("oxyn-approval")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.colors.scrim)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.ui_size)
            .capture_key_down(cx.listener(Self::on_key))
            .child(
                div()
                    .w(px(560.0))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    // Le rayon de surface de la maquette : c'est un panneau,
                    // pas un contrôle.
                    .rounded(theme.radii.surface)
                    .border_1()
                    .border_color(if production {
                        theme.colors.danger
                    } else {
                        theme.colors.border
                    })
                    .bg(theme.colors.surface)
                    .text_color(theme.colors.text)
                    .child(self.render_header(&demande, couleur_environnement, &theme))
                    .child(
                        div()
                            .text_color(theme.colors.text_muted)
                            .child(demande.reason.clone()),
                    )
                    .when_some(demande.preview.clone(), |element, apercu| {
                        element.child(self.render_preview(&apercu, &theme))
                    })
                    .child(self.render_actions(production, &theme, cx)),
            )
            .into_any_element()
    }
}

impl ApprovalDialog {
    /// Le titre : qui demande, et sur quel environnement.
    fn render_header(
        &self,
        demande: &ApprovalRequest,
        couleur_environnement: Hsla,
        theme: &Theme,
    ) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_color(theme.colors.text)
                    .child(actor_label(&demande.actor)),
            )
            .child(
                div()
                    .flex_none()
                    .px_1p5()
                    // Une pastille : le rayon `full` de la maquette.
                    .rounded(theme.radii.full)
                    .border_1()
                    .border_color(couleur_environnement)
                    .text_color(couleur_environnement)
                    .text_size(theme.typography.small_size)
                    .child(SharedString::from(
                        demande.environment.as_str().to_uppercase(),
                    )),
            )
            .into_any_element()
    }

    /// L'instruction exacte, la connexion nommée, l'estimation quand elle
    /// existe.
    fn render_preview(&self, apercu: &Preview, theme: &Theme) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .child("Connexion")
                    // Le NOM, jamais l'identifiant : `Preview::connection` porte
                    // déjà cette garantie côté `oxyn-core`.
                    .child(
                        div()
                            .text_color(theme.colors.text)
                            .child(SharedString::from(apercu.connection.clone())),
                    ),
            )
            .child(
                div()
                    .id("approval-statement")
                    .max_h(px(220.0))
                    .overflow_scroll()
                    .track_scroll(&self.preview_scroll)
                    .p_2()
                    .rounded(theme.radii.surface)
                    .border_1()
                    .border_color(theme.colors.border)
                    .bg(theme.colors.background)
                    .font_family(theme.typography.mono_family.clone())
                    .text_size(theme.typography.mono_size)
                    .text_color(theme.colors.text)
                    // Texte exact, jamais reformulé : c'est ce sur quoi
                    // l'utilisateur se prononce.
                    .child(SharedString::from(apercu.statement.clone())),
            )
            .child(match apercu.estimated_rows {
                Some(lignes) => div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.warning)
                    .child(SharedString::from(format!(
                        "about {lignes} affected row{}",
                        if lignes == 1 { "" } else { "s" }
                    ))),
                // Pas d'estimation : on le dit. Un chiffre inventé serait pire
                // que pas de chiffre (oxyn-core, `Preview::estimated_rows`).
                None => div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_faint)
                    .child("Number of affected rows is unknown."),
            })
            .into_any_element()
    }

    /// Les deux actions. Le refus est à gauche, en premier dans l'ordre de
    /// tabulation, et c'est lui que porte `Échap`.
    fn render_actions(
        &self,
        production: bool,
        theme: &Theme,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .gap(theme.spacing.small)
            .justify_end()
            .child(
                control(
                    "oxyn-approval-reject",
                    ControlState::Enabled,
                    ControlTone::Neutral,
                    theme,
                    cx.listener(|ecran, _event: &ClickEvent, _window, cx| {
                        ecran.decide(ApprovalOutcome::Rejected, cx);
                    }),
                )
                .px(theme.spacing.medium)
                .py(theme.spacing.tiny)
                .track_focus(&self.cancel_focus)
                .child("Cancel"),
            )
            .child(
                control(
                    "oxyn-approval-approve",
                    ControlState::Enabled,
                    // Sur une connexion de production, l'action porte la
                    // tonalité de danger : elle ne doit pas ressembler au
                    // bouton de validation d'un formulaire ordinaire.
                    if production {
                        ControlTone::Danger
                    } else {
                        ControlTone::Primary
                    },
                    theme,
                    cx.listener(|ecran, _event: &ClickEvent, _window, cx| {
                        ecran.decide(ApprovalOutcome::Approved, cx);
                    }),
                )
                // Second dans l'ordre de tabulation : le refus vient d'abord,
                // et c'est lui que porte `Échap`.
                .track_focus(&self.approve_focus)
                .tab_index(1)
                .px(theme.spacing.medium)
                .py(theme.spacing.tiny)
                .child(if production {
                    self.request
                        .as_ref()
                        .and_then(|request| request.preview.as_ref())
                        .map(|preview| format!("Execute on {}", preview.connection))
                        .unwrap_or_else(|| "Execute in PRODUCTION".into())
                } else {
                    "Execute".into()
                }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::{AgentId, AgentSessionId};

    use super::*;

    fn apercu() -> Preview {
        Preview::new("DELETE FROM commandes", "Base client")
    }

    #[gpui::test]
    fn production_focus_stays_inside_review_and_enter_never_approves(
        cx: &mut gpui::TestAppContext,
    ) {
        let (dialog, cx) = cx.add_window_view(|window, cx| {
            let mut dialog = ApprovalDialog::new(cx);
            let request = ApprovalRequest::from_decision(
                ApprovalId::new(1),
                Actor::Human,
                Environment::Production,
                &Decision::approval("Production write", Some(apercu())),
            )
            .expect("production requires review");
            dialog.present(request, window, cx);
            dialog
        });
        cx.run_until_parked();
        cx.update(|window, cx| assert!(dialog.read(cx).cancel_focus.is_focused(window)));
        cx.simulate_keystrokes("enter");
        assert!(dialog.read_with(cx, |dialog, _| dialog.is_open()));
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| assert!(dialog.read(cx).approve_focus.is_focused(window)));
        cx.simulate_keystrokes("enter");
        assert!(dialog.read_with(cx, |dialog, _| dialog.is_open()));
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| assert!(dialog.read(cx).cancel_focus.is_focused(window)));
        cx.simulate_keystrokes("escape");
        assert!(!dialog.read_with(cx, |dialog, _| dialog.is_open()));
    }

    #[gpui::test]
    fn long_sql_is_scrollable_without_approving_on_enter(cx: &mut gpui::TestAppContext) {
        let sql = "DELETE FROM audit;\n".repeat(100);
        let (dialog, cx) = cx.add_window_view(|window, cx| {
            let mut dialog = ApprovalDialog::new(cx);
            let request = ApprovalRequest::from_decision(
                ApprovalId::new(42),
                Actor::Human,
                Environment::Production,
                &Decision::approval(
                    "Production write",
                    Some(Preview::new(&sql, "QA production")),
                ),
            )
            .expect("write requires a preview");
            dialog.present(request, window, cx);
            dialog
        });
        cx.run_until_parked();
        assert!(
            dialog.read_with(cx, |dialog, _| dialog.preview_scroll.max_offset().height
                > px(0.))
        );
        cx.simulate_keystrokes("pagedown");
        cx.run_until_parked();
        assert!(dialog.read_with(cx, |dialog, _| dialog.preview_scroll.offset().y < px(0.)));
        cx.simulate_keystrokes("enter");
        assert!(dialog.read_with(cx, |dialog, _| dialog.is_open()));
        cx.simulate_keystrokes("escape");
        assert!(!dialog.read_with(cx, |dialog, _| dialog.is_open()));
    }

    #[test]
    fn une_decision_dautorisation_nouvre_aucun_ecran() {
        // Un écran d'approbation sur une commande autorisée entraînerait
        // l'utilisateur à cliquer sans lire.
        assert!(
            ApprovalRequest::from_decision(
                ApprovalId::new(1),
                Actor::Human,
                Environment::Local,
                &Decision::Allow,
            )
            .is_none()
        );
    }

    #[test]
    fn un_refus_nouvre_aucun_ecran() {
        // ADR-0004 : aucune confirmation ne débloque un `Deny`. Lui donner deux
        // boutons suggérerait le contraire.
        assert!(
            ApprovalRequest::from_decision(
                ApprovalId::new(1),
                Actor::agent(AgentId::new(), AgentSessionId::new()),
                Environment::Production,
                &Decision::deny("les agents ne peuvent pas écrire en production"),
            )
            .is_none()
        );
    }

    #[test]
    fn une_demande_dapprobation_conserve_lacteur_et_lapercu() {
        let decision = Decision::approval("écriture non bornée", Some(apercu()));
        let demande = ApprovalRequest::from_decision(
            ApprovalId::new(7),
            Actor::agent(AgentId::new(), AgentSessionId::new()),
            Environment::Staging,
            &decision,
        )
        .expect("une approbation ouvre un écran");

        assert_eq!(demande.id, ApprovalId::new(7));
        assert!(demande.is_from_agent());
        assert_eq!(demande.reason, "écriture non bornée");
        let preview = demande.preview.expect("l'aperçu est transmis tel quel");
        assert_eq!(preview.statement, "DELETE FROM commandes");
        assert_eq!(preview.connection, "Base client");
    }

    #[test]
    fn lacteur_est_nomme_et_les_deux_libelles_different() {
        // « Un agent demande » et « vous demandez » ne se lisent pas de la même
        // façon ; c'est toute la raison d'être de l'écran.
        let humain = actor_label(&Actor::Human);
        let agent = actor_label(&Actor::agent(AgentId::new(), AgentSessionId::new()));
        assert_ne!(humain, agent);
        assert!(agent.contains("agent"));
    }

    #[test]
    fn le_libelle_dun_agent_ne_porte_aucun_identifiant() {
        // I-03 : ni identifiant de session, ni identifiant d'agent à l'écran.
        let agent = AgentId::new();
        let session = AgentSessionId::new();
        let libelle = actor_label(&Actor::agent(agent, session));
        assert!(!libelle.contains(&agent.to_string()));
        assert!(!libelle.contains(&session.to_string()));
    }

    #[test]
    fn une_approbation_sans_estimation_reste_presentable() {
        // `Preview::estimated_rows` vaut `None` tant qu'aucun driver ne sait
        // estimer ; l'écran doit s'ouvrir quand même.
        let decision = Decision::approval("DDL", Some(apercu()));
        let demande = ApprovalRequest::from_decision(
            ApprovalId::new(2),
            Actor::Human,
            Environment::Development,
            &decision,
        )
        .expect("une approbation ouvre un écran");
        assert_eq!(
            demande
                .preview
                .as_ref()
                .and_then(|apercu| apercu.estimated_rows),
            None
        );
    }

    #[test]
    fn une_approbation_sans_apercu_est_acceptee() {
        // Un export ou un rafraîchissement de catalogue n'a pas d'instruction à
        // montrer ; l'écran ne doit pas exiger un aperçu qui n'existe pas.
        let decision = Decision::approval("export vers un fichier", None);
        let demande = ApprovalRequest::from_decision(
            ApprovalId::new(3),
            Actor::Human,
            Environment::Local,
            &decision,
        )
        .expect("une approbation ouvre un écran");
        assert!(demande.preview.is_none());
    }

    #[test]
    fn lidentifiant_de_demande_fait_laller_retour() {
        // Répondre « oui » sans dire à quoi serait une faille : deux demandes
        // peuvent être ouvertes.
        let identifiant = ApprovalId::new(u64::MAX);
        assert_eq!(identifiant.get(), u64::MAX);
        assert_ne!(identifiant, ApprovalId::new(0));
    }
}
