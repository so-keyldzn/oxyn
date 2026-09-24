//! L'ordonnanceur : le point de passage **obligé** de toute commande.
//!
//! Si un chemin permet d'atteindre un driver sans passer par
//! [`Executor::dispatch`], l'architecture de sûreté du produit est cassée : le
//! second chemin ne sera pas audité comme le premier, et c'est celui-là que l'IA
//! empruntera ([I-01](../../../CLAUDE.md#i-01), ADR-0004).
//!
//! # La séquence, dans cet ordre et sans raccourci
//!
//! 1. **Reclassifier.** L'intention portée par la commande vient de l'appelant,
//!    et un agent est un appelant. `oxyn-query` relit le texte ; c'est le
//!    résultat de cette relecture qui est soumis, journalisé et exécuté — jamais
//!    ce que l'appelant a déclaré (ARCHITECTURE §8, I-07).
//! 2. **Soumettre au `PolicyGate`**, avec l'environnement de la connexion visée
//!    et non celui que l'appelant annonce.
//! 3. **Sur `RequireApproval`, ne rien exécuter.** La commande est mise de côté
//!    ([`crate::approval`]) et l'interface reçoit
//!    [`Event::ApprovalRequested`]. L'exécution ne reprend que par
//!    [`Executor::approve`], sur l'identifiant exact de la commande.
//! 4. **Journaliser avant et après.** La décision de politique est écrite
//!    *avant* toute exécution, le résultat *après*. Une commande refusée figure
//!    au journal comme les autres : un journal qui ne consigne que ce qui a
//!    marché ne dit rien de ce qu'un agent a tenté.
//! 5. **Exécuter en flux**, en alimentant un
//!    [`oxyn_data::ResultBuffer`] par un
//!    [`oxyn_data::BatchSink`] — donc avec contre-pression et
//!    débordement disque (I-06).
//! 6. **Émettre les événements** vers l'interface par [`EventBus`].
//!
//! # Ce qui bloque une exécution, et ce qui ne la bloque pas
//!
//! Un échec d'écriture du journal **avant** exécution empêche l'exécution : la
//! piste d'audit est la promesse, pas un effet de bord. Un échec d'écriture du
//! journal **après** exécution ne l'annule pas — la commande a eu lieu, et
//! renvoyer une erreur laisserait croire le contraire ; il est crié au niveau
//! `error`.
//!
//! # Ce que cette version ne fait pas
//!
//! Page and value reads, preferences, the query library, connections (save,
//! delete, read), credential resolution, exports, and every write to the
//! audit journal and the query history all go through the application's
//! Tokio blocking pool as owned operations
//! ([ADR-0035](../../../docs/adr/0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md)).
//! The executor does not create a runtime of its own: without one, the
//! handlers that need it return a configuration error, while audit writes
//! run inline instead of failing outright — the audit trail must not go
//! silent at shutdown. Evicting retained results after an execution
//! ([`prune_results`](Executor::prune_results), called from
//! `execute_statement`), which can delete spill files, stays on the
//! dispatching worker on purpose.
//! [`load_connections`](Executor::load_connections) is synchronous too, but
//! never runs there: it is reserved for assembling state before the window
//! opens. Because journal rows are
//! now inserted from the blocking pool, their insertion order can differ from
//! the order of their `ts` field. An abandonment while a `RequireApproval`
//! decision is being written leaves no pending approval request behind: the
//! outcome is the same as one later rejected or expired.

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxyn_catalog::SharedCatalog;
use oxyn_core::{
    Actor, CancelToken, CatalogRefreshScope, Command, CommandId, ConnectionConfig, ConnectionId,
    Decision, DocumentId, Environment, Event, ExecRequest, ExecStats, OxynError, PolicyGate,
    Preview, Result, ResultId, SessionId, StatementHandle, WorkspaceId,
};
use oxyn_data::{
    BatchProgress, BatchSink, BatchSource, BufferLimits, DEFAULT_MEMORY_BUDGET, ExportOptions,
    ResultBuffer, SinkOutcome, export,
};
use oxyn_driver::{Cursor, DriverRegistry};
use oxyn_store::history::Reconciliation;
use oxyn_store::{Document, HistoryRecord, JournalRecord, Store};
use parking_lot::{Mutex, RwLock};

use crate::abandon::{AbandonGuard, AbandonedOutcomes, OutcomeGuard};
use crate::approval::{ApprovalRegistry, PendingCommand};
use crate::cancel::{CancelRegistry, CancelReport, RunningStatement};
use crate::events::EventBus;
use crate::sessions::{CredentialResolver, NoCredentials, SessionRegistry, SessionSlot};

/// Ce qu'une commande a produit.
///
/// `#[non_exhaustive]` : de nouvelles issues apparaîtront avec de nouvelles
/// commandes, et un appelant ne doit pas casser pour autant. C'est l'inverse du
/// choix fait sur [`Command`], dont l'exhaustivité **doit** casser le dispatch.
#[derive(Debug)]
#[non_exhaustive]
pub enum Outcome {
    /// Une session a été ouverte.
    Connected {
        /// La connexion.
        connection: ConnectionId,
        /// La session ouverte.
        session: SessionId,
    },

    /// Les sessions d'une connexion ont été fermées.
    /// One session was closed; sibling sessions and the catalog remain available.
    SessionClosed { session: SessionId },

    /// The server accepted a new resolution context for one session.
    ///
    /// Carries what the session reports afterwards, not what was requested: a
    /// server may normalise or reject part of it, and the interface must show
    /// the former.
    SessionContextSet {
        /// The session that moved. No sibling session is affected.
        session: SessionId,
        /// What the session reports now, `None` when it reports nothing.
        context: Option<oxyn_driver::SessionContext>,
    },
    Disconnected {
        /// La connexion.
        connection: ConnectionId,
        /// Combien de sessions ont été fermées.
        closed: usize,
    },

    /// Metadata was refreshed without including its contents in the outcome.
    CatalogRefreshed {
        /// Connection owning the updated cache.
        connection: ConnectionId,
        /// Requested scope, including Root for the legacy command.
        scope: CatalogRefreshScope,
    },

    /// The local catalog cache, handed to whoever was allowed to read it.
    ///
    /// The cache itself, not a rendering: rendering is `oxyn-ai`'s, under the
    /// connection's privacy tier.
    CatalogDescribed {
        /// Connection owning the cache.
        connection: ConnectionId,
        /// The cache, shared rather than copied.
        catalog: oxyn_catalog::CatalogHandle,
    },

    /// Une instruction a été exécutée et son résultat est disponible.
    Executed {
        /// Le résultat, tel que l'interface le désignera ensuite.
        result: ResultId,
        /// La poignée de l'exécution, cible d'une annulation.
        statement: StatementHandle,
        /// Le tampon, partagé avec l'interface **sans copie** : c'est ce que la
        /// grille lit pendant que les lots continuent d'arriver.
        buffer: Arc<ResultBuffer>,
        /// Ce que l'exécution a coûté.
        stats: ExecStats,
        /// Pourquoi le flux s'est arrêté. Seul
        /// [`Exhausted`](SinkOutcome::Exhausted) décrit un résultat entier.
        sink: SinkOutcome,
    },

    /// A bounded text page of a single value. Its Debug implementation hides text.
    ValueInspected {
        /// The page, without changing the result buffer.
        page: oxyn_data::value_page::ValuePage,
    },

    /// A locally stored page is now available in the result's bounded cache.
    ResultPageRead {
        /// Existing result; no new execution was created.
        result: ResultId,
        /// Zero-based batch index.
        batch: usize,
    },

    /// Preferences read or saved in the local workspace.
    WorkspacePreferences {
        snapshot: oxyn_core::PreferencesSnapshot,
    },

    /// A bounded library page, with no complete SQL bodies.
    QueryDocumentsListed {
        page: oxyn_store::documents::DocumentPage,
    },
    /// A local history page; retained identities may expire before opening.
    HistoryListed {
        page: oxyn_store::history::HistoryPage,
    },
    /// One selected execution record, never automatically replayed.
    HistoryEntryRead {
        entry: Box<oxyn_store::HistoryEntry>,
    },
    /// The user declared an unresolved write inspected; it no longer warns.
    HistoryEntryReconciled { entry: i64 },
    /// A bounded page of the connections history recorded, removed ones included.
    HistoryConnectionsListed {
        page: oxyn_store::history::HistoryConnectionPage,
    },
    /// A document was closed or deleted locally.
    DocumentClosed { document: DocumentId },
    /// The same retained buffer, without an execution event.
    RetainedResultOpened {
        result: ResultId,
        buffer: Arc<ResultBuffer>,
    },

    /// Une annulation a été demandée.
    Cancelled {
        /// Ce qui a effectivement été fait, client et serveur.
        report: CancelReport,
    },

    /// Un résultat a été écrit dans un fichier.
    Exported {
        /// Le résultat exporté.
        result: ResultId,
        /// Lignes écrites.
        rows: usize,
        /// Octets écrits.
        bytes: u64,
    },

    /// Un document du workspace a été relu.
    DocumentOpened {
        /// Le document. Encadré : c'est la plus grosse variante de loin.
        document: Box<Document>,
    },

    /// Un document du workspace a été écrit.
    DocumentWritten {
        /// Le document.
        document: DocumentId,
    },

    /// A configuration opened a session, which was closed at once.
    ///
    /// Nothing was saved and no session remains: the outcome says the server
    /// accepted these parameters, at this moment, and nothing more.
    ConnectionTested {
        /// The configuration tried, never registered with the executor.
        connection: ConnectionId,
    },

    /// Une connexion a été enregistrée ou modifiée.
    ConnectionSaved {
        /// La connexion.
        connection: ConnectionId,
    },

    /// The AI providers declared on this machine, in label order.
    ///
    /// An empty list is the default installation, not a failure: it is what
    /// decides whether the AI workspace exists at all. No reach is reported —
    /// classification is recomputed elsewhere, never stored
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    AiProvidersListed {
        /// Declarations, without any key. Boxed nowhere: the list is short and
        /// bounded by what the user typed.
        providers: Vec<oxyn_core::AiProviderConfig>,
    },

    /// An AI provider declaration was written locally.
    AiProviderSaved {
        /// The declaration that now exists.
        provider: oxyn_core::ProviderId,
    },

    /// An AI provider declaration was removed locally.
    ///
    /// Documents keep the provenance they were written with: it says where a
    /// text came from, not which provider is still declared.
    AiProviderRemoved {
        /// The declaration asked for.
        provider: oxyn_core::ProviderId,
        /// Did it exist?
        existed: bool,
    },

    /// The external agents declared on this machine, in label order.
    ///
    /// No reach is reported, and unlike a provider it is not because the value
    /// would go stale: an external agent's reach is **unknowable**
    /// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
    ExternalAgentsListed {
        /// Declarations. There is no key to withhold: this mode holds none.
        agents: Vec<oxyn_core::ExternalAgentConfig>,
    },

    /// An external agent declaration was written locally.
    ExternalAgentSaved {
        /// The declaration that now exists.
        agent: oxyn_core::ProviderId,
    },

    /// An external agent declaration was removed locally.
    ExternalAgentRemoved {
        /// The declaration asked for.
        agent: oxyn_core::ProviderId,
        /// Did it exist?
        existed: bool,
    },

    /// Une connexion a été supprimée du workspace.
    ConnectionDeleted {
        /// La connexion.
        connection: ConnectionId,
        /// Existait-elle ?
        existed: bool,
    },

    /// **Rien n'a été exécuté.** La commande attend un accord explicite.
    NeedsApproval {
        /// L'identifiant sous lequel l'accord se donne
        /// ([`Executor::approve`]).
        command: CommandId,
        /// Ce sur quoi l'utilisateur doit se prononcer.
        reason: String,
        /// De quoi juger sans aller lire ailleurs.
        preview: Option<Preview>,
    },

    /// **Rien n'a été exécuté**, et aucun accord ne débloquera la commande.
    Denied {
        /// La commande refusée, telle qu'elle figure au journal.
        command: CommandId,
        /// Le motif, montrable tel quel.
        reason: String,
    },
}

impl Outcome {
    /// La commande a-t-elle été refusée ?
    #[must_use]
    pub const fn is_denied(&self) -> bool {
        matches!(self, Self::Denied { .. })
    }

    /// La commande attend-elle un accord ?
    #[must_use]
    pub const fn needs_approval(&self) -> bool {
        matches!(self, Self::NeedsApproval { .. })
    }

    /// La commande a-t-elle produit un effet ?
    ///
    /// `false` pour [`NeedsApproval`](Self::NeedsApproval) et
    /// [`Denied`](Self::Denied). Le piège qu'elle ferme : un appelant — un agent
    /// en particulier — qui suppose qu'un `INSERT` a eu lieu et enchaîne sur
    /// cette hypothèse.
    #[must_use]
    pub const fn took_effect(&self) -> bool {
        !matches!(self, Self::NeedsApproval { .. } | Self::Denied { .. })
    }

    /// Les lignes produites ou affectées, quand la notion a un sens.
    #[must_use]
    pub fn rows(&self) -> Option<u64> {
        match self {
            Self::Executed { stats, .. } => Some(stats.rows),
            Self::Exported { rows, .. } => u64::try_from(*rows).ok(),
            _ => None,
        }
    }
}

pub(crate) struct StoredResult {
    pub(crate) connection: ConnectionId,
    pub(crate) buffer: Arc<ResultBuffer>,
}

/// L'ordonnanceur.
///
/// Se partage par `Arc` entre l'interface, le runtime d'agents
/// ([`crate::ExecutorSink`]) et les tâches de fond. Toutes ses méthodes prennent
/// `&self`.
pub struct Executor {
    drivers: Arc<DriverRegistry>,
    store: Arc<Store>,
    policy: Arc<dyn PolicyGate>,
    credentials: Arc<dyn CredentialResolver>,
    sessions: SessionRegistry,
    running: CancelRegistry,
    approvals: ApprovalRegistry,
    events: EventBus,
    results: RwLock<crate::retained::RetainedResults>,
    catalogs: RwLock<HashMap<ConnectionId, Arc<crate::catalog::ConnectionCatalog>>>,
    connections: Arc<RwLock<HashMap<ConnectionId, ConnectionConfig>>>,
    /// Orders every write of a connection — to the store, then to
    /// `connections` — inside one blocking task. The cache feeds the
    /// environment given to the `PolicyGate` (I-02): two saves whose disk and
    /// cache writes interleave, or a save abandoned between the two, would
    /// leave `production` on disk and `development` in the cache. Readers of
    /// `connections` never take this lock.
    connection_writes: Arc<Mutex<()>>,
    workspace: WorkspaceId,
    memory_budget: usize,
    abandoned: AbandonedOutcomes,
    /// History rows still `running` since then belong to this launch: their
    /// outcome is not known yet, so they cannot be declared reconciled.
    started_at: chrono::DateTime<chrono::Utc>,
}

impl Executor {
    /// Commence le câblage d'un ordonnanceur.
    ///
    /// Le `Store` et le `PolicyGate` sont exigés dès l'appel : un ordonnanceur
    /// sans journal ou sans politique n'a pas de forme dégradée acceptable.
    #[must_use]
    pub fn builder(store: Arc<Store>, policy: Arc<dyn PolicyGate>) -> ExecutorBuilder {
        ExecutorBuilder::new(store, policy)
    }

    // ── Le point de passage ─────────────────────────────────────────────────

    /// Soumet une commande.
    ///
    /// C'est **la** méthode : l'interface, les agents et les plugins passent
    /// tous par là, avec le même code derrière.
    ///
    /// Un refus et une demande d'approbation ne sont **pas** des erreurs : ce
    /// sont des [`Outcome`]. Une `Err` décrit une panne — un serveur
    /// injoignable, un délai dépassé, un journal illisible.
    ///
    /// # Erreurs
    /// Toute erreur du driver, du tampon ou de l'état local ; et
    /// [`OxynError::Internal`] si la décision de politique n'a pas pu être
    /// journalisée avant exécution — auquel cas **rien n'est exécuté**.
    pub async fn dispatch(
        &self,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        self.dispatch_as(CommandId::new(), actor, command, cancel)
            .await
    }

    /// [`dispatch`](Self::dispatch), sous un identifiant fourni par l'appelant.
    ///
    /// Sert à corréler **avant** que la réponse n'arrive : un puits d'agent
    /// rend l'identifiant dans son rapport, une interface l'affiche dans sa
    /// barre d'état. `id` doit être frais — le réutiliser mélangerait deux
    /// commandes dans le journal d'audit, où il est la clé de corrélation.
    ///
    /// # Erreurs
    /// Celles de [`dispatch`](Self::dispatch).
    pub async fn dispatch_as(
        &self,
        id: CommandId,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        // 1. Ce que le texte fait, pas ce que l'appelant en dit.
        let command = reclassified(command);
        let connection = command.target_connection();

        // 2. L'environnement de la connexion visée, pas celui qu'on annonce.
        let env = self.environment_of(&command);

        // 3. Le point de passage unique.
        let decision = self.policy.authorize(&actor, &command, env);

        // 4. Journal AVANT, dans chaque branche : voir ADR-0035. Une décision
        // qui ne s'écrit pas ne s'exécute pas.
        match &decision {
            Decision::Deny { reason } => {
                let reason = reason.clone();
                let record = decision_record(id, &actor, &command, &decision, None);
                let denied = self
                    .history_record(&actor, &command)
                    .map(|entry| entry.denied(reason.clone()));
                // Both writes are attempted independently: a failure on the
                // decision must never swallow the denial's history entry, and
                // vice versa — unchanged by this move to the blocking pool.
                if self
                    .write_audit(move |store| {
                        if let Err(erreur) = store.journal().append(&record) {
                            tracing::error!(error = %erreur, command = %id, "policy decision could not be journaled");
                        }
                        if let Some(entry) = denied
                            && let Err(erreur) = store.history().record(&entry)
                        {
                            tracing::error!(error = %erreur, "a denied statement could not be recorded in the query history");
                        }
                    })
                    .await
                    .is_err()
                {
                    tracing::error!(command = %id, "policy decision could not be journaled");
                }
                self.events.publish(
                    id,
                    connection,
                    Event::failed(&OxynError::PolicyDenied {
                        reason: reason.clone(),
                    }),
                );
                Ok(Outcome::Denied {
                    command: id,
                    reason,
                })
            }

            Decision::RequireApproval { reason, preview } => {
                let reason = reason.clone();
                let preview = preview.clone();
                let record = decision_record(id, &actor, &command, &decision, None);
                let write = self
                    .write_audit(move |store| -> Result<()> {
                        store.journal().append(&record)?;
                        Ok(())
                    })
                    .await
                    .and_then(|inner| inner);
                if let Err(erreur) = write {
                    tracing::error!(error = %erreur, command = %id, "policy decision could not be journaled");
                    return Err(erreur);
                }
                // If the caller drops this future right here, the decision
                // stays in the journal with no approval request pending:
                // nothing is queued and nothing runs — the same outcome as a
                // request later rejected or expired (see ADR-0035).
                //
                // Rien n'est exécuté. La commande mise de côté est celle que le
                // gate a vue — reclassifiée — pas le texte d'origine.
                let attente = self.approvals.submit(id, actor, command, reason, preview)?;
                self.events.publish(
                    id,
                    connection,
                    Event::ApprovalRequested {
                        command: id,
                        reason: attente.reason.clone(),
                        preview: attente.preview.clone(),
                    },
                );
                Ok(Outcome::NeedsApproval {
                    command: id,
                    reason: attente.reason,
                    preview: attente.preview,
                })
            }

            Decision::Allow => {
                self.run(id, &actor, &command, &decision, cancel, None)
                    .await
            }
        }
    }

    /// Donne l'accord attendu par une commande, et l'exécute.
    ///
    /// `approved_by` est **qui** a approuvé : le journal le consigne. Il n'y a
    /// pas d'`Actor` en paramètre à dessein — cette méthode est appelée depuis
    /// l'interface, par un humain. Un agent n'a aucun moyen de l'atteindre : les
    /// outils exposés aux agents sont exactement les [`Command`], et il n'existe
    /// pas de commande d'approbation.
    ///
    /// La commande **repasse par le `PolicyGate`** avant de partir : la
    /// connexion a pu être marquée production ou lecture seule pendant
    /// l'attente, et un refus l'emporte alors sur l'accord donné.
    ///
    /// # Erreurs
    /// [`OxynError::PolicyDenied`] si aucune commande n'attend sous cet
    /// identifiant ou si la demande a expiré — dans les deux cas **rien n'est
    /// exécuté** ; sinon les erreurs de [`dispatch`](Self::dispatch).
    pub async fn approve(
        &self,
        approved_by: &str,
        command: CommandId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let attente = self.approvals.take(command)?;
        let connection = attente.command.target_connection();

        let env = self.environment_of(&attente.command);
        let decision = self.policy.authorize(&attente.actor, &attente.command, env);

        if let Decision::Deny { reason } = &decision {
            let reason = reason.clone();
            let record =
                decision_record(command, &attente.actor, &attente.command, &decision, None);
            let denied = self
                .history_record(&attente.actor, &attente.command)
                .map(|entry| entry.denied(reason.clone()));
            // Le refus tardif est journalisé lui aussi : c'est même la trace la
            // plus intéressante de toutes, puisqu'un accord avait été donné.
            // Both writes are attempted independently, same as an ordinary
            // denial in `dispatch_as`.
            if self
                .write_audit(move |store| {
                    if let Err(erreur) = store.journal().append(&record) {
                        tracing::error!(error = %erreur, command = %command, "late denial could not be journaled");
                    }
                    if let Some(entry) = denied
                        && let Err(erreur) = store.history().record(&entry)
                    {
                        tracing::error!(error = %erreur, "a denied statement could not be recorded in the query history");
                    }
                })
                .await
                .is_err()
            {
                tracing::error!(command = %command, "late denial could not be journaled");
            }
            self.events.publish(
                command,
                connection,
                Event::failed(&OxynError::PolicyDenied {
                    reason: reason.clone(),
                }),
            );
            return Ok(Outcome::Denied { command, reason });
        }

        // Plus de destructuration de `attente` avant l'exécution : `run` écrit
        // lui-même la décision, comme première opération de sa fenêtre.
        self.run(
            command,
            &attente.actor,
            &attente.command,
            &decision,
            cancel,
            Some(approved_by),
        )
        .await
    }

    /// Retire une demande à laquelle l'utilisateur a répondu « non ».
    ///
    /// Rien n'est exécuté, et la commande ne pourra plus l'être : il faudra la
    /// réémettre, donc repasser par le gate.
    pub fn reject(&self, command: CommandId) -> Option<PendingCommand> {
        self.approvals.reject(command)
    }

    // ── Exécution ───────────────────────────────────────────────────────────

    /// Exécute une commande déjà autorisée, et journalise sa décision et son
    /// issue autour de l'exécution.
    ///
    /// `decision` est ce que `PolicyGate::authorize` a rendu à l'appelant —
    /// jamais `Deny`, que les deux appelants traitent avant d'atteindre cette
    /// méthode.
    ///
    /// # Erreurs
    /// Celles d'[`execute_command`](Self::execute_command), et
    /// [`OxynError::Internal`] si la décision n'a pas pu être journalisée —
    /// auquel cas **rien n'est exécuté**.
    async fn run(
        &self,
        id: CommandId,
        actor: &Actor,
        command: &Command,
        decision: &Decision,
        cancel: &CancelToken,
        approved_by: Option<&str>,
    ) -> Result<Outcome> {
        // Armed as soon as the policy allows this command, before its
        // decision is even written (ADR-0035): no `.await` runs between the
        // `Allow` a caller matched and this line, so no abandonment can slip
        // in ahead of the guard.
        let guard = OutcomeGuard::new(&self.abandoned, id, actor, command, approved_by);

        // One operation: the decision, then — only if it was written — the
        // "in progress" entry in the query history. L'historique s'inscrit
        // ici et pas au dispatch : une commande mise en attente d'accord puis
        // rejetée laisserait sinon une ligne « en cours » éternelle, alors
        // qu'elle n'a jamais été soumise au serveur.
        let record = decision_record(id, actor, command, decision, approved_by);
        let starting = self.history_record(actor, command);
        let write = self
            .write_audit(move |store| -> Result<Option<(i64, HistoryRecord)>> {
                store.journal().append(&record)?;
                Ok(
                    starting.and_then(|record| match store.history().record(&record) {
                        Ok(history_id) => Some((history_id, record)),
                        Err(erreur) => {
                            tracing::error!(
                                error = %erreur,
                                "a submitted statement could not be recorded in the query history"
                            );
                            None
                        }
                    }),
                )
            })
            .await
            .and_then(|inner| inner);

        let en_cours = match write {
            Ok(en_cours) => en_cours,
            Err(erreur) => {
                tracing::error!(error = %erreur, command = %id, "policy decision could not be journaled");
                // Nothing ran: there is no outcome to write, and no hole in
                // the audit trail to leave behind.
                guard.settle();
                return Err(erreur);
            }
        };

        let debut = Instant::now();
        let issue = self.execute_command(id, command, cancel).await;
        guard.settle();
        let duree = debut.elapsed();

        let mut outcome = outcome_record(
            id,
            actor,
            command,
            duree,
            issue.as_ref().ok().and_then(Outcome::rows),
            approved_by,
        );
        if let Err(erreur) = &issue {
            outcome = outcome.failed(erreur);
        }
        let finishing = finished_history_record(en_cours, &issue, duree);

        // Submitted right away, with no `.await` between `guard.settle()`
        // above and this call: the window `OutcomeGuard` covers stays
        // exactly the one it covers today, between the start of execution
        // and the moment this operation is queued.
        if self
            .write_audit(move |store| {
                if let Err(erreur) = store.journal().append(&outcome) {
                    tracing::error!(
                        error = %erreur,
                        command = %id,
                        "failed to journal the outcome of a command that already ran"
                    );
                }
                if let Some((history_id, record)) = finishing
                    && let Err(erreur) = store.history().finish(history_id, &record)
                {
                    tracing::error!(error = %erreur, "the outcome of a statement could not be written to the query history");
                }
            })
            .await
            .is_err()
        {
            tracing::error!(command = %id, "the audit writer stopped after a command already ran");
        }

        issue
    }

    /// Le dispatch proprement dit.
    ///
    /// Le `match` est **exhaustif** et sans `_ =>` : [`Command`] est une
    /// énumération fermée précisément pour qu'ajouter une commande fasse échouer
    /// la compilation ici. Un `_ =>` avalerait en silence une commande que le
    /// `PolicyGate` vient pourtant d'autoriser.
    async fn execute_command(
        &self,
        id: CommandId,
        command: &Command,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        match command {
            Command::Connect { connection } => self.connect(*connection, cancel).await,
            Command::TestConnection { config } => self.test_connection(config, cancel).await,

            Command::Disconnect { connection } => self.disconnect(*connection).await,
            Command::CloseSession {
                connection,
                session,
            } => {
                if cancel.is_cancelled() {
                    return Err(OxynError::Cancelled);
                }
                self.close_session(*connection, *session).await
            }

            Command::SetSessionContext {
                connection,
                session,
                catalog,
                namespace,
            } => {
                if cancel.is_cancelled() {
                    return Err(OxynError::Cancelled);
                }
                self.set_session_context(
                    *connection,
                    *session,
                    catalog.as_deref(),
                    namespace.as_deref(),
                    cancel,
                )
                .await
            }

            Command::Execute {
                connection,
                session,
                request,
            } => {
                self.execute_statement(id, *connection, *session, (**request).clone(), cancel, None)
                    .await
            }

            Command::PreviewRelation {
                connection,
                session,
                catalog,
                namespace,
                relation,
                limit,
                shape,
            } => {
                if !(1..=1000).contains(limit) {
                    return Err(OxynError::Config(
                        "preview limit must be in 1..=1000".into(),
                    ));
                }
                let path = oxyn_catalog::CatalogPath::for_relation(
                    catalog.as_deref(),
                    namespace.as_deref(),
                    relation,
                )?;
                let slot = self
                    .sessions
                    .get(*session)
                    .ok_or_else(|| OxynError::Connection("preview session is not open".into()))?;
                if slot.connection() != *connection {
                    return Err(OxynError::Config(
                        "preview session belongs to another connection".into(),
                    ));
                }
                // Refusé ici plutôt que laissé au driver : la capacité est ce
                // que **cette session** déclare, et une demande qu'elle ne sait
                // pas honorer ne doit pas atteindre la composition du SQL, où
                // la tentation serait de l'ignorer (ADR-0003, ADR-0020).
                let capabilities = slot.capabilities();
                if !shape.sort.is_empty()
                    && !capabilities.contains(oxyn_core::Capabilities::PREVIEW_SORT)
                {
                    return Err(OxynError::NotSupported {
                        capability: "preview sort".to_owned(),
                    });
                }
                if shape.predicate().is_some()
                    && !capabilities.contains(oxyn_core::Capabilities::PREVIEW_FILTER)
                {
                    return Err(OxynError::NotSupported {
                        capability: "preview filter".to_owned(),
                    });
                }
                let request = slot.preview_request(&path, *limit, shape, cancel).await?;
                let mut request = oxyn_query::reclassify(&request).qualify(request);
                if request.is_mutating() {
                    // Deux refus distincts, parce qu'ils demandent deux gestes
                    // différents. `Unknown` veut dire « ce texte n'a pas pu être
                    // classé », et sur un aperçu la seule part écrite à la main
                    // est le prédicat : le dire « pas en lecture seule »
                    // enverrait l'utilisateur chercher un droit manquant alors
                    // qu'il a une faute de frappe. Le refus reste dans les deux
                    // cas — un texte que le classificateur ne comprend pas
                    // compte pour mutant, et c'est cette prudence qui protège.
                    let reason = if request.intent == oxyn_core::StatementIntent::Unknown {
                        "this preview filter could not be read as a condition; \
                         check its syntax"
                    } else {
                        "driver preview request is not read-only"
                    };
                    return Err(OxynError::PolicyDenied {
                        reason: reason.to_owned(),
                    });
                }
                // Enforce the command contract even if a driver omitted its limits.
                request.limits.read_only = true;
                let row_limit = usize::try_from(*limit).map_err(|_| {
                    OxynError::Config("preview limit exceeds platform capacity".into())
                })?;
                // The SQL is already bounded. One extra receive slot lets the
                // cursor report its real end instead of a client-side truncation.
                // The buffer still rejects every row beyond the requested limit.
                request.limits.max_rows = Some(row_limit.saturating_add(1));
                self.execute_statement(id, *connection, *session, request, cancel, Some(row_limit))
                    .await
            }

            Command::Cancel { statement, .. } => {
                let report = self.running.cancel(&self.sessions, *statement).await;
                Ok(Outcome::Cancelled { report })
            }

            Command::RefreshCatalog { connection } => {
                self.refresh_catalog(id, *connection, &CatalogRefreshScope::Root, cancel)
                    .await
            }
            Command::RefreshCatalogScope { connection, scope } => {
                self.refresh_catalog(id, *connection, scope, cancel).await
            }
            Command::DescribeCatalog { connection, focus } => {
                if focus
                    .as_deref()
                    .is_some_and(|focus| focus.len() > oxyn_core::MAX_CATALOG_FOCUS_BYTES)
                {
                    return Err(OxynError::Config(format!(
                        "a catalog focus is limited to {} bytes",
                        oxyn_core::MAX_CATALOG_FOCUS_BYTES
                    )));
                }
                // The cache as it stands: no server is contacted, and no
                // rendering is done here — what of it reaches a prompt is
                // decided under the connection's tier, in `oxyn-ai`.
                let catalog = self.catalog(*connection).ok_or_else(|| {
                    OxynError::Config(
                        "this connection has no catalog loaded yet: the user must open it in \
                         the explorer first"
                            .into(),
                    )
                })?;
                Ok(Outcome::CatalogDescribed {
                    connection: *connection,
                    catalog: oxyn_catalog::CatalogHandle::new(catalog),
                })
            }

            Command::InspectResultValue {
                connection,
                result,
                row,
                column,
                offset,
            } => {
                let buffer = self.result_on_connection(*connection, *result)?;
                let (row, column, offset) = (*row, *column, *offset);
                let cancel = cancel.clone();
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("value inspection requires the application runtime".into())
                })?;
                let page = runtime
                    .spawn_blocking(move || -> Result<_> {
                        let (batch, row) = buffer.read_row(row, &cancel)?.ok_or_else(|| {
                            OxynError::Config("selected row is no longer available".into())
                        })?;
                        oxyn_data::value_page::inspect_value(&batch, row, column, offset, &cancel)?
                            .ok_or_else(|| {
                                OxynError::Config("selected column is no longer available".into())
                            })
                    })
                    .await
                    .map_err(|_| OxynError::Internal("value inspection worker stopped".into()))??;
                Ok(Outcome::ValueInspected { page })
            }

            Command::ReadResultPage {
                connection,
                result,
                batch,
            } => {
                let buffer = self.result_on_connection(*connection, *result)?;
                let cancel = cancel.clone();
                let position = *batch;
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("result page loading requires the application runtime".into())
                })?;
                let loaded = runtime
                    .spawn_blocking(move || {
                        buffer.load_page(oxyn_data::BatchIndex::new(position), &cancel)
                    })
                    .await
                    .map_err(|_| OxynError::Internal("result page worker stopped".into()))??;
                if !loaded {
                    return Err(OxynError::Config(
                        "result page is no longer available".into(),
                    ));
                }
                Ok(Outcome::ResultPageRead {
                    result: *result,
                    batch: *batch,
                })
            }

            Command::Export {
                connection,
                result,
                format,
                destination,
                ..
            } => {
                let buffer = self.result_on_connection(*connection, *result)?;
                // Avant `File::create` : sinon un format que cette version ne sait pas
                // écrire laisse un fichier de zéro octet à l'emplacement que
                // l'utilisateur vient de nommer. La vérification vit ici et non dans la
                // vue parce que cette commande est aussi atteignable par un plugin et
                // par un `Actor::Agent` — un contrôle qui n'existe que dans l'interface
                // n'est pas un contrôle du bus (I-01).
                if !oxyn_data::is_supported(*format) {
                    return Err(OxynError::NotSupported {
                        capability: format!("export:{}", format.extension()),
                    });
                }
                let format = *format;
                let destination = destination.clone();
                let cancel_owned = cancel.clone();
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("export requires the application runtime".into())
                })?;
                let resume = runtime
                    .spawn_blocking(move || -> Result<_> {
                        let fichier = File::create(&destination)?;
                        Ok(export(
                            &buffer,
                            format,
                            BufWriter::new(fichier),
                            &ExportOptions::default(),
                            &cancel_owned,
                        )?)
                    })
                    .await
                    .map_err(|_| OxynError::Internal("export worker stopped".into()))??;
                Ok(Outcome::Exported {
                    result: *result,
                    rows: resume.rows,
                    bytes: resume.bytes,
                })
            }

            Command::ReadWorkspacePreferences { workspace } => {
                if *workspace != self.workspace {
                    return Err(OxynError::Config(
                        "workspace does not match this executor".into(),
                    ));
                }
                let store = self.store.clone();
                let workspace = *workspace;
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("preference loading requires the application runtime".into())
                })?;
                let snapshot = runtime
                    .spawn_blocking(move || store.preferences().load(workspace))
                    .await
                    .map_err(|_| OxynError::Internal("preference worker stopped".into()))??;
                Ok(Outcome::WorkspacePreferences { snapshot })
            }
            Command::WriteWorkspacePreferences {
                workspace,
                snapshot,
            } => {
                if *workspace != self.workspace {
                    return Err(OxynError::Config(
                        "workspace does not match this executor".into(),
                    ));
                }
                snapshot.validate()?;
                let store = self.store.clone();
                let workspace = *workspace;
                let snapshot = (**snapshot).clone();
                let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                    OxynError::Config("preference saving requires the application runtime".into())
                })?;
                let snapshot = runtime
                    .spawn_blocking(move || store.preferences().save(workspace, &snapshot))
                    .await
                    .map_err(|_| OxynError::Internal("preference worker stopped".into()))??;
                Ok(Outcome::WorkspacePreferences { snapshot })
            }

            Command::ListQueryDocuments { workspace, filter } => {
                self.check_workspace(*workspace)?;
                filter.validate()?;
                let store = self.store.clone();
                let workspace = *workspace;
                let filter = (**filter).clone();
                let page = self
                    .local_worker(cancel, move |cancel| {
                        store.documents().page(workspace, &filter, &cancel)
                    })
                    .await?;
                Ok(Outcome::QueryDocumentsListed { page })
            }
            Command::SaveQueryDocument { workspace, update } => {
                self.check_workspace(*workspace)?;
                update.validate()?;
                let store = self.store.clone();
                let workspace = *workspace;
                let update = (**update).clone();
                let document = self
                    .local_worker(cancel, move |cancel| {
                        store.documents().update_query(workspace, &update, &cancel)
                    })
                    .await?;
                Ok(Outcome::DocumentOpened {
                    document: Box::new(document),
                })
            }
            Command::CloseQueryDocument {
                expected_revision,
                workspace,
                document,
                revision,
                discard,
            } => {
                self.check_workspace(*workspace)?;
                let store = self.store.clone();
                let workspace = *workspace;
                let target = *document;
                let revision = *revision;
                let discard = *discard;
                let expected_revision = *expected_revision;
                self.local_worker(cancel, move |cancel| {
                    store.documents().close_query_checked(
                        workspace,
                        target,
                        oxyn_store::documents::DocumentRevision {
                            next: revision,
                            expected: expected_revision,
                        },
                        discard,
                        false,
                        &cancel,
                    )
                })
                .await?;
                Ok(Outcome::DocumentClosed { document: target })
            }
            Command::DeleteQueryDocument {
                workspace,
                document,
                revision,
            } => {
                self.check_workspace(*workspace)?;
                let store = self.store.clone();
                let workspace = *workspace;
                let target = *document;
                let revision = *revision;
                self.local_worker(cancel, move |cancel| {
                    store
                        .documents()
                        .close_query(workspace, target, revision, true, true, &cancel)
                })
                .await?;
                Ok(Outcome::DocumentClosed { document: target })
            }
            Command::ReadHistory { filter } => {
                filter.validate()?;
                let store = self.store.clone();
                let filter = (**filter).clone();
                let page = self
                    .local_worker(cancel, move |cancel| store.history().page(&filter, &cancel))
                    .await?;
                Ok(Outcome::HistoryListed { page })
            }
            Command::ReadHistoryEntry { entry } => {
                let store = self.store.clone();
                let target = *entry;
                let entry = self
                    .local_worker(cancel, move |cancel| store.history().get(target, &cancel))
                    .await?
                    .ok_or_else(|| {
                        OxynError::Config("history entry is no longer available".into())
                    })?;
                Ok(Outcome::HistoryEntryRead {
                    entry: Box::new(entry),
                })
            }
            Command::ReconcileHistoryEntry { entry } => {
                let store = self.store.clone();
                let target = *entry;
                let live_since = self.started_at;
                let reconciliation = self
                    .local_worker(cancel, move |cancel| {
                        store.history().reconcile(target, live_since, &cancel)
                    })
                    .await?;
                match reconciliation {
                    Reconciliation::Recorded => {
                        Ok(Outcome::HistoryEntryReconciled { entry: target })
                    }
                    Reconciliation::StillRunning => Err(OxynError::Config(
                        "this write is still running; wait for its outcome before reconciling it"
                            .into(),
                    )),
                    _ => Err(OxynError::Config(
                        "history entry is gone or has no unresolved outcome".into(),
                    )),
                }
            }
            Command::ListHistoryConnections { workspace, filter } => {
                self.check_workspace(*workspace)?;
                filter.validate()?;
                let store = self.store.clone();
                let workspace = *workspace;
                let filter = *filter;
                let page = self
                    .local_worker(cancel, move |cancel| {
                        store.history().connections(workspace, &filter, &cancel)
                    })
                    .await?;
                Ok(Outcome::HistoryConnectionsListed { page })
            }
            Command::OpenRetainedResult { connection, result } => {
                Ok(Outcome::RetainedResultOpened {
                    result: *result,
                    buffer: self.result_on_connection(*connection, *result)?,
                })
            }
            Command::OpenDocument {
                workspace,
                document,
            } => {
                self.check_workspace(*workspace)?;
                self.open_document(*workspace, *document, cancel).await
            }
            Command::WriteDocument {
                workspace,
                document,
                text,
            } => {
                self.check_workspace(*workspace)?;
                self.write_document(*workspace, *document, text, cancel)
                    .await
            }

            Command::CreateConnection { config } | Command::UpdateConnection { config } => {
                self.save_connection(config, cancel).await
            }

            Command::DeleteConnection { connection } => {
                self.delete_connection(*connection, cancel).await
            }

            // Les trois commandes de fournisseur restent locales : rien ici ne
            // résout un nom ni n'ouvre de connexion vers un modèle. Le
            // classement local/distant se recalcule à l'ouverture d'un runtime
            // (ADR-0023), et une résolution DNS faite ici la ferait vieillir en
            // base sous un autre nom.
            Command::ListAiProviders => {
                let store = self.store.clone();
                let providers = self
                    .local_worker(cancel, move |_cancel| store.providers().list())
                    .await?;
                Ok(Outcome::AiProvidersListed { providers })
            }
            Command::SaveAiProvider { config } => {
                // Validée avant d'atteindre le pool : une URL portant des
                // identifiants ne doit pas voyager plus loin que nécessaire.
                config.validate()?;
                let store = self.store.clone();
                let config = (**config).clone();
                let provider = config.id.clone();
                self.local_worker(cancel, move |_cancel| store.providers().save(&config))
                    .await?;
                Ok(Outcome::AiProviderSaved { provider })
            }
            Command::RemoveAiProvider { id } => {
                let store = self.store.clone();
                let provider = id.clone();
                let cible = id.clone();
                let existed = self
                    .local_worker(cancel, move |_cancel| store.providers().remove(&cible))
                    .await?;
                Ok(Outcome::AiProviderRemoved { provider, existed })
            }

            // Les trois mêmes gestes pour un agent externe. Aucune validation
            // d'URL ici : il n'y en a pas. La validation de la déclaration est
            // faite avant le pool, pour la raison qui vaut aussi pour les
            // fournisseurs — une commande porteuse d'un caractère de contrôle ne
            // doit pas voyager plus loin que nécessaire.
            Command::ListExternalAgents => {
                let store = self.store.clone();
                let agents = self
                    .local_worker(cancel, move |_cancel| store.external_agents().list())
                    .await?;
                Ok(Outcome::ExternalAgentsListed { agents })
            }
            Command::SaveExternalAgent { agent } => {
                agent.validate()?;
                let store = self.store.clone();
                let declaration = (**agent).clone();
                let identite = declaration.id.clone();
                self.local_worker(cancel, move |_cancel| {
                    store.external_agents().save(&declaration)
                })
                .await?;
                Ok(Outcome::ExternalAgentSaved { agent: identite })
            }
            Command::RemoveExternalAgent { id } => {
                let store = self.store.clone();
                let agent = id.clone();
                let cible = id.clone();
                let existed = self
                    .local_worker(cancel, move |_cancel| {
                        store.external_agents().remove(&cible)
                    })
                    .await?;
                Ok(Outcome::ExternalAgentRemoved { agent, existed })
            }
        }
    }

    /// Ouvre une session.
    async fn connect(&self, connection: ConnectionId, cancel: &CancelToken) -> Result<Outcome> {
        let config = self.connection_config(connection, cancel).await?;
        let session = self.open_driver_session(&config, cancel).await?;
        let slot = self.sessions.insert(SessionSlot::new(connection, session));
        let catalog = self
            .catalogs
            .write()
            .entry(connection)
            .or_insert_with(|| Arc::new(crate::catalog::ConnectionCatalog::new()))
            .clone();
        let mut preferred = catalog.session.write();
        if preferred
            .and_then(|id| self.sessions.get(id))
            .is_none_or(|session| !session.is_open())
        {
            *preferred = Some(slot.id());
        }
        drop(preferred);
        Ok(Outcome::Connected {
            connection,
            session: slot.id(),
        })
    }

    /// Opens a session on a configuration the workspace does not hold, then
    /// closes it.
    ///
    /// The session never enters the registry: nothing can run on it, and no
    /// catalog is attached to it. The error, if any, is the driver's own,
    /// class included — the one a real opening would give.
    async fn test_connection(
        &self,
        config: &ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let session = self.open_driver_session(config, cancel).await?;
        if let Err(erreur) = session.close().await {
            // The opening succeeded, which is what was asked; a refused close
            // frees the local resources all the same.
            tracing::warn!(error = %erreur, "the server refused a clean close of a test session");
        }
        Ok(Outcome::ConnectionTested {
            connection: config.id,
        })
    }

    /// Resolves the credentials and asks the driver for a session.
    async fn open_driver_session(
        &self,
        config: &ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<Box<dyn oxyn_driver::Session>> {
        let driver = self.drivers.require(&config.driver)?;
        // The system keyring lookup is blocking; it never runs on the shared
        // runtime that also carries drivers, network I/O and LLM calls (I-05).
        let resolver = Arc::clone(&self.credentials);
        let lookup = config.clone();
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| OxynError::Config("connecting requires the application runtime".into()))?;
        let credentials = runtime
            .spawn_blocking(move || resolver.resolve(&lookup))
            .await
            .map_err(|_| OxynError::Internal("credential worker stopped".into()))??;
        driver.connect(config, &credentials, cancel).await
    }

    /// Ferme les sessions d'une connexion, après avoir annulé ce qui y tourne.
    async fn disconnect(&self, connection: ConnectionId) -> Result<Outcome> {
        // L'annulation d'abord : fermer sans annuler laisse les requêtes tourner
        // côté serveur, connexion prise et verrou posé.
        self.running
            .cancel_connection(&self.sessions, connection)
            .await;

        if let Some(catalog) = self.catalogs.write().remove(&connection) {
            catalog.closed.cancel();
        }
        let mut closed = 0;
        for slot in self.sessions.drain_connection(connection) {
            if let Err(erreur) = slot.close().await {
                // Les ressources locales sont libérées dans tous les cas ; une
                // fermeture refusée par le serveur ne se rattrape pas.
                tracing::warn!(error = %erreur, "the server refused a clean session close");
            }
            closed += 1;
        }
        Ok(Outcome::Disconnected { connection, closed })
    }

    async fn close_session(&self, connection: ConnectionId, session: SessionId) -> Result<Outcome> {
        let Some(slot) = self.sessions.get(session) else {
            return Ok(Outcome::SessionClosed { session });
        };
        if slot.connection() != connection {
            return Err(OxynError::PolicyDenied {
                reason: "session does not belong to this connection".into(),
            });
        }
        slot.begin_close();
        for running in self.running.for_session(session) {
            self.running.cancel(&self.sessions, running.statement).await;
        }
        self.sessions.remove(session);
        slot.close().await?;
        Ok(Outcome::SessionClosed { session })
    }

    /// Declares where one session resolves unqualified names.
    ///
    /// Refuses the session the catalog and previews read through: the explorer
    /// shows a qualified tree, and moving it under the user because a console
    /// changed schema would make the same click mean two things on two days
    /// ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)).
    async fn set_session_context(
        &self,
        connection: ConnectionId,
        session: SessionId,
        catalog: Option<&str>,
        namespace: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let Some(slot) = self.sessions.get(session) else {
            return Err(OxynError::PolicyDenied {
                reason: "unknown session".into(),
            });
        };
        if slot.connection() != connection {
            return Err(OxynError::PolicyDenied {
                reason: "session does not belong to this connection".into(),
            });
        }
        if !slot
            .capabilities()
            .contains(oxyn_core::Capabilities::SESSION_CONTEXT)
        {
            return Err(OxynError::NotSupported {
                capability: "session context".to_owned(),
            });
        }
        let reserved = self
            .catalogs
            .read()
            .get(&connection)
            .and_then(|state| *state.session.read());
        if reserved == Some(session) {
            return Err(OxynError::PolicyDenied {
                reason: "this session serves the catalog and previews; \
                         open a console to change context"
                    .into(),
            });
        }
        // Validation by the catalog path rather than here: a control character
        // in a name must be refused before it reaches a driver about to quote it.
        let path = oxyn_catalog::CatalogPath::from_levels(
            catalog.map(ToOwned::to_owned),
            namespace.map(ToOwned::to_owned),
            None,
        )
        .map_err(|error| OxynError::Config(error.to_string()))?;
        let context = oxyn_driver::SessionContext::from_path(&path);
        slot.set_context(&context, cancel).await?;
        Ok(Outcome::SessionContextSet {
            session,
            context: slot.context(cancel).await?,
        })
    }

    /// Returns the connected source's in-memory cache without any I/O.
    ///
    /// None before Connect or after Disconnect. Hold read guards briefly and
    /// never across await; refreshes are issued through dispatch only.
    #[must_use]
    pub fn catalog(&self, connection: ConnectionId) -> Option<SharedCatalog> {
        self.catalogs
            .read()
            .get(&connection)
            .map(|state| Arc::clone(&state.cache))
    }

    /// Marks a connection's whole catalog stale after a DDL succeeds.
    ///
    /// The scope is the whole connection, never the touched object: naming
    /// that object would mean reconstructing an identifier from the executed
    /// SQL text, which I-10 forbids. Data is kept, only marked
    /// [`Invalidated`](oxyn_catalog::Freshness::Invalidated) — the next read
    /// re-fetches it.
    fn invalidate_catalog(&self, connection: ConnectionId) {
        if let Some(state) = self.catalogs.read().get(&connection).cloned() {
            state.cache.write().invalidate_all();
        }
    }

    async fn refresh_catalog(
        &self,
        id: CommandId,
        connection: ConnectionId,
        scope: &CatalogRefreshScope,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let state = self
            .catalogs
            .read()
            .get(&connection)
            .cloned()
            .ok_or_else(|| OxynError::Connection("no open catalog session".to_owned()))?;
        let mut budget = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(OxynError::Cancelled),
            _ = state.closed.cancelled() => return Err(OxynError::Cancelled),
            guard = state.refresh.lock() => guard,
        };
        let preferred = *state.session.read();
        let slot = preferred
            .and_then(|session| self.sessions.get(session))
            .or_else(|| {
                if preferred.is_none() {
                    self.sessions
                        .for_connection(connection)
                        .into_iter()
                        .find(|slot| slot.is_open())
                } else {
                    None
                }
            })
            .filter(|slot| slot.is_open())
            .ok_or_else(|| OxynError::Connection("no open catalog session".to_owned()))?;
        let operation = cancel.child();
        let read = slot.read_catalog(scope, &operation);
        tokio::pin!(read);
        let patch = tokio::select! {
            biased;
            _ = state.closed.cancelled() => {
                operation.cancel();
                // Let the provider finish cancellation and protocol cleanup.
                let _ = read.await;
                return Err(OxynError::Cancelled);
            }
            result = &mut read => result?,
        };
        // The registry read guard orders publication against Disconnect.
        let catalogs = self.catalogs.read();
        if cancel.is_cancelled()
            || state.closed.is_cancelled()
            || !catalogs
                .get(&connection)
                .is_some_and(|current| Arc::ptr_eq(current, &state))
        {
            return Err(OxynError::Cancelled);
        }
        let evicted = budget.reserve(scope, &patch)?;
        {
            let mut cache = state.cache.write();
            for scope in &evicted {
                cache.evict(scope);
            }
            patch.apply(&mut cache)?;
        }
        self.events
            .publish(id, Some(connection), Event::CatalogUpdated);
        Ok(Outcome::CatalogRefreshed {
            connection,
            scope: scope.clone(),
        })
    }

    /// Exécute une instruction et draine son curseur dans un tampon.
    async fn execute_statement(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        request: ExecRequest,
        cancel: &CancelToken,
        preview_limit: Option<usize>,
    ) -> Result<Outcome> {
        // `ExecLimits` par défaut interdit l'écriture : une demande mutante qui
        // n'a pas explicitement levé cette borne est incohérente, et
        // l'incohérence se tranche du côté prudent. C'est la dernière barrière
        // avant le driver.
        if request.is_mutating() && request.limits.read_only {
            return Err(OxynError::PolicyDenied {
                reason: "this execution is bounded to read-only: \
                         a write must lift that bound explicitly"
                    .to_owned(),
            });
        }

        let Some(slot) = self.sessions.get(session) else {
            return Err(OxynError::Connection(
                "no session is open under this identifier".to_owned(),
            ));
        };
        if slot.connection() != connection {
            return Err(OxynError::Internal(
                "the target session does not belong to the command's connection".to_owned(),
            ));
        }

        let limits = request.limits.clone();
        // Capturé avant que `request` ne soit déplacé dans `slot.execute` :
        // c'est le seul signal qu'on garde du texte exécuté (I-10).
        let intent = request.intent;
        // Un jeton **fils** : annuler cette exécution n'annule pas l'onglet qui
        // l'a lancée, alors qu'annuler l'onglet l'annule bien.
        let ct = cancel.child();

        let operation = async {
            // Armed before the driver is called: a caller that drops this future
            // at any `.await` below never reaches the cleanup written after it.
            let mut guard =
                AbandonGuard::new(&self.running, &self.events, id, connection, ct.clone());
            let cursor = match slot.execute(request, &ct).await {
                Ok(cursor) => cursor,
                Err(erreur) => {
                    guard.settle();
                    return Err(erreur);
                }
            };
            let statement = cursor.handle();
            self.running.register(RunningStatement::new(
                statement,
                id,
                connection,
                session,
                slot.capabilities(),
                ct.clone(),
            ));

            let result = ResultId::new();
            let buffer = Arc::new(ResultBuffer::with_limits(
                BatchSource::schema(&cursor),
                BufferLimits::default()
                    .with_memory_budget(self.memory_budget)
                    .with_max_rows(preview_limit.or(limits.max_rows)),
            ));
            self.results.write().insert(
                result,
                StoredResult {
                    connection,
                    buffer: Arc::clone(&buffer),
                },
            );
            guard.track(statement, Arc::clone(&buffer));

            // Le schéma est connu avant la première ligne : la grille dessine ses
            // colonnes pendant que les données arrivent.
            self.events
                .publish(id, Some(connection), Event::SchemaReady { result });

            let issue = self
                .drain(
                    Coordinates {
                        command: id,
                        connection,
                        result,
                    },
                    &buffer,
                    cursor,
                    &ct,
                    limits.timeout,
                    preview_limit.is_some(),
                )
                .await;

            // Un abandon — expiration ou annulation — doit atteindre le serveur.
            let interrompue = matches!(issue, Ok(SinkOutcome::Cancelled))
                || matches!(issue, Err(OxynError::Timeout { .. }));
            if interrompue {
                self.running.cancel(&self.sessions, statement).await;
            }
            guard.settle();
            self.prune_results();

            match issue {
                Ok(sink) => {
                    let stats = buffer.stats();
                    if sink != SinkOutcome::Cancelled && intent == oxyn_core::StatementIntent::Ddl {
                        // Après un DDL confirmé par le serveur, pas avant : un
                        // sink annulé peut n'avoir rien changé.
                        self.invalidate_catalog(connection);
                        self.events
                            .publish(id, Some(connection), Event::CatalogUpdated);
                    }
                    let event = if sink == SinkOutcome::Cancelled {
                        Event::Cancelled
                    } else {
                        Event::Completed {
                            result,
                            stats,
                            intent,
                        }
                    };
                    self.events.publish(id, Some(connection), event);
                    Ok(Outcome::Executed {
                        result,
                        statement,
                        buffer,
                        stats,
                        sink,
                    })
                }
                Err(erreur) => {
                    self.events
                        .publish(id, Some(connection), Event::failed(&erreur));
                    Err(erreur)
                }
            }
        };
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => result,
            _ = slot.closing_token().cancelled() => { ct.cancel(); operation.await }
        }
    }

    /// Draine un curseur, sous contre-pression et sous délai.
    ///
    /// Le délai est appliqué **ici** et non dans `oxyn-data` : c'est
    /// l'ordonnanceur qui possède le journal et le droit d'émettre l'annulation
    /// côté serveur. Un délai posé plus bas n'abandonnerait que le futur.
    async fn drain(
        &self,
        coords: Coordinates,
        buffer: &Arc<ResultBuffer>,
        cursor: Box<dyn Cursor>,
        ct: &CancelToken,
        timeout: Option<Duration>,
        confirm_preview_end: bool,
    ) -> Result<SinkOutcome> {
        let sink = BatchSink::new(Arc::clone(buffer));
        let sink = if confirm_preview_end {
            sink.with_end_confirmation()
        } else {
            sink
        };
        let mut source = cursor;
        let events = &self.events;

        // Émettre sur un canal, oui ; dessiner, non : ce crochet s'exécute
        // *dans* la boucle de drainage et retarde le lot suivant.
        let on_batch = |progress: BatchProgress| {
            events.publish(
                coords.command,
                Some(coords.connection),
                Event::BatchReady {
                    result: coords.result,
                    rows: progress.rows,
                },
            );
        };

        let Some(duree) = timeout else {
            return sink.drain_with(&mut source, ct, on_batch).await;
        };

        match tokio::time::timeout(duree, sink.drain_with(&mut source, ct, on_batch)).await {
            Ok(issue) => issue,
            Err(_) => {
                // Le futur de drainage vient d'être abandonné, peut-être après
                // avoir consommé des octets du flux : le curseur est brûlé. Le
                // tampon est clos pour que l'interface cesse d'attendre — ce qui
                // est déjà reçu reste lisible, et marqué tronqué.
                buffer.mark_truncated();
                buffer.mark_complete(BatchSource::stats(&source));
                Err(OxynError::Timeout { after: duree })
            }
        }
    }

    fn check_workspace(&self, workspace: WorkspaceId) -> Result<()> {
        if workspace != self.workspace {
            return Err(OxynError::Config(
                "workspace does not match this executor".into(),
            ));
        }
        Ok(())
    }

    async fn local_worker<T: Send + 'static>(
        &self,
        cancel: &CancelToken,
        task: impl FnOnce(CancelToken) -> oxyn_store::Result<T> + Send + 'static,
    ) -> Result<T> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            OxynError::Config("local library operations require the application runtime".into())
        })?;
        let cancel = cancel.clone();
        runtime
            .spawn_blocking(move || task(cancel))
            .await
            .map_err(|_| OxynError::Internal("local library worker stopped".into()))?
            .map_err(Into::into)
    }

    /// Runs a write meant for the audit trail — the policy journal or the
    /// query history — on the blocking pool, and awaits it before returning.
    ///
    /// Unlike [`local_worker`](Self::local_worker), this never fails for want
    /// of a runtime: the audit trail must not go silent at shutdown, when no
    /// Tokio runtime may be current any more ([`journal_abandoned_off_runtime`](Self::journal_abandoned_off_runtime)
    /// already relied on this fallback before it moved here). With a runtime,
    /// `spawn_blocking` is the first thing this call does — before any other
    /// `.await` in its body — so a caller that submits an operation and
    /// awaits it right away leaves no window where the operation is queued
    /// but not yet running. Without one, `write` runs inline.
    ///
    /// # Errors
    /// [`OxynError::Internal`] if the blocking task was cancelled or
    /// panicked. Once submitted, though, the write belongs to that task:
    /// dropping the future this call returns does not cancel it.
    async fn write_audit<T: Send + 'static>(
        &self,
        write: impl FnOnce(&Store) -> T + Send + 'static,
    ) -> Result<T> {
        let store = Arc::clone(&self.store);
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => runtime
                .spawn_blocking(move || write(&store))
                .await
                .map_err(|_| OxynError::Internal("audit writer stopped".into())),
            Err(_) => Ok(write(&store)),
        }
    }

    async fn open_document(
        &self,
        workspace: WorkspaceId,
        document: DocumentId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let store = self.store.clone();
        let doc = self
            .local_worker(cancel, move |cancel| {
                store.documents().get_cancellable(document, &cancel)
            })
            .await?
            .filter(|doc| doc.workspace == workspace)
            .ok_or_else(|| OxynError::Config("document does not exist in this workspace".into()))?;
        Ok(Outcome::DocumentOpened {
            document: Box::new(doc),
        })
    }

    async fn write_document(
        &self,
        workspace: WorkspaceId,
        document: DocumentId,
        text: &str,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let store = self.store.clone();
        let text = text.to_owned();
        let doc = self
            .local_worker(cancel, move |cancel| {
                let doc = store
                    .documents()
                    .get_cancellable(document, &cancel)?
                    .filter(|doc| doc.workspace == workspace)
                    .ok_or_else(|| oxyn_store::StoreError::Corrupted {
                        field: "documents",
                        detail: "document is not in this workspace".into(),
                    })?;
                let revision = doc
                    .revision
                    .max(doc.saved_revision)
                    .checked_add(1)
                    .ok_or_else(|| oxyn_store::StoreError::Corrupted {
                        field: "documents.revision",
                        detail: "revision exhausted".into(),
                    })?;
                let update = oxyn_core::QueryDocumentUpdate {
                    expected_revision: None,
                    document,
                    revision,
                    title: doc.title,
                    language: doc.language,
                    text,
                    connection: doc.connection,
                    save_named: doc.is_saved,
                    is_open: doc.is_open,
                    // Ce chemin réécrit le texte d'un document existant sans
                    // rien savoir de son origine. `None` ne l'efface pas : la
                    // provenance déjà posée reste (ADR-0023).
                    provenance: None,
                };
                let saved = store
                    .documents()
                    .update_query(workspace, &update, &cancel)?;
                if saved.content != update.text || saved.revision != update.revision {
                    return Err(oxyn_store::StoreError::Corrupted {
                        field: "documents.revision",
                        detail: "document changed while writing".into(),
                    });
                }
                Ok(saved)
            })
            .await?;
        Ok(Outcome::DocumentWritten { document: doc.id })
    }

    /// Enregistre ou met à jour une connexion.
    async fn save_connection(
        &self,
        config: &ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        // `Connections::save` **refuse** un paramètre portant un nom de secret :
        // c'est le dernier point où un mot de passe peut être arrêté avant le
        // disque (I-03). Rien n'est dupliqué ici.
        let store = Arc::clone(&self.store);
        let connections = Arc::clone(&self.connections);
        let writes = Arc::clone(&self.connection_writes);
        let workspace = self.workspace;
        let config = config.clone();
        let connection = config.id;
        // The cache is updated by the blocking task itself, under
        // `connection_writes`: it runs to completion even if this future is
        // dropped, and in the same order as the disk writes.
        self.local_worker(cancel, move |_cancel| {
            let _ordered = writes.lock();
            store.connections().save(workspace, &config)?;
            connections.write().insert(config.id, config);
            Ok(())
        })
        .await?;
        Ok(Outcome::ConnectionSaved { connection })
    }

    /// Supprime une connexion, après avoir fermé ce qui l'utilisait.
    async fn delete_connection(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        self.running
            .cancel_connection(&self.sessions, connection)
            .await;
        for slot in self.sessions.drain_connection(connection) {
            if let Err(erreur) = slot.close().await {
                tracing::warn!(error = %erreur, "the server refused a clean session close");
            }
        }
        let store = Arc::clone(&self.store);
        let connections = Arc::clone(&self.connections);
        let writes = Arc::clone(&self.connection_writes);
        let existed = self
            .local_worker(cancel, move |_cancel| {
                let _ordered = writes.lock();
                let existed = store.connections().delete(connection)?;
                connections.write().remove(&connection);
                Ok(existed)
            })
            .await?;
        Ok(Outcome::ConnectionDeleted {
            connection,
            existed,
        })
    }

    // ── Journal et historique ────────────────────────────────────────────────
    //
    // Chaque écriture est construite en mémoire ici — [`decision_record`],
    // [`Self::history_record`], [`finished_history_record`] — puis soumise au
    // pool bloquant par [`Self::write_audit`], appelée depuis [`Self::run`] et
    // [`Self::dispatch_as`] (ADR-0035). Aucune de ces méthodes ne touche le
    // `Store` : elles ne font que fabriquer la valeur qu'une opération possédée
    // écrira plus loin.
    //
    // L'historique répond à « qu'est-ce que j'ai lancé hier ? », là où le
    // journal d'audit répond à « qu'est-ce qui a été autorisé, et à qui ? ».
    // Seules les exécutions y figurent : `HistoryRecord::from_command` rend
    // `None` pour tout le reste, et une `Connect` au milieu des requêtes rendrait
    // la liste illisible. Le journal, lui, les consigne toutes. Un historique
    // qu'on ne peut pas écrire est une gêne, pas une promesse rompue : l'échec
    // est crié, jamais propagé.

    /// L'entrée d'historique d'une commande d'exécution, connexion nommée.
    ///
    /// Le nom est recopié pour que la ligne reste lisible après la suppression
    /// de la connexion. Ce qui n'y entre **pas** : les valeurs liées, que
    /// `ExecRequest` garde séparées du texte, et qui contiennent précisément ce
    /// qu'un historique relu six mois plus tard ne doit pas exposer (I-03).
    fn history_record(&self, actor: &Actor, command: &Command) -> Option<HistoryRecord> {
        let mut record = HistoryRecord::from_command(actor, command)?;
        record.connection_name = record
            .connection
            .and_then(|id| self.connections.read().get(&id).map(|c| c.name.clone()));
        Some(record)
    }

    // ── Ce que l'ordonnanceur sait des connexions ───────────────────────────

    /// Fait connaître une connexion à l'ordonnanceur.
    ///
    /// Sert à deux choses : retrouver l'environnement à soumettre au
    /// `PolicyGate`, et ouvrir la session sans relire l'état local.
    ///
    /// **Ne remplace pas l'enregistrement auprès de la politique.**
    /// [`PolicyGate`] n'expose aucune méthode d'enregistrement — c'est une
    /// frontière, pas un registre —, donc `oxyn-desktop` appelle aussi
    /// [`DefaultPolicy::register`](oxyn_core::DefaultPolicy::register).
    pub fn register_connection(&self, config: &ConnectionConfig) {
        let _ordered = self.connection_writes.lock();
        self.connections.write().insert(config.id, config.clone());
    }

    /// Oublie une connexion.
    pub fn forget_connection(&self, connection: ConnectionId) {
        let _ordered = self.connection_writes.lock();
        self.connections.write().remove(&connection);
    }

    /// Charge dans l'ordonnanceur les connexions d'un workspace, et rend leur
    /// nombre.
    ///
    /// # Erreurs
    /// Celles de l'état local.
    pub fn load_connections(&self) -> Result<usize> {
        let _ordered = self.connection_writes.lock();
        let configs = self.store.connections().list(self.workspace)?;
        let mut guard = self.connections.write();
        for config in &configs {
            guard.insert(config.id, config.clone());
        }
        Ok(configs.len())
    }

    /// La configuration d'une connexion, du cache ou de l'état local.
    async fn connection_config(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<ConnectionConfig> {
        // Read in its own statement: the guard is released before the `.await`
        // below, never held across it.
        let cached = self.connections.read().get(&connection).cloned();
        if let Some(config) = cached {
            return Ok(config);
        }
        let store = Arc::clone(&self.store);
        let connections = Arc::clone(&self.connections);
        let writes = Arc::clone(&self.connection_writes);
        // Read and cached under `connection_writes`: a delete that lands
        // between the two would otherwise be undone by re-caching its config.
        let found = self
            .local_worker(cancel, move |_cancel| {
                let _ordered = writes.lock();
                let found = store.connections().get(connection)?;
                if let Some(config) = &found {
                    connections.write().insert(config.id, config.clone());
                }
                Ok(found)
            })
            .await?;
        match found {
            Some(config) => Ok(config),
            None => Err(OxynError::Config(
                "this connection does not exist in the workspace".to_owned(),
            )),
        }
    }

    /// L'environnement à soumettre au `PolicyGate`.
    ///
    /// Une connexion que l'ordonnanceur ne connaît pas vaut
    /// [`Production`](Environment::Production) : c'est le même « fermé par
    /// défaut » que le gate lui-même, et ignorer un marquage inconnu
    /// reviendrait à traiter la production comme du local.
    fn environment_of(&self, command: &Command) -> Environment {
        match command {
            // La connexion n'est pas encore enregistrée : c'est sa propre
            // déclaration qui fait foi, et le gate la recoupera.
            Command::CreateConnection { config }
            | Command::UpdateConnection { config }
            | Command::TestConnection { config } => config.environment,
            autre => autre
                .target_connection()
                .and_then(|id| self.connections.read().get(&id).map(|c| c.environment))
                .unwrap_or_default(),
        }
    }

    // ── Accès ───────────────────────────────────────────────────────────────

    /// Le canal d'événements.
    #[must_use]
    pub const fn events(&self) -> &EventBus {
        &self.events
    }

    /// Ouvre un abonnement aux événements d'exécution.
    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<crate::ExecEvent> {
        self.events.subscribe()
    }

    /// Les commandes en attente d'accord.
    #[must_use]
    pub const fn approvals(&self) -> &ApprovalRegistry {
        &self.approvals
    }

    /// Les exécutions en cours.
    #[must_use]
    pub const fn running(&self) -> &CancelRegistry {
        &self.running
    }

    /// Les sessions ouvertes.
    #[must_use]
    pub const fn sessions(&self) -> &SessionRegistry {
        &self.sessions
    }

    /// L'état local.
    #[must_use]
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Le workspace courant.
    #[must_use]
    pub const fn workspace(&self) -> WorkspaceId {
        self.workspace
    }

    fn result_on_connection(
        &self,
        connection: ConnectionId,
        result: ResultId,
    ) -> Result<Arc<ResultBuffer>> {
        let results = self.results.read();
        let entry = results
            .get(&result)
            .ok_or_else(|| OxynError::Config("result is no longer available".into()))?;
        if entry.connection != connection {
            return Err(OxynError::PolicyDenied {
                reason: "result does not belong to this connection".into(),
            });
        }
        Ok(Arc::clone(&entry.buffer))
    }

    /// Enforces idle retention limits. Call off the UI thread: evictions can remove spill files.
    pub fn prune_results(&self) {
        let evicted = self.results.write().prune();
        drop(evicted);
    }

    /// Le tampon d'un résultat, tant qu'il est retenu.
    #[must_use]
    pub fn result(&self, result: ResultId) -> Option<Arc<ResultBuffer>> {
        self.results
            .read()
            .get(&result)
            .map(|entry| Arc::clone(&entry.buffer))
    }

    /// Oublie un résultat : l'onglet a été fermé.
    ///
    /// Le tampon n'est libéré que quand plus personne ne le tient — la grille
    /// peut être en train de le lire.
    pub fn forget_result(&self, result: ResultId) -> Option<Arc<ResultBuffer>> {
        self.results
            .write()
            .remove(&result)
            .map(|entry| entry.buffer)
    }

    /// Annule tout et ferme toutes les sessions.
    ///
    /// À appeler à la fermeture de l'application : sans cela, les requêtes en
    /// cours continuent côté serveur.
    ///
    /// Ne rend pas d'erreur — il n'y a plus rien à faire d'un échec de
    /// fermeture à cet instant : il est journalisé au niveau `warn`. Rend le
    /// nombre de sessions fermées.
    pub async fn shutdown(&self) -> usize {
        // Before closing sessions: a caller usually bounds this call, and a
        // server that never answers the close would cut it before the end —
        // taking with it every outcome already queued.
        self.journal_abandoned_off_runtime().await;
        for (_, catalog) in self.catalogs.write().drain() {
            catalog.closed.cancel();
        }
        let sessions = self.sessions.drain_all();
        for slot in &sessions {
            for entree in self.running.for_session(slot.id()) {
                entree.token().cancel();
            }
            if let Err(erreur) = slot.close().await {
                tracing::warn!(error = %erreur, "the server refused a clean session close");
            }
        }
        // Again, last: closing may have abandoned more, and nothing after this
        // point runs the periodic writer.
        self.journal_abandoned_off_runtime().await;
        sessions.len()
    }

    /// [`journal_abandoned`](Self::journal_abandoned) for an async caller.
    ///
    /// The queue is taken here, in memory; the write goes through
    /// [`write_audit`](Self::write_audit). Once taken, the records belong to
    /// the blocking task: dropping this future does not cancel it, so they
    /// are written even if the caller gives up.
    async fn journal_abandoned_off_runtime(&self) {
        let records = self.abandoned.take();
        if records.is_empty() {
            return;
        }
        if self
            .write_audit(move |store| append_outcomes(store, records))
            .await
            .is_err()
        {
            tracing::error!("the audit writer stopped during shutdown");
        }
    }

    /// Writes to the audit journal the outcomes of commands whose caller
    /// dropped them before they ended, and returns how many were written.
    ///
    /// Such a command has its policy decision journaled but never reaches the
    /// line that journals its outcome. Its record is built when its future is
    /// dropped — same constructor as an ordinary outcome, marked
    /// `abandoned by its caller; outcome unknown` and classed
    /// [`Ambiguous`](oxyn_core::ErrorClass::Ambiguous) — and waits in memory
    /// for this call.
    ///
    /// **Blocks on disk I/O**: call it from the blocking pool, next to
    /// [`prune_results`](Self::prune_results), never from the UI thread.
    /// [`shutdown`](Self::shutdown) calls it last.
    ///
    /// # Limits
    /// A queued outcome lives in memory only until this runs. **It is lost if
    /// the process dies before** — within the second that separates two calls
    /// of the periodic writer, or when an application exits without
    /// [`shutdown`](Self::shutdown). At most 256 outcomes wait at once; beyond,
    /// or if the queue is being emptied at the instant of the drop, the outcome
    /// is lost and `tracing::error!` reports the count, never the command.
    pub fn journal_abandoned(&self) -> usize {
        append_outcomes(&self.store, self.abandoned.take())
    }
}

/// Appends queued abandoned outcomes; blocks on disk. Returns how many were written.
fn append_outcomes(store: &Store, records: std::collections::VecDeque<JournalRecord>) -> usize {
    let mut written = 0;
    for record in records {
        match store.journal().append(&record) {
            Ok(_) => written += 1,
            Err(erreur) => tracing::error!(
                error = %erreur,
                command = ?record.command_id,
                "failed to journal the outcome of an abandoned command"
            ),
        }
    }
    written
}

/// Builds the audit record for a policy decision, without writing it.
///
/// Pure: no `Store` access, so the record it returns is safe to move into a
/// `spawn_blocking` closure ([`Executor::write_audit`]).
fn decision_record(
    id: CommandId,
    actor: &Actor,
    command: &Command,
    decision: &Decision,
    approved_by: Option<&str>,
) -> JournalRecord {
    let mut record = JournalRecord::new(actor, command, decision).with_command_id(id);
    if let Some(who) = approved_by {
        record = record.approved_by(who);
    }
    record
}

/// Builds the finished entry for [`Executor::history_record`]'s "in
/// progress" row, without writing it. `None` when nothing was started —
/// either the command carries no history entry, or starting it already
/// failed.
///
/// Pure, for the same reason as [`decision_record`].
fn finished_history_record(
    en_cours: Option<(i64, HistoryRecord)>,
    issue: &Result<Outcome>,
    duration: Duration,
) -> Option<(i64, HistoryRecord)> {
    let (id, record) = en_cours?;
    let mut record = match issue {
        // Un drainage interrompu rend `Ok` : la commande n'a pas échoué,
        // mais elle n'a pas rendu son résultat entier. La classer « réussie »
        // ferait lire un résultat tronqué comme un résultat complet.
        Ok(Outcome::Executed {
            stats,
            sink: SinkOutcome::Cancelled,
            ..
        }) => record
            .succeeded(duration, Some(stats.rows))
            .failed(&OxynError::Cancelled),
        Ok(outcome) => record.succeeded(duration, outcome.rows()),
        Err(erreur) => {
            // Un échec a une durée, lui aussi : « expiré après 30 s » et
            // « rejeté en 2 ms » ne décrivent pas le même incident.
            let mut echouee = record.failed(erreur);
            echouee.duration = Some(duration);
            echouee
        }
    };
    if let Ok(Outcome::Executed { result, .. }) = issue {
        record.result = Some(*result);
    }
    Some((id, record))
}

/// The audit record of a command that ran, before its failure is known.
///
/// The one constructor for outcomes — ordinary and abandoned — so that both
/// carry exactly the same fields, and no more.
pub(crate) fn outcome_record(
    id: CommandId,
    actor: &Actor,
    command: &Command,
    duration: Duration,
    rows: Option<u64>,
    approved_by: Option<&str>,
) -> JournalRecord {
    let decision = match approved_by {
        Some(_) => Decision::approval("command executed after explicit approval", None),
        None => Decision::Allow,
    };
    let record = JournalRecord::new(actor, command, &decision)
        .with_command_id(id)
        .completed(duration, rows);
    match approved_by {
        Some(who) => record.approved_by(who),
        None => record,
    }
}

impl fmt::Debug for Executor {
    /// Rend des compteurs, pas des contenus : ni configuration de connexion, ni
    /// texte de requête, ni identifiants ne doivent atterrir dans une trace
    /// (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Executor")
            .field("policy", &self.policy.name())
            .field("sessions", &self.sessions.len())
            .field("running", &self.running.len())
            .field("pending_approvals", &self.approvals.len())
            .field("results", &self.results.read().len())
            .finish_non_exhaustive()
    }
}

/// Ce qu'un événement de drainage doit nommer pour que l'interface sache
/// quel onglet mettre à jour.
///
/// Transportées ensemble parce qu'elles ne servent qu'ensemble — et parce que
/// les passer une par une ferait de la boucle de drainage une fonction à huit
/// paramètres, ce qui est le signe qu'on a manqué un type.
#[derive(Debug, Clone, Copy)]
struct Coordinates {
    /// La commande à l'origine de l'exécution.
    command: CommandId,
    /// La connexion visée.
    connection: ConnectionId,
    /// Le résultat alimenté.
    result: ResultId,
}

/// Reclassifie une commande d'exécution à partir de son seul texte.
///
/// L'intention et le risque portés par la commande viennent de l'appelant, et un
/// agent est un appelant : ils sont **remplacés**, jamais recoupés. Les autres
/// commandes traversent inchangées — leur intention est une propriété de leur
/// variante, pas une déclaration.
fn reclassified(command: Command) -> Command {
    match command {
        Command::Execute {
            connection,
            session,
            request,
        } => {
            let classification = oxyn_query::reclassify(&request);
            Command::Execute {
                connection,
                session,
                request: Box::new(classification.qualify(*request)),
            }
        }
        autre => autre,
    }
}

/// Le câblage d'un [`Executor`].
pub struct ExecutorBuilder {
    drivers: Arc<DriverRegistry>,
    store: Arc<Store>,
    policy: Arc<dyn PolicyGate>,
    credentials: Arc<dyn CredentialResolver>,
    approvals: ApprovalRegistry,
    events: EventBus,
    workspace: WorkspaceId,
    memory_budget: usize,
}

impl ExecutorBuilder {
    /// Câblage minimal : un état local et une politique.
    #[must_use]
    pub fn new(store: Arc<Store>, policy: Arc<dyn PolicyGate>) -> Self {
        Self {
            drivers: Arc::new(DriverRegistry::new()),
            store,
            policy,
            credentials: Arc::new(NoCredentials),
            approvals: ApprovalRegistry::new(),
            events: EventBus::new(),
            workspace: WorkspaceId::new(),
            memory_budget: DEFAULT_MEMORY_BUDGET,
        }
    }

    /// Les drivers compilés dans ce binaire.
    #[must_use]
    pub fn with_drivers(mut self, drivers: Arc<DriverRegistry>) -> Self {
        self.drivers = drivers;
        self
    }

    /// Ce qui résout les identifiants — `oxyn-secrets`, en production.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Arc<dyn CredentialResolver>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Le workspace dans lequel les connexions et documents sont écrits.
    #[must_use]
    pub fn with_workspace(mut self, workspace: WorkspaceId) -> Self {
        self.workspace = workspace;
        self
    }

    /// La durée de validité d'une demande d'approbation.
    #[must_use]
    pub fn with_approval_ttl(mut self, ttl: Duration) -> Self {
        self.approvals = ApprovalRegistry::with_ttl(ttl);
        self
    }

    /// Le budget mémoire d'un tampon de résultats, en octets.
    #[must_use]
    pub fn with_memory_budget(mut self, bytes: usize) -> Self {
        self.memory_budget = bytes;
        self
    }

    /// La profondeur du canal d'événements.
    #[must_use]
    pub fn with_event_capacity(mut self, capacity: usize) -> Self {
        self.events = EventBus::with_capacity(capacity);
        self
    }

    /// Construit l'ordonnanceur.
    #[must_use]
    pub fn build(self) -> Executor {
        Executor {
            drivers: self.drivers,
            store: self.store,
            policy: self.policy,
            credentials: self.credentials,
            sessions: SessionRegistry::new(),
            running: CancelRegistry::new(),
            approvals: self.approvals,
            events: self.events,
            results: RwLock::new(crate::retained::RetainedResults::default()),
            catalogs: RwLock::new(HashMap::new()),
            connections: Arc::new(RwLock::new(HashMap::new())),
            connection_writes: Arc::new(Mutex::new(())),
            workspace: self.workspace,
            memory_budget: self.memory_budget,
            abandoned: AbandonedOutcomes::default(),
            started_at: chrono::Utc::now(),
        }
    }
}

impl fmt::Debug for ExecutorBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutorBuilder")
            .field("policy", &self.policy.name())
            .field("drivers", &self.drivers.len())
            .field("memory_budget", &self.memory_budget)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures::executor::block_on;
    use oxyn_core::{
        AgentId, AgentSessionId, Capabilities, DefaultPolicy, DriverId, ErrorClass, ExecLimits,
        MutationRisk, QueryLanguage, SqlDialect, StatementIntent,
    };
    use oxyn_store::{ActorKind, HistoryStatus, PolicyOutcome};

    /// Un banc complet : état local en mémoire, politique par défaut, aucune
    /// session ouverte. Aucun driver n'est enregistré — les tests qui suivent
    /// portent sur ce qui se passe **avant** qu'un driver ne soit atteint, et
    /// c'est précisément ce qui compte.
    struct Banc {
        executeur: Executor,
        politique: Arc<DefaultPolicy>,
        store: Arc<Store>,
    }

    impl Banc {
        fn new(connexion: &ConnectionConfig) -> Self {
            let store = Arc::new(Store::open_in_memory().expect("état local en mémoire"));
            let atelier = store
                .workspaces()
                .create("tests")
                .expect("création du workspace");
            store
                .connections()
                .save(atelier.id, connexion)
                .expect("enregistrement de la connexion");

            let politique = Arc::new(DefaultPolicy::new());
            politique.register(connexion);

            // `politique.clone()` et non `Arc::clone(&politique)` : la forme
            // fonction résout `T` depuis le type attendu — donc `dyn PolicyGate` —
            // et réclame un `&Arc<dyn PolicyGate>` avant toute coercition. En
            // syntaxe de méthode, `T` vient du receveur, et l'`Arc<DefaultPolicy>`
            // obtenu se coerce à l'affectation.
            let gate: Arc<dyn PolicyGate> = politique.clone();
            let executeur = Executor::builder(Arc::clone(&store), gate)
                .with_workspace(atelier.id)
                .build();
            executeur.register_connection(connexion);

            Self {
                executeur,
                politique,
                store,
            }
        }
    }

    fn agent() -> Actor {
        Actor::agent(AgentId::new(), AgentSessionId::new())
    }

    fn execution(connexion: ConnectionId, texte: &str, intent: StatementIntent) -> Command {
        Command::Execute {
            connection: connexion,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), texte)
                    .with_intent(intent)
                    .with_limits(
                        ExecLimits::default()
                            .writable()
                            .with_timeout(None::<Duration>),
                    ),
            ),
        }
    }

    // ── Les deux tests qui protègent la promesse du produit ─────────────────

    /// **Un agent ne supprime rien en production.**
    ///
    /// L'agent déclare une lecture ; le texte dit autre chose. La
    /// reclassification a lieu *avant* le gate, donc le gate voit un `DELETE`
    /// non borné sur une connexion de production, et refuse — un refus, pas une
    /// confirmation renforcée (I-02, ADR-0004).
    ///
    /// La preuve que la reclassification a bien eu lieu n'est pas dans le
    /// refus : elle est dans le **journal**, qui a consigné l'intention
    /// `write`, pas celle que l'agent avait déclarée.
    #[test]
    fn un_agent_ne_peut_pas_supprimer_sur_une_connexion_de_production() {
        let connexion = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let banc = Banc::new(&connexion);
        let mut evenements = banc.executeur.subscribe();

        // L'agent s'auto-déclare en lecture seule.
        let commande = execution(
            connexion.id,
            "DELETE FROM clients WHERE 1=1",
            StatementIntent::Read,
        );

        let issue = block_on(
            banc.executeur
                .dispatch(agent(), commande, &CancelToken::new()),
        )
        .expect("un refus n'est pas une panne");

        let Outcome::Denied { command, reason } = issue else {
            panic!("un agent doit être refusé sur une connexion de production : {issue:?}");
        };
        assert!(
            reason.contains("production"),
            "le motif doit nommer la production : {reason}"
        );

        // Rien n'attend d'accord : c'est un refus, pas une confirmation.
        assert!(
            banc.executeur.approvals().is_empty(),
            "un refus ne met rien en attente d'approbation"
        );

        // Le journal a vu l'intention **réelle**, pas celle qui a été déclarée.
        let trace = banc
            .store
            .journal()
            .recent(1)
            .expect("relecture du journal")
            .pop()
            .expect("une entrée");
        assert_eq!(trace.record.command_id, Some(command));
        assert_eq!(trace.record.actor_kind, ActorKind::Agent);
        assert_eq!(trace.record.decision, PolicyOutcome::Denied);
        assert_eq!(
            trace.record.intent,
            StatementIntent::Write,
            "l'intention déclarée par l'agent doit avoir été écrasée"
        );
        assert_eq!(trace.record.risk, MutationRisk::UnboundedDelete);
        assert_eq!(
            trace.record.statement.as_deref(),
            Some("DELETE FROM clients WHERE 1=1")
        );

        // Et l'interface l'apprend par le canal, pas en scrutant un état.
        let recu = evenements.try_recv().expect("un événement a été émis");
        assert_eq!(recu.command, command);
        assert!(matches!(recu.event, Event::Failed { .. }), "{recu:?}");
    }

    /// **Une commande refusée laisse une trace.**
    ///
    /// Un journal qui ne consigne que ce qui a marché ne dit rien de ce qui a
    /// été tenté — et c'est exactement ce qu'un audit cherche.
    #[test]
    fn le_journal_contient_une_entree_meme_quand_la_commande_est_refusee() {
        // Connexion en lecture seule : le refus vaut pour un humain aussi.
        let connexion = ConnectionConfig::new("réplica", DriverId::postgres())
            .with_environment(Environment::Local)
            .read_only();
        let banc = Banc::new(&connexion);

        assert_eq!(
            banc.store.journal().count().expect("comptage"),
            0,
            "le journal part vide"
        );

        let commande = execution(connexion.id, "DROP TABLE clients", StatementIntent::Unknown);
        let issue = block_on(
            banc.executeur
                .dispatch(Actor::Human, commande, &CancelToken::new()),
        )
        .expect("un refus n'est pas une panne");
        assert!(issue.is_denied(), "{issue:?}");
        assert!(!issue.took_effect());

        assert_eq!(
            banc.store.journal().count().expect("comptage"),
            1,
            "une commande refusée est journalisée comme les autres"
        );
        let trace = banc
            .store
            .journal()
            .recent(1)
            .expect("relecture")
            .pop()
            .expect("une entrée");
        assert_eq!(trace.record.decision, PolicyOutcome::Denied);
        assert_eq!(trace.record.actor_kind, ActorKind::Human);
        assert_eq!(trace.record.command_kind, "Execute");
        assert_eq!(
            trace.record.statement.as_deref(),
            Some("DROP TABLE clients")
        );
        assert!(
            trace
                .record
                .decision_reason
                .as_deref()
                .is_some_and(|motif| motif.contains("read-only")),
            "{:?}",
            trace.record.decision_reason
        );

        // Et la trace survit à ce que l'utilisateur peut effacer.
        banc.store.history().clear().expect("purge de l'historique");
        assert_eq!(banc.store.journal().count().expect("comptage"), 1);
    }

    // ── Le reste de la séquence ─────────────────────────────────────────────

    #[test]
    fn une_ecriture_d_agent_hors_production_attend_un_accord_et_n_execute_rien() {
        let connexion = ConnectionConfig::new("atelier", DriverId::postgres())
            .with_environment(Environment::Development);
        let banc = Banc::new(&connexion);

        let commande = execution(
            connexion.id,
            "UPDATE clients SET actif = true WHERE id = 1",
            StatementIntent::Read,
        );
        let issue = block_on(
            banc.executeur
                .dispatch(agent(), commande, &CancelToken::new()),
        )
        .expect("une demande d'accord n'est pas une panne");

        let Outcome::NeedsApproval {
            command, preview, ..
        } = issue
        else {
            panic!("une écriture d'agent doit demander un accord : {issue:?}");
        };

        // La prévisualisation nomme la connexion, jamais son identifiant.
        let preview = preview.expect("une prévisualisation");
        assert_eq!(preview.connection, "atelier");
        assert!(!preview.connection.contains(&connexion.id.to_string()));

        // Rien n'a été exécuté : aucune session n'a même été cherchée.
        assert_eq!(banc.executeur.approvals().len(), 1);
        assert!(banc.executeur.running().is_empty());

        // Et la trace de la demande est déjà au journal.
        let trace = banc
            .store
            .journal()
            .recent(1)
            .expect("relecture")
            .pop()
            .expect("une entrée");
        assert_eq!(trace.record.decision, PolicyOutcome::ApprovalRequired);
        assert_eq!(trace.record.command_id, Some(command));
    }

    #[test]
    fn un_accord_inconnu_ou_perime_n_execute_rien() {
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let banc = Banc::new(&connexion);

        let issue = block_on(banc.executeur.approve(
            "nicolas",
            CommandId::new(),
            &CancelToken::new(),
        ));
        let erreur = issue.expect_err("un accord sans objet est refusé");
        assert!(
            matches!(erreur, OxynError::PolicyDenied { .. }),
            "{erreur:?}"
        );
        assert_eq!(
            banc.store.journal().count().expect("comptage"),
            0,
            "rien n'a traversé le gate : il n'y a rien à journaliser"
        );
    }

    #[test]
    fn un_accord_ne_survit_pas_a_un_marquage_devenu_plus_strict() {
        // La connexion est ouverte quand l'accord est demandé, marquée lecture
        // seule quand il est donné. Le refus tardif l'emporte sur l'accord.
        let connexion = ConnectionConfig::new("atelier", DriverId::postgres())
            .with_environment(Environment::Development);
        let banc = Banc::new(&connexion);

        let commande = execution(
            connexion.id,
            "UPDATE clients SET actif = true WHERE id = 1",
            StatementIntent::Write,
        );
        let issue = block_on(
            banc.executeur
                .dispatch(agent(), commande, &CancelToken::new()),
        )
        .expect("demande d'accord");
        let Outcome::NeedsApproval { command, .. } = issue else {
            panic!("{issue:?}");
        };

        // Entre-temps, l'utilisateur marque la connexion en lecture seule.
        let stricte = connexion.clone().read_only();
        banc.politique.register(&stricte);
        banc.executeur.register_connection(&stricte);

        let issue = block_on(
            banc.executeur
                .approve("nicolas", command, &CancelToken::new()),
        )
        .expect("un refus tardif n'est pas une panne");
        assert!(issue.is_denied(), "{issue:?}");
    }

    #[test]
    fn une_ecriture_sous_des_limites_en_lecture_seule_est_refusee() {
        // `ExecLimits::default()` est en lecture seule : écrire est toujours
        // une demande explicite. C'est la dernière barrière avant le driver.
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let banc = Banc::new(&connexion);

        let commande = Command::Execute {
            connection: connexion.id,
            session: SessionId::new(),
            request: Box::new(ExecRequest::new(
                QueryLanguage::Sql(SqlDialect::Sqlite),
                "INSERT INTO clients (nom) VALUES ('x')",
            )),
        };

        let erreur = block_on(
            banc.executeur
                .dispatch(Actor::Human, commande, &CancelToken::new()),
        )
        .expect_err("l'incohérence est tranchée du côté prudent");
        assert!(
            matches!(erreur, OxynError::PolicyDenied { .. }),
            "{erreur:?}"
        );

        // Deux entrées : la décision de politique, puis l'issue.
        assert_eq!(banc.store.journal().count().expect("comptage"), 2);
    }

    #[test]
    fn une_lecture_locale_ne_demande_rien_et_echoue_faute_de_session() {
        // Le gate autorise ; l'exécution échoue parce qu'aucune session n'est
        // ouverte. Ce que ce test vérifie, c'est que l'échec arrive **après** le
        // gate, et que les deux entrées de journal sont écrites.
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let banc = Banc::new(&connexion);

        let commande = execution(connexion.id, "SELECT 1", StatementIntent::Unknown);
        let erreur = block_on(
            banc.executeur
                .dispatch(Actor::Human, commande, &CancelToken::new()),
        )
        .expect_err("aucune session n'est ouverte");
        assert!(matches!(erreur, OxynError::Connection(_)), "{erreur:?}");

        let entrees = banc.store.journal().recent(2).expect("relecture");
        assert_eq!(entrees.len(), 2, "décision avant, issue après");
        assert!(
            entrees
                .iter()
                .all(|e| e.record.decision == PolicyOutcome::Allowed),
            "la lecture a bien été autorisée"
        );
        assert!(
            entrees.iter().any(|e| e.record.error.is_some()),
            "l'échec d'exécution figure au journal"
        );
    }

    #[test]
    fn une_connexion_inconnue_de_l_ordonnanceur_vaut_production() {
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let banc = Banc::new(&connexion);
        banc.executeur.forget_connection(connexion.id);

        let commande = execution(connexion.id, "SELECT 1", StatementIntent::Read);
        assert_eq!(
            banc.executeur.environment_of(&commande),
            Environment::Production
        );
    }

    #[test]
    fn le_debug_de_l_ordonnanceur_ne_montre_aucun_contenu() {
        let connexion = ConnectionConfig::new("base client", DriverId::postgres())
            .with_param("host", "interne.example");
        let banc = Banc::new(&connexion);
        let rendu = format!("{:?}", banc.executeur);
        assert!(!rendu.contains("interne.example"), "{rendu}");
        assert!(rendu.contains("DefaultPolicy"), "{rendu}");
    }

    #[test]
    fn une_annulation_est_toujours_autorisee() {
        // Refuser une annulation ne protège rien et laisse une requête tourner.
        let connexion = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let banc = Banc::new(&connexion);

        let issue = block_on(banc.executeur.dispatch(
            agent(),
            Command::Cancel {
                connection: connexion.id,
                statement: StatementHandle::new(),
            },
            &CancelToken::new(),
        ))
        .expect("annuler ne se refuse pas");

        let Outcome::Cancelled { report } = issue else {
            panic!("{issue:?}");
        };
        assert!(!report.was_running, "l'exécution visée n'existe pas");
    }

    // ── L'historique du requêteur ───────────────────────────────────────────

    /// Une session qui rend un seul lot de deux lignes, sans serveur.
    ///
    /// Elle ne sert qu'à prouver le chemin nominal de l'historique : sans
    /// exécution qui aboutit, la ligne `succeeded` n'existe dans aucun test, et
    /// c'est exactement celle qui manquait jusqu'ici.
    struct SessionFactice;

    #[async_trait::async_trait]
    impl oxyn_driver::Session for SessionFactice {
        fn capabilities(&self) -> Capabilities {
            Capabilities::SQL | Capabilities::TABLES
        }
        async fn execute(&self, _: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
            Ok(Box::new(CurseurFactice {
                handle: StatementHandle::new(),
                rendu: false,
                stats: ExecStats::default(),
            }))
        }
        async fn cancel(&self, _: StatementHandle) -> Result<()> {
            Ok(())
        }
        fn catalog(&self) -> &dyn oxyn_catalog::CatalogProvider {
            unreachable!("les tests d'historique n'introspectent rien")
        }
        async fn ping(&self) -> Result<Duration> {
            Ok(Duration::ZERO)
        }
        async fn close(self: Box<Self>) -> Result<()> {
            Ok(())
        }
    }

    struct CurseurFactice {
        handle: StatementHandle,
        rendu: bool,
        stats: ExecStats,
    }

    fn schema_factice() -> arrow::datatypes::SchemaRef {
        Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int32, false),
        ]))
    }

    #[async_trait::async_trait]
    impl Cursor for CurseurFactice {
        fn handle(&self) -> StatementHandle {
            self.handle
        }
        fn schema(&self) -> arrow::datatypes::SchemaRef {
            schema_factice()
        }
        async fn next_batch(&mut self) -> Result<Option<arrow::record_batch::RecordBatch>> {
            if self.rendu {
                return Ok(None);
            }
            self.rendu = true;
            let lot = arrow::record_batch::RecordBatch::try_new(
                schema_factice(),
                vec![Arc::new(arrow::array::Int32Array::from(vec![1, 2]))],
            )
            .expect("la colonne correspond au schéma construit juste au-dessus");
            self.stats.record_batch(2, 0);
            Ok(Some(lot))
        }
        fn stats(&self) -> ExecStats {
            self.stats
        }
    }

    /// **Une exécution qui aboutit laisse une trace complète.**
    ///
    /// Durée et lignes comprises : un historique qui ne dit pas combien de
    /// lignes une requête a rendues ne répond pas à la question qu'on lui pose.
    #[test]
    fn une_execution_reussie_est_inscrite_avec_sa_duree_et_ses_lignes() {
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let banc = Banc::new(&connexion);
        let session = banc
            .executeur
            .sessions
            .insert(SessionSlot::new(connexion.id, Box::new(SessionFactice)));

        let commande = Command::Execute {
            connection: connexion.id,
            session: session.id(),
            request: Box::new(
                ExecRequest::new(
                    QueryLanguage::Sql(SqlDialect::Sqlite),
                    "SELECT id FROM clients",
                )
                .with_intent(StatementIntent::Read)
                // Sans délai : `block_on` n'a pas d'horloge tokio, et le
                // délai par défaut en réclamerait une.
                .with_limits(ExecLimits::default().with_timeout(None::<Duration>)),
            ),
        };
        block_on(
            banc.executeur
                .dispatch(Actor::Human, commande, &CancelToken::new()),
        )
        .expect("l'exécution aboutit");

        let entree = banc
            .store
            .history()
            .recent(10)
            .expect("relecture de l'historique")
            .pop()
            .expect("une exécution laisse une entrée");
        assert_eq!(entree.record.status, HistoryStatus::Succeeded);
        assert_eq!(entree.record.rows, Some(2));
        assert!(entree.record.duration.is_some(), "une durée est mesurée");
        assert_eq!(entree.record.statement, "SELECT id FROM clients");
        // Le nom est recopié pour survivre à la suppression de la connexion.
        assert_eq!(entree.record.connection_name.as_deref(), Some("atelier"));
        assert!(entree.record.error.is_none());
    }

    /// **Une exécution qui échoue laisse l'erreur, pas un silence.**
    ///
    /// Avec sa **famille**, qui est la donnée dont dépend le droit de rejouer :
    /// c'est elle qui interdira d'offrir « relancer » sur un `INSERT` dont
    /// l'effet côté serveur est inconnu (I-13).
    #[test]
    fn un_echec_est_inscrit_avec_son_erreur_et_sa_famille() {
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let banc = Banc::new(&connexion);

        // Aucune session n'est ouverte : l'exécution échoue avant le driver.
        let commande = execution(connexion.id, "SELECT 1", StatementIntent::Read);
        block_on(
            banc.executeur
                .dispatch(Actor::Human, commande, &CancelToken::new()),
        )
        .expect_err("aucune session sous cet identifiant");

        let entree = banc
            .store
            .history()
            .recent(10)
            .expect("relecture")
            .pop()
            .expect("un échec laisse une entrée");
        assert_eq!(entree.record.status, HistoryStatus::Failed);
        assert!(entree.record.error.is_some(), "l'erreur est conservée");
        assert!(entree.record.duration.is_some(), "un échec a une durée");
        // La famille est retenue en tant que donnée, jamais déduite du message.
        assert_eq!(
            entree.record.error_class,
            Some(ErrorClass::Transient),
            "une session absente se rouvre : c'est transitoire"
        );
    }

    /// **Un refus figure à l'historique, avec son motif.**
    ///
    /// Un historique qui ne montre que ce qui a marché laisse l'utilisateur
    /// chercher une requête qu'il a bel et bien lancée.
    #[test]
    fn un_refus_est_inscrit_avec_son_motif_et_sans_valeur_liee() {
        let connexion = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let banc = Banc::new(&connexion);

        // Le secret voyage en valeur liée, jamais dans le texte (I-03).
        let commande = Command::Execute {
            connection: connexion.id,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(
                    QueryLanguage::Sql(SqlDialect::Postgres),
                    "DELETE FROM clients WHERE jeton = $1",
                )
                .with_intent(StatementIntent::Read)
                .with_params(vec![oxyn_core::ScalarValue::Text(
                    "hunter2-le-secret".to_owned(),
                )])
                .with_limits(ExecLimits::default().writable()),
            ),
        };
        let issue = block_on(
            banc.executeur
                .dispatch(agent(), commande, &CancelToken::new()),
        )
        .expect("un refus n'est pas une panne");
        assert!(issue.is_denied(), "{issue:?}");

        let entrees = banc.store.history().recent(10).expect("relecture");
        assert_eq!(entrees.len(), 1, "un refus, une ligne — pas deux");
        let record = &entrees[0].record;
        assert_eq!(record.status, HistoryStatus::Denied);
        assert!(
            record
                .error
                .as_deref()
                .is_some_and(|motif| motif.contains("production")),
            "{:?}",
            record.error
        );
        // L'intention inscrite est celle que le texte porte, pas celle que
        // l'agent a déclarée.
        assert_eq!(record.intent, StatementIntent::Write);
        assert!(
            !format!("{record:?}").contains("hunter2"),
            "aucune valeur liée ne rejoint l'historique (I-03)"
        );
    }

    /// **L'historique ne consigne que des exécutions.**
    ///
    /// Une `Connect` ou une `Cancel` au milieu des requêtes rendrait la liste
    /// illisible ; elles restent au journal, qui les consigne toutes.
    #[test]
    fn une_commande_qui_n_est_pas_une_execution_ne_touche_pas_l_historique() {
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let banc = Banc::new(&connexion);

        block_on(banc.executeur.dispatch(
            Actor::Human,
            Command::Cancel {
                connection: connexion.id,
                statement: StatementHandle::new(),
            },
            &CancelToken::new(),
        ))
        .expect("annuler ne se refuse pas");

        assert_eq!(banc.store.history().count().expect("comptage"), 0);
        assert!(
            banc.store.journal().count().expect("comptage") > 0,
            "le journal, lui, les consigne toutes"
        );
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
pub(crate) mod catalog_tests;

#[cfg(test)]
#[path = "preview_tests.rs"]
mod preview_tests;

#[cfg(test)]
#[path = "result_page_tests.rs"]
mod result_page_tests;

#[cfg(test)]
#[path = "preference_tests.rs"]
mod preference_tests;

#[cfg(test)]
#[path = "library_tests.rs"]
mod library_tests;

#[cfg(test)]
#[path = "session_close_tests.rs"]
mod session_close_tests;

#[cfg(test)]
#[path = "provider_tests.rs"]
mod provider_tests;

#[cfg(test)]
#[path = "abandon_tests.rs"]
mod abandon_tests;

#[cfg(test)]
#[path = "connection_tests.rs"]
mod connection_tests;
