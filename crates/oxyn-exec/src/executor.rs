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
//!    [`ResultBuffer`](oxyn_data::ResultBuffer) par un
//!    [`BatchSink`](oxyn_data::BatchSink) — donc avec contre-pression et
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
//! `oxyn-exec` ne possède pas de runtime : `tokio` n'est au contrat de
//! dépendances qu'avec `sync`, `time` et `macros`. Il n'y a donc ni
//! `spawn_blocking` ni `spawn` ici, et les appels à `oxyn-store` — SQLite local,
//! de l'ordre de la dizaine de microsecondes — sont faits en ligne.
// TODO(phase 1) : porter les accès au `Store` et l'écriture d'export sur le pool
// bloquant, dès que `oxyn-exec` reçoit une poignée de runtime. Aujourd'hui ils
// s'exécutent sur le thread appelant du runtime, ce qui est acceptable pour du
// SQLite local et ne l'est plus pour un export de dix gigaoctets.

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxyn_core::{
    Actor, CancelToken, Command, CommandId, ConnectionConfig, ConnectionId, Decision, DocumentId,
    Environment, Event, ExecRequest, ExecStats, ExportFormat, OxynError, PolicyGate, Preview,
    Result, ResultId, SessionId, StatementHandle, WorkspaceId,
};
use oxyn_data::{
    BatchProgress, BatchSink, BatchSource, BufferLimits, DEFAULT_MEMORY_BUDGET, ExportOptions,
    ResultBuffer, SinkOutcome, export,
};
use oxyn_driver::{Cursor, DriverRegistry};
use oxyn_store::{Document, JournalRecord, Store};
use parking_lot::RwLock;

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
    Disconnected {
        /// La connexion.
        connection: ConnectionId,
        /// Combien de sessions ont été fermées.
        closed: usize,
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

    /// Une connexion a été enregistrée ou modifiée.
    ConnectionSaved {
        /// La connexion.
        connection: ConnectionId,
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
    results: RwLock<HashMap<ResultId, Arc<ResultBuffer>>>,
    connections: RwLock<HashMap<ConnectionId, ConnectionConfig>>,
    workspace: WorkspaceId,
    memory_budget: usize,
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

        // 4. Journal AVANT. Une décision qui ne s'écrit pas ne s'exécute pas.
        if let Err(erreur) = self.journal_decision(id, &actor, &command, &decision, None) {
            tracing::error!(error = %erreur, command = %id, "policy decision could not be journaled");
            if !decision.is_denied() {
                return Err(erreur);
            }
        }

        match decision {
            Decision::Deny { reason } => {
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

            Decision::Allow => self.run(id, &actor, command, cancel, None).await,
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
            // Le refus tardif est journalisé lui aussi : c'est même la trace la
            // plus intéressante de toutes, puisqu'un accord avait été donné.
            if let Err(erreur) =
                self.journal_decision(command, &attente.actor, &attente.command, &decision, None)
            {
                tracing::error!(error = %erreur, command = %command, "late denial could not be journaled");
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

        self.journal_decision(
            command,
            &attente.actor,
            &attente.command,
            &decision,
            Some(approved_by),
        )?;

        let PendingCommand {
            actor,
            command: a_executer,
            ..
        } = attente;
        self.run(command, &actor, a_executer, cancel, Some(approved_by))
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

    /// Exécute une commande autorisée, et journalise son issue.
    async fn run(
        &self,
        id: CommandId,
        actor: &Actor,
        command: Command,
        cancel: &CancelToken,
        approved_by: Option<&str>,
    ) -> Result<Outcome> {
        let debut = Instant::now();
        let issue = self.execute_command(id, &command, cancel).await;
        self.journal_result(id, actor, &command, &issue, debut.elapsed(), approved_by);
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

            Command::Disconnect { connection } => self.disconnect(*connection).await,

            Command::Execute {
                connection,
                session,
                request,
            } => {
                self.execute_statement(id, *connection, *session, (**request).clone(), cancel)
                    .await
            }

            Command::Cancel { statement, .. } => {
                let report = self.running.cancel(&self.sessions, *statement).await;
                Ok(Outcome::Cancelled { report })
            }

            // TODO(phase 1) : câbler sur le cache de `oxyn-catalog`. Refuser
            // franchement plutôt que de rendre un succès qui n'a rien relu :
            // une arborescence qui prétend être à jour est pire qu'une
            // arborescence qui dit ne pas savoir.
            Command::RefreshCatalog { .. } => Err(OxynError::NotSupported {
                capability: "catalog refresh".to_owned(),
            }),

            Command::Export {
                result,
                format,
                destination,
                ..
            } => self.export_result(*result, *format, destination, cancel),

            Command::OpenDocument { document, .. } => self.open_document(*document),

            Command::WriteDocument { document, text, .. } => self.write_document(*document, text),

            Command::CreateConnection { config } | Command::UpdateConnection { config } => {
                self.save_connection(config)
            }

            Command::DeleteConnection { connection } => self.delete_connection(*connection).await,
        }
    }

    /// Ouvre une session.
    async fn connect(&self, connection: ConnectionId, cancel: &CancelToken) -> Result<Outcome> {
        let config = self.connection_config(connection)?;
        let driver = self.drivers.require(&config.driver)?;
        let credentials = self.credentials.resolve(&config)?;
        let session = driver.connect(&config, &credentials, cancel).await?;
        let slot = self.sessions.insert(SessionSlot::new(connection, session));
        Ok(Outcome::Connected {
            connection,
            session: slot.id(),
        })
    }

    /// Ferme les sessions d'une connexion, après avoir annulé ce qui y tourne.
    async fn disconnect(&self, connection: ConnectionId) -> Result<Outcome> {
        // L'annulation d'abord : fermer sans annuler laisse les requêtes tourner
        // côté serveur, connexion prise et verrou posé.
        self.running
            .cancel_connection(&self.sessions, connection)
            .await;

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

    /// Exécute une instruction et draine son curseur dans un tampon.
    async fn execute_statement(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        request: ExecRequest,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        // `ExecLimits` par défaut interdit l'écriture : une demande mutante qui
        // n'a pas explicitement levé cette borne est incohérente, et
        // l'incohérence se tranche du côté prudent. C'est la dernière barrière
        // avant le driver.
        if request.is_mutating() && request.limits.read_only {
            return Err(OxynError::PolicyDenied {
                reason: "cette exécution est bornée en lecture seule : \
                         une écriture doit lever la borne explicitement"
                    .to_owned(),
            });
        }

        let Some(slot) = self.sessions.get(session) else {
            return Err(OxynError::Connection(
                "aucune session ouverte sous cet identifiant".to_owned(),
            ));
        };
        if slot.connection() != connection {
            return Err(OxynError::Internal(
                "la session visée n'appartient pas à la connexion de la commande".to_owned(),
            ));
        }

        let limits = request.limits.clone();
        // Un jeton **fils** : annuler cette exécution n'annule pas l'onglet qui
        // l'a lancée, alors qu'annuler l'onglet l'annule bien.
        let ct = cancel.child();

        let cursor = slot.execute(request, &ct).await?;
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
                .with_max_rows(limits.max_rows),
        ));
        self.results.write().insert(result, Arc::clone(&buffer));

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
            )
            .await;

        // Un abandon — expiration ou annulation — doit atteindre le serveur.
        let interrompue = matches!(issue, Ok(SinkOutcome::Cancelled))
            || matches!(issue, Err(OxynError::Timeout { .. }));
        if interrompue {
            self.running.cancel(&self.sessions, statement).await;
        }
        self.running.finish(statement);

        match issue {
            Ok(sink) => {
                let stats = buffer.stats();
                let event = if sink == SinkOutcome::Cancelled {
                    Event::Cancelled
                } else {
                    Event::Completed { result, stats }
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
    ) -> Result<SinkOutcome> {
        let sink = BatchSink::new(Arc::clone(buffer));
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

    /// Écrit un résultat dans un fichier.
    ///
    /// Synchrone : voir la note de module sur l'absence de pool bloquant.
    fn export_result(
        &self,
        result: ResultId,
        format: ExportFormat,
        destination: &Path,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let buffer = self
            .result(result)
            .ok_or_else(|| OxynError::Config("ce résultat n'est plus disponible".to_owned()))?;

        let fichier = File::create(destination)?;
        let resume = export(
            &buffer,
            format,
            BufWriter::new(fichier),
            &ExportOptions::default(),
            cancel,
        )?;

        Ok(Outcome::Exported {
            result,
            rows: resume.rows,
            bytes: resume.bytes,
        })
    }

    /// Relit un document du workspace.
    fn open_document(&self, document: DocumentId) -> Result<Outcome> {
        match self.store.documents().get(document)? {
            Some(doc) => Ok(Outcome::DocumentOpened {
                document: Box::new(doc),
            }),
            None => Err(OxynError::Config(
                "ce document n'existe pas dans le workspace".to_owned(),
            )),
        }
    }

    /// Écrit un document du workspace.
    ///
    /// N'atteint aucune base : c'est un fichier local, et le `PolicyGate` le
    /// traite comme tel.
    fn write_document(&self, document: DocumentId, text: &str) -> Result<Outcome> {
        let documents = self.store.documents();
        let Some(mut doc) = documents.get(document)? else {
            return Err(OxynError::Config(
                "ce document n'existe pas dans le workspace".to_owned(),
            ));
        };
        doc.content = text.to_owned();
        documents.save(&doc)?;
        Ok(Outcome::DocumentWritten { document })
    }

    /// Enregistre ou met à jour une connexion.
    fn save_connection(&self, config: &ConnectionConfig) -> Result<Outcome> {
        // `Connections::save` **refuse** un paramètre portant un nom de secret :
        // c'est le dernier point où un mot de passe peut être arrêté avant le
        // disque (I-03). Rien n'est dupliqué ici.
        self.store.connections().save(self.workspace, config)?;
        self.register_connection(config);
        Ok(Outcome::ConnectionSaved {
            connection: config.id,
        })
    }

    /// Supprime une connexion, après avoir fermé ce qui l'utilisait.
    async fn delete_connection(&self, connection: ConnectionId) -> Result<Outcome> {
        self.running
            .cancel_connection(&self.sessions, connection)
            .await;
        for slot in self.sessions.drain_connection(connection) {
            if let Err(erreur) = slot.close().await {
                tracing::warn!(error = %erreur, "the server refused a clean session close");
            }
        }
        let existed = self.store.connections().delete(connection)?;
        self.connections.write().remove(&connection);
        Ok(Outcome::ConnectionDeleted {
            connection,
            existed,
        })
    }

    // ── Journal ─────────────────────────────────────────────────────────────

    /// Écrit la décision de politique, **avant** toute exécution.
    ///
    /// # Erreurs
    /// Celles de l'état local. L'appelant en fait un refus d'exécuter.
    fn journal_decision(
        &self,
        id: CommandId,
        actor: &Actor,
        command: &Command,
        decision: &Decision,
        approved_by: Option<&str>,
    ) -> Result<()> {
        let mut record = JournalRecord::new(actor, command, decision).with_command_id(id);
        if let Some(who) = approved_by {
            record = record.approved_by(who);
        }
        self.store.journal().append(&record)?;
        Ok(())
    }

    /// Écrit l'issue de l'exécution, **après** coup.
    ///
    /// Ne rend pas d'erreur : la commande a eu lieu, et la faire échouer
    /// maintenant laisserait croire le contraire. L'échec est crié.
    fn journal_result(
        &self,
        id: CommandId,
        actor: &Actor,
        command: &Command,
        issue: &Result<Outcome>,
        duration: Duration,
        approved_by: Option<&str>,
    ) {
        let decision = match approved_by {
            Some(_) => Decision::approval("commande exécutée après accord explicite", None),
            None => Decision::Allow,
        };
        let mut record = JournalRecord::new(actor, command, &decision)
            .with_command_id(id)
            .completed(duration, issue.as_ref().ok().and_then(Outcome::rows));
        if let Some(who) = approved_by {
            record = record.approved_by(who);
        }
        if let Err(erreur) = issue {
            record = record.failed(erreur);
        }

        if let Err(erreur) = self.store.journal().append(&record) {
            tracing::error!(
                error = %erreur,
                command = %id,
                "failed to journal the outcome of a command that already ran"
            );
        }
    }

    // ── Ce que l'ordonnanceur sait des connexions ───────────────────────────

    /// Fait connaître une connexion à l'ordonnanceur.
    ///
    /// Sert à deux choses : retrouver l'environnement à soumettre au
    /// `PolicyGate`, et ouvrir la session sans relire l'état local.
    ///
    /// **Ne remplace pas l'enregistrement auprès de la politique.**
    /// [`PolicyGate`] n'expose aucune méthode d'enregistrement — c'est une
    /// frontière, pas un registre —, donc `oxyn-app` appelle aussi
    /// [`DefaultPolicy::register`](oxyn_core::DefaultPolicy::register).
    pub fn register_connection(&self, config: &ConnectionConfig) {
        self.connections.write().insert(config.id, config.clone());
    }

    /// Oublie une connexion.
    pub fn forget_connection(&self, connection: ConnectionId) {
        self.connections.write().remove(&connection);
    }

    /// Charge dans l'ordonnanceur les connexions d'un workspace, et rend leur
    /// nombre.
    ///
    /// # Erreurs
    /// Celles de l'état local.
    pub fn load_connections(&self) -> Result<usize> {
        let configs = self.store.connections().list(self.workspace)?;
        let mut guard = self.connections.write();
        for config in &configs {
            guard.insert(config.id, config.clone());
        }
        Ok(configs.len())
    }

    /// La configuration d'une connexion, du cache ou de l'état local.
    fn connection_config(&self, connection: ConnectionId) -> Result<ConnectionConfig> {
        if let Some(config) = self.connections.read().get(&connection).cloned() {
            return Ok(config);
        }
        match self.store.connections().get(connection)? {
            Some(config) => {
                self.register_connection(&config);
                Ok(config)
            }
            None => Err(OxynError::Config(
                "cette connexion n'existe pas dans le workspace".to_owned(),
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
            Command::CreateConnection { config } | Command::UpdateConnection { config } => {
                config.environment
            }
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

    /// Le tampon d'un résultat, tant qu'il est retenu.
    #[must_use]
    pub fn result(&self, result: ResultId) -> Option<Arc<ResultBuffer>> {
        self.results.read().get(&result).map(Arc::clone)
    }

    /// Oublie un résultat : l'onglet a été fermé.
    ///
    /// Le tampon n'est libéré que quand plus personne ne le tient — la grille
    /// peut être en train de le lire.
    pub fn forget_result(&self, result: ResultId) -> Option<Arc<ResultBuffer>> {
        self.results.write().remove(&result)
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
        let sessions = self.sessions.drain_all();
        for slot in &sessions {
            for entree in self.running.for_session(slot.id()) {
                entree.token().cancel();
            }
            if let Err(erreur) = slot.close().await {
                tracing::warn!(error = %erreur, "the server refused a clean session close");
            }
        }
        sessions.len()
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
            results: RwLock::new(HashMap::new()),
            connections: RwLock::new(HashMap::new()),
            workspace: self.workspace,
            memory_budget: self.memory_budget,
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
        AgentId, AgentSessionId, DefaultPolicy, DriverId, ExecLimits, MutationRisk, QueryLanguage,
        SqlDialect, StatementIntent,
    };
    use oxyn_store::{ActorKind, PolicyOutcome};

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
                .is_some_and(|motif| motif.contains("lecture seule")),
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
}
