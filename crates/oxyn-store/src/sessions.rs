//! Ce qui distingue un arrêt propre d'un plantage, constaté et non deviné.
//!
//! Un lancement s'inscrit à l'ouverture, bat pendant qu'il travaille, et note sa
//! fermeture quand elle est ordinaire. Au lancement suivant, une session laissée
//! sans fermeture **et** dont le battement a vieilli est un arrêt anormal ; la
//! même sans fermeture au battement récent est une autre instance, bien vivante
//! ([ADR-0021](../../../docs/adr/0021-marqueur-d-arret.md)).
//!
//! # Pourquoi deux conditions et pas une
//!
//! Le drapeau `documents.is_open` ne dit que « ce document n'a pas été fermé
//! explicitement » : il vaut vrai après un `⌘Q` ordinaire, et l'écran de reprise
//! s'affichait donc à chaque démarrage. Un écran montré tout le temps cesse
//! d'être lu, et c'est le jour où une écriture a été interrompue qu'il faut
//! qu'il le soit.
//!
//! Un simple drapeau « une session est ouverte » ne suffirait pas non plus :
//! deux instances d'Oxyn sur le même store se déclareraient mutuellement
//! anormales. C'est le battement qui les sépare.
//!
//! # Un plantage s'annonce une fois
//!
//! Le lancement qui constate une session abandonnée la marque `reported_at`.
//! Sans ce marquage, elle restait abandonnée pour toujours, et chaque lancement
//! suivant — fermetures propres comprises — rouvrait l'écran de reprise.
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne retient pas le pid. Le vérifier demanderait ce que la politique
//! `unsafe` du dépôt refuse, et un pid réutilisé ferait mentir le test. Le
//! battement dit la même chose sans mentir : il vieillit.
//!
//! Toutes les méthodes peuvent bloquer : elles ne s'appellent jamais depuis le
//! thread d'interface ([I-05](../../../CLAUDE.md#i-05)).

use chrono::{DateTime, Duration, Utc};
use oxyn_core::{AppSessionId, WorkspaceId};
use rusqlite::params;

use crate::{Result, Store};

/// Intervalle entre deux battements.
///
/// Choix de produit, pas mesure : assez espacé pour qu'une écriture périodique
/// reste négligeable sur une machine portable, assez court pour que le seuil
/// d'abandon ne fasse pas attendre l'utilisateur.
pub const HEARTBEAT_INTERVAL: Duration = Duration::seconds(30);

/// Au-delà, une session sans fermeture est réputée abandonnée.
///
/// Quatre battements manqués : une veille brève ou un système chargé ne suffit
/// pas à conclure au plantage.
pub const ABANDONED_AFTER: Duration = Duration::seconds(120);

/// Comment le lancement précédent s'est terminé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreviousShutdown {
    /// Aucune session antérieure : première ouverture de ce workspace.
    Never,
    /// La dernière session s'est fermée normalement.
    Clean,
    /// Une session est restée ouverte et son battement a vieilli.
    Abnormal,
}

impl PreviousShutdown {
    /// L'écran de reprise a-t-il quelque chose à annoncer ?
    #[must_use]
    pub const fn needs_recovery(self) -> bool {
        matches!(self, Self::Abnormal)
    }
}

/// Accès typé aux sessions d'application. Toutes les méthodes peuvent bloquer.
#[derive(Debug)]
pub struct Sessions<'a> {
    store: &'a Store,
}

impl<'a> Sessions<'a> {
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Constate comment le lancement précédent s'est terminé, puis inscrit
    /// celui-ci.
    ///
    /// L'ordre importe : le constat est fait **avant** que la nouvelle ligne
    /// existe, sinon elle se compterait elle-même comme une session ouverte.
    /// Les deux tiennent dans une transaction, pour que deux lancements
    /// simultanés ne lisent pas le même état à moitié écrit.
    ///
    /// # Erreurs
    /// Les erreurs de stockage. Le workspace doit exister.
    pub fn begin(&self, workspace: WorkspaceId) -> Result<(AppSessionId, PreviousShutdown)> {
        let now = Utc::now();
        let cutoff = now - ABANDONED_AFTER;
        let id = AppSessionId::new();
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let verdict = previous(&transaction, workspace, cutoff, now)?;
            transaction.execute(
                "INSERT INTO app_sessions (id, workspace_id, started_at, heartbeat_at, closed_at)
                 VALUES (?1, ?2, ?3, ?3, NULL)",
                params![id.to_string(), workspace.to_string(), now],
            )?;
            transaction.commit()?;
            Ok((id, verdict))
        })
    }

    /// Renouvelle le battement de cette session.
    ///
    /// Silencieux si la ligne a disparu — le workspace a pu être supprimé
    /// pendant l'exécution, et cesser de battre est alors la bonne réponse.
    ///
    /// # Erreurs
    /// Les erreurs de stockage.
    pub fn heartbeat(&self, session: AppSessionId) -> Result<()> {
        self.store.with_connection(|connection| {
            connection.execute(
                "UPDATE app_sessions SET heartbeat_at = ?2 WHERE id = ?1 AND closed_at IS NULL",
                params![session.to_string(), Utc::now()],
            )?;
            Ok(())
        })
    }

    /// Inscrit la fermeture ordinaire de cette session.
    ///
    /// À n'appeler qu'**après** avoir vidé les écritures locales en attente :
    /// inscrite avant, elle marquerait un arrêt propre sur du travail non écrit,
    /// c'est-à-dire précisément le cas où la reprise doit se déclencher.
    ///
    /// # Erreurs
    /// Les erreurs de stockage.
    pub fn close(&self, session: AppSessionId) -> Result<()> {
        self.store.with_connection(|connection| {
            connection.execute(
                "UPDATE app_sessions SET closed_at = ?2 WHERE id = ?1 AND closed_at IS NULL",
                params![session.to_string(), Utc::now()],
            )?;
            Ok(())
        })
    }

    /// Oublie les sessions closes plus anciennes que `keep`.
    ///
    /// Sans cet entretien, la table grandit d'une ligne par lancement pour
    /// toujours. Les sessions **non** closes ne sont jamais effacées : ce sont
    /// elles qui portent le constat.
    ///
    /// # Erreurs
    /// Les erreurs de stockage.
    pub fn forget_closed_before(&self, keep: DateTime<Utc>) -> Result<usize> {
        self.store.with_connection(|connection| {
            let effacees = connection.execute(
                "DELETE FROM app_sessions WHERE closed_at IS NOT NULL AND closed_at < ?1",
                params![keep],
            )?;
            Ok(effacees)
        })
    }
}

impl Store {
    /// Vieillit le battement d'une session, pour les tests des crates voisines.
    ///
    /// Le seuil d'abandon est de deux minutes : un test qui les attendrait ne
    /// serait plus un test. Réservé aux tests, et absent d'une compilation
    /// ordinaire.
    ///
    /// # Erreurs
    /// Les erreurs de stockage.
    #[cfg(any(test, feature = "test-support"))]
    pub fn mark_session_stale_for_tests(&self, session: AppSessionId) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE app_sessions SET heartbeat_at = ?2 WHERE id = ?1",
                params![session.to_string(), Utc::now() - ABANDONED_AFTER * 2],
            )?;
            Ok(())
        })
    }
}

/// Ce que les sessions déjà inscrites disent du lancement précédent.
///
/// Une session abandonnée n'est annoncée **qu'une fois** : le constat la marque
/// `reported_at`, dans la transaction de `begin`. Sans cela, un plantage
/// unique rendait anormaux tous les lancements suivants, et l'écran de reprise
/// ne distinguait plus rien.
fn previous(
    connection: &rusqlite::Connection,
    workspace: WorkspaceId,
    cutoff: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<PreviousShutdown> {
    let annoncees = connection.execute(
        "UPDATE app_sessions SET reported_at = ?3
         WHERE workspace_id = ?1 AND closed_at IS NULL AND reported_at IS NULL
           AND heartbeat_at < ?2",
        params![workspace.to_string(), cutoff, now],
    )?;
    if annoncees > 0 {
        return Ok(PreviousShutdown::Abnormal);
    }
    let connues: i64 = connection.query_row(
        "SELECT COUNT(*) FROM app_sessions WHERE workspace_id = ?1",
        params![workspace.to_string()],
        |row| row.get(0),
    )?;
    if connues > 0 {
        Ok(PreviousShutdown::Clean)
    } else {
        Ok(PreviousShutdown::Never)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atelier() -> (Store, WorkspaceId) {
        let store = Store::open_in_memory().expect("store en mémoire");
        let workspace = store.workspaces().create("atelier").expect("workspace").id;
        (store, workspace)
    }

    /// Vieillit le battement d'une session, pour simuler le temps qui passe sans
    /// attendre deux minutes dans un test.
    fn vieillir(store: &Store, session: AppSessionId, de: Duration) {
        store
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE app_sessions SET heartbeat_at = ?2 WHERE id = ?1",
                    params![session.to_string(), Utc::now() - de],
                )?;
                Ok(())
            })
            .expect("vieillissement");
    }

    #[test]
    fn une_premiere_ouverture_ne_signale_aucun_arret_anormal() {
        let (store, atelier) = atelier();
        let (_, verdict) = store.sessions().begin(atelier).expect("ouverture");
        assert_eq!(verdict, PreviousShutdown::Never);
        assert!(!verdict.needs_recovery());
    }

    /// Le défaut que ce module corrige : un `⌘Q` ordinaire ne doit pas ressembler
    /// à un plantage.
    #[test]
    fn une_fermeture_ordinaire_ne_declenche_pas_la_reprise() {
        let (store, atelier) = atelier();
        let (session, _) = store.sessions().begin(atelier).expect("ouverture");
        store.sessions().close(session).expect("fermeture");

        let (_, verdict) = store.sessions().begin(atelier).expect("relance");
        assert_eq!(verdict, PreviousShutdown::Clean);
        assert!(!verdict.needs_recovery());
    }

    #[test]
    fn une_session_laissee_ouverte_et_muette_est_un_arret_anormal() {
        let (store, atelier) = atelier();
        let (session, _) = store.sessions().begin(atelier).expect("ouverture");
        // Ni `close`, ni battement : le processus est mort sans rien dire.
        vieillir(&store, session, ABANDONED_AFTER + Duration::seconds(1));

        let (_, verdict) = store.sessions().begin(atelier).expect("relance");
        assert_eq!(verdict, PreviousShutdown::Abnormal);
        assert!(verdict.needs_recovery());
    }

    /// Le test qui empêche d'accuser une instance qui travaille.
    #[test]
    fn une_instance_qui_bat_encore_n_est_pas_un_plantage() {
        let (store, atelier) = atelier();
        let (vivante, _) = store.sessions().begin(atelier).expect("première instance");
        vieillir(&store, vivante, ABANDONED_AFTER + Duration::seconds(1));
        // Elle donne signe de vie juste avant que la seconde démarre.
        store.sessions().heartbeat(vivante).expect("battement");

        let (_, verdict) = store.sessions().begin(atelier).expect("seconde instance");
        assert_eq!(
            verdict,
            PreviousShutdown::Clean,
            "une session au battement récent est vivante, pas plantée"
        );
    }

    /// Le défaut observé : un plantage ancien rendait la reprise permanente.
    #[test]
    fn un_arret_anormal_n_est_annonce_qu_une_fois() {
        let (store, atelier) = atelier();
        let (plantee, _) = store.sessions().begin(atelier).expect("ouverture");
        vieillir(&store, plantee, ABANDONED_AFTER + Duration::seconds(1));

        let (relance, verdict) = store.sessions().begin(atelier).expect("relance");
        assert_eq!(verdict, PreviousShutdown::Abnormal);
        store
            .sessions()
            .close(relance)
            .expect("fermeture ordinaire");

        let (_, verdict) = store.sessions().begin(atelier).expect("seconde relance");
        assert_eq!(
            verdict,
            PreviousShutdown::Clean,
            "le plantage a déjà été annoncé, et la dernière session s'est fermée"
        );
    }

    #[test]
    fn une_session_annoncee_garde_sa_fermeture_absente() {
        let (store, atelier) = atelier();
        let (plantee, _) = store.sessions().begin(atelier).expect("ouverture");
        vieillir(&store, plantee, ABANDONED_AFTER + Duration::seconds(1));
        store.sessions().begin(atelier).expect("relance");

        let (fermee, annoncee): (Option<String>, Option<String>) = store
            .with_connection(|connection| {
                Ok(connection.query_row(
                    "SELECT closed_at, reported_at FROM app_sessions WHERE id = ?1",
                    params![plantee.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?)
            })
            .expect("lecture");
        assert!(
            fermee.is_none(),
            "un plantage ne devient pas un arrêt propre"
        );
        assert!(annoncee.is_some());
    }

    #[test]
    fn un_battement_ne_ressuscite_pas_une_session_fermee() {
        let (store, atelier) = atelier();
        let (session, _) = store.sessions().begin(atelier).expect("ouverture");
        store.sessions().close(session).expect("fermeture");
        store
            .sessions()
            .heartbeat(session)
            .expect("battement tardif");

        let (_, verdict) = store.sessions().begin(atelier).expect("relance");
        assert_eq!(verdict, PreviousShutdown::Clean);
    }

    #[test]
    fn l_entretien_efface_les_sessions_closes_sans_toucher_au_constat() {
        let (store, atelier) = atelier();
        let (close, _) = store.sessions().begin(atelier).expect("ouverture");
        store.sessions().close(close).expect("fermeture");
        let (abandonnee, _) = store.sessions().begin(atelier).expect("seconde");
        vieillir(&store, abandonnee, ABANDONED_AFTER + Duration::seconds(1));

        let efface = store
            .sessions()
            .forget_closed_before(Utc::now() + Duration::seconds(1))
            .expect("entretien");
        assert_eq!(efface, 1, "seule la session close est oubliée");

        let (_, verdict) = store.sessions().begin(atelier).expect("relance");
        assert_eq!(
            verdict,
            PreviousShutdown::Abnormal,
            "l'entretien n'efface pas ce qui porte le constat"
        );
    }
}
