//! La session : une connexion ouverte, et ce qu'elle sait faire.
//!
//! # L'annulation n'est pas côté serveur, parce qu'il n'y a pas de serveur
//!
//! SQLite tourne **dans le processus d'Oxyn**. `sqlite3_interrupt` arrête une
//! instruction en cours, mais c'est une annulation *locale* : aucune connexion
//! distante n'est libérée, aucun verrou serveur n'est relâché, parce qu'il n'y
//! en a pas.
//!
//! La session ne déclare donc **pas**
//! [`Capabilities::SERVER_SIDE_CANCEL`](oxyn_core::Capabilities::SERVER_SIDE_CANCEL),
//! et [`SqliteSession::cancel`] refuse. Ce n'est pas une lacune à combler : le
//! drapeau sert à l'interface pour dire « le bouton Annuler coupe vraiment la
//! requête sur le serveur », et l'afficher face à une base embarquée serait
//! affirmer quelque chose qui n'a pas de sens.
//!
//! **L'interruption existe quand même**, et elle est complète : elle passe par
//! le [`CancelToken`] remis à [`execute`](SqliteSession::execute), que le
//! curseur consulte entre deux lots et surveille pendant qu'il en attend un.
//! Voir [`crate::SqliteCursor`].
//!
//! # Une session fait une chose à la fois
//!
//! C'est la sémantique d'une connexion SQLite, pas une limitation du transport :
//! une introspection demandée pendant qu'un curseur diffuse attend que le lot en
//! cours soit produit. L'interruption, elle, ne fait pas la queue.
//!
//! # Ce que la session ne change pas dans son dos
//!
//! Ni `PRAGMA foreign_keys`, ni `PRAGMA busy_timeout`, ni `PRAGMA journal_mode`.
//! SQLite laisse l'application choisir, et un `PRAGMA` posé en silence changerait
//! le sens des requêtes suivantes de l'utilisateur. Conséquence assumée : les
//! clés étrangères ne sont **pas** vérifiées par défaut — c'est le comportement
//! de SQLite — et un fichier verrouillé rend immédiatement `SQLITE_BUSY`, classé
//! transitoire, que l'appelant peut retenter s'il le juge rejouable.

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use oxyn_catalog::provider::CatalogProvider;
use oxyn_core::{CancelToken, Capabilities, ExecRequest, OxynError, Result, StatementHandle};
use oxyn_driver::{Cursor, Session};
use rusqlite::Connection;
use tokio::sync::{mpsc, oneshot};

use crate::catalog::SqliteCatalog;
use crate::cursor::SqliteCursor;
use crate::error::{self, Effect};
use crate::options::BatchLimits;
use crate::stream::StreamJob;
use crate::worker::WorkerHandle;

/// Une connexion SQLite ouverte.
#[derive(Debug)]
pub struct SqliteSession {
    worker: WorkerHandle,
    /// La poignée du thread porteur, gardée pour qu'il ne soit jamais orphelin.
    ///
    /// Elle n'est pas attendue à la fermeture : le thread ferme la connexion
    /// **avant** de répondre, si bien qu'attendre le déroulement de sa pile
    /// bloquerait le runtime pour rien.
    thread: Option<JoinHandle<()>>,
    capabilities: Capabilities,
    catalog: SqliteCatalog,
    limits: BatchLimits,
}

impl SqliteSession {
    /// Assemble une session autour d'un thread porteur déjà ouvert.
    pub(crate) fn new(
        worker: WorkerHandle,
        thread: JoinHandle<()>,
        capabilities: Capabilities,
        limits: BatchLimits,
    ) -> Self {
        Self {
            catalog: SqliteCatalog::new(worker.clone(), capabilities),
            worker,
            thread: Some(thread),
            capabilities,
            limits,
        }
    }

    /// Les bornes d'un lot Arrow, pour cette session.
    #[must_use]
    pub const fn batch_limits(&self) -> BatchLimits {
        self.limits
    }

    /// Émet une instruction de transaction.
    async fn transaction(
        &self,
        cancel: &CancelToken,
        sql: &'static str,
        effect: Effect,
    ) -> Result<()> {
        self.capabilities.require(Capabilities::TRANSACTIONS)?;
        self.worker
            .call(cancel, move |connection: &Connection| {
                connection
                    .execute_batch(sql)
                    .map_err(|err| error::engine(err, effect))
            })
            .await
    }
}

#[async_trait]
impl Session for SqliteSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Exécute une demande et rend un curseur sur ses lots.
    ///
    /// Rend la main **dès que le schéma est connu** — c'est-à-dire dès que le
    /// premier lot est prêt, puisqu'en SQLite le type d'une colonne se lit dans
    /// les valeurs autant que dans la déclaration
    /// ([`convert`](crate::convert)). Ce premier lot voyage avec le curseur : il
    /// n'est pas relu.
    ///
    /// Le **dialecte** SQL n'est pas vérifié : il est « une nuance de grammaire,
    /// pas une capacité distincte » (ADR-0003). Une requête écrite pour
    /// PostgreSQL part donc telle quelle, et c'est l'analyseur de SQLite qui la
    /// rejette — avec son propre message, que l'utilisateur reconnaît.
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si le langage n'est pas SQL,
    /// [`OxynError::PolicyDenied`] si la demande se déclare en lecture seule et
    /// qu'une instruction écrit, [`OxynError::Cancelled`] si le jeton se
    /// déclenche, ou l'erreur du moteur.
    async fn execute(
        &self,
        mut request: ExecRequest,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        if self.capabilities.contains(Capabilities::READ_ONLY_SESSION) {
            request.limits.read_only = true;
        }

        let handle = StatementHandle::new();
        let started = Instant::now();
        let (pulls, orders) = mpsc::unbounded_channel();
        let (start, opened) = oneshot::channel();

        let work = self.worker.start_stream(Box::new(StreamJob {
            request,
            limits: self.limits,
            start,
            pulls: orders,
        }))?;

        // C'est pendant cette attente que SQLite calcule le premier lot — tout
        // un `count(*)`, par exemple. Abandonner ce futur interrompt la tâche :
        // voir `WorkerHandle::await_reply`.
        let begun = self.worker.await_reply(opened, cancel, work).await?;
        Ok(Box::new(SqliteCursor::new(
            handle,
            begun,
            pulls,
            self.worker.clone(),
            work,
            cancel.clone(),
            started,
        )))
    }

    /// Refuse : SQLite n'a pas de serveur à qui demander une annulation.
    ///
    /// Voir la note de module. L'annulation existe, elle passe par le
    /// [`CancelToken`] remis à [`execute`](Self::execute).
    ///
    /// # Erreurs
    /// Toujours [`OxynError::NotSupported`].
    async fn cancel(&self, _statement: StatementHandle) -> Result<()> {
        Err(OxynError::NotSupported {
            capability: "SERVER_SIDE_CANCEL".to_owned(),
        })
    }

    /// Compose l'aperçu d'une relation, et lit ce que sa forme exige.
    ///
    /// La description de la relation — ses colonnes et sa clé primaire — n'est
    /// lue que si un tri ou une page demande un ordre, ou qu'une projection
    /// nomme des colonnes à vérifier : un aperçu sans demande
    /// ne paie pas un `PRAGMA table_info` pour une clause qu'il ne compose pas.
    /// Cette lecture passe par le thread porteur et s'interrompt comme les
    /// autres.
    ///
    /// # Erreurs
    /// [`OxynError::Cancelled`] si le jeton se déclenche, celles de la
    /// composition — colonne de tri ou de projection inconnue, projection vide
    /// ou démesurée, page sans clé unique —, et celles
    /// du moteur pendant l'introspection.
    async fn preview_request(
        &self,
        path: &oxyn_catalog::CatalogPath,
        limit: u32,
        shape: &oxyn_core::PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        // A projection is checked against the same description: a name the
        // relation does not declare is refused here, not by the engine.
        let facts = if shape.needs_total_order() || shape.columns.is_some() {
            crate::preview::RelationFacts::of(&self.catalog.describe_relation(path, cancel).await?)
        } else {
            crate::preview::RelationFacts::default()
        };
        crate::preview::request(path, limit, shape, &facts)
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// Vérifie que la connexion est vivante.
    ///
    /// La durée mesurée est celle d'un aller-retour vers le thread porteur, pas
    /// celle d'un réseau : elle dit surtout si le thread est libre ou occupé par
    /// un flux en cours.
    ///
    /// # Erreurs
    /// L'erreur du moteur, ou une erreur de driver si le thread a disparu.
    async fn ping(&self) -> Result<Duration> {
        let started = Instant::now();
        // `ping` n'a pas de jeton au contrat : un jeton neuf, jamais annulé,
        // laisse l'appel se terminer.
        let cancel = CancelToken::new();
        self.worker
            .call(&cancel, |connection: &Connection| {
                connection
                    .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                    .map(|_| ())
                    .map_err(|err| error::engine(err, Effect::ReadOnly))
            })
            .await?;
        Ok(started.elapsed())
    }

    /// Ferme la connexion et libère le fichier.
    ///
    /// # Erreurs
    /// L'erreur du moteur à la fermeture. Une session déjà fermée n'en est pas
    /// une : les ressources locales sont libérées dans tous les cas.
    async fn close(self: Box<Self>) -> Result<()> {
        let Self { worker, thread, .. } = *self;
        let outcome = worker.close().await;
        // Le canal se ferme ici : le thread sort de sa boucle même si la
        // fermeture ci-dessus a échoué.
        drop(worker);
        drop(thread);
        outcome
    }

    async fn begin(&self, cancel: &CancelToken) -> Result<()> {
        // `BEGIN` seul ne modifie rien : interrompu, il n'a rien laissé derrière.
        self.transaction(cancel, "BEGIN", Effect::ReadOnly).await
    }

    async fn commit(&self, cancel: &CancelToken) -> Result<()> {
        // Un `COMMIT` interrompu laisse l'effet inconnu : ambigu, donc jamais
        // rejoué (I-13).
        self.transaction(cancel, "COMMIT", Effect::Mutating).await
    }

    async fn rollback(&self, cancel: &CancelToken) -> Result<()> {
        self.transaction(cancel, "ROLLBACK", Effect::Mutating).await
    }
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use oxyn_core::{ConnectionId, QueryLanguage, SqlDialect};

    use super::*;
    use crate::worker::{self, OpenSpec, OpenTarget};

    /// Un `count(*)` sur une suite sans fin : il ne rend jamais la main tout seul.
    const SANS_FIN: &str =
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";

    #[tokio::test]
    async fn abandonner_execute_pendant_le_premier_lot_libere_le_moteur() {
        // Le scénario : l'onglet se ferme pendant que SQLite calcule le premier
        // lot d'une agrégation. Le curseur n'existe pas encore, donc personne
        // d'autre ne peut interrompre. Sans interruption, la session reste
        // occupée pour toute la durée du calcul — ici, pour toujours.
        let jeton = CancelToken::new();
        let spec = OpenSpec {
            target: OpenTarget::Memory(ConnectionId::new()),
            read_only: false,
        };
        let (handle, thread) = worker::spawn(spec, &jeton).await.expect("ouverture");
        let session = SqliteSession::new(handle, thread, Capabilities::SQL, BatchLimits::new());

        {
            let demande = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), SANS_FIN);
            let mut execution = pin!(session.execute(demande, &jeton));
            assert!(futures::poll!(execution.as_mut()).is_pending());
            // Le thread porteur s'est saisi de l'exécution : on n'abandonne pas
            // une tâche encore en file, ce qui prouverait autre chose.
            tokio::time::timeout(Duration::from_secs(10), async {
                while session.worker.running().is_none() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("le thread porteur prend l'exécution");
        }

        // La preuve côté moteur : il répond à un `SELECT 1`.
        let reponse = tokio::time::timeout(Duration::from_secs(10), session.ping()).await;
        let aller_retour = reponse
            .expect("le moteur doit être libéré par l'abandon")
            .expect("ping");
        assert!(
            aller_retour < Duration::from_secs(1),
            "le moteur doit répondre vite : {aller_retour:?}"
        );

        Box::new(session).close().await.expect("fermeture");
    }
}
