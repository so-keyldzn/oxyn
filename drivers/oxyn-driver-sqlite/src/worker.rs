//! Le thread qui détient la connexion, et le canal qui y mène.
//!
//! # Pourquoi un thread dédié, et pas `spawn_blocking`
//!
//! `rusqlite::Connection` est `Send` mais **pas** `Sync`, et surtout : un
//! `Statement<'conn>` et les `Rows<'stmt>` qu'il produit **empruntent** la
//! connexion. Diffuser un résultat lot par lot demande de garder ces emprunts
//! vivants entre deux `await` — ce qu'un futur ne peut pas faire sans structure
//! auto-référentielle, et donc sans `unsafe`, que le workspace refuse
//! (`unsafe_code = "deny"`).
//!
//! Un `spawn_blocking` par lot aurait le même problème : il faudrait rendre la
//! connexion entre deux lots, donc refermer le curseur, donc **rejouer la
//! requête** à chaque page. C'est exactement ce qu'[I-06](../../../CLAUDE.md#i-06)
//! interdit.
//!
//! La connexion vit donc sur un thread à elle, du début à la fin de la session.
//! Les emprunts ne quittent jamais sa pile ; ce qui traverse le canal, ce sont
//! des `RecordBatch` Arrow, c'est-à-dire des données déjà converties.
//!
//! # Ce que le thread garantit, et ce qu'il ne garantit pas
//!
//! * **Une session SQLite fait une chose à la fois.** Une introspection ou un
//!   `ping` demandé pendant qu'un curseur diffuse attend son tour. C'est la
//!   sémantique d'une connexion SQLite, pas une limitation du transport.
//!   `Session::cancel` échappe à la file : il ne passe pas par le thread.
//! * **L'interruption, elle, ne fait pas la queue.** `InterruptHandle` est
//!   `Send + Sync` et vise le moteur directement : c'est ce qui permet à un
//!   `Échap` d'atteindre un `sqlite3_step` déjà parti
//!   ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md)). Elle vise une
//!   **tâche**, pas la connexion : voir le module `interrupt`.
//! * **Toute attente abandonnée interrompt sa tâche.** Détruire le futur de
//!   `execute`, d'une introspection ou d'un lot arrête le moteur, ou retire la
//!   tâche de la file si elle n'a pas commencé.
//! * Le thread s'arrête quand son canal se ferme, même si personne n'appelle
//!   [`Session::close`](oxyn_driver::Session::close) : une session oubliée ne
//!   laisse pas de thread derrière elle.

use std::path::PathBuf;
use std::pin::pin;
use std::sync::Arc;
use std::thread::JoinHandle;

use futures::future::{Either, select};
use oxyn_core::{CancelToken, OxynError, Result};
use rusqlite::{Connection, OpenFlags};
use tokio::sync::{mpsc, oneshot};

use crate::error;
use crate::interrupt::{AbandonGuard, Interrupter, WorkId};
use crate::stream::{self, StreamJob};

/// Une tâche courte à exécuter sur le thread porteur.
///
/// La tâche emporte son propre canal de réponse : c'est ce qui permet à un seul
/// type de commande de servir des réponses de types différents — l'introspection
/// rend des relations, le `ping` ne rend rien.
pub(crate) type Job = Box<dyn FnOnce(&Connection) + Send + 'static>;

/// Ce qui est demandé au thread porteur de la connexion.
pub(crate) enum WorkerCommand {
    /// Une opération courte : `ping`, transaction, introspection.
    Job(WorkId, Job),
    /// Une exécution en flux. Le thread garde la main jusqu'à ce que le curseur
    /// soit épuisé ou détruit.
    Stream(WorkId, Box<StreamJob>),
    /// Ferme la connexion et termine le thread.
    Close(oneshot::Sender<Result<()>>),
}

/// Ce qu'il faut ouvrir.
///
/// Le `Debug` est écrit à la main : un chemin de fichier est une **valeur de
/// paramètre de connexion**, qu'un driver n'a pas le droit de journaliser
/// ([I-03](../../../CLAUDE.md#i-03)). Un `Debug` dérivé est le mode de fuite le
/// plus fréquent, parce qu'il est invisible à la relecture.
#[derive(Clone)]
pub(crate) struct OpenSpec {
    /// La base visée.
    pub target: OpenTarget,
    /// Ouvrir en lecture seule, au niveau du moteur.
    pub read_only: bool,
}

impl std::fmt::Debug for OpenSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSpec")
            .field("target", &self.target)
            .field("read_only", &self.read_only)
            .finish()
    }
}

/// La base visée par une ouverture.
#[derive(Clone)]
pub(crate) enum OpenTarget {
    /// One named in-memory database per configured connection, shared by its sessions.
    Memory(oxyn_core::ConnectionId),
    /// Un fichier. Son chemin ne sort jamais dans un rendu de diagnostic.
    File(PathBuf),
}

impl std::fmt::Debug for OpenTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(_) => f.write_str("Memory"),
            Self::File(_) => f.write_str("File(<redacted path>)"),
        }
    }
}

/// La poignée partagée vers le thread porteur.
///
/// Clonée par la session, son catalogue et chacun de ses curseurs.
#[derive(Clone)]
pub(crate) struct WorkerHandle {
    commands: mpsc::UnboundedSender<WorkerCommand>,
    interrupter: Arc<Interrupter>,
}

impl std::fmt::Debug for WorkerHandle {
    /// `InterruptHandle` n'a pas de `Debug`, et un pointeur de connexion n'a
    /// rien à faire dans un journal de toute façon.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerHandle")
            .field("alive", &!self.commands.is_closed())
            .finish_non_exhaustive()
    }
}

impl WorkerHandle {
    /// Exécute une tâche courte sur la connexion et rend son résultat.
    ///
    /// L'annulation est traitée **ici** : si le jeton se déclenche pendant
    /// l'attente, ou si le futur est abandonné, la tâche est interrompue et
    /// l'appel rend [`OxynError::Cancelled`]. Abandonner le futur sans
    /// interrompre laisserait le thread bloqué dans `sqlite3_step`.
    ///
    /// # Erreurs
    /// Celle de la tâche, [`OxynError::Cancelled`] si le jeton se déclenche, ou
    /// une erreur de driver si le thread porteur a disparu.
    pub(crate) async fn call<T, F>(&self, cancel: &CancelToken, job: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let (reply, answer) = oneshot::channel();
        let wrapped: Job = Box::new(move |conn| {
            // L'échec d'envoi signifie que l'appelant a renoncé : rien à faire.
            let _ = reply.send(job(conn));
        });
        let id = self.submit(|id| WorkerCommand::Job(id, wrapped))?;
        self.await_reply(answer, cancel, id).await
    }

    /// Démarre une exécution en flux, et rend l'identité de sa tâche.
    ///
    /// L'identité sert à toutes les attentes de ce flux : le premier lot, puis
    /// chaque lot réclamé par le curseur.
    ///
    /// # Erreurs
    /// Une erreur de driver si le thread porteur a disparu.
    pub(crate) fn start_stream(&self, job: Box<StreamJob>) -> Result<WorkId> {
        self.submit(|id| WorkerCommand::Stream(id, job))
    }

    /// Enregistre une tâche, puis l'envoie au thread porteur.
    fn submit(&self, command: impl FnOnce(WorkId) -> WorkerCommand) -> Result<WorkId> {
        let id = self.interrupter.enqueue();
        if self.commands.send(command(id)).is_err() {
            self.interrupter.withdraw(id);
            return Err(error::closed());
        }
        Ok(id)
    }

    /// Attend la réponse de la tâche `id`, ou l'annulation.
    ///
    /// Le jeton qui se déclenche **et** le futur abandonné interrompent la
    /// tâche `id` — elle seule : si elle est déjà terminée, la tâche suivante
    /// n'est pas touchée.
    ///
    /// # Erreurs
    /// Celle de la tâche, [`OxynError::Cancelled`], ou une erreur de driver si
    /// le thread a disparu avant de répondre.
    pub(crate) async fn await_reply<T>(
        &self,
        answer: oneshot::Receiver<Result<T>>,
        cancel: &CancelToken,
        id: WorkId,
    ) -> Result<T> {
        // Armée avant la première suspension : c'est pendant l'attente qu'un
        // onglet se ferme.
        let mut abandon = AbandonGuard::new(&self.interrupter, id);
        let answer = pin!(answer);
        let cancelled = pin!(cancel.cancelled());
        match select(answer, cancelled).await {
            Either::Left((Ok(result), _)) => {
                abandon.disarm();
                result
            }
            Either::Left((Err(_), _)) => {
                abandon.disarm();
                Err(error::closed())
            }
            // Le jeton signale ; la garde, en tombant, en fait une interruption
            // de la tâche.
            Either::Right(((), _)) => Err(OxynError::Cancelled),
        }
    }

    /// La tâche que le thread porteur exécute, pour les tests.
    #[cfg(test)]
    pub(crate) fn running(&self) -> Option<WorkId> {
        self.interrupter.running()
    }

    /// Interrompt la tâche `id`, pour les tests qui simulent une interruption
    /// arrivée trop tard.
    #[cfg(test)]
    pub(crate) fn interrupt(&self, id: WorkId) {
        self.interrupter.interrupt(id);
    }

    /// Ferme la connexion et termine le thread.
    ///
    /// # Erreurs
    /// L'erreur du moteur à la fermeture. Une session déjà fermée n'est **pas**
    /// une erreur : les ressources locales sont libérées dans tous les cas.
    pub(crate) async fn close(&self) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(WorkerCommand::Close(reply)).is_err() {
            // Le thread est déjà parti : la connexion est fermée.
            return Ok(());
        }
        match answer.await {
            Ok(result) => result,
            Err(_) => Ok(()),
        }
    }
}

/// Ouvre la base sur un thread neuf et rend de quoi lui parler.
///
/// La connexion est ouverte **sur le thread porteur**, pas ici : c'est le seul
/// endroit où elle vivra, et une ouverture est un appel bloquant qui n'a rien à
/// faire sur le fil d'exécution asynchrone.
///
/// Le jeton est consulté **avant** de démarrer quoi que ce soit ; une fois
/// l'ouverture lancée elle va à son terme, parce qu'ouvrir un fichier SQLite est
/// une opération courte et qu'il n'y a rien à interrompre à mi-chemin.
///
/// # Erreurs
/// [`OxynError::Io`] si le thread ne peut pas démarrer, [`OxynError::Connection`]
/// si la base ne s'ouvre pas, [`OxynError::Cancelled`] si le jeton est déjà
/// déclenché.
pub(crate) async fn spawn(
    spec: OpenSpec,
    cancel: &CancelToken,
) -> Result<(WorkerHandle, JoinHandle<()>)> {
    if cancel.is_cancelled() {
        return Err(OxynError::Cancelled);
    }
    let (commands, orders) = mpsc::unbounded_channel();
    let (ready, opened) = oneshot::channel();

    let thread = std::thread::Builder::new()
        .name("oxyn-sqlite".to_owned())
        .spawn(move || {
            let connection = match open(&spec) {
                Ok(connection) => connection,
                Err(err) => {
                    let _ = ready.send(Err(err));
                    return;
                }
            };
            let interrupter = Arc::new(Interrupter::new(connection.get_interrupt_handle()));
            if ready.send(Ok(Arc::clone(&interrupter))).is_err() {
                // L'appelant a renoncé pendant l'ouverture.
                let _ = connection.close();
                return;
            }
            run(connection, &interrupter, orders);
        })?;

    let interrupter = match opened.await {
        Ok(Ok(interrupter)) => interrupter,
        Ok(Err(err)) => return Err(err),
        Err(_) => {
            return Err(OxynError::Connection(
                "the connection thread ended before opening the database".to_owned(),
            ));
        }
    };

    Ok((
        WorkerHandle {
            commands,
            interrupter,
        },
        thread,
    ))
}

/// Ouvre la connexion selon la spécification.
fn open(spec: &OpenSpec) -> Result<Connection> {
    // Les drapeaux par défaut de rusqlite, moins la création quand la session
    // est en lecture seule : `SQLITE_OPEN_READ_ONLY` fait refuser l'écriture par
    // le **moteur**, ce qui est une garantie autrement plus solide qu'un
    // filtrage côté client.
    let flags = if spec.read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::default()
    };
    let connection = match &spec.target {
        OpenTarget::Memory(connection) => Connection::open_with_flags(
            format!("file:oxyn-memory-{connection}?mode=memory&cache=shared"),
            OpenFlags::default(),
        ),
        OpenTarget::File(path) => Connection::open_with_flags(path, flags),
    };
    let connection = connection.map_err(error::open)?;
    if spec.read_only {
        connection
            .pragma_update(None, "query_only", true)
            .map_err(error::open)?;
    }
    Ok(connection)
}

/// La boucle du thread porteur.
///
/// Sortir de la boucle ferme la connexion **dans tous les cas** : sur un
/// `Close`, mais aussi quand le canal se ferme parce que la session a été
/// abandonnée. Sans cela, le fichier resterait verrouillé jusqu'à la fin du
/// processus.
///
/// Chaque tâche est encadrée par [`Interrupter::begin`] et
/// [`Interrupter::end`] : `end` n'est appelé qu'une fois la tâche revenue, donc
/// ses instructions finalisées, et c'est ce qui borne une interruption à la
/// tâche qu'elle vise. Une tâche abandonnée pendant qu'elle attendait est
/// détruite sans être exécutée ; son canal de réponse tombe avec elle.
fn run(
    connection: Connection,
    interrupter: &Interrupter,
    mut orders: mpsc::UnboundedReceiver<WorkerCommand>,
) {
    let mut closing = None;
    while let Some(command) = orders.blocking_recv() {
        match command {
            WorkerCommand::Job(id, job) => {
                if interrupter.begin(id) {
                    job(&connection);
                    interrupter.end();
                }
            }
            WorkerCommand::Stream(id, job) => {
                if interrupter.begin(id) {
                    stream::run(&connection, *job, interrupter);
                    interrupter.end();
                }
            }
            WorkerCommand::Close(reply) => {
                closing = Some(reply);
                break;
            }
        }
    }
    // La connexion est fermée **avant** la réponse : l'appelant qui attend
    // celle-ci sait que le fichier est relâché.
    let outcome = shutdown(connection);
    if let Some(reply) = closing {
        let _ = reply.send(outcome);
    }
}

/// Ferme la connexion en rendant l'erreur du moteur, s'il y en a une.
fn shutdown(connection: Connection) -> Result<()> {
    connection
        .close()
        .map_err(|(_, err)| error::engine(err, error::Effect::Mutating))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memoire() -> OpenSpec {
        OpenSpec {
            target: OpenTarget::Memory(oxyn_core::ConnectionId::new()),
            read_only: false,
        }
    }

    #[tokio::test]
    async fn une_tache_s_execute_sur_le_thread_porteur() {
        let jeton = CancelToken::new();
        let (handle, thread) = spawn(memoire(), &jeton).await.expect("ouverture");

        let reponse: i64 = handle
            .call(&jeton, |conn: &Connection| {
                conn.query_row("SELECT 40 + 2", [], |row| row.get(0))
                    .map_err(|err| error::engine(err, error::Effect::ReadOnly))
            })
            .await
            .expect("appel");
        assert_eq!(reponse, 42);

        handle.close().await.expect("fermeture");
        thread.join().expect("le thread se termine");
    }

    #[tokio::test]
    async fn une_tache_sur_un_jeton_deja_annule_ne_part_pas() {
        let jeton = CancelToken::new();
        let (handle, thread) = spawn(memoire(), &jeton).await.expect("ouverture");

        let enfant = jeton.child();
        enfant.cancel();
        let issue: oxyn_core::Result<i64> = handle.call(&enfant, |_: &Connection| Ok(1)).await;
        assert!(
            issue.expect_err("refus attendu").is_cancelled(),
            "un jeton déjà annulé ne doit pas lancer de travail"
        );

        handle.close().await.expect("fermeture");
        thread.join().expect("le thread se termine");
    }

    /// Une suite finie qui garde une instruction **active** entre deux pas.
    const SUITE: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                         WHERE x < 50000) SELECT x FROM c";

    /// Soumet une tâche qui lit une ligne de [`SUITE`], signale qu'elle est au
    /// milieu de son instruction, attend le feu vert, puis compte le reste.
    ///
    /// Ce qui est garanti quand `active` répond : le thread porteur exécute
    /// cette tâche et son instruction est active (`nVdbeActive > 0`). C'est
    /// l'état exact où un `sqlite3_interrupt` frappe l'instruction en cours.
    fn tache_suspendue(
        handle: &WorkerHandle,
    ) -> (
        WorkId,
        oneshot::Receiver<()>,
        std::sync::mpsc::Sender<()>,
        oneshot::Receiver<Result<i64>>,
    ) {
        let (active, actif) = oneshot::channel();
        let (feu_vert, attente) = std::sync::mpsc::channel::<()>();
        let (reply, answer) = oneshot::channel();
        let job: Job = Box::new(move |conn: &Connection| {
            let issue = (|| {
                let mut statement = conn
                    .prepare(SUITE)
                    .map_err(|err| error::engine(err, error::Effect::ReadOnly))?;
                let mut rows = statement.raw_query();
                let mut lues = 0_i64;
                let step = |rows: &mut rusqlite::Rows<'_>| {
                    rows.next()
                        .map(|row| row.is_some())
                        .map_err(|err| error::engine(err, error::Effect::ReadOnly))
                };
                if step(&mut rows)? {
                    lues += 1;
                }
                let _ = active.send(());
                let _ = attente.recv();
                while step(&mut rows)? {
                    lues += 1;
                }
                Ok(lues)
            })();
            let _ = reply.send(issue);
        });
        let id = handle
            .submit(|id| WorkerCommand::Job(id, job))
            .expect("soumission");
        (id, actif, feu_vert, answer)
    }

    #[tokio::test]
    async fn une_interruption_tardive_ne_frappe_pas_la_tache_suivante() {
        // Le scénario : un onglet est fermé au moment exact où sa requête se
        // termine, et le thread porteur est déjà dans la requête suivante.
        // `sqlite3_interrupt` vise la connexion ; sans ciblage, c'est la
        // requête suivante qui mourrait.
        let jeton = CancelToken::new();
        let (handle, thread) = spawn(memoire(), &jeton).await.expect("ouverture");

        let (reply, answer) = oneshot::channel();
        let precedente = handle
            .submit(|id| {
                WorkerCommand::Job(
                    id,
                    Box::new(move |_: &Connection| {
                        let _ = reply.send(Ok(()));
                    }),
                )
            })
            .expect("soumission");
        handle
            .await_reply(answer, &jeton, precedente)
            .await
            .expect("la tâche précédente se termine");

        let (suivante, actif, feu_vert, reponse) = tache_suspendue(&handle);
        actif
            .await
            .expect("la tâche suivante est dans son instruction");
        assert_eq!(handle.running(), Some(suivante));

        // L'interruption de la tâche précédente arrive trop tard.
        handle.interrupt(precedente);
        feu_vert.send(()).expect("feu vert");

        let lues = handle
            .await_reply(reponse, &jeton, suivante)
            .await
            .expect("la tâche suivante ne doit pas être interrompue");
        assert_eq!(lues, 50_000);

        handle.close().await.expect("fermeture");
        thread.join().expect("le thread se termine");
    }

    #[tokio::test]
    async fn l_interruption_de_la_tache_en_cours_l_arrete() {
        // Le pendant du test précédent : le ciblage ne doit pas désarmer
        // l'interruption légitime.
        let jeton = CancelToken::new();
        let (handle, thread) = spawn(memoire(), &jeton).await.expect("ouverture");

        let (en_cours, actif, feu_vert, reponse) = tache_suspendue(&handle);
        actif.await.expect("la tâche est dans son instruction");

        handle.interrupt(en_cours);
        feu_vert.send(()).expect("feu vert");

        let issue = handle.await_reply(reponse, &jeton, en_cours).await;
        assert!(
            matches!(issue, Err(ref err) if err.is_cancelled()),
            "la tâche visée doit être interrompue : {issue:?}"
        );

        handle.close().await.expect("fermeture");
        thread.join().expect("le thread se termine");
    }

    #[tokio::test]
    async fn une_tache_abandonnee_en_file_n_est_pas_executee() {
        // Un onglet fermé pendant que sa requête attend son tour : l'exécuter
        // ensuite occuperait la session pour personne.
        let jeton = CancelToken::new();
        let (handle, thread) = spawn(memoire(), &jeton).await.expect("ouverture");

        let (bloquante, actif, feu_vert, reponse) = tache_suspendue(&handle);
        actif.await.expect("le thread porteur est occupé");

        let executee = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let temoin = Arc::clone(&executee);
        let (reply, answer) = oneshot::channel::<Result<()>>();
        let en_file = handle
            .submit(|id| {
                WorkerCommand::Job(
                    id,
                    Box::new(move |_: &Connection| {
                        temoin.store(true, std::sync::atomic::Ordering::SeqCst);
                        let _ = reply.send(Ok(()));
                    }),
                )
            })
            .expect("soumission");
        {
            // Le futur de l'attente est détruit avant d'avoir abouti.
            let mut attente = pin!(handle.await_reply(answer, &jeton, en_file));
            assert!(futures::poll!(attente.as_mut()).is_pending());
        }

        feu_vert.send(()).expect("feu vert");
        handle
            .await_reply(reponse, &jeton, bloquante)
            .await
            .expect("la tâche bloquante se termine");
        let apres: i64 = handle
            .call(&jeton, |conn: &Connection| {
                conn.query_row("SELECT 1", [], |row| row.get(0))
                    .map_err(|err| error::engine(err, error::Effect::ReadOnly))
            })
            .await
            .expect("la session répond");
        assert_eq!(apres, 1);
        assert!(
            !executee.load(std::sync::atomic::Ordering::SeqCst),
            "une tâche abandonnée en file ne doit pas s'exécuter"
        );

        handle.close().await.expect("fermeture");
        thread.join().expect("le thread se termine");
    }

    #[tokio::test]
    async fn le_thread_s_arrete_quand_la_session_est_abandonnee() {
        // Une session oubliée ne doit pas laisser un thread et un verrou de
        // fichier derrière elle.
        let jeton = CancelToken::new();
        let (handle, thread) = spawn(memoire(), &jeton).await.expect("ouverture");
        drop(handle);
        thread.join().expect("le thread se termine de lui-même");
    }

    #[tokio::test]
    async fn fermer_deux_fois_n_est_pas_une_erreur() {
        let jeton = CancelToken::new();
        let (handle, thread) = spawn(memoire(), &jeton).await.expect("ouverture");
        handle.close().await.expect("première fermeture");
        handle
            .close()
            .await
            .expect("une session déjà fermée se referme sans erreur");
        thread.join().expect("le thread se termine");
    }

    #[tokio::test]
    async fn une_base_absente_en_lecture_seule_est_une_erreur_de_connexion() {
        let jeton = CancelToken::new();
        let spec = OpenSpec {
            target: OpenTarget::File(PathBuf::from(
                "/oxyn-inexistant/base-qui-n-existe-pas.sqlite",
            )),
            read_only: true,
        };
        let err = spawn(spec, &jeton).await.expect_err("ouverture impossible");
        assert!(matches!(err, OxynError::Connection(_)), "{err:?}");
        assert!(
            !err.to_string().contains("base-qui-n-existe-pas"),
            "le chemin ne doit pas ressortir : {err}"
        );
    }
}
