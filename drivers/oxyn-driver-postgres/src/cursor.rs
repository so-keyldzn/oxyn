//! Le flux de lots, et l'annulation qui atteint vraiment le serveur.
//!
//! # Pourquoi une tâche et un canal
//!
//! `sqlx` rend un flux de lignes qui **emprunte** la connexion et l'instruction
//! préparée. Un curseur qui posséderait les trois serait auto-référentiel, ce
//! qui en Rust demande soit `unsafe` — interdit par le workspace — soit une
//! dépendance de plus. La tâche possède les trois et pousse ses lots dans un
//! canal borné ; le curseur n'en tient que la réception.
//!
//! Le canal a **une place**. Ce n'est pas de la frilosité : c'est la
//! contre-pression. La tâche ne décode le lot suivant que si le précédent a été
//! pris, donc un `SELECT *` sur 500 Go ne fait jamais grossir la mémoire au-delà
//! de deux lots ([I-06](../../../CLAUDE.md#i-06)).
//!
//! # L'abandon d'un curseur coupe la requête
//!
//! Fermer un onglet détruit le curseur. Son `Drop` **annule son jeton**, ce qui
//! réveille la tâche, lui fait émettre `pg_cancel_backend` depuis une seconde
//! connexion, puis fermer la sienne. Sans cela, l'agrégation de quatre minutes
//! continuerait sur le serveur, la connexion prise et le verrou posé — et au
//! dixième onglet fermé la base refuserait les connexions
//! ([DRIVER-CONTRACT §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! C'est aussi pourquoi `Drop` **n'avorte pas** la tâche : une tâche avortée ne
//! peut plus rien annuler. On lui demande de s'arrêter, on ne la tue pas.

use std::time::Instant;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use futures::StreamExt as _;
use oxyn_core::{
    CancelToken, DriverId, ErrorClass, ExecLimits, ExecStats, OxynError, Result, StatementHandle,
    StatementIntent,
};
use oxyn_driver::Cursor;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgArguments, PgStatement, Postgres};
use sqlx::{Either, Statement as _};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::decode::BatchAssembler;
use crate::error::map_stream_error;
use crate::session::{BackendCanceller, SQL_ROLLBACK, StatementRegistry};
use crate::types::PgDecoding;

/// Octets accumulés au-delà desquels un lot est clos et émis.
///
/// **En octets, pas en lignes.** Mille lignes portant chacune un BLOB d'un
/// mégaoctet font un gigaoctet ; un seuil compté en lignes marche sur les tables
/// de démonstration et déclenche l'OOM sur les vraies.
pub const BATCH_BYTE_BUDGET: usize = 1 << 20;

/// Lignes au-delà desquelles un lot est clos, quelle que soit sa taille.
///
/// Complète le seuil en octets par le bas : sans lui, un million de booléens ne
/// remplirait jamais un mégaoctet et le premier lot n'arriverait jamais. Le
/// budget de premier affichage est de 100 ms
/// ([PERFORMANCE](../../../docs/PERFORMANCE.md)).
pub const BATCH_ROW_CEILING: usize = 8_192;

/// Ce que la tâche de flux pousse vers le curseur.
#[derive(Debug)]
enum CursorEvent {
    /// Un lot prêt à afficher.
    Batch(RecordBatch),
    /// Le flux est épuisé, ou borné.
    Finished {
        /// Lignes affectées, pour une instruction qui ne rend pas de colonnes.
        affected_rows: u64,
        /// Le résultat a-t-il été coupé par les bornes d'exécution ?
        truncated: bool,
    },
    /// Le flux s'est interrompu sur une erreur déjà classée.
    Failed(Box<OxynError>),
}

/// Pourquoi la boucle de flux s'est arrêtée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Halt {
    /// Le serveur n'a plus rien à envoyer.
    Exhausted,
    /// [`ExecLimits::max_rows`] est atteint.
    RowLimit,
    /// Le jeton d'annulation s'est déclenché.
    Cancelled,
    /// [`ExecLimits::timeout`] est écoulé.
    TimedOut,
    /// Le curseur a été détruit : plus personne n'attend les lots.
    Abandoned,
    /// Une erreur a été rencontrée et déjà émise.
    Failed,
}

impl Halt {
    /// Faut-il demander au serveur d'arrêter la requête ?
    ///
    /// Tout ce qui n'est pas un épuisement laisse une requête en cours côté
    /// serveur. Abandonner le flux côté client ne libère ni la connexion ni le
    /// verrou.
    const fn needs_server_cancel(self) -> bool {
        !matches!(self, Self::Exhausted | Self::Failed)
    }
}

/// Un flux de `RecordBatch` alimenté par une exécution PostgreSQL.
///
/// `Debug` est écrit à la main : le canal et la poignée de tâche n'apprennent
/// rien à personne, et le schéma est ce qu'on veut voir en diagnostic.
pub struct PostgresCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    events: mpsc::Receiver<CursorEvent>,
    /// Le jeton **propre** à cette exécution. Annulé par `Drop`, il est ce qui
    /// transforme la fermeture d'un onglet en `pg_cancel_backend`.
    cancel: CancelToken,
    task: JoinHandle<()>,
    stats: ExecStats,
    started: Instant,
    finished: bool,
    /// L'instruction rend-elle des colonnes ? Sinon, `rows` compte les lignes
    /// **affectées**, ce qui n'est pas la même mesure.
    projects_columns: bool,
}

impl PostgresCursor {
    /// La tâche de flux est-elle terminée ?
    ///
    /// Sert au diagnostic et aux tests : après une annulation, la tâche doit
    /// s'arrêter d'elle-même, sans avoir été avortée.
    #[must_use]
    pub fn task_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl std::fmt::Debug for PostgresCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresCursor")
            .field("handle", &self.handle)
            .field("columns", &self.schema.fields().len())
            .field("finished", &self.finished)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Drop for PostgresCursor {
    /// Demande l'arrêt ; ne l'impose pas.
    ///
    /// Avorter la tâche la priverait du droit d'émettre `pg_cancel_backend` et
    /// de rendre sa connexion — c'est-à-dire exactement de ce qui rend
    /// l'annulation réelle.
    fn drop(&mut self) {
        if !self.finished {
            self.cancel.cancel();
        }
    }
}

#[async_trait]
impl Cursor for PostgresCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        SchemaRef::clone(&self.schema)
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.finished {
            return Ok(None);
        }
        match self.events.recv().await {
            Some(CursorEvent::Batch(lot)) => {
                self.stats.record_batch(
                    u64::try_from(lot.num_rows()).unwrap_or(u64::MAX),
                    u64::try_from(lot.get_array_memory_size()).unwrap_or(u64::MAX),
                );
                Ok(Some(lot))
            }
            Some(CursorEvent::Finished {
                affected_rows,
                truncated,
            }) => {
                self.seal(truncated);
                if !self.projects_columns {
                    self.stats.rows = affected_rows;
                }
                Ok(None)
            }
            Some(CursorEvent::Failed(erreur)) => {
                self.seal(true);
                Err(*erreur)
            }
            // Le canal ne se ferme sans `Finished` que si la tâche a disparu
            // sans conclure : c'est un bug du driver, pas une donnée fautive.
            None => {
                self.seal(true);
                Err(OxynError::Internal(
                    "la tâche de flux PostgreSQL s'est arrêtée sans conclure".to_owned(),
                ))
            }
        }
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}

impl PostgresCursor {
    /// Clôt le curseur : plus rien ne viendra.
    fn seal(&mut self, truncated: bool) {
        self.finished = true;
        self.stats.total_time = self.started.elapsed();
        if truncated {
            self.stats.mark_truncated();
        }
    }
}

/// Ce qu'il faut à la tâche de flux pour vivre sa vie.
///
/// Un type plutôt qu'onze paramètres : la fonction qui les prenait tous
/// dépassait la limite de `clippy::too_many_arguments` et, surtout, personne ne
/// pouvait relire l'ordre des arguments.
pub(crate) struct StreamRequest {
    /// Le driver, pour classer les erreurs.
    pub(crate) driver: DriverId,
    /// La connexion qui portera l'exécution, empruntée au bassin.
    pub(crate) connection: PoolConnection<Postgres>,
    /// L'instruction préparée : c'est elle qui a donné le schéma.
    pub(crate) statement: PgStatement,
    /// Les paramètres liés, déjà encodés.
    pub(crate) arguments: PgArguments,
    /// Le schéma des lots, connu avant la première ligne.
    pub(crate) schema: SchemaRef,
    /// Le plan de décodage, aligné sur le schéma.
    pub(crate) decodings: Vec<PgDecoding>,
    /// Les bornes de l'exécution.
    pub(crate) limits: ExecLimits,
    /// L'intention, qui décide de la classe des erreurs de transport.
    pub(crate) intent: StatementIntent,
    /// Le pid du serveur qui exécute, pour l'annulation.
    pub(crate) backend_pid: i32,
    /// De quoi ouvrir la seconde connexion qui annulera.
    pub(crate) canceller: std::sync::Arc<BackendCanceller>,
    /// Le registre des exécutions en cours, dont la tâche s'efface en partant.
    pub(crate) statements: std::sync::Arc<StatementRegistry>,
    /// La poignée que [`Cursor::handle`] expose.
    pub(crate) handle: StatementHandle,
}

/// Lance le flux et rend le curseur qui le draine.
///
/// L'appel rend la main immédiatement : le schéma est déjà connu — il vient de
/// l'instruction préparée — donc la grille dessine ses colonnes pendant que la
/// première ligne voyage encore.
pub(crate) fn spawn(request: StreamRequest, parent: &CancelToken) -> PostgresCursor {
    let (envoi, reception) = mpsc::channel(1);
    // Un **enfant** du jeton de l'appelant : annuler l'appelant annule cette
    // exécution, mais détruire ce curseur n'annule pas les autres onglets.
    let cancel = parent.child();

    let schema = SchemaRef::clone(&request.schema);
    let handle = request.handle;
    let projects_columns = !schema.fields().is_empty();
    let jeton = cancel.clone();

    let task = tokio::spawn(async move {
        run(request, jeton, envoi).await;
    });

    PostgresCursor {
        handle,
        schema,
        events: reception,
        cancel,
        task,
        stats: ExecStats::default(),
        started: Instant::now(),
        finished: false,
        projects_columns,
    }
}

/// Le corps de la tâche de flux.
#[allow(clippy::too_many_lines)]
async fn run(request: StreamRequest, cancel: CancelToken, events: mpsc::Sender<CursorEvent>) {
    let StreamRequest {
        driver,
        mut connection,
        statement,
        arguments,
        schema,
        decodings,
        limits,
        intent,
        backend_pid,
        canceller,
        statements,
        handle,
    } = request;

    let mut assembleur = BatchAssembler::new(schema, &decodings);
    let mut affectees: u64 = 0;
    let mut produites: usize = 0;
    let echeance = limits
        .timeout
        .map(|duree| tokio::time::Instant::now() + duree);

    let arret = {
        // `Statement::query_with` plutôt que `sqlx::query_statement_with` : le type
        // de base y est celui de l'instruction, sans inférence à faire remonter.
        let requete = statement.query_with(arguments);
        // `fetch_many` est déprécié parce que le multi-instruction n'a jamais
        // marché qu'en SQLite. Ce n'est pas ce qu'on en fait : c'est le seul
        // flux qui rende aussi le `QueryResult` final, donc le seul qui donne
        // `rows_affected()` — le bras `Either::Left` plus bas. Le `raw_sql()`
        // que la dépréciation propose abandonnerait l'instruction préparée,
        // donc les valeurs liées, donc I-10 : c'est un recul de sûreté, pas un
        // remplacement.
        //
        // TODO(2026-12-01, oxyn-driver-postgres) : revenir à une API non
        // dépréciée quand sqlx exposera le compte de lignes affectées sur
        // `fetch()`. Suivi : https://github.com/launchbadge/sqlx/issues/3108
        #[expect(
            deprecated,
            reason = "seul flux exposant rows_affected() ; voir ci-dessus"
        )]
        let mut flux = requete.fetch_many(&mut *connection);

        loop {
            let etape = tokio::select! {
                // `biased` : une annulation déjà demandée gagne toujours contre
                // un lot prêt. Sans cela, un flux rapide peut faire attendre
                // l'annulation indéfiniment.
                biased;
                () = cancel.cancelled() => Etape::Interrompu(Halt::Cancelled),
                () = attendre(echeance) => Etape::Interrompu(Halt::TimedOut),
                recu = flux.next() => match recu {
                    Some(resultat) => Etape::Recu(resultat),
                    None => Etape::Fin,
                },
            };

            let resultat = match etape {
                Etape::Fin => break Halt::Exhausted,
                Etape::Interrompu(raison) => break raison,
                Etape::Recu(resultat) => resultat,
            };

            let element = match resultat {
                Ok(element) => element,
                Err(erreur) => {
                    let oxyn = map_stream_error(&driver, intent, limits.read_only, erreur);
                    let _ = events.send(CursorEvent::Failed(Box::new(oxyn))).await;
                    break Halt::Failed;
                }
            };

            let ligne = match element {
                // Fin d'une instruction : le compte de lignes affectées est ce
                // qu'une écriture a de plus utile à dire.
                Either::Left(resume) => {
                    affectees = affectees.saturating_add(resume.rows_affected());
                    continue;
                }
                Either::Right(ligne) => ligne,
            };

            if let Err(erreur) = assembleur.push(&ligne) {
                let oxyn = OxynError::driver(driver.clone(), ErrorClass::Permanent, erreur);
                let _ = events.send(CursorEvent::Failed(Box::new(oxyn))).await;
                break Halt::Failed;
            }
            produites = produites.saturating_add(1);

            let borne_atteinte = limits.max_rows.is_some_and(|max| produites >= max);
            let lot_plein =
                assembleur.bytes() >= BATCH_BYTE_BUDGET || assembleur.rows() >= BATCH_ROW_CEILING;

            if lot_plein || borne_atteinte {
                match emettre(&mut assembleur, &driver, &events).await {
                    Emission::Poursuivre => {}
                    Emission::Abandonne => break Halt::Abandoned,
                    Emission::Echouee => break Halt::Failed,
                }
            }
            if borne_atteinte {
                break Halt::RowLimit;
            }
        }
    };

    // L'exécution ne peut plus être annulée par la session : la tâche s'efface
    // du registre avant même de nettoyer, pour qu'un `cancel` concurrent ne
    // vise pas un pid qui va être rendu au bassin.
    statements.forget(handle);

    // Le flux est détruit : l'emprunt sur la connexion est levé.
    if arret.needs_server_cancel() {
        // Une connexion dont le flux a été abandonné a pu garder des octets non
        // lus : la rendre au bassin désynchroniserait le prochain emprunteur.
        connection.close_on_drop();
        if let Err(erreur) = canceller.cancel_backend(backend_pid).await {
            // Signalé, pas propagé : l'exécution s'arrête de toute façon, et
            // l'utilisateur n'a rien à faire de cette information.
            tracing::warn!(
                target: "oxyn::driver::postgres",
                erreur = %erreur,
                "l'annulation côté serveur n'a pas abouti"
            );
        }
    } else if limits.read_only {
        // La transaction ouverte par `BEGIN READ ONLY` doit être refermée avant
        // le retour au bassin : une connexion laissée `idle in transaction`
        // garde des verrous et bloque le `VACUUM` de toute la base.
        let referme = sqlx::raw_sql(SQL_ROLLBACK).execute(&mut *connection).await;
        if let Err(erreur) = referme {
            tracing::warn!(
                target: "oxyn::driver::postgres",
                erreur = %erreur,
                "la transaction en lecture seule n'a pas pu être refermée"
            );
            connection.close_on_drop();
        }
    }
    drop(connection);

    match arret {
        Halt::Failed => {}
        Halt::Cancelled | Halt::Abandoned => {
            let _ = events
                .send(CursorEvent::Failed(Box::new(OxynError::Cancelled)))
                .await;
        }
        Halt::TimedOut => {
            let delai = limits.timeout.unwrap_or_default();
            // `Timeout` est ambigu par construction : le serveur a peut-être
            // appliqué l'écriture (I-13). C'est `OxynError::class` qui le dit,
            // pas ce message.
            let _ = events
                .send(CursorEvent::Failed(Box::new(OxynError::Timeout {
                    after: delai,
                })))
                .await;
        }
        Halt::Exhausted | Halt::RowLimit => {
            if !assembleur.is_empty()
                && let Emission::Echouee | Emission::Abandonne =
                    emettre(&mut assembleur, &driver, &events).await
            {
                return;
            }
            let _ = events
                .send(CursorEvent::Finished {
                    affected_rows: affectees,
                    truncated: arret == Halt::RowLimit,
                })
                .await;
        }
    }
}

/// Ce qu'une itération de la boucle a produit.
enum Etape {
    /// Le flux a rendu un élément.
    Recu(
        std::result::Result<
            Either<sqlx::postgres::PgQueryResult, sqlx::postgres::PgRow>,
            sqlx::Error,
        >,
    ),
    /// Le flux est épuisé.
    Fin,
    /// Une interruption a gagné la course.
    Interrompu(Halt),
}

/// Ce qu'a donné l'émission d'un lot.
enum Emission {
    /// Le lot est parti, on continue.
    Poursuivre,
    /// Plus personne n'écoute : le curseur a été détruit.
    Abandonne,
    /// Le lot n'a pas pu être construit ; l'erreur est déjà émise.
    Echouee,
}

/// Clôt le lot courant et l'envoie, en respectant la contre-pression du canal.
async fn emettre(
    assembleur: &mut BatchAssembler,
    driver: &DriverId,
    events: &mpsc::Sender<CursorEvent>,
) -> Emission {
    let lot = match assembleur.finish() {
        Ok(lot) => lot,
        Err(erreur) => {
            let oxyn = OxynError::driver(driver.clone(), ErrorClass::Permanent, erreur);
            let _ = events.send(CursorEvent::Failed(Box::new(oxyn))).await;
            return Emission::Echouee;
        }
    };
    // `send` sur un canal plein **attend** : c'est là que la contre-pression
    // s'exerce, et c'est ce qui empêche la mémoire de gonfler.
    if events.send(CursorEvent::Batch(lot)).await.is_err() {
        return Emission::Abandonne;
    }
    Emission::Poursuivre
}

/// Attend l'échéance, ou jamais quand il n'y en a pas.
///
/// L'instant est **absolu** : recréer ce futur à chaque tour de boucle ne
/// repousse donc pas le délai, ce qu'une durée relative ferait.
async fn attendre(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(instant) => tokio::time::sleep_until(instant).await,
        None => std::future::pending().await,
    }
}

// Le dimensionnement des lots, vérifié à la **compilation**.
//
// Un `assert!` d'exécution sur des constantes ne peut pas échouer autrement
// qu'en refusant de compiler plus tard : autant le dire ici. Le seuil en octets
// protège de l'OOM sur des BLOB ; le plafond de lignes garantit qu'un premier
// lot arrive vite sur des colonnes étroites
// ([drivers.md](../../../.claude/rules/drivers.md) — « le lot se dimensionne en
// octets, pas en lignes »).
const _: () = {
    assert!(BATCH_BYTE_BUDGET == 1 << 20);
    assert!(BATCH_ROW_CEILING > 0);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tout_arret_qui_n_est_pas_un_epuisement_demande_l_annulation_serveur() {
        // C'est la règle de DRIVER-CONTRACT §2 réduite à une ligne : abandonner
        // le flux côté client ne libère ni la connexion ni le verrou.
        assert!(!Halt::Exhausted.needs_server_cancel());
        assert!(
            !Halt::Failed.needs_server_cancel(),
            "le serveur a déjà fini"
        );
        for raison in [
            Halt::Cancelled,
            Halt::TimedOut,
            Halt::RowLimit,
            Halt::Abandoned,
        ] {
            assert!(
                raison.needs_server_cancel(),
                "{raison:?} laisse une requête en cours"
            );
        }
    }
}
