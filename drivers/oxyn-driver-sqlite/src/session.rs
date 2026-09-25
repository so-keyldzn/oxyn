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
use oxyn_core::{
    CancelToken, Capabilities, ExecRequest, OxynError, Result, StatementHandle, TransactionState,
};
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

    /// Reads `sqlite3_get_autocommit` on the carrier thread.
    ///
    /// Submitted as a task, not read from a stored value: the thread runs its
    /// tasks in submission order, so this one runs only once the previous one
    /// has returned from `sqlite3_step` — including the automatic rollback an
    /// interruption triggers, which "the only way to find out" about is this
    /// call (ADR-0039).
    async fn transaction_state(&self, cancel: &CancelToken) -> TransactionState {
        let autocommit = self
            .worker
            .call(cancel, |connection: &Connection| {
                Ok(connection.is_autocommit())
            })
            .await;
        match autocommit {
            Ok(true) => TransactionState::Idle,
            Ok(false) => TransactionState::Open,
            Err(_) => TransactionState::Unknown,
        }
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

    /// A session opened through the driver, as the executor gets it.
    async fn ouvrir(chemin: &str) -> Box<dyn Session> {
        use oxyn_core::{ConnectionConfig, DriverId, Environment};
        use oxyn_driver::{Credentials, Driver};

        let connexion = ConnectionConfig::new("transactions", DriverId::sqlite())
            .with_environment(Environment::Local)
            .with_param(crate::SqliteDriver::PATH, chemin);
        crate::SqliteDriver::new()
            .connect(&connexion, &Credentials::new(), &CancelToken::new())
            .await
            .unwrap_or_else(|err| panic!("ouverture : {err}"))
    }

    /// Runs a statement to its end, cursor released.
    async fn executer(session: &dyn Session, sql: &str) {
        let demande = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), sql)
            .with_limits(oxyn_core::ExecLimits::unbounded());
        let mut curseur = match session.execute(demande, &CancelToken::new()).await {
            Ok(curseur) => curseur,
            Err(err) => panic!("exécution de `{sql}` : {err}"),
        };
        while curseur.next_batch().await.expect("lot suivant").is_some() {}
    }

    async fn etat(session: &dyn Session) -> TransactionState {
        session.transaction_state(&CancelToken::new()).await
    }

    #[tokio::test]
    async fn l_etat_suit_les_transactions_ecrites_et_celles_du_trait() {
        // ADR-0039 §2, the contract of a session that declares TRANSACTIONS.
        let session = ouvrir(crate::SqliteDriver::MEMORY).await;
        assert!(session.capabilities().contains(Capabilities::TRANSACTIONS));
        assert_eq!(
            etat(session.as_ref()).await,
            TransactionState::Idle,
            "a fresh connection is in autocommit, and says so"
        );
        executer(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;

        for fin in ["COMMIT", "ROLLBACK", "END"] {
            executer(session.as_ref(), "BEGIN").await;
            assert_eq!(etat(session.as_ref()).await, TransactionState::Open);
            executer(session.as_ref(), "INSERT INTO t(v) VALUES (1)").await;
            assert_eq!(etat(session.as_ref()).await, TransactionState::Open);
            executer(session.as_ref(), fin).await;
            assert_eq!(
                etat(session.as_ref()).await,
                TransactionState::Idle,
                "{fin}"
            );
        }

        let jeton = CancelToken::new();
        session.begin(&jeton).await.expect("begin");
        assert_eq!(etat(session.as_ref()).await, TransactionState::Open);
        session.commit(&jeton).await.expect("commit");
        assert_eq!(etat(session.as_ref()).await, TransactionState::Idle);

        session.begin(&jeton).await.expect("begin");
        assert_eq!(etat(session.as_ref()).await, TransactionState::Open);
        session.rollback(&jeton).await.expect("rollback");
        assert_eq!(etat(session.as_ref()).await, TransactionState::Idle);

        session.close().await.expect("fermeture");
    }

    #[tokio::test]
    async fn un_jeton_deja_declenche_rend_inconnu_jamais_idle() {
        let session = ouvrir(crate::SqliteDriver::MEMORY).await;
        let jeton = CancelToken::new();
        jeton.cancel();
        assert_eq!(
            session.transaction_state(&jeton).await,
            TransactionState::Unknown
        );
        session.close().await.expect("fermeture");
    }

    #[tokio::test]
    async fn l_annulation_d_office_apres_un_stop_est_constatee() {
        // The scenario: `BEGIN`, a write, then Stop on an endless `INSERT`.
        // SQLite rolls the whole transaction back on `SQLITE_INTERRUPT`, while
        // `execute` returns `Cancelled` as soon as the token fires — before
        // the carrier thread is even out of `sqlite3_step`. A state read then
        // must come after that step, not before.
        let dossier = tempfile::tempdir().expect("dossier temporaire");
        let base = dossier.path().join("transactions.sqlite");
        let journal = dossier.path().join("transactions.sqlite-journal");
        let session = ouvrir(base.to_str().expect("chemin UTF-8")).await;

        executer(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;
        executer(session.as_ref(), "BEGIN").await;
        assert!(
            !journal.exists(),
            "a deferred BEGIN writes nothing: the journal marks the first write"
        );

        let jeton = CancelToken::new();
        let demande = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Sqlite),
            "INSERT INTO t(v) SELECT x FROM \
             (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c)",
        )
        .with_limits(oxyn_core::ExecLimits::unbounded());
        let mut ecriture = Box::pin(session.execute(demande, &jeton));
        assert!(futures::poll!(ecriture.as_mut()).is_pending());
        // The rollback journal appears on the first page written: the INSERT
        // is then inside `sqlite3_step`, past the point where an interrupt
        // could be lost. A condition, not a delay.
        tokio::time::timeout(Duration::from_secs(10), async {
            while !journal.exists() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the endless INSERT starts writing");

        jeton.cancel();
        match ecriture.await {
            Ok(_) => panic!("an endless INSERT only ends interrupted"),
            Err(err) => assert!(err.is_cancelled(), "{err:?}"),
        }

        assert_eq!(
            etat(session.as_ref()).await,
            TransactionState::Idle,
            "the interrupted write rolled the transaction back"
        );
        session.close().await.expect("fermeture");
    }
}
