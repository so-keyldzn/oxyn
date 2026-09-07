//! La barre d'état.
//!
//! Trois choses y vivent, et la première est un invariant, pas une décoration :
//!
//! 1. **Le badge d'environnement**, rouge et **permanent** sur une connexion de
//!    production ([I-02](../../../CLAUDE.md#i-02)). Il n'apparaît pas au moment
//!    d'écrire : à ce moment-là, l'utilisateur a déjà tapé sa requête. Une
//!    connexion sans environnement renseigné vaut `Production`, c'est le défaut
//!    d'[`Environment`], et la barre n'a rien à en décider.
//! 2. **Le coût de la dernière exécution** — lignes et durée —, lu sur
//!    [`ExecStats`], jamais recalculé.
//! 3. **L'état d'annulation**, qui distingue « annulation demandée » de
//!    « annulée » : entre les deux, la requête tourne encore côté serveur, et
//!    dire « annulée » trop tôt est un mensonge que l'utilisateur découvre en
//!    voyant sa connexion occupée.
//!
//! # Ce qui n'y apparaît jamais
//!
//! Un identifiant de connexion, une valeur liée
//! ([I-03](../../../CLAUDE.md#i-03)). La barre affiche le **nom** de la
//! connexion — celui que l'utilisateur a donné — et rien d'autre de la
//! configuration.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, EventEmitter, Hsla, SharedString, Window, div, px};
use oxyn_core::{Capabilities, Environment, ExecStats};

use crate::session_capabilities::cancel_caveat;
use crate::theme::Theme;

/// Où en est la dernière commande soumise.
///
/// Les cinq états d'[UX-SPEC](../../../docs/UX-SPEC.md#états-dune-vue), vus
/// depuis la barre.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub enum ExecutionStatus {
    /// Rien n'a encore été exécuté.
    #[default]
    Idle,
    /// Une exécution est en cours.
    Running {
        /// Lignes déjà reçues, pour que l'attente montre un progrès.
        rows: u64,
    },
    /// L'annulation a été demandée, la confirmation du serveur n'est pas
    /// arrivée.
    ///
    /// **Distinct de [`Cancelled`](Self::Cancelled)** : tant que le serveur n'a
    /// pas confirmé, la requête tourne et la connexion est prise
    /// ([UX-SPEC](../../../docs/UX-SPEC.md#annulation)).
    Cancelling,
    /// L'exécution a été annulée, et le serveur l'a confirmé.
    Cancelled,
    /// L'exécution est terminée.
    Completed(ExecStats),
    /// L'exécution a échoué.
    Failed {
        /// Le message du serveur, code compris.
        message: SharedString,
        /// L'erreur est-elle retentable ? Vient de
        /// [`ErrorClass`](oxyn_core::ErrorClass), pas d'une analyse du message
        /// ([I-13](../../../CLAUDE.md#i-13)).
        retryable: bool,
    },
}

impl ExecutionStatus {
    /// Une exécution est-elle en cours, annulation comprise ?
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        matches!(self, Self::Running { .. } | Self::Cancelling)
    }

    /// L'annulation est-elle proposable ?
    ///
    /// Seulement pendant [`Running`](Self::Running) : reproposer l'annulation
    /// pendant [`Cancelling`](Self::Cancelling) ferait envoyer une seconde
    /// annulation au serveur, sans effet et sans retour.
    #[must_use]
    pub const fn is_cancellable(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

/// La connexion active, telle que la barre en parle.
///
/// Ne porte **ni** [`ConnectionId`](oxyn_core::ConnectionId) **ni** paramètre de
/// connexion : la barre ne doit pas pouvoir afficher ce qu'elle n'a pas le droit
/// d'afficher, et le plus sûr est qu'elle ne l'ait pas.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActiveConnection {
    /// Le nom donné par l'utilisateur.
    pub name: SharedString,
    /// Le driver, par protocole.
    pub driver: SharedString,
    /// L'environnement marqué sur la connexion.
    pub environment: Environment,
    /// La connexion est-elle en lecture seule ?
    pub read_only: bool,
}

impl ActiveConnection {
    /// Une connexion active.
    #[must_use]
    pub fn new(
        name: impl Into<SharedString>,
        driver: impl Into<SharedString>,
        environment: Environment,
    ) -> Self {
        Self {
            name: name.into(),
            driver: driver.into(),
            environment,
            read_only: false,
        }
    }

    /// Marque la connexion en lecture seule.
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }
}

/// Ce que la barre d'état demande.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusBarEvent {
    /// L'utilisateur demande l'annulation de l'exécution en cours.
    CancelRequested,
}

/// La barre d'état.
#[derive(Debug)]
pub struct StatusBar {
    connection: Option<ActiveConnection>,
    status: ExecutionStatus,
    /// Message d'une seule ligne, effacé au prochain changement d'état.
    notice: Option<SharedString>,
    /// Ce que la session déclare. Décide de ce que le bouton « Annuler » a le
    /// droit de promettre ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
    capabilities: Capabilities,
}

impl EventEmitter<StatusBarEvent> for StatusBar {}

impl Default for StatusBar {
    fn default() -> Self {
        Self {
            connection: None,
            status: ExecutionStatus::default(),
            notice: None,
            // `Capabilities` n'a pas de `Default` : ne rien déclarer est le seul
            // défaut sûr, et l'absence de dérivation le rend explicite.
            capabilities: Capabilities::empty(),
        }
    }
}

impl StatusBar {
    /// Une barre sans connexion.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// La connexion active.
    #[must_use]
    pub fn connection(&self) -> Option<&ActiveConnection> {
        self.connection.as_ref()
    }

    /// L'état d'exécution.
    #[must_use]
    pub fn status(&self) -> &ExecutionStatus {
        &self.status
    }

    /// Ce que la session déclare.
    #[must_use]
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Déclare ce que la session sait faire.
    pub fn set_capabilities(&mut self, capabilities: Capabilities, cx: &mut Context<'_, Self>) {
        if self.capabilities != capabilities {
            self.capabilities = capabilities;
            cx.notify();
        }
    }

    /// Change la connexion active.
    pub fn set_connection(
        &mut self,
        connection: Option<ActiveConnection>,
        cx: &mut Context<'_, Self>,
    ) {
        self.connection = connection;
        cx.notify();
    }

    /// Change l'état d'exécution.
    pub fn set_status(&mut self, status: ExecutionStatus, cx: &mut Context<'_, Self>) {
        self.status = status;
        self.notice = None;
        cx.notify();
    }

    /// Affiche un message d'une ligne.
    pub fn set_notice(
        &mut self,
        notice: Option<impl Into<SharedString>>,
        cx: &mut Context<'_, Self>,
    ) {
        self.notice = notice.map(Into::into);
        cx.notify();
    }
}

/// Le libellé du badge d'environnement.
///
/// En capitales pour la production : c'est le seul environnement dont
/// l'utilisateur doit être averti sans le chercher.
#[must_use]
pub fn environment_label(environment: Environment) -> &'static str {
    match environment {
        Environment::Local => "local",
        Environment::Development => "dev",
        Environment::Staging => "staging",
        Environment::Production => "PRODUCTION",
    }
}

/// Une durée écrite pour être lue d'un coup d'œil.
///
/// Sous la milliseconde, la précision n'apprend rien ; au-delà de la minute,
/// les millisecondes non plus.
#[must_use]
pub fn format_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1 {
        return "<1 ms".to_owned();
    }
    if millis < 1_000 {
        return format!("{millis} ms");
    }
    let secondes = duration.as_secs_f64();
    if secondes < 60.0 {
        return format!("{secondes:.2} s");
    }
    let minutes = duration.as_secs() / 60;
    let reste = duration.as_secs() % 60;
    format!("{minutes} min {reste} s")
}

/// Le résumé chiffré d'une exécution terminée.
///
/// Le temps serveur n'apparaît que si le driver l'a donné : l'inventer à partir
/// du temps client ferait passer la latence réseau pour du temps de calcul.
#[must_use]
pub fn format_stats(stats: &ExecStats) -> String {
    let mut sortie = format!(
        "{} ligne{} en {}",
        stats.rows,
        if stats.rows == 1 { "" } else { "s" },
        format_duration(stats.total_time)
    );
    if let Some(serveur) = stats.server_time {
        sortie.push_str(&format!(" (serveur {})", format_duration(serveur)));
    }
    if stats.truncated {
        sortie.push_str(" — tronqué");
    }
    sortie
}

impl Render for StatusBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        div()
            .id("oxyn-status-bar")
            .w_full()
            .flex_none()
            .h(theme.metrics.header_height)
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_3()
            .bg(theme.colors.surface_raised)
            .border_t_1()
            .border_color(theme.colors.border)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.small_size)
            .text_color(theme.colors.text_muted)
            .child(self.render_connection(cx))
            .child(div().flex_1())
            .child(self.render_status(cx))
            .when_some(self.notice.clone(), |element, message| {
                element.child(div().text_color(theme.colors.text_faint).child(message))
            })
    }
}

impl StatusBar {
    fn render_connection(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let Some(connexion) = &self.connection else {
            return div()
                .text_color(theme.colors.text_faint)
                .child("Aucune connexion")
                .into_any_element();
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_color(theme.colors.text)
                    .child(connexion.name.clone()),
            )
            .child(
                div()
                    .text_color(theme.colors.text_faint)
                    .child(connexion.driver.clone()),
            )
            // Le badge est là quel que soit l'état : c'est ce que demande I-02.
            .child(badge(
                environment_label(connexion.environment),
                theme.colors.environment(connexion.environment),
                theme,
            ))
            .when(connexion.read_only, |element| {
                element.child(badge("lecture seule", theme.colors.text_faint, theme))
            })
            .into_any_element()
    }

    fn render_status(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let ligne = match &self.status {
            ExecutionStatus::Idle => div()
                .text_color(theme.colors.text_faint)
                .child("Prêt")
                .into_any_element(),
            ExecutionStatus::Running { rows } => div()
                .text_color(theme.colors.text)
                .child(SharedString::from(format!("Exécution… {rows} lignes")))
                .into_any_element(),
            ExecutionStatus::Cancelling => div()
                .text_color(theme.colors.warning)
                // Le serveur n'a pas confirmé : ne pas écrire « annulée ».
                .child("Annulation demandée…")
                .into_any_element(),
            ExecutionStatus::Cancelled => div()
                .text_color(theme.colors.text_muted)
                .child("Annulée")
                .into_any_element(),
            ExecutionStatus::Completed(stats) => div()
                .text_color(theme.colors.text)
                .child(SharedString::from(format_stats(stats)))
                .into_any_element(),
            ExecutionStatus::Failed { message, retryable } => div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .text_color(theme.colors.danger)
                .child(message.clone())
                .child(badge(
                    if *retryable {
                        "retentable"
                    } else {
                        "non retentable"
                    },
                    theme.colors.text_faint,
                    theme,
                ))
                .into_any_element(),
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(ligne)
            .when(self.status.is_cancellable(), |element| {
                element.child(
                    div()
                        .id("oxyn-status-cancel")
                        // Atteignable au clavier : GPUI n'offre pas
                        // l'accessibilité gratuitement, et la rattraper après
                        // coup coûte une réécriture (ADR-0001).
                        .tab_index(0)
                        .px_2()
                        .rounded_sm()
                        .border_1()
                        .border_color(theme.colors.border)
                        .text_color(theme.colors.text)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.colors.hover))
                        .on_click(cx.listener(|_barre, _event: &ClickEvent, _window, cx| {
                            cx.emit(StatusBarEvent::CancelRequested);
                        }))
                        .child("Annuler"),
                )
            })
            // La réserve accompagne le bouton, elle ne le remplace pas : couper
            // le flux reste utile, prétendre couper la requête ne l'est pas.
            .when_some(
                cancel_caveat(self.capabilities).filter(|_| self.status.is_cancellable()),
                |element, reserve| element.child(badge(reserve, theme.colors.warning, theme)),
            )
            .into_any_element()
    }
}

/// Une pastille de texte colorée.
fn badge(texte: &'static str, couleur: Hsla, theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .px_1p5()
        .rounded_sm()
        .border_1()
        .border_color(couleur)
        .text_color(couleur)
        .text_size(px(f32::from(theme.typography.small_size) - 1.0))
        .child(texte)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_production_porte_un_libelle_qui_se_voit() {
        // I-02 : le marquage n'est pas discret. Les trois autres le sont.
        assert_eq!(environment_label(Environment::Production), "PRODUCTION");
        for environnement in [
            Environment::Local,
            Environment::Development,
            Environment::Staging,
        ] {
            let libelle = environment_label(environnement);
            assert_eq!(libelle, libelle.to_lowercase());
        }
    }

    #[test]
    fn une_connexion_sans_environnement_est_de_production() {
        // Le défaut d'`Environment` est le plus contraignant ; la barre en
        // hérite sans avoir à le redécider.
        let connexion = ActiveConnection::new("client", "postgres", Environment::default());
        assert!(connexion.environment.is_production());
    }

    #[test]
    fn annulation_demandee_et_annulee_ne_se_confondent_pas() {
        // Entre les deux, la requête tourne encore côté serveur.
        assert!(ExecutionStatus::Cancelling.is_busy());
        assert!(!ExecutionStatus::Cancelled.is_busy());
        assert!(!ExecutionStatus::Cancelling.is_cancellable());
        assert!(ExecutionStatus::Running { rows: 0 }.is_cancellable());
    }

    #[test]
    fn un_etat_termine_nest_ni_occupe_ni_annulable() {
        let termine = ExecutionStatus::Completed(ExecStats::default());
        assert!(!termine.is_busy());
        assert!(!termine.is_cancellable());
        let echec = ExecutionStatus::Failed {
            message: SharedString::new_static("57014: requête annulée"),
            retryable: false,
        };
        assert!(!echec.is_busy());
        assert!(!echec.is_cancellable());
    }

    #[test]
    fn les_durees_changent_dunite_aux_bons_seuils() {
        assert_eq!(format_duration(Duration::from_micros(400)), "<1 ms");
        assert_eq!(format_duration(Duration::from_millis(37)), "37 ms");
        assert_eq!(format_duration(Duration::from_millis(1_500)), "1.50 s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1 min 30 s");
    }

    #[test]
    fn le_resume_accorde_le_pluriel_et_marque_la_troncature() {
        let mut stats = ExecStats {
            rows: 1,
            total_time: Duration::from_millis(12),
            ..ExecStats::default()
        };
        assert_eq!(format_stats(&stats), "1 ligne en 12 ms");

        stats.rows = 4;
        stats.truncated = true;
        assert_eq!(format_stats(&stats), "4 lignes en 12 ms — tronqué");
    }

    #[test]
    fn le_temps_serveur_napparait_que_si_le_driver_la_donne() {
        // ADR-0003 : rien n'est simulé. Sans mesure serveur, pas de mention.
        let stats = ExecStats {
            rows: 2,
            total_time: Duration::from_millis(30),
            ..ExecStats::default()
        };
        assert!(!format_stats(&stats).contains("serveur"));

        let stats = ExecStats {
            server_time: Some(Duration::from_millis(8)),
            ..stats
        };
        assert_eq!(format_stats(&stats), "2 lignes en 30 ms (serveur 8 ms)");
    }
}
