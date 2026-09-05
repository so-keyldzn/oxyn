//! Le curseur : un lot Arrow à la fois, et une annulation qui coupe vraiment.
//!
//! # L'annulation, ici, n'est pas décorative
//!
//! Le jeton est consulté **avant chaque demande de lot** et surveillé **pendant
//! l'attente**. Quand il se déclenche, `sqlite3_interrupt` part vers le moteur :
//! un `sqlite3_step` déjà lancé sur une agrégation de quatre minutes s'arrête,
//! le thread porteur redevient disponible, et le curseur devient inutilisable.
//!
//! Abandonner le futur ne suffirait pas — c'est exactement la panne que
//! [`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md) décrit. C'est aussi
//! ce que fait [`Drop`] : un curseur détruit sans avoir été épuisé interrompt ce
//! qui tourne encore.
//!
//! # Ce que ce curseur ne prétend pas être
//!
//! `sqlite3_interrupt` est une annulation **locale** : SQLite n'a pas de
//! serveur, il tourne dans le processus d'Oxyn. La session ne déclare donc pas
//! [`Capabilities::SERVER_SIDE_CANCEL`](oxyn_core::Capabilities::SERVER_SIDE_CANCEL),
//! et [`SqliteSession::cancel`](crate::SqliteSession) refuse. La voie d'annulation
//! est le [`CancelToken`], et c'est la seule.
//!
//! # Après une annulation, le curseur ne reprend pas
//!
//! Une instruction interrompue laisse le moteur à un point que rien ne permet de
//! reprendre proprement. Le curseur se marque terminé et refuse la suite : un
//! point de reprise se conçoit, il ne s'improvise pas.

use std::time::{Duration, Instant};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{CancelToken, ExecStats, OxynError, Result, StatementHandle};
use oxyn_driver::Cursor;
use tokio::sync::{mpsc, oneshot};

use crate::error;
use crate::stream::{Pull, Pulled, StreamStart};
use crate::worker::WorkerHandle;

/// Un flux de `RecordBatch` alimenté par une instruction SQLite.
pub struct SqliteCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    /// Le premier lot, produit avant même que le curseur existe : résoudre le
    /// type des colonnes a demandé de lire des lignes.
    pending: Option<RecordBatch>,
    pulls: mpsc::UnboundedSender<Pull>,
    worker: WorkerHandle,
    cancel: CancelToken,
    stats: ExecStats,
    started: Instant,
    elapsed: Option<Duration>,
    finished: bool,
    /// Une demande de lot est-elle partie sans que sa réponse soit revenue ?
    ///
    /// C'est ce qui distingue « le thread porteur est peut-être dans
    /// `sqlite3_step` » de « il attend tranquillement ». Interrompre dans le
    /// second cas serait au mieux inutile, au pire un drapeau posé sur le moteur
    /// dont la prochaine instruction hériterait.
    pulling: bool,
}

impl std::fmt::Debug for SqliteCursor {
    /// Ni le schéma ni les lots : un `Debug` de curseur sert à savoir où en est
    /// le flux, pas à imprimer des données.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteCursor")
            .field("handle", &self.handle)
            .field("columns", &self.schema.fields().len())
            .field("finished", &self.finished)
            .field("rows", &self.stats.rows)
            .finish_non_exhaustive()
    }
}

impl SqliteCursor {
    /// Construit le curseur à partir de ce que le thread porteur a déjà produit.
    pub(crate) fn new(
        handle: StatementHandle,
        start: StreamStart,
        pulls: mpsc::UnboundedSender<Pull>,
        worker: WorkerHandle,
        cancel: CancelToken,
        started: Instant,
    ) -> Self {
        let mut stats = ExecStats::default();
        if start.schema.fields().is_empty() {
            // Aucune colonne : ce que l'utilisateur attend, c'est le compte de
            // lignes affectées. `ExecStats::rows` porte les deux sens.
            stats.rows = start.affected;
        }

        let mut cursor = Self {
            handle,
            schema: start.schema,
            pending: None,
            pulls,
            worker,
            cancel,
            stats,
            started,
            elapsed: None,
            finished: false,
            pulling: false,
        };

        match start.first {
            Pulled::Batch(batch) => {
                cursor.record(&batch);
                cursor.pending = Some(batch);
            }
            Pulled::Done { truncated } => cursor.close(truncated),
        }
        cursor
    }

    /// Enregistre un lot dans les statistiques.
    fn record(&mut self, batch: &RecordBatch) {
        self.stats.record_batch(
            u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
            u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
        );
    }

    /// Marque le flux terminé et fige la durée.
    fn close(&mut self, truncated: bool) {
        self.finished = true;
        if truncated {
            // Doit remonter jusqu'à l'écran : un résultat tronqué qui a l'air
            // complet conduit à des conclusions fausses sur des données réelles.
            self.stats.mark_truncated();
        }
        if self.elapsed.is_none() {
            self.elapsed = Some(self.started.elapsed());
        }
    }

    /// Rend l'annulation demandée avant qu'un lot n'ait été réclamé.
    ///
    /// **Sans interrompre** : aucune demande n'est en vol, donc le thread
    /// porteur n'est pas dans `sqlite3_step`. Poser le drapeau d'interruption
    /// sur un moteur au repos n'annulerait rien et risquerait de retomber sur
    /// l'instruction suivante.
    fn abort(&mut self) -> OxynError {
        self.close(true);
        OxynError::Cancelled
    }
}

#[async_trait]
impl Cursor for SqliteCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        SchemaRef::clone(&self.schema)
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if let Some(batch) = self.pending.take() {
            return Ok(Some(batch));
        }
        if self.finished {
            return Ok(None);
        }
        if self.cancel.is_cancelled() {
            return Err(self.abort());
        }

        let (reply, answer) = oneshot::channel();
        if self.pulls.send(reply).is_err() {
            self.close(true);
            return Err(error::closed());
        }

        // Le jeton est cloné : `await_reply` l'emprunte, et `self` est déjà
        // emprunté mutablement.
        let cancel = self.cancel.clone();
        // À partir d'ici, le thread porteur peut être dans `sqlite3_step`. Si ce
        // futur est abandonné avant la ligne qui suit, `pulling` reste vrai et
        // c'est `Drop` qui interrompra.
        self.pulling = true;
        let pulled = self.worker.await_reply(answer, &cancel).await;
        self.pulling = false;

        match pulled {
            Ok(Pulled::Batch(batch)) => {
                self.record(&batch);
                Ok(Some(batch))
            }
            Ok(Pulled::Done { truncated }) => {
                self.close(truncated);
                Ok(None)
            }
            Err(err) => {
                // Y compris l'annulation : `await_reply` a déjà interrompu le
                // moteur. Le curseur ne reprend pas.
                self.close(true);
                Err(err)
            }
        }
    }

    fn stats(&self) -> ExecStats {
        let mut stats = self.stats;
        stats.total_time = self.elapsed.unwrap_or_else(|| self.started.elapsed());
        // `server_time` reste `None` : SQLite tourne dans le processus d'Oxyn,
        // il n'y a pas d'horloge serveur à opposer à l'horloge cliente. La
        // mesurer par instruction coûterait deux lectures d'horloge par ligne,
        // sur le chemin le plus chaud du driver.
        stats
    }
}

impl Drop for SqliteCursor {
    /// Un curseur détruit **pendant** qu'il attend un lot interrompt le moteur.
    ///
    /// C'est le cas d'un futur `next_batch` abandonné : fermer un onglet pendant
    /// une agrégation de quatre minutes laisserait sinon le thread porteur
    /// bloqué dans `sqlite3_step`, et la session entière avec lui.
    ///
    /// Quand aucune demande n'est en vol, il n'y a rien à interrompre : la
    /// destruction du canal, juste après, suffit à faire sortir le thread de sa
    /// boucle de diffusion.
    fn drop(&mut self) {
        if self.pulling {
            self.worker.interrupt();
        }
    }
}
