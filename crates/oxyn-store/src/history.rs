//! La table `query_history` : ce que l'utilisateur a exécuté.
//!
//! # Ce qui la distingue du journal d'audit
//!
//! L'historique et le journal ([`crate::journal`]) consignent la même chose,
//! mais ne répondent pas à la même question, et **seul l'historique est
//! effaçable** :
//!
//! | | `query_history` | `audit_journal` |
//! |---|---|---|
//! | Question | « qu'est-ce que j'ai lancé hier ? » | « qu'est-ce qui a été autorisé, et à qui ? » |
//! | Contenu | les exécutions | **toutes** les commandes, refus compris |
//! | Effaçable | oui, [`purge_before`](History::purge_before) et [`clear`](History::clear) | **non** |
//!
//! Purger l'historique **ne touche pas** au journal. C'est ce qui permet
//! d'offrir un « effacer mon historique » sans ouvrir un moyen d'effacer la
//! trace d'un agent — la fonction commode qui, sans cette séparation, finirait
//! par être le trou dans la piste d'audit.
//!
//! # Le nom de connexion est recopié
//!
//! La colonne `connection_id` n'a pas de clé étrangère et le nom est
//! dénormalisé à l'écriture : supprimer une connexion n'efface pas l'historique,
//! et une ligne dont la connexion n'existe plus reste lisible.

mod listing;
pub use listing::{HistoryConnectionPage, HistoryConnectionSummary, HistoryPage, HistorySummary};

use chrono::{DateTime, Utc};
use oxyn_core::{
    Actor, AgentId, Command, ConnectionId, ErrorClass, OxynError, QueryLanguage, StatementIntent,
};
use rusqlite::{Row, params};
use std::time::Duration;

use crate::encoding::{
    count_from_i64, count_to_i64, duration_to_ms, error_class_from_text, escape_like,
    intent_from_text, limit_to_i64, parse_id_opt, tag_from_json, tag_to_json,
};
use crate::error::Result;
use crate::journal::ActorKind;
use crate::store::Store;

/// Comment une exécution s'est terminée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HistoryStatus {
    /// Soumise, pas encore terminée.
    Running,
    /// Terminée sans erreur.
    Succeeded,
    /// Le serveur ou le driver a rendu une erreur.
    Failed,
    /// Interrompue à la demande.
    Cancelled,
    /// Refusée par le `PolicyGate` : elle n'a jamais atteint le serveur.
    Denied,
}

impl HistoryStatus {
    /// Nom stable, celui qui est écrit dans la colonne `status`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Denied => "denied",
        }
    }

    /// Relit la colonne `status`.
    ///
    /// Une valeur inconnue rend [`Failed`](Self::Failed) : une exécution dont
    /// on ne sait pas dire qu'elle a réussi n'a pas réussi.
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "denied" => Self::Denied,
            _ => {
                tracing::warn!(
                    column = "status",
                    "unknown history status in local state, falling back to `failed`"
                );
                Self::Failed
            }
        }
    }
}

impl std::fmt::Display for HistoryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Une exécution, telle qu'on l'inscrit à l'historique.
#[derive(Debug, Clone)]
pub struct HistoryRecord {
    /// Quand l'exécution a été soumise.
    pub ts: DateTime<Utc>,
    /// La connexion visée.
    pub connection: Option<ConnectionId>,
    /// Le **nom** de la connexion, recopié pour survivre à sa suppression.
    pub connection_name: Option<String>,
    /// Humain ou agent.
    pub actor_kind: ActorKind,
    /// Quel agent, le cas échéant.
    pub actor_id: Option<AgentId>,
    /// Le langage de la requête, dialecte compris.
    pub language: QueryLanguage,
    /// Le texte, tel que l'utilisateur ou l'agent l'a écrit — sans les valeurs
    /// liées (I-03).
    pub statement: String,
    /// L'intention retenue.
    pub intent: StatementIntent,
    /// Durée de l'exécution, quand elle est terminée.
    pub duration: Option<Duration>,
    /// Lignes produites ou affectées, quand elles sont connues.
    pub rows: Option<u64>,
    /// Comment cela s'est terminé.
    pub status: HistoryStatus,
    /// Message d'erreur ou motif de refus.
    pub error: Option<String>,
    /// La famille de l'erreur, quand il y en a une.
    ///
    /// Séparée du message à dessein : c'est **elle** qu'un appelant lit pour
    /// décider s'il peut proposer de relancer, jamais le texte. Un message
    /// change — il se reformule, il se traduit — et un appelant qui l'analysait
    /// casse alors en silence ([`ErrorClass`],
    /// [I-13](../../../CLAUDE.md#i-13)).
    ///
    /// [`Ambiguous`](ErrorClass::Ambiguous) est le cas qui compte : le serveur a
    /// peut-être appliqué l'écriture, et rejouer crée un doublon dans les
    /// données de l'utilisateur.
    ///
    /// **`None` a deux sens, et c'est pourquoi on ne le lit pas directement.**
    /// Sur une ligne réussie, il dit « aucune erreur » ; sur une ligne écrite
    /// par une version d'Oxyn antérieure à la colonne, il dit « famille
    /// inconnue » — et ces lignes-là portent précisément les écritures expirées
    /// qu'il ne faut pas rejouer. Passer par [`Self::is_retryable`] plutôt que
    /// par ce champ ferme la confusion.
    ///
    /// Sur une ligne [`Denied`](HistoryStatus::Denied), la famille ne décrit
    /// **pas** un verdict du serveur — rien ne l'a atteint. Elle dit seulement
    /// que la ligne n'est pas rejouable. Un comptage d'échecs serveur se filtre
    /// donc sur `status`, pas sur cette colonne seule.
    pub error_class: Option<ErrorClass>,
    /// A result identity from this application run; it may have expired.
    pub result: Option<oxyn_core::ResultId>,
}

impl HistoryRecord {
    /// Whether replay controls must be withheld until the server state is reconciled.
    pub fn requires_reconciliation(&self) -> bool {
        requires_reconciliation(self.intent, self.status, self.error_class)
    }

    /// Construit une entrée d'historique pour une exécution qui démarre.
    #[must_use]
    pub fn new(actor: &Actor, language: QueryLanguage, statement: impl Into<String>) -> Self {
        Self {
            ts: Utc::now(),
            connection: None,
            connection_name: None,
            actor_kind: ActorKind::from(actor),
            actor_id: match actor {
                Actor::Human => None,
                Actor::Agent { id, .. } => Some(*id),
            },
            language,
            statement: statement.into(),
            intent: StatementIntent::Unknown,
            duration: None,
            rows: None,
            status: HistoryStatus::Running,
            error: None,
            error_class: None,
            result: None,
        }
    }

    /// Construit une entrée à partir d'une commande d'exécution.
    ///
    /// Rend `None` pour toute autre commande : une `Connect` ou un `Export` ne
    /// sont pas des requêtes, et les faire figurer dans l'historique du
    /// *requêteur* le rendrait illisible. Elles restent au journal d'audit, qui
    /// les consigne toutes ([`crate::journal`]).
    #[must_use]
    pub fn from_command(actor: &Actor, command: &Command) -> Option<Self> {
        match command {
            Command::Execute {
                connection,
                request,
                ..
            } => {
                let mut record = Self::new(actor, request.language, request.text.clone());
                record.connection = Some(*connection);
                record.intent = request.intent;
                Some(record)
            }
            _ => None,
        }
    }

    /// Nomme la connexion visée.
    #[must_use]
    pub fn on_connection(mut self, id: ConnectionId, name: impl Into<String>) -> Self {
        self.connection = Some(id);
        self.connection_name = Some(name.into());
        self
    }

    /// Déclare l'intention retenue.
    #[must_use]
    pub fn with_intent(mut self, intent: StatementIntent) -> Self {
        self.intent = intent;
        self
    }

    /// Marque l'exécution comme réussie.
    #[must_use]
    pub fn succeeded(mut self, duration: Duration, rows: Option<u64>) -> Self {
        self.status = HistoryStatus::Succeeded;
        self.duration = Some(duration);
        self.rows = rows;
        self.error = None;
        self.error_class = None;
        self
    }

    /// Marque l'exécution comme échouée.
    ///
    /// Une annulation ([`OxynError::Cancelled`]) est classée
    /// [`Cancelled`](HistoryStatus::Cancelled), pas `Failed` : ce n'est pas une
    /// panne, c'est une décision de l'utilisateur, et les confondre fausse toute
    /// lecture du taux d'échec.
    ///
    /// Un refus de la politique ([`OxynError::PolicyDenied`]) est classé
    /// [`Denied`](HistoryStatus::Denied), pas `Failed` : rien n'a été soumis au
    /// serveur, et le présenter comme une panne enverrait l'utilisateur
    /// chercher un incident qui n'a pas eu lieu. La dernière barrière avant le
    /// driver rend ce refus sous forme d'erreur ; il doit se lire comme les
    /// refus rendus plus tôt par le `PolicyGate`.
    ///
    /// La famille de l'erreur est retenue **à part** dans
    /// [`error_class`](Self::error_class), jamais fondue dans le message
    /// (I-13).
    #[must_use]
    pub fn failed(mut self, error: &OxynError) -> Self {
        self.status = match error {
            _ if error.is_cancelled() => HistoryStatus::Cancelled,
            OxynError::PolicyDenied { .. } => HistoryStatus::Denied,
            _ => HistoryStatus::Failed,
        };
        self.error = Some(error.to_string());
        self.error_class = Some(error.class());
        self
    }

    /// Marque l'exécution comme refusée par la politique.
    ///
    /// La famille retenue est [`Permanent`](ErrorClass::Permanent) : un refus ne
    /// se rejoue pas, il se corrige. Toute ligne dont la famille n'est pas
    /// [`Transient`](ErrorClass::Transient) est hors de portée d'un bouton
    /// « relancer ».
    #[must_use]
    pub fn denied(mut self, reason: impl Into<String>) -> Self {
        self.status = HistoryStatus::Denied;
        self.error = Some(reason.into());
        self.error_class = Some(ErrorClass::Permanent);
        self
    }

    /// Cette exécution peut-elle être resoumise telle quelle ?
    ///
    /// **Seule** la famille [`Transient`](ErrorClass::Transient) répond `true`.
    /// Tout le reste répond `false`, y compris l'absence de famille : une ligne
    /// écrite avant que la colonne n'existe peut être une écriture expirée, et
    /// la rejouer créerait un doublon silencieux dans les données de
    /// l'utilisateur ([I-13](../../../CLAUDE.md#i-13)).
    ///
    /// C'est la seule question qu'un appelant a besoin de poser : lire
    /// [`error_class`](Self::error_class) pour y répondre soi-même, c'est
    /// réintroduire l'interprétation de `None` que cette méthode existe pour
    /// éviter.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        // La règle vit dans `ErrorClass`, pas ici : la recopier ferait diverger
        // deux définitions de « rejouable » le jour où une famille s'ajoute.
        self.error_class.is_some_and(|class| class.is_retryable())
    }
}

/// Une entrée relue de l'historique.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// Numéro d'ordre.
    pub id: i64,
    /// Le contenu de l'entrée.
    pub record: HistoryRecord,
}

/// Accès typé à la table `query_history`.
#[derive(Debug)]
pub struct History<'a> {
    store: &'a Store,
}

impl<'a> History<'a> {
    /// Rattache l'accesseur à son `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Inscrit une exécution et rend son numéro d'ordre.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Json`].
    pub fn record(&self, record: &HistoryRecord) -> Result<i64> {
        let language = tag_to_json(&record.language)?;

        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO query_history
                     (ts, connection_id, connection_name, actor_kind, actor_id, language,
                      statement, intent, duration_ms, row_count, status, error, error_class, result_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    record.ts,
                    record.connection.map(|id| id.to_string()),
                    record.connection_name,
                    record.actor_kind.as_str(),
                    record.actor_id.map(|id| id.to_string()),
                    language,
                    record.statement,
                    record.intent.as_str(),
                    record.duration.map(duration_to_ms),
                    record.rows.map(count_to_i64),
                    record.status.as_str(),
                    record.error,
                    record.error_class.map(|class| class.as_str()),
                    record.result.map(|result| result.to_string()),
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Complète une entrée inscrite pendant l'exécution : issue, durée, lignes,
    /// erreur. Rend `true` si la ligne existait.
    ///
    /// L'entrée est d'abord inscrite en [`Running`](HistoryStatus::Running) au
    /// moment de la soumission — l'utilisateur voit sa requête dans
    /// l'historique pendant qu'elle tourne —, puis complétée à la fin.
    ///
    /// Le texte, la connexion et l'horodatage ne sont **pas** réécrits : ce qui
    /// a été soumis ne change pas rétroactivement. C'est aussi la différence
    /// avec le journal d'audit, où cette méthode n'existe pas et où le fichier
    /// lui-même la refuserait ([`crate::journal`]).
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si l'écriture échoue.
    pub fn finish(&self, id: i64, record: &HistoryRecord) -> Result<bool> {
        self.store.with_connection(|conn| {
            let touchees = conn.execute(
                "UPDATE query_history
                    SET status = ?2, duration_ms = ?3, row_count = ?4, error = ?5,
                        error_class = ?6, result_id = ?7
                  WHERE id = ?1",
                params![
                    id,
                    record.status.as_str(),
                    record.duration.map(duration_to_ms),
                    record.rows.map(count_to_i64),
                    record.error,
                    record.error_class.map(|class| class.as_str()),
                    record.result.map(|result| result.to_string()),
                ],
            )?;
            Ok(touchees > 0)
        })
    }

    /// Les `limit` exécutions les plus récentes.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(&format!(
                "{SELECT_COLONNES} ORDER BY ts DESC, id DESC LIMIT ?1"
            ))?;
            requete
                .query_and_then(params![limit_to_i64(limit)], depuis_ligne)?
                .collect()
        })
    }

    /// Les `limit` exécutions les plus récentes sur une connexion.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn for_connection(
        &self,
        connection: ConnectionId,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>> {
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(&format!(
                "{SELECT_COLONNES} WHERE connection_id = ?1 ORDER BY ts DESC, id DESC LIMIT ?2"
            ))?;
            requete
                .query_and_then(
                    params![connection.to_string(), limit_to_i64(limit)],
                    depuis_ligne,
                )?
                .collect()
        })
    }

    /// Cherche `needle` dans le texte des requêtes, sans tenir compte de la
    /// casse.
    ///
    /// Les métacaractères `LIKE` de `needle` sont échappés : chercher `100%`
    /// trouve `100%`, pas toutes les lignes. Le motif est **lié**, jamais
    /// concaténé (I-10).
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn search(&self, needle: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let motif = format!("%{}%", escape_like(needle));
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(&format!(
                "{SELECT_COLONNES} WHERE statement LIKE ?1 ESCAPE '\\' \
                 ORDER BY ts DESC, id DESC LIMIT ?2"
            ))?;
            requete
                .query_and_then(params![motif, limit_to_i64(limit)], depuis_ligne)?
                .collect()
        })
    }

    /// Nombre d'entrées.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la lecture échoue.
    pub fn count(&self) -> Result<u64> {
        self.store.with_connection(|conn| {
            let total: i64 =
                conn.query_row("SELECT COUNT(*) FROM query_history", [], |row| row.get(0))?;
            Ok(count_from_i64(total))
        })
    }

    /// Efface les entrées antérieures à `cutoff` et rend leur nombre.
    ///
    /// **Ne touche pas au journal d'audit.**
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la suppression échoue.
    pub fn purge_before(&self, cutoff: DateTime<Utc>) -> Result<usize> {
        self.store.with_connection(|conn| {
            Ok(conn.execute("DELETE FROM query_history WHERE ts < ?1", params![cutoff])?)
        })
    }

    /// Efface tout l'historique et rend le nombre d'entrées supprimées.
    ///
    /// **Ne touche pas au journal d'audit** : c'est précisément la garantie qui
    /// permet d'offrir cette fonction.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la suppression échoue.
    pub fn clear(&self) -> Result<usize> {
        self.store
            .with_connection(|conn| Ok(conn.execute("DELETE FROM query_history", [])?))
    }
}

/// One rule for a row read whole and for the startup scan, which reads only
/// these three columns: two copies would drift, and the recovery warning would
/// then stay silent over a write the history still flags.
fn requires_reconciliation(
    intent: StatementIntent,
    status: HistoryStatus,
    error_class: Option<ErrorClass>,
) -> bool {
    error_class == Some(ErrorClass::Ambiguous)
        || (intent.is_mutating()
            && (matches!(status, HistoryStatus::Running | HistoryStatus::Cancelled)
                || status == HistoryStatus::Failed && error_class.is_none()))
}

/// La liste de colonnes, partagée par toutes les lectures.
const SELECT_COLONNES: &str = "SELECT id, ts, connection_id, connection_name, actor_kind, \
     actor_id, language, statement, intent, duration_ms, row_count, status, error, error_class, result_id \
     FROM query_history";

/// Reconstruit une [`HistoryEntry`] à partir d'une ligne.
fn depuis_ligne(row: &Row<'_>) -> Result<HistoryEntry> {
    let actor_kind: String = row.get("actor_kind")?;
    let language: String = row.get("language")?;
    let intent: String = row.get("intent")?;
    let status: String = row.get("status")?;
    let duration_ms: Option<i64> = row.get("duration_ms")?;
    let rows: Option<i64> = row.get("row_count")?;
    let error_class: Option<String> = row.get("error_class")?;

    Ok(HistoryEntry {
        id: row.get("id")?,
        record: HistoryRecord {
            ts: row.get("ts")?,
            connection: parse_id_opt(row.get("connection_id")?, "query_history.connection_id")?,
            connection_name: row.get("connection_name")?,
            actor_kind: ActorKind::from_text(&actor_kind),
            actor_id: parse_id_opt(row.get("actor_id")?, "query_history.actor_id")?,
            language: tag_from_json(&language, "query_history.language")?,
            statement: row.get("statement")?,
            intent: intent_from_text(&intent),
            duration: duration_ms.map(|ms| Duration::from_millis(count_from_i64(ms))),
            rows: rows.map(count_from_i64),
            status: HistoryStatus::from_text(&status),
            error: row.get("error")?,
            error_class: error_class.as_deref().map(error_class_from_text),
            result: parse_id_opt(row.get("result_id")?, "query_history.result_id")?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{
        AgentSessionId, ErrorClass, ExecRequest, ScalarValue, SessionId, SqlDialect,
        StatementHandle,
    };

    fn lecture(texte: &str) -> HistoryRecord {
        HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, texte)
            .with_intent(StatementIntent::Read)
    }

    /// Une erreur ambiguë survit à l'aller-retour **en tant que donnée** (I-13).
    ///
    /// Le message ne suffit pas : « délai dépassé après 30 s » ne dit pas que le
    /// serveur a peut-être appliqué l'écriture. Et l'analyser serait exactement
    /// ce que `rust.md` interdit — un message se reformule, un appelant qui le
    /// lisait casse alors sans que rien n'échoue. C'est donc la colonne que ce
    /// test verrouille, pas le texte.
    #[test]
    fn la_famille_d_une_erreur_survit_a_la_relecture() {
        let store = Store::open_in_memory().expect("ouverture");
        let expire =
            lecture("INSERT INTO commandes (client) VALUES (1)").failed(&OxynError::Timeout {
                after: Duration::from_secs(30),
            });
        let franche = lecture("SELECT * FROM absente")
            .failed(&OxynError::Query("relation « absente » inexistante".into()));

        store.history().record(&franche).expect("écriture");
        store.history().record(&expire).expect("écriture");
        let relues = store.history().recent(10).expect("relecture");

        let ambigue = relues
            .iter()
            .find(|entree| entree.record.statement.starts_with("INSERT"))
            .expect("l'écriture expirée");
        assert_eq!(ambigue.record.error_class, Some(ErrorClass::Ambiguous));
        assert!(
            !ambigue
                .record
                .error_class
                .expect("une famille")
                .is_retryable(),
            "un `INSERT` expiré ne se rejoue pas : le serveur a peut-être appliqué"
        );

        let permanente = relues
            .iter()
            .find(|entree| entree.record.statement.starts_with("SELECT"))
            .expect("la requête rejetée");
        assert_eq!(permanente.record.error_class, Some(ErrorClass::Permanent));

        // Le statut ne distingue pas les deux : c'est bien la famille qui porte
        // l'information, et elle seule.
        assert_eq!(ambigue.record.status, HistoryStatus::Failed);
        assert_eq!(permanente.record.status, HistoryStatus::Failed);
    }

    /// Ne pas savoir, c'est ne pas rejouer — aux deux endroits où l'on peut
    /// ignorer la famille d'une erreur.
    #[test]
    fn une_famille_inconnue_interdit_la_reprise() {
        // À la relecture d'une valeur que ce binaire ne connaît pas.
        assert_eq!(
            crate::encoding::error_class_from_text("vaporisée"),
            ErrorClass::Ambiguous
        );

        // Et sur une ligne écrite avant que la colonne n'existe : son `None` ne
        // veut pas dire « aucune erreur », il veut dire « famille inconnue ».
        // Rejouer l'`INSERT` expiré qu'elle porte peut-être créerait un doublon.
        let mut heritee = lecture("INSERT INTO commandes (client) VALUES (1)");
        heritee.status = HistoryStatus::Failed;
        heritee.error = Some("délai dépassé après 30s".to_owned());
        assert_eq!(heritee.error_class, None);
        assert!(!heritee.is_retryable());

        // Seule la famille transitoire ouvre la reprise.
        let coupure = lecture("SELECT 1").failed(&OxynError::Connection("coupure".into()));
        assert!(coupure.is_retryable());
        for interdite in [
            lecture("x").failed(&OxynError::Timeout {
                after: Duration::from_secs(1),
            }),
            lecture("x").failed(&OxynError::Query("syntaxe".into())),
            lecture("x").denied("lecture seule"),
            lecture("x").succeeded(Duration::from_millis(1), Some(0)),
        ] {
            assert!(!interdite.is_retryable(), "{:?}", interdite.error_class);
        }
    }

    /// Un refus rendu par la dernière barrière se lit comme un refus.
    ///
    /// Cette barrière-là rend une `Err`, là où le `PolicyGate` rend une
    /// décision. Sans ce classement, deux refus identiques pour l'utilisateur
    /// apparaîtraient l'un en « refusé », l'autre en « échec » — et le second
    /// l'enverrait chercher un incident serveur qui n'a pas eu lieu.
    #[test]
    fn un_refus_de_politique_n_est_pas_une_panne() {
        let refus = lecture("DELETE FROM clients").failed(&OxynError::PolicyDenied {
            reason: "lecture seule".to_owned(),
        });
        assert_eq!(refus.status, HistoryStatus::Denied);
        assert_eq!(refus.error_class, Some(ErrorClass::Permanent));
    }

    #[test]
    fn aller_retour_d_une_execution() {
        let store = Store::open_in_memory().expect("ouverture");
        let connexion = ConnectionId::new();
        let record = HistoryRecord::new(
            &Actor::Human,
            QueryLanguage::Sql(SqlDialect::Postgres),
            "SELECT * FROM clients",
        )
        .on_connection(connexion, "base client")
        .with_intent(StatementIntent::Read)
        .succeeded(Duration::from_millis(87), Some(1_204));

        let id = store.history().record(&record).expect("écriture");
        let relu = store.history().recent(10).expect("relecture").remove(0);

        assert_eq!(relu.id, id);
        assert_eq!(relu.record.connection, Some(connexion));
        assert_eq!(relu.record.connection_name.as_deref(), Some("base client"));
        assert_eq!(
            relu.record.language,
            QueryLanguage::Sql(SqlDialect::Postgres),
            "le dialecte doit survivre à l'aller-retour"
        );
        assert_eq!(relu.record.status, HistoryStatus::Succeeded);
        assert_eq!(relu.record.duration, Some(Duration::from_millis(87)));
        assert_eq!(relu.record.rows, Some(1_204));
        assert_eq!(relu.record.actor_kind, ActorKind::Human);
    }

    #[test]
    fn une_execution_s_inscrit_en_cours_puis_se_complete() {
        let store = Store::open_in_memory().expect("ouverture");
        let en_cours = lecture("SELECT count(*) FROM ventes");
        assert_eq!(en_cours.status, HistoryStatus::Running);

        let id = store.history().record(&en_cours).expect("écriture");
        assert_eq!(
            store.history().recent(1).expect("relecture")[0]
                .record
                .status,
            HistoryStatus::Running
        );

        let terminee = en_cours.succeeded(Duration::from_millis(410), Some(3));
        assert!(store.history().finish(id, &terminee).expect("achèvement"));

        let relu = store.history().recent(1).expect("relecture").remove(0);
        assert_eq!(relu.id, id);
        assert_eq!(relu.record.status, HistoryStatus::Succeeded);
        assert_eq!(relu.record.rows, Some(3));
        assert_eq!(relu.record.statement, "SELECT count(*) FROM ventes");

        // Le même geste sur le journal d'audit est impossible : il n'y a pas de
        // méthode, et le fichier lui-même le refuserait.
        assert!(
            !store
                .history()
                .finish(id + 1_000, &terminee)
                .expect("aucune ligne"),
            "une ligne absente ne se complète pas en silence"
        );
    }

    #[test]
    fn une_annulation_n_est_pas_un_echec() {
        let store = Store::open_in_memory().expect("ouverture");
        store
            .history()
            .record(&lecture("SELECT pg_sleep(60)").failed(&OxynError::Cancelled))
            .expect("écriture");
        store
            .history()
            .record(&lecture("SELECT 1/0").failed(&OxynError::Query("division par zéro".into())))
            .expect("écriture");

        let entrees = store.history().recent(10).expect("relecture");
        let statuts: Vec<HistoryStatus> = entrees.iter().map(|e| e.record.status).collect();
        assert!(statuts.contains(&HistoryStatus::Cancelled));
        assert!(statuts.contains(&HistoryStatus::Failed));
    }

    #[test]
    fn une_recherche_n_interprete_pas_les_metacaracteres() {
        let store = Store::open_in_memory().expect("ouverture");
        for texte in [
            "SELECT taux FROM remises WHERE taux = '100%'",
            "SELECT * FROM clients",
            "SELECT a_b FROM t",
        ] {
            store.history().record(&lecture(texte)).expect("écriture");
        }

        let sur_pourcent = store.history().search("100%", 50).expect("recherche");
        assert_eq!(
            sur_pourcent.len(),
            1,
            "`%` doit être littéral, pas un joker"
        );

        let sur_souligne = store.history().search("a_b", 50).expect("recherche");
        assert_eq!(sur_souligne.len(), 1, "`_` doit être littéral");

        let rien = store.history().search("a%b", 50).expect("recherche");
        assert!(rien.is_empty(), "`a%b` ne doit rien trouver littéralement");
    }

    #[test]
    fn purger_l_historique_ne_touche_pas_au_journal() {
        // C'est la garantie qui permet d'offrir « effacer mon historique »
        // sans ouvrir un moyen d'effacer la trace d'un agent.
        use crate::journal::JournalRecord;
        use oxyn_core::Decision;

        let store = Store::open_in_memory().expect("ouverture");
        let connexion = ConnectionId::new();
        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
        let commande = Command::Execute {
            connection: connexion,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "DELETE FROM clients")
                    .with_intent(StatementIntent::Write),
            ),
        };

        store
            .journal()
            .append(&JournalRecord::new(
                &agent,
                &commande,
                &Decision::approval("écriture par un agent", None),
            ))
            .expect("journal");
        store
            .history()
            .record(
                &HistoryRecord::from_command(&agent, &commande).expect("une Execute a un texte"),
            )
            .expect("historique");

        assert_eq!(store.history().count().expect("comptage"), 1);
        assert_eq!(store.journal().count().expect("comptage"), 1);

        let efface = store.history().clear().expect("purge");
        assert_eq!(efface, 1);
        assert_eq!(store.history().count().expect("comptage"), 0);
        assert_eq!(
            store.journal().count().expect("comptage"),
            1,
            "le journal d'audit ne se purge pas"
        );
    }

    #[test]
    fn la_purge_par_date_ne_prend_que_l_anterieur() {
        let store = Store::open_in_memory().expect("ouverture");
        let mut ancienne = lecture("SELECT 'vieux'");
        ancienne.ts = Utc::now() - chrono::Duration::days(30);
        store.history().record(&ancienne).expect("écriture");
        store
            .history()
            .record(&lecture("SELECT 'récent'"))
            .expect("écriture");

        let coupure = Utc::now() - chrono::Duration::days(7);
        assert_eq!(store.history().purge_before(coupure).expect("purge"), 1);

        let restant = store.history().recent(10).expect("relecture");
        assert_eq!(restant.len(), 1);
        assert_eq!(restant[0].record.statement, "SELECT 'récent'");
    }

    #[test]
    fn seules_les_executions_deviennent_des_entrees_d_historique() {
        let non_executions = [
            Command::Connect {
                connection: ConnectionId::new(),
            },
            Command::Cancel {
                connection: ConnectionId::new(),
                statement: StatementHandle::new(),
            },
            Command::RefreshCatalog {
                connection: ConnectionId::new(),
            },
        ];
        for commande in &non_executions {
            assert!(
                HistoryRecord::from_command(&Actor::Human, commande).is_none(),
                "`{}` n'est pas une requête",
                commande.name()
            );
        }
    }

    #[test]
    fn les_valeurs_liees_n_entrent_pas_dans_l_historique() {
        let store = Store::open_in_memory().expect("ouverture");
        let commande = Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "SELECT * FROM comptes WHERE jeton = $1")
                    .with_intent(StatementIntent::Read)
                    .with_params(vec![ScalarValue::Text("hunter2".to_owned())]),
            ),
        };
        store
            .history()
            .record(&HistoryRecord::from_command(&Actor::Human, &commande).expect("une Execute"))
            .expect("écriture");

        let tout: String = store
            .with_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT group_concat(statement || COALESCE(error, '')) FROM query_history",
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
    fn l_historique_survit_a_la_suppression_de_la_connexion() {
        let store = Store::open_in_memory().expect("ouverture");
        let connexion = ConnectionId::new();
        store
            .history()
            .record(&lecture("SELECT 1").on_connection(connexion, "base disparue"))
            .expect("écriture");

        // Aucune clé étrangère : la ligne subsiste et reste lisible.
        let relu = store
            .history()
            .for_connection(connexion, 10)
            .expect("relecture");
        assert_eq!(relu.len(), 1);
        assert_eq!(
            relu[0].record.connection_name.as_deref(),
            Some("base disparue")
        );
    }
}

#[cfg(test)]
mod library_tests;
