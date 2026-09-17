//! L'annulation côté serveur, sans jamais frapper la requête d'à côté.
//!
//! # La fenêtre que ce module ferme
//!
//! `pg_cancel_backend(pid)` vise un **processus serveur**, pas une requête.
//! Envoyer l'annulation depuis [`Session::cancel`](oxyn_driver::Session::cancel)
//! — lire le pid, ouvrir une seconde connexion, puis appeler la fonction —
//! laisse une fenêtre de la durée d'une poignée de main : si la requête se
//! termine entre-temps, la tâche de flux rend sa connexion au bassin, la
//! requête suivante l'emprunte, et c'est elle que l'annulation interrompt. Il
//! suffit d'appuyer sur Échap au moment où une requête finit.
//!
//! Relire le pid juste avant l'envoi n'y change rien : la requête suivante
//! tourne sur **le même** processus, donc sous le même pid.
//!
//! # La règle
//!
//! **Seule la tâche de flux envoie `pg_cancel_backend`, et elle le fait en
//! tenant sa connexion**, déjà marquée à fermer. `Session::cancel` ne fait que
//! déclencher le jeton de l'exécution, puis attend le [`Verdict`] de la tâche.
//! Deux issues seulement :
//!
//! * la tâche avait déjà constaté la fin du flux — le jeton arrive trop tard,
//!   rien n'est envoyé, et la connexion repart au bassin sans annulation en vol ;
//! * la tâche voit le jeton — elle envoie l'annulation **avant** de lâcher la
//!   connexion, qui est ensuite fermée et ne sert plus jamais personne.
//!
//! Aucune requête ne peut donc démarrer sur ce processus pendant qu'une
//! annulation le vise. C'est la connexion tenue qui le garantit, pas un délai.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use oxyn_core::{CancelToken, DriverId, OxynError, Result, StatementHandle, StatementIntent};
use sqlx::{ConnectOptions as _, Connection as _};
use tokio::sync::watch;

use crate::error::{map_connect_error, map_exec_error};
use crate::options::ConnectSpec;

/// Demande au serveur d'interrompre la requête d'un autre processus.
pub(crate) const SQL_CANCEL_BACKEND: &str = "SELECT pg_catalog.pg_cancel_backend($1)";

/// Ce que la tâche de flux a fait de l'annulation qu'on lui a demandée.
#[derive(Debug, Clone)]
pub(crate) enum Verdict {
    /// Le flux s'est terminé sans qu'il faille arrêter le serveur.
    Finished,
    /// `pg_cancel_backend` est parti pendant que la connexion était tenue.
    Cancelled,
    /// La demande d'annulation n'a pas abouti ; la requête tourne peut-être.
    CancelFailed(Arc<OxynError>),
}

/// Par où la tâche de flux rend son [`Verdict`].
#[derive(Debug)]
pub(crate) struct VerdictSender(watch::Sender<Option<Verdict>>);

impl VerdictSender {
    /// Publie le verdict. Ne peut pas échouer : un `Session::cancel` qui
    /// arriverait après trouvera l'exécution absente du registre.
    pub(crate) fn settle(&self, verdict: Verdict) {
        self.0.send_replace(Some(verdict));
    }
}

/// Une exécution en cours, telle que `Session::cancel` la voit.
#[derive(Debug, Clone)]
struct RunningExecution {
    /// Le jeton **propre** à l'exécution : celui que surveille la tâche de flux.
    token: CancelToken,
    verdict: watch::Receiver<Option<Verdict>>,
}

/// Associe chaque exécution au jeton qui l'arrête.
///
/// Partagé entre la session — qui annule — et les tâches de flux — qui
/// s'effacent en partant. Sans cet effacement, une session ouverte une journée
/// accumulerait une entrée par requête exécutée.
#[derive(Debug, Default)]
pub(crate) struct StatementRegistry {
    entries: Mutex<HashMap<StatementHandle, RunningExecution>>,
}

impl StatementRegistry {
    /// Retient une exécution qui démarre, et rend de quoi publier son verdict.
    pub(crate) fn register(&self, handle: StatementHandle, token: CancelToken) -> VerdictSender {
        let (sender, verdict) = watch::channel(None);
        self.lock()
            .insert(handle, RunningExecution { token, verdict });
        VerdictSender(sender)
    }

    /// Oublie une exécution terminée.
    pub(crate) fn forget(&self, handle: StatementHandle) {
        self.lock().remove(&handle);
    }

    /// Nombre d'exécutions en cours. Réservé au diagnostic et aux tests.
    #[doc(hidden)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Arrête une exécution côté serveur, par la tâche qui tient sa connexion.
    ///
    /// Rend quand la tâche a statué. Annuler une exécution inconnue ou déjà
    /// terminée n'est pas une erreur.
    ///
    /// # Erreurs
    /// Celle de la demande d'annulation, telle que la tâche l'a reçue du
    /// serveur, rattachée à sa famille d'origine.
    pub(crate) async fn cancel(&self, driver: &DriverId, handle: StatementHandle) -> Result<()> {
        let Some(RunningExecution { token, mut verdict }) = self.lock().get(&handle).cloned()
        else {
            return Ok(());
        };
        token.cancel();
        let issue = match verdict.wait_for(Option::is_some).await {
            Ok(rendu) => rendu.clone(),
            // La tâche a disparu sans statuer : il n'y a plus rien à couper.
            Err(_) => None,
        };
        match issue {
            Some(Verdict::CancelFailed(erreur)) => {
                Err(OxynError::driver(driver.clone(), erreur.class(), erreur))
            }
            Some(Verdict::Finished | Verdict::Cancelled) | None => Ok(()),
        }
    }

    /// Un verrou empoisonné ne doit pas propager la panique d'une autre tâche :
    /// la table reste exploitable, et perdre une entrée coûte moins qu'une
    /// session inutilisable ([I-09](../../../CLAUDE.md#i-09)).
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<StatementHandle, RunningExecution>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// De quoi demander au serveur d'arrêter une requête.
///
/// Porte les paramètres de connexion parce que l'annulation ouvre sa **propre**
/// connexion : emprunter celle du bassin ferait attendre l'annulation derrière
/// les requêtes qu'elle doit couper.
///
/// N'est appelé que par qui **tient** la connexion visée — la tâche de flux, ou
/// l'introspection — et l'a marquée à fermer : c'est ce qui garantit que le pid
/// ne sert aucune autre requête pendant l'envoi.
#[derive(Debug)]
pub(crate) struct BackendCanceller {
    spec: ConnectSpec,
    driver: DriverId,
    #[cfg(test)]
    gate: Mutex<Option<tests_gate::CancelGate>>,
}

impl BackendCanceller {
    /// Prépare l'annulateur d'une session.
    pub(crate) fn new(spec: ConnectSpec, driver: DriverId) -> Self {
        Self {
            spec,
            driver,
            #[cfg(test)]
            gate: Mutex::new(None),
        }
    }

    /// Demande au serveur d'interrompre la requête du processus `backend_pid`.
    ///
    /// Interrompre une requête déjà terminée n'est **pas** une erreur : le
    /// serveur rend `false` et on n'en fait rien. Ce qui compte est qu'aucune
    /// requête ne survive à la fermeture d'un onglet.
    ///
    /// # Erreurs
    /// [`OxynError::Connection`] si la seconde connexion ne s'ouvre pas,
    /// [`OxynError::Driver`] si le serveur refuse l'appel — typiquement faute de
    /// droits sur un pid appartenant à un autre rôle.
    pub(crate) async fn cancel_backend(&self, backend_pid: i32) -> Result<()> {
        #[cfg(test)]
        self.pass_gate(backend_pid).await;

        let mut connexion = self
            .spec
            .options()
            .connect()
            .await
            .map_err(|erreur| map_connect_error(&erreur))?;

        let issue = sqlx::query(SQL_CANCEL_BACKEND)
            .bind(backend_pid)
            .fetch_optional(&mut connexion)
            .await;

        // Fermée dans tous les cas : cette connexion n'a plus d'usage, et la
        // laisser filer en ouvrirait une par annulation.
        let _ = connexion.close().await;

        issue.map_err(|erreur| map_exec_error(&self.driver, StatementIntent::Read, erreur))?;
        Ok(())
    }
}

/// La barrière qui rend la fenêtre d'annulation reproductible dans un test.
#[cfg(test)]
pub(crate) mod tests_gate {
    use tokio::sync::oneshot;

    use super::BackendCanceller;

    /// Retient la prochaine annulation juste avant qu'elle ouvre sa connexion.
    #[derive(Debug)]
    pub(crate) struct CancelGate {
        reached: oneshot::Sender<i32>,
        proceed: oneshot::Receiver<()>,
    }

    impl BackendCanceller {
        /// Arme la barrière : la prochaine annulation signale le pid qu'elle
        /// vise, puis attend le feu vert.
        pub(crate) fn hold_next_cancel(&self) -> (oneshot::Receiver<i32>, oneshot::Sender<()>) {
            let (reached, atteinte) = oneshot::channel();
            let (feu_vert, proceed) = oneshot::channel();
            *self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(CancelGate { reached, proceed });
            (atteinte, feu_vert)
        }

        pub(super) async fn pass_gate(&self, backend_pid: i32) {
            let gate = self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(CancelGate { reached, proceed }) = gate {
                let _ = reached.send(backend_pid);
                let _ = proceed.await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_execution_terminee_ne_reste_pas_dans_le_registre() {
        // Sans cet effacement, une session ouverte une journée accumule une
        // entrée par requête exécutée.
        let registre = StatementRegistry::default();
        let handle = StatementHandle::new();
        let _verdict = registre.register(handle, CancelToken::new());
        assert_eq!(registre.len(), 1);
        registre.forget(handle);
        assert_eq!(registre.len(), 0);
    }

    #[tokio::test]
    async fn annuler_une_execution_inconnue_ne_doit_rien_couter() {
        // « Annuler une instruction déjà terminée n'est pas une erreur. »
        let registre = StatementRegistry::default();
        registre
            .cancel(&DriverId::postgres(), StatementHandle::new())
            .await
            .expect("rien à annuler");
    }

    #[tokio::test]
    async fn annuler_declenche_le_jeton_de_l_execution_et_attend_son_verdict() {
        // L'annulation ne part pas d'ici : c'est la tâche qui tient la connexion
        // qui l'envoie. `cancel` doit donc la réveiller, puis rendre ce qu'elle
        // a constaté — pas un succès supposé.
        let registre = Arc::new(StatementRegistry::default());
        let visee = StatementHandle::new();
        let voisine = StatementHandle::new();
        let jeton = CancelToken::new();
        let jeton_voisin = CancelToken::new();
        let verdict = registre.register(visee, jeton.clone());
        let _verdict_voisin = registre.register(voisine, jeton_voisin.clone());

        let tache = tokio::spawn({
            let jeton = jeton.clone();
            async move {
                jeton.cancelled().await;
                verdict.settle(Verdict::CancelFailed(Arc::new(OxynError::Connection(
                    "refused".to_owned(),
                ))));
            }
        });

        let issue = registre.cancel(&DriverId::postgres(), visee).await;
        let erreur = match issue {
            Ok(()) => panic!("l'échec de la tâche doit remonter"),
            Err(erreur) => erreur,
        };
        assert_eq!(erreur.class(), oxyn_core::ErrorClass::Transient);
        assert!(jeton.is_cancelled());
        assert!(
            !jeton_voisin.is_cancelled(),
            "seule l'exécution visée est annulée"
        );
        tache.await.expect("la tâche statue");
    }

    #[tokio::test]
    async fn une_tache_disparue_sans_verdict_ne_bloque_pas_l_annulation() {
        let registre = StatementRegistry::default();
        let handle = StatementHandle::new();
        let verdict = registre.register(handle, CancelToken::new());
        drop(verdict);
        registre
            .cancel(&DriverId::postgres(), handle)
            .await
            .expect("plus rien à couper");
    }
}
