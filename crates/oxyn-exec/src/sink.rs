//! La porte par laquelle les agents atteignent l'exécution — **la même que
//! l'interface**.
//!
//! Le runtime d'agents (`oxyn-ai`) confie ses commandes à un `CommandSink`.
//! [`ExecutorSink`] est ce à quoi ce puits se branche : il n'appelle rien
//! d'autre que [`Executor::dispatch`], la méthode que l'interface appelle
//! elle-même. Il n'existe donc pas de seconde API « pour l'IA » à auditer
//! séparément — c'est tout l'objet d'ADR-0004, et c'est ce qui rend la
//! promesse tenable : un agent ne peut rien faire d'inaccessible à
//! l'utilisateur, et tout ce qu'il fait apparaît dans le même journal.
//!
//! # Pourquoi ce module n'implémente pas littéralement `CommandSink`
//!
//! `oxyn-ai` n'est **pas** au contrat de dépendances d'`oxyn-exec` (voir son
//! `Cargo.toml`), et le contrat de dépendances est un choix d'architecture
//! arrêté. Le trait ne peut donc pas être implémenté ici. Ce module fournit
//! l'exécution complète et un [`DispatchReport`] dont la forme est celle de
//! `DispatchOutcome`, pour que le câblage dans `oxyn-desktop` — qui dépend des
//! deux — soit une traduction sans logique :
//!
//! ```ignore
//! #[async_trait]
//! impl CommandSink for MonPuits {
//!     async fn dispatch(&self, actor: Actor, command: Command, cancel: &CancelToken)
//!         -> DispatchOutcome
//!     {
//!         match self.0.dispatch(actor, command, cancel).await {
//!             DispatchReport::Completed { stats: Some(stats), .. } => {
//!                 DispatchOutcome::completed(&stats)
//!             }
//!             DispatchReport::Completed { summary, .. } => {
//!                 DispatchOutcome::Completed { summary }
//!             }
//!             DispatchReport::AwaitingApproval { reason, .. } => {
//!                 DispatchOutcome::AwaitingApproval { reason }
//!             }
//!             DispatchReport::Denied { reason, .. } => DispatchOutcome::Denied { reason },
//!             DispatchReport::Failed { class, error, .. } => {
//!                 DispatchOutcome::Failed { class, message: error }
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! Le puits rend des **faits**, jamais un texte déjà filtré : il ne mentionne ni
//! `ToolOutcome` ni le niveau de confidentialité. C'est le runtime IA qui
//! applique celui-ci, parce qu'il appartient à la connexion et que lui seul le
//! connaît ([I-04](../../../CLAUDE.md#i-04)). Un puits qui filtrerait de son
//! côté serait un second endroit où l'oublier.
//!
//! Les quatre règles du contrat de `CommandSink` sont tenues par
//! [`Executor::dispatch`] lui-même : reclassification avant décision, `Actor`
//! transmis sans modification, journalisation du refus compris, annulation
//! propagée jusqu'au serveur.
//!
//! # Ce que ce puits ajoute
//!
//! Une seule chose, et elle n'est pas cosmétique :
//! [`ExecutorSink::for_agent`] **refuse** toute commande dont l'acteur n'est
//! pas l'agent auquel le puits est attaché. Un puits d'agent qui laisserait
//! passer `Actor::Human` offrirait à un agent le moyen de se faire passer pour
//! l'utilisateur, et donc d'échapper à toute la moitié « agent » de la matrice
//! de politique.

use std::fmt;
use std::sync::Arc;

use oxyn_core::{
    Actor, AgentId, AgentSessionId, CancelToken, Command, CommandId, ErrorClass, ExecStats,
    OxynError, ResultId,
};

use crate::executor::{Executor, Outcome};

/// Ce qu'il faut dire au modèle d'une commande qu'il a demandée.
///
/// Volontairement pauvre : le modèle apprend ce qui s'est passé, jamais les
/// lignes. Les résultats vivent en `RecordBatch` dans le tampon et sont montrés
/// à l'**utilisateur** ; les faire transiter par la conversation les enverrait
/// chez le fournisseur, ce que le niveau de confidentialité de la connexion
/// n'autorise pas nécessairement (I-04).
///
/// Il n'y a **pas** de variante d'erreur qui interrompt la conversation : un
/// échec d'exécution est une réponse à donner au modèle, pas un incident.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DispatchReport {
    /// La commande a produit un effet.
    Completed {
        /// La commande, pour corréler avec le journal.
        command: CommandId,
        /// Ce qu'il faut en dire, **en anglais** : c'est une invite, pas un
        /// message d'interface.
        summary: String,
        /// Les mesures, quand la commande en produit — une exécution en
        /// produit, une ouverture de document non.
        stats: Option<ExecStats>,
        /// Le résultat retenu par l'ordonnanceur, quand la commande en produit
        /// un : c'est ce qui permet de montrer les lignes **à l'utilisateur**,
        /// par le même chemin que la grille d'une console.
        ///
        /// Il ne dit rien au modèle et ne doit jamais lui parvenir : le
        /// câblage vers `DispatchOutcome` l'ignore, et c'est voulu
        /// ([I-04](../../../CLAUDE.md#i-04)).
        result: Option<ResultId>,
    },

    /// Le catalogue local a été lu : la poignée du cache, pas un rendu.
    ///
    /// Ce que le modèle en apprend est décidé par `oxyn-ai`, sous le niveau
    /// de la connexion ([I-04](../../../CLAUDE.md#i-04)) : ce rapport porte
    /// les faits, il ne rend rien.
    CatalogRead {
        /// La commande.
        command: CommandId,
        /// Le cache de la connexion.
        catalog: oxyn_catalog::CatalogHandle,
    },

    /// **Rien ne s'est exécuté.** L'utilisateur a été sollicité et n'a pas
    /// encore répondu.
    AwaitingApproval {
        /// La commande mise de côté ; c'est sous cet identifiant que l'accord
        /// se donnera.
        command: CommandId,
        /// Le motif, tel que le `PolicyGate` l'a rédigé.
        reason: String,
    },

    /// **Rien ne s'est exécuté**, et aucune confirmation ne débloquera la
    /// commande.
    Denied {
        /// La commande refusée.
        command: CommandId,
        /// Le motif, tel que le `PolicyGate` l'a rédigé.
        reason: String,
    },

    /// L'exécution a échoué.
    Failed {
        /// La commande.
        command: CommandId,
        /// La famille de l'erreur, portée comme **donnée**.
        ///
        /// Sans elle, un appelant devrait la redeviner à partir du message —
        /// ce que le contrat de driver interdit explicitement, parce qu'un
        /// message change et qu'un appelant qui l'analysait casse en silence
        /// ([DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md)).
        class: ErrorClass,
        /// Le message, tel qu'il sera montré **à l'utilisateur** : celui du
        /// serveur, code compris.
        ///
        /// Ce qu'un agent en voit est une autre question, et elle ne se décide
        /// pas ici : le niveau de confidentialité appartient à la connexion, et
        /// c'est le runtime IA qui l'applique ([I-04](../../../CLAUDE.md#i-04)).
        /// Ce rapport porte les faits ; il ne filtre pas.
        error: String,
    },
}

impl DispatchReport {
    /// La commande a-t-elle réellement produit un effet ?
    ///
    /// `false` pour une approbation en attente. Le piège qu'elle ferme : un
    /// modèle qui suppose que son `INSERT` a eu lieu et enchaîne sur cette
    /// hypothèse.
    #[must_use]
    pub const fn took_effect(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::CatalogRead { .. })
    }

    /// La commande qui a produit ce rapport.
    #[must_use]
    pub const fn command(&self) -> CommandId {
        match self {
            Self::Completed { command, .. }
            | Self::CatalogRead { command, .. }
            | Self::AwaitingApproval { command, .. }
            | Self::Denied { command, .. }
            | Self::Failed { command, .. } => *command,
        }
    }

    /// Traduit une issue d'exécution.
    #[must_use]
    fn from_outcome(command: CommandId, outcome: Outcome) -> Self {
        match outcome {
            Outcome::Executed {
                result,
                stats,
                sink,
                ..
            } => {
                let mut summary = format!("{} rows, {} batches", stats.rows, stats.batches);
                if stats.truncated || sink.is_truncated() {
                    // Dit explicitement au modèle que ce n'est pas tout : un
                    // résultat tronqué qui a l'air complet conduit à des
                    // conclusions fausses sur des données réelles.
                    summary.push_str(" (truncated; this is not the whole result)");
                }
                Self::Completed {
                    command,
                    summary,
                    stats: Some(stats),
                    result: Some(result),
                }
            }

            Outcome::CatalogDescribed { catalog, .. } => Self::CatalogRead { command, catalog },

            Outcome::NeedsApproval {
                command, reason, ..
            } => Self::AwaitingApproval { command, reason },

            Outcome::Denied { command, reason } => Self::Denied { command, reason },

            autre => Self::Completed {
                command,
                summary: summarize(&autre),
                stats: None,
                result: None,
            },
        }
    }

    /// Traduit une panne.
    #[must_use]
    fn from_error(command: CommandId, error: &OxynError) -> Self {
        Self::Failed {
            command,
            class: error.class(),
            error: error.to_string(),
        }
    }
}

/// Résume une issue sans mesures, **en anglais** : c'est une invite.
fn summarize(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Connected { .. } => "session opened".to_owned(),
        Outcome::CatalogRefreshed { .. } => "catalog refreshed".to_owned(),
        Outcome::Disconnected { closed, .. } => format!("{closed} session(s) closed"),
        Outcome::Cancelled { report } => {
            if report.was_running {
                "cancellation requested".to_owned()
            } else {
                "nothing was running under that handle".to_owned()
            }
        }
        Outcome::Exported { rows, .. } => format!("{rows} rows written"),
        Outcome::DocumentOpened { .. } => "document read".to_owned(),
        Outcome::DocumentWritten { .. } => "document written".to_owned(),
        Outcome::ConnectionSaved { .. } => "connection saved".to_owned(),
        Outcome::ConnectionDeleted { existed, .. } => {
            if *existed {
                "connection deleted".to_owned()
            } else {
                "no such connection".to_owned()
            }
        }
        // Les trois autres variantes sont traitées avant d'arriver ici ; le
        // rendu générique évite un `unreachable!` sur un chemin qu'un ajout de
        // variante pourrait rendre atteignable.
        _ => "done".to_owned(),
    }
}

/// Le puits de commandes d'un agent.
///
/// Ne fait **rien** d'autre que d'appeler [`Executor::dispatch`] et de traduire
/// l'issue. C'est délibérément un objet sans logique : toute règle qui vivrait
/// ici serait une règle que l'interface n'applique pas.
pub struct ExecutorSink {
    executor: Arc<Executor>,
    /// L'agent auquel ce puits est attaché, s'il l'est. Une commande venue d'un
    /// autre acteur est alors refusée.
    bound: Option<(AgentId, AgentSessionId)>,
}

impl ExecutorSink {
    /// Puits ouvert : l'acteur reçu est transmis tel quel.
    ///
    /// À réserver au câblage général — un puits attaché à un agent précis
    /// ([`for_agent`](Self::for_agent)) refuse une usurpation, celui-ci non.
    #[must_use]
    pub fn new(executor: Arc<Executor>) -> Self {
        Self {
            executor,
            bound: None,
        }
    }

    /// Puits attaché à un agent et à sa conversation.
    ///
    /// Toute commande portant un autre acteur — `Actor::Human` compris — est
    /// **refusée sans atteindre l'ordonnanceur**. C'est la barrière qui empêche
    /// un agent de se présenter comme l'utilisateur, et donc d'échapper à la
    /// moitié « agent » de la matrice de politique.
    #[must_use]
    pub fn for_agent(executor: Arc<Executor>, agent: AgentId, session: AgentSessionId) -> Self {
        Self {
            executor,
            bound: Some((agent, session)),
        }
    }

    /// L'ordonnanceur derrière ce puits.
    #[must_use]
    pub fn executor(&self) -> &Arc<Executor> {
        &self.executor
    }

    /// Soumet une commande et rend ce qu'il faut en dire au modèle.
    ///
    /// Ne rend pas de `Result` : un échec d'exécution **est** une réponse
    /// ([`DispatchReport::Failed`]), pas un incident qui interrompt la
    /// conversation. Ce qui l'interrompt vient du fournisseur, pas de la base.
    pub async fn dispatch(
        &self,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> DispatchReport {
        // L'identifiant est frappé ici pour que le rapport le porte, y compris
        // quand rien n'atteint l'ordonnanceur : c'est la clé de corrélation
        // avec le journal.
        let id = CommandId::new();
        if let Some(refus) = self.reject_impersonation(id, &actor) {
            return refus;
        }

        match self.executor.dispatch_as(id, actor, command, cancel).await {
            Ok(outcome) => DispatchReport::from_outcome(id, outcome),
            Err(erreur) => DispatchReport::from_error(id, &erreur),
        }
    }

    /// Refuse une commande dont l'acteur n'est pas celui du puits.
    fn reject_impersonation(&self, id: CommandId, actor: &Actor) -> Option<DispatchReport> {
        let (agent, session) = self.bound?;
        let attendu = Actor::agent(agent, session);
        if *actor == attendu {
            return None;
        }
        Some(DispatchReport::Denied {
            command: id,
            reason: "this actor is not the one bound to the conversation: \
                     an agent command cannot be presented as coming from the user"
                .to_owned(),
        })
    }
}

impl fmt::Debug for ExecutorSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutorSink")
            .field("bound_to_agent", &self.bound.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::executor::block_on;
    use oxyn_core::{
        ConnectionConfig, ConnectionId, DefaultPolicy, DriverId, Environment, ExecLimits,
        ExecRequest, PolicyGate, QueryLanguage, SessionId, SqlDialect, StatementIntent,
    };
    use oxyn_store::Store;

    use super::*;

    fn banc(connexion: &ConnectionConfig) -> Arc<Executor> {
        let store = Arc::new(Store::open_in_memory().expect("état local en mémoire"));
        let atelier = store.workspaces().create("tests").expect("workspace");
        store
            .connections()
            .save(atelier.id, connexion)
            .expect("connexion");

        let politique = Arc::new(DefaultPolicy::new());
        politique.register(connexion);
        let politique: Arc<dyn PolicyGate> = politique;

        let executeur = Executor::builder(store, politique)
            .with_workspace(atelier.id)
            .build();
        executeur.register_connection(connexion);
        Arc::new(executeur)
    }

    fn execution(connexion: ConnectionId, texte: &str) -> Command {
        Command::Execute {
            connection: connexion,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), texte).with_limits(
                    ExecLimits::default()
                        .writable()
                        .with_timeout(None::<Duration>),
                ),
            ),
        }
    }

    #[test]
    fn un_agent_ne_peut_pas_se_faire_passer_pour_l_utilisateur() {
        let connexion = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let puits =
            ExecutorSink::for_agent(banc(&connexion), AgentId::new(), AgentSessionId::new());

        let rapport = block_on(puits.dispatch(
            Actor::Human,
            execution(connexion.id, "DELETE FROM clients"),
            &CancelToken::new(),
        ));

        assert!(
            matches!(rapport, DispatchReport::Denied { .. }),
            "{rapport:?}"
        );
        assert!(!rapport.took_effect());
        // Et rien n'a atteint l'ordonnanceur : le journal est vide.
        assert_eq!(
            puits
                .executor()
                .store()
                .journal()
                .count()
                .expect("comptage"),
            0
        );
    }

    #[test]
    fn le_puits_passe_par_le_meme_dispatch_que_l_interface() {
        // La preuve : un agent qui demande un DELETE en production reçoit un
        // refus rédigé par le `PolicyGate`, et le journal en garde la trace —
        // exactement comme si l'interface avait soumis la commande.
        let connexion = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        let executeur = banc(&connexion);
        let agent = AgentId::new();
        let session = AgentSessionId::new();
        let puits = ExecutorSink::for_agent(Arc::clone(&executeur), agent, session);

        let rapport = block_on(puits.dispatch(
            Actor::agent(agent, session),
            execution(connexion.id, "DELETE FROM clients"),
            &CancelToken::new(),
        ));

        let DispatchReport::Denied { reason, .. } = &rapport else {
            panic!("{rapport:?}");
        };
        assert!(reason.contains("production"), "{reason}");
        assert_eq!(executeur.store().journal().count().expect("comptage"), 1);

        let trace = executeur
            .store()
            .journal()
            .for_agent(agent, 1)
            .expect("relecture")
            .pop()
            .expect("une entrée imputée à l'agent");
        assert_eq!(trace.record.intent, StatementIntent::Write);
    }

    #[test]
    fn une_ecriture_d_agent_hors_production_est_rendue_comme_une_attente() {
        let connexion = ConnectionConfig::new("atelier", DriverId::postgres())
            .with_environment(Environment::Development);
        let agent = AgentId::new();
        let session = AgentSessionId::new();
        let puits = ExecutorSink::for_agent(banc(&connexion), agent, session);

        let rapport = block_on(puits.dispatch(
            Actor::agent(agent, session),
            execution(connexion.id, "UPDATE clients SET actif = true WHERE id = 1"),
            &CancelToken::new(),
        ));

        let DispatchReport::AwaitingApproval { command, .. } = rapport else {
            panic!("{rapport:?}");
        };
        // L'identifiant rendu est celui sous lequel l'accord se donnera.
        assert!(
            puits
                .executor()
                .approvals()
                .pending()
                .iter()
                .any(|p| p.id == command)
        );
    }

    #[test]
    fn une_panne_est_une_reponse_au_modele_et_non_un_incident() {
        // Aucune session n'est ouverte : l'exécution échoue après le gate.
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let agent = AgentId::new();
        let session = AgentSessionId::new();
        let puits = ExecutorSink::for_agent(banc(&connexion), agent, session);

        let rapport = block_on(puits.dispatch(
            Actor::agent(agent, session),
            execution(connexion.id, "SELECT 1"),
            &CancelToken::new(),
        ));

        let DispatchReport::Failed { class, .. } = rapport else {
            panic!("{rapport:?}");
        };
        assert!(
            class.is_retryable(),
            "une session fermée se rouvre : {class:?}"
        );
    }

    #[test]
    fn un_puits_ouvert_transmet_l_acteur_tel_quel() {
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let puits = ExecutorSink::new(banc(&connexion));

        let rapport = block_on(puits.dispatch(
            Actor::Human,
            execution(connexion.id, "SELECT 1"),
            &CancelToken::new(),
        ));
        // Le gate a autorisé (lecture, connexion locale) : l'échec vient de
        // l'absence de session, pas d'un refus.
        assert!(
            matches!(rapport, DispatchReport::Failed { .. }),
            "{rapport:?}"
        );
    }
}
