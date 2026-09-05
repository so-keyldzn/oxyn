//! Le registre des exécutions en cours, et l'annulation qui va **jusqu'au
//! serveur**.
//!
//! `Échap` doit annuler vraiment. Abandonner le futur côté client ne libère ni
//! la connexion, ni le verrou posé, ni le plan en cours d'exécution : au dixième
//! onglet fermé, la base refuse les connexions et l'utilisateur conclut qu'Oxyn
//! a cassé sa production ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md),
//! [I-13](../../../CLAUDE.md#i-13)).
//!
//! L'annulation se fait donc **dans cet ordre**, et l'ordre est porté par
//! [`CancelRegistry::cancel`] plutôt que par ses appelants :
//!
//! 1. le [`CancelToken`] de l'exécution est déclenché — la boucle de drainage
//!    rend la main au prochain point de contrôle, sans attendre le réseau ;
//! 2. si — et seulement si — la session déclare
//!    [`Capabilities::SERVER_SIDE_CANCEL`], l'annulation est demandée au serveur
//!    (`pg_cancel_backend`, `KILL QUERY`, `sqlite3_interrupt`).
//!
//! Le deuxième temps est conditionnel parce qu'une session qui ne le déclare pas
//! rendrait [`OxynError::NotSupported`](oxyn_core::OxynError) : appeler quand
//! même produirait une erreur dans le journal à chaque `Échap` sur SQLite, et
//! une erreur systématique cesse d'être lue.
//!
//! # Ce que le registre ne fait pas
//!
//! Il ne retire pas l'entrée à l'annulation. C'est la boucle de drainage qui
//! appelle [`finish`](CancelRegistry::finish) quand elle a réellement rendu la
//! main : un `Échap` appuyé deux fois doit être sans effet, pas une erreur
//! « exécution inconnue » alors qu'elle tourne encore.

use std::collections::HashMap;
use std::time::Instant;

use oxyn_core::{CancelToken, Capabilities, CommandId, ConnectionId, SessionId, StatementHandle};
use parking_lot::RwLock;

use crate::sessions::SessionRegistry;

/// Une exécution en cours, telle que le registre la connaît.
///
/// Ne porte **ni** le texte de la requête **ni** ses paramètres : ce registre
/// est consulté sur le chemin de l'annulation, pas sur celui de l'audit, et
/// dupliquer le texte ici en ferait un second endroit d'où il peut fuir (I-03).
#[derive(Debug, Clone)]
pub struct RunningStatement {
    /// La poignée frappée par le driver, cible de l'annulation serveur.
    pub statement: StatementHandle,
    /// La commande qui l'a lancée, pour corréler avec le journal.
    pub command: CommandId,
    /// La connexion visée.
    pub connection: ConnectionId,
    /// La session sur laquelle elle tourne.
    pub session: SessionId,
    /// Ce que la session sait faire — c'est ici que se lit
    /// [`Capabilities::SERVER_SIDE_CANCEL`].
    pub capabilities: Capabilities,
    /// Quand l'exécution a été soumise.
    pub started_at: Instant,
    /// Le jeton propre à cette exécution. Fils du jeton de l'appelant :
    /// l'annuler n'annule pas l'onglet.
    token: CancelToken,
}

impl RunningStatement {
    /// Enregistre une exécution qui démarre.
    #[must_use]
    pub fn new(
        statement: StatementHandle,
        command: CommandId,
        connection: ConnectionId,
        session: SessionId,
        capabilities: Capabilities,
        token: CancelToken,
    ) -> Self {
        Self {
            statement,
            command,
            connection,
            session,
            capabilities,
            started_at: Instant::now(),
            token,
        }
    }

    /// Le jeton de cette exécution.
    #[must_use]
    pub fn token(&self) -> &CancelToken {
        &self.token
    }

    /// La session sait-elle annuler côté serveur ?
    #[must_use]
    pub const fn supports_server_cancel(&self) -> bool {
        self.capabilities.contains(Capabilities::SERVER_SIDE_CANCEL)
    }

    /// L'exécution a-t-elle déjà été annulée ?
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

/// Ce qui a été tenté côté serveur.
///
/// Distingue « pas essayé », « pas possible » et « essayé sans succès » : les
/// trois se ressemblent à l'écran et n'appellent pas la même conclusion. Une
/// requête que le serveur n'a pas su interrompre tourne encore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServerCancel {
    /// Rien n'a été tenté : l'exécution n'était plus en cours.
    NotAttempted,
    /// La session ne déclare pas [`Capabilities::SERVER_SIDE_CANCEL`].
    ///
    /// Ce n'est pas une panne : c'est une limite du driver, honnêtement
    /// déclarée. La requête peut continuer côté serveur jusqu'à son terme.
    Unsupported,
    /// Le serveur a accepté la demande d'interruption.
    Requested,
    /// Le serveur a refusé ou n'a pas répondu.
    Failed,
}

impl ServerCancel {
    /// L'interruption a-t-elle réellement été demandée au serveur ?
    #[must_use]
    pub const fn reached_server(&self) -> bool {
        matches!(self, Self::Requested)
    }
}

/// Ce qu'une annulation a effectivement fait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CancelReport {
    /// L'exécution visée.
    pub statement: StatementHandle,
    /// Était-elle encore en cours au moment de la demande ?
    pub was_running: bool,
    /// Le jeton client a-t-il été déclenché ?
    pub client: bool,
    /// Ce qui a été tenté côté serveur.
    pub server: ServerCancel,
}

/// Les exécutions en cours, indexées par leur poignée.
///
/// Le verrou est un `RwLock` : la lecture est fréquente — chaque annulation,
/// chaque fermeture d'onglet — et l'écriture ne se produit qu'au début et à la
/// fin d'une exécution.
#[derive(Debug, Default)]
pub struct CancelRegistry {
    running: RwLock<HashMap<StatementHandle, RunningStatement>>,
}

impl CancelRegistry {
    /// Registre vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre une exécution qui démarre.
    pub fn register(&self, entry: RunningStatement) {
        self.running.write().insert(entry.statement, entry);
    }

    /// Retire une exécution terminée et rend ce que le registre en savait.
    ///
    /// À appeler quand la boucle de drainage a **réellement** rendu la main,
    /// pas quand l'annulation est demandée.
    pub fn finish(&self, statement: StatementHandle) -> Option<RunningStatement> {
        self.running.write().remove(&statement)
    }

    /// Ce que le registre sait d'une exécution.
    #[must_use]
    pub fn get(&self, statement: StatementHandle) -> Option<RunningStatement> {
        self.running.read().get(&statement).cloned()
    }

    /// Les exécutions en cours sur une connexion.
    #[must_use]
    pub fn for_connection(&self, connection: ConnectionId) -> Vec<RunningStatement> {
        self.running
            .read()
            .values()
            .filter(|e| e.connection == connection)
            .cloned()
            .collect()
    }

    /// Les exécutions en cours sur une session.
    #[must_use]
    pub fn for_session(&self, session: SessionId) -> Vec<RunningStatement> {
        self.running
            .read()
            .values()
            .filter(|e| e.session == session)
            .cloned()
            .collect()
    }

    /// Nombre d'exécutions en cours.
    #[must_use]
    pub fn len(&self) -> usize {
        self.running.read().len()
    }

    /// Aucune exécution en cours ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.running.read().is_empty()
    }

    /// Déclenche le jeton d'une exécution, **sans** toucher au serveur.
    ///
    /// Utile quand l'appelant sait déjà que la session est perdue — coupure,
    /// fermeture — et qu'une demande d'interruption n'aurait nulle part où
    /// aller. Dans tous les autres cas, [`cancel`](Self::cancel) est ce qu'il
    /// faut appeler : ne signaler que le client laisse la requête tourner.
    pub fn cancel_client(&self, statement: StatementHandle) -> Option<RunningStatement> {
        let entry = self.running.read().get(&statement).cloned()?;
        entry.token.cancel();
        Some(entry)
    }

    /// Annule une exécution : jeton client d'abord, serveur ensuite.
    ///
    /// L'ordre compte. Le jeton rend la main au prochain point de contrôle,
    /// donc immédiatement du point de vue de l'utilisateur ; la demande au
    /// serveur, elle, est un aller-retour réseau qui peut durer. Les inverser
    /// ferait attendre l'interface pour rien.
    ///
    /// Annuler une exécution déjà terminée n'est **pas** une erreur : le
    /// rapport le dit avec `was_running: false`.
    pub async fn cancel(
        &self,
        sessions: &SessionRegistry,
        statement: StatementHandle,
    ) -> CancelReport {
        let Some(entry) = self.cancel_client(statement) else {
            return CancelReport {
                statement,
                was_running: false,
                client: false,
                server: ServerCancel::NotAttempted,
            };
        };

        let server = if entry.supports_server_cancel() {
            match sessions.get(entry.session) {
                Some(slot) => match slot.cancel_statement(statement).await {
                    Ok(()) => ServerCancel::Requested,
                    Err(erreur) => {
                        // Journalisé au niveau `warn` et non remonté : l'appelant
                        // vient de demander une annulation, lui rendre une erreur
                        // ne lui laisserait rien à faire de plus.
                        tracing::warn!(
                            error = %erreur,
                            "server-side cancellation refused; the statement may still run"
                        );
                        ServerCancel::Failed
                    }
                },
                None => ServerCancel::Failed,
            }
        } else {
            ServerCancel::Unsupported
        };

        CancelReport {
            statement,
            was_running: true,
            client: true,
            server,
        }
    }

    /// Annule toutes les exécutions d'une connexion.
    ///
    /// C'est ce que fait la fermeture d'une connexion : sans cela, les requêtes
    /// lancées depuis ses onglets continueraient de tourner côté serveur.
    pub async fn cancel_connection(
        &self,
        sessions: &SessionRegistry,
        connection: ConnectionId,
    ) -> Vec<CancelReport> {
        let mut rapports = Vec::new();
        for entry in self.for_connection(connection) {
            rapports.push(self.cancel(sessions, entry.statement).await);
        }
        rapports
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entree(capabilities: Capabilities) -> RunningStatement {
        RunningStatement::new(
            StatementHandle::new(),
            CommandId::new(),
            ConnectionId::new(),
            SessionId::new(),
            capabilities,
            CancelToken::new(),
        )
    }

    #[test]
    fn une_annulation_declenche_le_jeton_de_l_execution() {
        let registre = CancelRegistry::new();
        let entree = entree(Capabilities::empty());
        let jeton = entree.token().clone();
        let poignee = entree.statement;
        registre.register(entree);

        assert!(!jeton.is_cancelled());
        let annulee = registre
            .cancel_client(poignee)
            .expect("l'exécution est enregistrée");
        assert!(jeton.is_cancelled());
        assert!(annulee.is_cancelled());
    }

    #[test]
    fn l_entree_survit_a_l_annulation() {
        // `Échap` appuyé deux fois ne doit pas produire « exécution inconnue »
        // alors qu'elle tourne encore : c'est la boucle de drainage qui retire.
        let registre = CancelRegistry::new();
        let entree = entree(Capabilities::empty());
        let poignee = entree.statement;
        registre.register(entree);

        registre.cancel_client(poignee);
        assert_eq!(registre.len(), 1);
        assert!(registre.cancel_client(poignee).is_some());

        assert!(registre.finish(poignee).is_some());
        assert!(registre.is_empty());
        assert!(registre.finish(poignee).is_none());
    }

    #[test]
    fn annuler_une_execution_inconnue_n_est_pas_une_erreur() {
        let registre = CancelRegistry::new();
        assert!(registre.cancel_client(StatementHandle::new()).is_none());
    }

    #[test]
    fn la_capacite_d_annulation_serveur_se_lit_sur_la_session() {
        assert!(!entree(Capabilities::SQL).supports_server_cancel());
        assert!(
            entree(Capabilities::SQL | Capabilities::SERVER_SIDE_CANCEL).supports_server_cancel()
        );
    }

    #[test]
    fn le_registre_retrouve_les_executions_d_une_connexion() {
        let registre = CancelRegistry::new();
        let connexion = ConnectionId::new();

        for _ in 0..3 {
            let mut e = entree(Capabilities::empty());
            e.connection = connexion;
            registre.register(e);
        }
        registre.register(entree(Capabilities::empty()));

        assert_eq!(registre.for_connection(connexion).len(), 3);
        assert_eq!(registre.len(), 4);
    }

    #[test]
    fn un_rapport_distingue_les_trois_issues_serveur() {
        assert!(ServerCancel::Requested.reached_server());
        assert!(!ServerCancel::Unsupported.reached_server());
        assert!(!ServerCancel::Failed.reached_server());
        assert!(!ServerCancel::NotAttempted.reached_server());
    }
}
