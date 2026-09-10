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
//!   ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md)).
//! * Le thread s'arrête quand son canal se ferme, même si personne n'appelle
//!   [`Session::close`](oxyn_driver::Session::close) : une session oubliée ne
//!   laisse pas de thread derrière elle.

use std::path::PathBuf;
use std::pin::pin;
use std::sync::Arc;
use std::thread::JoinHandle;

use futures::future::{Either, select};
use oxyn_core::{CancelToken, OxynError, Result};
use rusqlite::{Connection, InterruptHandle, OpenFlags};
use tokio::sync::{mpsc, oneshot};

use crate::error;
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
    Job(Job),
    /// Une exécution en flux. Le thread garde la main jusqu'à ce que le curseur
    /// soit épuisé ou détruit.
    Stream(Box<StreamJob>),
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
            Self::File(_) => f.write_str("File(<chemin masqué>)"),
        }
    }
}

/// La poignée partagée vers le thread porteur.
///
/// Clonée par la session, son catalogue et chacun de ses curseurs.
#[derive(Clone)]
pub(crate) struct WorkerHandle {
    commands: mpsc::UnboundedSender<WorkerCommand>,
    interrupt: Arc<InterruptHandle>,
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
    /// Demande au moteur d'interrompre ce qu'il est en train de faire.
    ///
    /// `sqlite3_interrupt` n'a **aucun effet** si aucune instruction ne tourne :
    /// l'appeler à tort n'annule pas la requête suivante.
    pub(crate) fn interrupt(&self) {
        self.interrupt.interrupt();
    }

    /// Exécute une tâche courte sur la connexion et rend son résultat.
    ///
    /// L'annulation est traitée **ici** : si le jeton se déclenche pendant
    /// l'attente, le moteur est interrompu et l'appel rend
    /// [`OxynError::Cancelled`]. Abandonner le futur sans interrompre laisserait
    /// le thread bloqué dans `sqlite3_step`.
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
        self.commands
            .send(WorkerCommand::Job(wrapped))
            .map_err(|_| error::closed())?;
        self.await_reply(answer, cancel).await
    }

    /// Démarre une exécution en flux.
    ///
    /// # Erreurs
    /// Une erreur de driver si le thread porteur a disparu.
    pub(crate) fn start_stream(&self, job: Box<StreamJob>) -> Result<()> {
        self.commands
            .send(WorkerCommand::Stream(job))
            .map_err(|_| error::closed())
    }

    /// Attend une réponse, ou l'annulation.
    ///
    /// # Erreurs
    /// Celle de la tâche, [`OxynError::Cancelled`], ou une erreur de driver si
    /// le thread a disparu avant de répondre.
    pub(crate) async fn await_reply<T>(
        &self,
        answer: oneshot::Receiver<Result<T>>,
        cancel: &CancelToken,
    ) -> Result<T> {
        let answer = pin!(answer);
        let cancelled = pin!(cancel.cancelled());
        match select(answer, cancelled).await {
            Either::Left((Ok(result), _)) => result,
            Either::Left((Err(_), _)) => Err(error::closed()),
            Either::Right(((), _)) => {
                // Le jeton signale ; c'est ici qu'il devient un
                // `sqlite3_interrupt`.
                self.interrupt();
                Err(OxynError::Cancelled)
            }
        }
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
            if ready.send(Ok(connection.get_interrupt_handle())).is_err() {
                // L'appelant a renoncé pendant l'ouverture.
                let _ = connection.close();
                return;
            }
            run(connection, orders);
        })?;

    let interrupt = match opened.await {
        Ok(Ok(interrupt)) => Arc::new(interrupt),
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
            interrupt,
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
fn run(connection: Connection, mut orders: mpsc::UnboundedReceiver<WorkerCommand>) {
    let mut closing = None;
    while let Some(command) = orders.blocking_recv() {
        match command {
            WorkerCommand::Job(job) => job(&connection),
            WorkerCommand::Stream(job) => stream::run(&connection, *job),
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
