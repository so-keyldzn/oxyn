//! La table `audit_journal` : la piste d'audit, en **ajout seul**.
//!
//! C'est l'historique de l'utilisateur *et* le journal des agents — un seul
//! mécanisme, comme le veut ARCHITECTURE §8. Une commande **refusée** y figure
//! aussi : un journal qui ne consigne que ce qui a marché ne dit rien de ce
//! qu'un agent a tenté.
//!
//! # L'inviolabilité tient à deux endroits, et il en faut deux
//!
//! * **L'API.** [`Journal`] expose [`append`](Journal::append) et des lectures.
//!   Il n'y a ni `update` ni `delete` ni `purge` : ce qui n'existe pas ne
//!   s'appelle pas par mégarde, et aucun outil d'agent ne peut l'atteindre
//!   puisque les outils sont exactement les `Command` (ADR-0004).
//! * **Le fichier.** Deux déclencheurs SQLite avortent tout `UPDATE` et tout
//!   `DELETE` sur la table — `audit_journal_forbid_update` et
//!   `audit_journal_forbid_delete`, posés par la migration initiale. C'est ce
//!   qui fait tenir la garantie même quand quelqu'un ouvre l'état local avec le
//!   `sqlite3` en ligne de commande.
//!
//! L'API seule ne serait qu'une convention ; le déclencheur seul laisserait
//! passer un `DELETE` écrit à l'intérieur de la crate.
//!
//! Ce que le déclencheur **ne** couvre **pas** : un `DROP TABLE`, un
//! `PRAGMA writable_schema`, la réécriture du fichier avec un éditeur
//! hexadécimal. La protection vise l'erreur et l'agent qui voudrait effacer sa
//! trace par les moyens ordinaires du produit — pas un attaquant qui a déjà les
//! droits d'écriture sur le disque de l'utilisateur.
//!
//! # Relire ce qu'on ne comprend pas
//!
//! Les colonnes qui ont une valeur **conservatrice** y retombent quand elles
//! sont illisibles : `intent` devient
//! [`Unknown`](oxyn_core::StatementIntent::Unknown), qui compte pour mutant, et
//! `policy_decision` devient [`PolicyOutcome::Denied`]. Celles qui n'en ont pas
//! — `risk` — font échouer la lecture : inventer un risque serait pire que
//! renvoyer l'opérateur vers la ligne brute, qui reste lisible au `sqlite3`
//! (I-11).
//!
//! # Ce qui n'entre jamais ici
//!
//! Le **texte** de l'instruction est consigné : c'est l'objet de l'audit. Les
//! **valeurs liées** ne le sont pas — [`Command::statement_text`] ne les rend
//! pas — et aucun secret ne transite par ce module (I-03).

use chrono::{DateTime, Utc};
use oxyn_core::{
    Actor, AgentId, AgentSessionId, Command, CommandId, ConnectionId, Decision, MutationRisk,
    OxynError, StatementIntent,
};
use rusqlite::{Row, params};
use std::time::Duration;

use crate::encoding::{
    count_from_i64, count_to_i64, duration_to_ms, intent_from_text, limit_to_i64, parse_id_opt,
    tag_from_json, tag_to_json,
};
use crate::error::Result;
use crate::store::Store;

/// Qui a émis la commande, réduit à ce qui se range dans une colonne.
///
/// Énumération **fermée**, comme [`Actor`] dont elle dérive : la dichotomie
/// humain/agent porte toute la politique (ADR-0004), et un troisième acteur
/// serait une décision d'ADR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorKind {
    /// L'utilisateur, par l'interface.
    Human,
    /// Un agent IA.
    Agent,
}

impl ActorKind {
    /// Nom stable, celui qui est écrit dans la colonne `actor_kind`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }

    /// Relit la colonne `actor_kind`.
    ///
    /// Une valeur inconnue rend [`Agent`](Self::Agent) : dans une piste
    /// d'audit, la retombée sûre est celle qui **déclenche** l'examen, pas
    /// celle qui l'évite. Attribuer à un humain une action qu'on ne sait pas
    /// attribuer serait exactement l'erreur à ne pas commettre.
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "human" => Self::Human,
            "agent" => Self::Agent,
            _ => {
                tracing::warn!(
                    column = "actor_kind",
                    "unknown actor kind in audit journal, falling back to `agent`"
                );
                Self::Agent
            }
        }
    }

    /// Est-ce un agent ?
    #[must_use]
    pub const fn is_agent(&self) -> bool {
        matches!(self, Self::Agent)
    }
}

impl From<&Actor> for ActorKind {
    fn from(actor: &Actor) -> Self {
        if actor.is_agent() {
            Self::Agent
        } else {
            Self::Human
        }
    }
}

impl std::fmt::Display for ActorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ce que le `PolicyGate` a répondu, réduit à ce qui se range dans une colonne.
///
/// Énumération **fermée**, comme [`Decision`] : la triade d'ADR-0004 est le
/// contrat du bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyOutcome {
    /// `Allow` : la commande a pu s'exécuter.
    Allowed,
    /// `RequireApproval` : un accord explicite était nécessaire.
    ApprovalRequired,
    /// `Deny` : la commande n'a pas eu lieu.
    Denied,
}

impl PolicyOutcome {
    /// Nom stable, celui qui est écrit dans la colonne `policy_decision`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::ApprovalRequired => "approval_required",
            Self::Denied => "denied",
        }
    }

    /// Relit la colonne `policy_decision`.
    ///
    /// Une valeur inconnue rend [`Denied`](Self::Denied). La question à
    /// laquelle sert une relecture du journal est « qu'est-ce qui a été
    /// autorisé ? » : une ligne qu'on ne sait pas classer ne doit pas grossir
    /// ce compte. L'incohérence reste visible — une ligne `denied` portant une
    /// durée et des lignes affectées se remarque, et c'est le but.
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "allowed" => Self::Allowed,
            "approval_required" => Self::ApprovalRequired,
            "denied" => Self::Denied,
            _ => {
                tracing::warn!(
                    column = "policy_decision",
                    "unknown policy decision in audit journal, falling back to `denied`"
                );
                Self::Denied
            }
        }
    }

    /// La commande a-t-elle été autorisée sans autre formalité ?
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

impl From<&Decision> for PolicyOutcome {
    fn from(decision: &Decision) -> Self {
        match decision {
            Decision::Allow => Self::Allowed,
            Decision::RequireApproval { .. } => Self::ApprovalRequired,
            Decision::Deny { .. } => Self::Denied,
        }
    }
}

impl std::fmt::Display for PolicyOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ce qu'on ajoute au journal.
///
/// Le chemin normal est [`JournalRecord::new`], qui dérive tout ce qu'il peut
/// de la commande et de la décision plutôt que de laisser l'appelant le
/// recopier — un champ recopié à la main est un champ qui finira par mentir.
#[derive(Debug, Clone)]
pub struct JournalRecord {
    /// Quand la commande a été soumise.
    pub ts: DateTime<Utc>,
    /// Clé de corrélation avec la demande d'approbation et l'historique.
    pub command_id: Option<CommandId>,
    /// Humain ou agent.
    pub actor_kind: ActorKind,
    /// Quel agent, le cas échéant.
    pub actor_id: Option<AgentId>,
    /// Dans quelle conversation, le cas échéant.
    pub agent_session: Option<AgentSessionId>,
    /// La connexion visée, quand la commande en vise une.
    pub connection: Option<ConnectionId>,
    /// Nom stable de la commande ([`Command::name`]).
    pub command_kind: String,
    /// Le texte de l'instruction, sans les valeurs liées.
    pub statement: Option<String>,
    /// L'intention retenue au moment de la décision.
    pub intent: StatementIntent,
    /// Le risque retenu au moment de la décision.
    pub risk: MutationRisk,
    /// Ce que le `PolicyGate` a répondu.
    pub decision: PolicyOutcome,
    /// Le motif rendu avec la décision, quand il y en a un.
    pub decision_reason: Option<String>,
    /// Qui a approuvé, pour une commande qui exigeait un accord.
    pub approved_by: Option<String>,
    /// Durée d'exécution, quand la commande a été exécutée.
    pub duration: Option<Duration>,
    /// Lignes affectées, quand le driver les rend.
    pub rows_affected: Option<u64>,
    /// Message d'erreur, si l'exécution a échoué.
    pub error: Option<String>,
}

impl JournalRecord {
    /// Construit une entrée à partir de la commande et de la décision rendue.
    ///
    /// L'horodatage est pris maintenant. Ce qui n'est connu qu'après
    /// l'exécution — durée, lignes, erreur — s'ajoute par
    /// [`completed`](Self::completed) ou [`failed`](Self::failed).
    #[must_use]
    pub fn new(actor: &Actor, command: &Command, decision: &Decision) -> Self {
        let (actor_id, agent_session) = match actor {
            Actor::Human => (None, None),
            Actor::Agent { id, session } => (Some(*id), Some(*session)),
        };
        let decision_reason = match decision {
            Decision::Allow => None,
            Decision::RequireApproval { reason, .. } | Decision::Deny { reason } => {
                Some(reason.clone())
            }
        };

        Self {
            ts: Utc::now(),
            command_id: None,
            actor_kind: ActorKind::from(actor),
            actor_id,
            agent_session,
            connection: command.target_connection(),
            command_kind: command.name().to_owned(),
            statement: command.statement_text().map(str::to_owned),
            intent: command.intent(),
            risk: command.mutation_risk(),
            decision: PolicyOutcome::from(decision),
            decision_reason,
            approved_by: None,
            duration: None,
            rows_affected: None,
            error: None,
        }
    }

    /// Rattache l'identifiant de commande, clé de corrélation avec
    /// l'historique et la demande d'approbation.
    #[must_use]
    pub fn with_command_id(mut self, command_id: CommandId) -> Self {
        self.command_id = Some(command_id);
        self
    }

    /// Note qui a donné l'accord.
    #[must_use]
    pub fn approved_by(mut self, who: impl Into<String>) -> Self {
        self.approved_by = Some(who.into());
        self
    }

    /// Note une exécution réussie.
    #[must_use]
    pub fn completed(mut self, duration: Duration, rows_affected: Option<u64>) -> Self {
        self.duration = Some(duration);
        self.rows_affected = rows_affected;
        self.error = None;
        self
    }

    /// Note un échec.
    ///
    /// Le message est celui de l'erreur du domaine. `oxyn-core` garantit qu'il
    /// ne porte ni secret ni valeur liée (I-03) ; c'est la responsabilité de
    /// qui construit la variante, pas de ce module.
    #[must_use]
    pub fn failed(mut self, error: &OxynError) -> Self {
        self.error = Some(error.to_string());
        self
    }
}

/// Une entrée relue du journal.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    /// Numéro d'ordre, croissant et jamais réutilisé.
    pub id: i64,
    /// Le contenu de l'entrée.
    pub record: JournalRecord,
}

/// Accès typé à la table `audit_journal`.
///
/// **Il n'existe volontairement aucune méthode d'écriture autre que
/// [`append`](Self::append).** L'absence est la moitié de la garantie ; l'autre
/// moitié est dans les déclencheurs SQLite du schéma.
#[derive(Debug)]
pub struct Journal<'a> {
    store: &'a Store,
}

impl<'a> Journal<'a> {
    /// Rattache l'accesseur à son `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Ajoute une entrée et rend son numéro d'ordre.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si l'écriture échoue,
    /// [`crate::StoreError::Json`] si le risque n'est pas sérialisable.
    pub fn append(&self, record: &JournalRecord) -> Result<i64> {
        let risk = tag_to_json(&record.risk)?;

        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO audit_journal
                     (ts, command_id, actor_kind, actor_id, agent_session_id, connection_id,
                      command_kind, statement, intent, risk, policy_decision, decision_reason,
                      approved_by, duration_ms, rows_affected, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    record.ts,
                    record.command_id.map(|id| id.to_string()),
                    record.actor_kind.as_str(),
                    record.actor_id.map(|id| id.to_string()),
                    record.agent_session.map(|id| id.to_string()),
                    record.connection.map(|id| id.to_string()),
                    record.command_kind,
                    record.statement,
                    record.intent.as_str(),
                    risk,
                    record.decision.as_str(),
                    record.decision_reason,
                    record.approved_by,
                    record.duration.map(duration_to_ms),
                    record.rows_affected.map(count_to_i64),
                    record.error,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Les `limit` dernières entrées, la plus récente d'abord.
    ///
    /// L'ordre est celui des numéros d'ordre, pas celui des horodatages :
    /// dans une piste d'audit, c'est l'ordre d'inscription qui fait foi, et il
    /// ne dépend pas de l'horloge de la machine.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn recent(&self, limit: usize) -> Result<Vec<JournalEntry>> {
        self.store.with_connection(|conn| {
            let mut requete =
                conn.prepare(&format!("{SELECT_COLONNES} ORDER BY id DESC LIMIT ?1"))?;
            requete
                .query_and_then(params![limit_to_i64(limit)], depuis_ligne)?
                .collect()
        })
    }

    /// Les `limit` dernières entrées visant une connexion donnée.
    ///
    /// Ces entrées survivent à la suppression de la connexion : la table ne
    /// porte aucune clé étrangère vers `connections`.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn for_connection(
        &self,
        connection: ConnectionId,
        limit: usize,
    ) -> Result<Vec<JournalEntry>> {
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(&format!(
                "{SELECT_COLONNES} WHERE connection_id = ?1 ORDER BY id DESC LIMIT ?2"
            ))?;
            requete
                .query_and_then(
                    params![connection.to_string(), limit_to_i64(limit)],
                    depuis_ligne,
                )?
                .collect()
        })
    }

    /// Les `limit` dernières entrées imputées à un agent.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn for_agent(&self, agent: AgentId, limit: usize) -> Result<Vec<JournalEntry>> {
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(&format!(
                "{SELECT_COLONNES} WHERE actor_id = ?1 ORDER BY id DESC LIMIT ?2"
            ))?;
            requete
                .query_and_then(
                    params![agent.to_string(), limit_to_i64(limit)],
                    depuis_ligne,
                )?
                .collect()
        })
    }

    /// Nombre total d'entrées. Ne décroît jamais.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la lecture échoue.
    pub fn count(&self) -> Result<u64> {
        self.store.with_connection(|conn| {
            let total: i64 =
                conn.query_row("SELECT COUNT(*) FROM audit_journal", [], |row| row.get(0))?;
            Ok(count_from_i64(total))
        })
    }
}

/// La liste de colonnes, partagée par toutes les lectures pour que
/// [`depuis_ligne`] n'ait qu'une seule forme de ligne à connaître.
const SELECT_COLONNES: &str = "SELECT id, ts, command_id, actor_kind, actor_id, agent_session_id, \
     connection_id, command_kind, statement, intent, risk, policy_decision, \
     decision_reason, approved_by, duration_ms, rows_affected, error \
     FROM audit_journal";

/// Reconstruit une [`JournalEntry`] à partir d'une ligne.
fn depuis_ligne(row: &Row<'_>) -> Result<JournalEntry> {
    let actor_kind: String = row.get("actor_kind")?;
    let intent: String = row.get("intent")?;
    let risk: String = row.get("risk")?;
    let decision: String = row.get("policy_decision")?;
    let duration_ms: Option<i64> = row.get("duration_ms")?;
    let rows_affected: Option<i64> = row.get("rows_affected")?;

    Ok(JournalEntry {
        id: row.get("id")?,
        record: JournalRecord {
            ts: row.get("ts")?,
            command_id: parse_id_opt(row.get("command_id")?, "audit_journal.command_id")?,
            actor_kind: ActorKind::from_text(&actor_kind),
            actor_id: parse_id_opt(row.get("actor_id")?, "audit_journal.actor_id")?,
            agent_session: parse_id_opt(
                row.get("agent_session_id")?,
                "audit_journal.agent_session_id",
            )?,
            connection: parse_id_opt(row.get("connection_id")?, "audit_journal.connection_id")?,
            command_kind: row.get("command_kind")?,
            statement: row.get("statement")?,
            intent: intent_from_text(&intent),
            risk: tag_from_json(&risk, "audit_journal.risk")?,
            decision: PolicyOutcome::from_text(&decision),
            decision_reason: row.get("decision_reason")?,
            approved_by: row.get("approved_by")?,
            duration: duration_ms.map(|ms| Duration::from_millis(count_from_i64(ms))),
            rows_affected: rows_affected.map(count_from_i64),
            error: row.get("error")?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{
        ConnectionConfig, DriverId, Environment, ExecRequest, Preview, QueryLanguage, ScalarValue,
        SessionId,
    };

    fn execution(connection: ConnectionId, texte: &str, intent: StatementIntent) -> Command {
        Command::Execute {
            connection,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, texte)
                    .with_intent(intent)
                    .with_params(vec![ScalarValue::Text("hunter2".to_owned())]),
            ),
        }
    }

    #[test]
    fn le_journal_refuse_les_mises_a_jour() {
        let store = Store::open_in_memory().expect("ouverture");
        let commande = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
        let id = store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &commande,
                &Decision::Allow,
            ))
            .expect("ajout");

        let refus = store.with_connection(|conn| {
            Ok(conn.execute(
                "UPDATE audit_journal SET statement = 'SELECT 2' WHERE id = ?1",
                params![id],
            )?)
        });
        let erreur = refus.expect_err("un UPDATE doit échouer");
        assert!(
            erreur.to_string().contains("append-only"),
            "le déclencheur doit être la cause : {erreur}"
        );

        let relu = store.journal().recent(1).expect("relecture");
        assert_eq!(
            relu[0].record.statement.as_deref(),
            Some("SELECT 1"),
            "la ligne ne doit pas avoir bougé"
        );
    }

    #[test]
    fn le_journal_refuse_les_suppressions() {
        let store = Store::open_in_memory().expect("ouverture");
        let commande = execution(
            ConnectionId::new(),
            "DROP TABLE clients",
            StatementIntent::Ddl,
        );
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &commande,
                &Decision::deny("connexion en lecture seule"),
            ))
            .expect("ajout");

        for sql in [
            "DELETE FROM audit_journal",
            "DELETE FROM audit_journal WHERE id = 1",
        ] {
            let refus = store.with_connection(|conn| Ok(conn.execute(sql, [])?));
            let erreur = refus.expect_err("un DELETE doit échouer");
            assert!(
                erreur.to_string().contains("append-only"),
                "{sql} : {erreur}"
            );
        }

        assert_eq!(store.journal().count().expect("comptage"), 1);
    }

    #[test]
    fn une_commande_refusee_est_journalisee_avec_son_motif() {
        // Un journal qui ne consigne que ce qui a marché ne dit rien de ce
        // qu'un agent a tenté.
        let store = Store::open_in_memory().expect("ouverture");
        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
        let commande = execution(
            ConnectionId::new(),
            "GRANT ALL ON clients TO PUBLIC",
            StatementIntent::Grant,
        );

        store
            .journal()
            .append(&JournalRecord::new(
                &agent,
                &commande,
                &Decision::deny("un agent ne modifie pas les droits"),
            ))
            .expect("ajout");

        let entree = store.journal().recent(10).expect("relecture").remove(0);
        assert_eq!(entree.record.decision, PolicyOutcome::Denied);
        assert!(entree.record.actor_kind.is_agent());
        assert_eq!(
            entree.record.decision_reason.as_deref(),
            Some("un agent ne modifie pas les droits")
        );
        assert_eq!(entree.record.intent, StatementIntent::Grant);
        assert!(
            entree.record.duration.is_none(),
            "elle n'a pas été exécutée"
        );
    }

    #[test]
    fn les_valeurs_liees_n_entrent_pas_dans_le_journal() {
        // I-03 : le texte de la requête est de l'audit, ses valeurs liées non.
        let store = Store::open_in_memory().expect("ouverture");
        let commande = execution(
            ConnectionId::new(),
            "SELECT * FROM comptes WHERE mot_de_passe = $1",
            StatementIntent::Read,
        );
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &commande,
                &Decision::Allow,
            ))
            .expect("ajout");

        let tout: String = store
            .with_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT group_concat(COALESCE(statement, '') || COALESCE(error, '')) \
                     FROM audit_journal",
                    [],
                    |row| row.get(0),
                )?)
            })
            .expect("lecture brute");
        assert!(
            !tout.contains("hunter2"),
            "une valeur liée a fuité : {tout}"
        );
    }

    #[test]
    fn aller_retour_complet_d_une_entree() {
        let store = Store::open_in_memory().expect("ouverture");
        let connexion = ConnectionId::new();
        let agent_id = AgentId::new();
        let session = AgentSessionId::new();
        let commande_id = CommandId::new();
        let acteur = Actor::agent(agent_id, session);
        let commande = execution(connexion, "DELETE FROM commandes", StatementIntent::Write);

        let record = JournalRecord::new(
            &acteur,
            &commande,
            &Decision::approval(
                "écriture demandée par un agent",
                Some(Preview::new("DELETE FROM commandes", "base client")),
            ),
        )
        .with_command_id(commande_id)
        .approved_by("nicolas")
        .completed(Duration::from_millis(1_234), Some(42));

        let id = store.journal().append(&record).expect("ajout");
        assert!(id > 0);

        let relu = store
            .journal()
            .for_connection(connexion, 10)
            .expect("relecture")
            .remove(0);

        assert_eq!(relu.id, id);
        assert_eq!(relu.record.command_id, Some(commande_id));
        assert_eq!(relu.record.actor_id, Some(agent_id));
        assert_eq!(relu.record.agent_session, Some(session));
        assert_eq!(relu.record.connection, Some(connexion));
        assert_eq!(relu.record.command_kind, "Execute");
        assert_eq!(
            relu.record.statement.as_deref(),
            Some("DELETE FROM commandes")
        );
        assert_eq!(relu.record.intent, StatementIntent::Write);
        assert_eq!(relu.record.decision, PolicyOutcome::ApprovalRequired);
        assert_eq!(relu.record.approved_by.as_deref(), Some("nicolas"));
        assert_eq!(relu.record.duration, Some(Duration::from_millis(1_234)));
        assert_eq!(relu.record.rows_affected, Some(42));
        assert!(relu.record.error.is_none());
    }

    #[test]
    fn le_risque_declare_est_conserve() {
        let store = Store::open_in_memory().expect("ouverture");
        let commande = Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "TRUNCATE TABLE clients")
                    .with_intent(StatementIntent::Ddl)
                    .with_risk(MutationRisk::Truncate),
            ),
        };
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &commande,
                &Decision::approval("TRUNCATE", None),
            ))
            .expect("ajout");

        let relu = store.journal().recent(1).expect("relecture").remove(0);
        assert_eq!(relu.record.risk, MutationRisk::Truncate);
    }

    #[test]
    fn un_echec_est_journalise_sans_masquer_la_decision() {
        let store = Store::open_in_memory().expect("ouverture");
        let commande = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
        let record = JournalRecord::new(&Actor::Human, &commande, &Decision::Allow)
            .completed(Duration::from_millis(5), None)
            .failed(&OxynError::Query("relation absente".into()));

        store.journal().append(&record).expect("ajout");
        let relu = store.journal().recent(1).expect("relecture").remove(0);

        assert_eq!(relu.record.decision, PolicyOutcome::Allowed);
        assert!(
            relu.record
                .error
                .as_deref()
                .is_some_and(|e| e.contains("relation absente"))
        );
    }

    #[test]
    fn le_journal_survit_a_la_suppression_de_la_connexion() {
        // C'est la propriété qui rend la piste d'audit utile : effacer la
        // connexion n'efface pas ce qu'on a fait avec.
        let store = Store::open_in_memory().expect("ouverture");
        let workspace = store.workspaces().create("atelier").expect("workspace");
        let config = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        store
            .connections()
            .save(workspace.id, &config)
            .expect("écriture");

        let commande = execution(config.id, "DELETE FROM clients", StatementIntent::Write);
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &commande,
                &Decision::approval("production", None),
            ))
            .expect("ajout");

        assert!(store.connections().delete(config.id).expect("suppression"));
        assert!(
            store
                .workspaces()
                .delete(workspace.id)
                .expect("suppression")
        );

        assert_eq!(store.journal().count().expect("comptage"), 1);
        let restant = store
            .journal()
            .for_connection(config.id, 10)
            .expect("relecture");
        assert_eq!(restant.len(), 1);
        assert_eq!(
            restant[0].record.statement.as_deref(),
            Some("DELETE FROM clients")
        );
    }

    #[test]
    fn une_decision_illisible_ne_compte_pas_comme_autorisee() {
        let store = Store::open_in_memory().expect("ouverture");
        let commande = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &commande,
                &Decision::Allow,
            ))
            .expect("ajout");

        // Un UPDATE est impossible ; on simule donc une ligne écrite par une
        // version future en insérant directement des étiquettes inconnues.
        store
            .with_connection(|conn| {
                Ok(conn.execute(
                    "INSERT INTO audit_journal
                         (ts, actor_kind, command_kind, intent, risk, policy_decision)
                     VALUES (?1, 'quantum', 'Execute', 'levitate', '\"none\"', 'maybe')",
                    params![Utc::now()],
                )?)
            })
            .expect("insertion");

        let relu = store.journal().recent(1).expect("relecture").remove(0);
        assert_eq!(relu.record.decision, PolicyOutcome::Denied);
        assert!(relu.record.actor_kind.is_agent());
        assert_eq!(relu.record.intent, StatementIntent::Unknown);
        assert!(relu.record.intent.is_mutating());
    }

    #[test]
    fn les_entrees_sortent_dans_l_ordre_d_inscription_inverse() {
        let store = Store::open_in_memory().expect("ouverture");
        let connexion = ConnectionId::new();
        for n in 0..5 {
            let commande = execution(connexion, &format!("SELECT {n}"), StatementIntent::Read);
            store
                .journal()
                .append(&JournalRecord::new(
                    &Actor::Human,
                    &commande,
                    &Decision::Allow,
                ))
                .expect("ajout");
        }

        let recentes = store.journal().recent(3).expect("relecture");
        assert_eq!(recentes.len(), 3);
        assert_eq!(recentes[0].record.statement.as_deref(), Some("SELECT 4"));
        assert_eq!(recentes[2].record.statement.as_deref(), Some("SELECT 2"));
    }
}
