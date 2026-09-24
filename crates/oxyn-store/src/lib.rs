//! L'état local persistant d'Oxyn : workspaces, connexions, historique,
//! **journal d'audit**, documents.
//!
//! Tout tient dans un fichier SQLite unique sous le répertoire de données de
//! l'utilisateur. Un fichier, un format ouvert, lisible sans Oxyn (I-11) : ce
//! que le produit écrit, l'utilisateur peut le récupérer avec le `sqlite3` en
//! ligne de commande.
//!
//! # Ce que porte cette crate
//!
//! | Module | Table | Autorité |
//! |---|---|---|
//! | [`store`] | — | ouverture, réglages, verrou |
//! | `schema` (interne) | `schema_version` | migrations numérotées |
//! | [`workspaces`] | `workspaces` | — |
//! | [`connections`] | `connections` | SECURITY — jamais de secret |
//! | [`history`] | `query_history` | — |
//! | [`journal`] | `audit_journal` | ARCHITECTURE §8 — **append-only** |
//! | [`documents`] | `documents` | — |
//! | [`providers`] | `ai_providers` | ADR-0023 — par machine, jamais de clé |
//! | [`egress`] | `ai_egress` | SECURITY — **append-only**, des noms de colonnes, jamais de valeur |
//! | [`conversations`] | `ai_conversations`, `ai_conversation_turns` | AI-PROVIDERS — le niveau est noté sur le **tour** |
//!
//! # Les trois choix qui gouvernent cette crate
//!
//! **La piste d'audit est un mécanisme, pas une convention.** Le journal
//! ([`journal`]) n'expose que [`Journal::append`] — il n'y a ni `update` ni
//! `delete` à appeler par mégarde — et deux déclencheurs SQLite avortent tout
//! `UPDATE` et tout `DELETE` sur la table, y compris venus d'un autre
//! programme. L'historique ([`history`]), lui, se purge : c'est ce qui permet
//! d'offrir « effacer mon historique » sans ouvrir un moyen d'effacer la trace
//! d'un agent.
//!
//! **Aucun secret ne descend ici.** [`Connections::save`] **refuse** une
//! configuration dont un paramètre porte un nom de secret. Un commentaire dans
//! le schéma n'empêche rien ; un refus, si (I-03).
//!
//! **Ce qu'on ne sait pas relire retombe du côté contraignant.** Un marquage
//! d'environnement illisible vaut `production`, une intention illisible vaut
//! `Unknown` — donc mutante —, une décision de politique illisible vaut
//! `Denied`. Ce sont les mêmes défauts que `oxyn-core`, appliqués jusque dans
//! la relecture d'un fichier qu'un tiers a pu modifier.
//!
//! # Ce que cette crate n'est pas
//!
//! **La croissance est bornée, et elle l'est par un appel.** `query_history` se
//! purge, `ai_conversations` s'élague ([`Conversations::prune`]) : aucune des
//! deux ne se borne toute seule, parce qu'une table qui effacerait du travail
//! de l'utilisateur sans qu'un appelant l'ait demandé serait une perte de
//! données déguisée en rangement. Le journal d'audit, lui, ne s'élague pas du
//! tout.
//!
//! Elle est **synchrone** et prend un verrou. Aucune de ses méthodes ne doit
//! être appelée depuis le thread UI (I-05) : c'est à `oxyn-exec` de les porter
//! sur le pool bloquant.
//!
//! `rusqlite` est par ailleurs une dépendance **publique** :
//! [`StoreError::Sqlite`] porte une `rusqlite::Error`, délibérément — le code
//! d'erreur SQLite (`SQLITE_BUSY`, violation de contrainte) est une information
//! de décision, et l'effacer derrière une chaîne la détruirait. Un appelant qui
//! a besoin de la lire ajoute `rusqlite` à ses dépendances ; il ne peut de toute
//! façon pas en prendre une autre version, le `links = "sqlite3"` de
//! `libsqlite3-sys` l'interdisant (ADR-0010).
//!
//! # Exemple
//!
//! ```
//! use oxyn_core::{Actor, Command, ConnectionConfig, Decision, DriverId, Environment,
//!                 ExecRequest, QueryLanguage, SessionId, StatementIntent};
//! use oxyn_store::{JournalRecord, Store};
//!
//! let store = Store::open_in_memory()?;
//! let atelier = store.workspaces().create("atelier")?;
//!
//! let connexion = ConnectionConfig::new("base client", DriverId::postgres())
//!     .with_environment(Environment::Production)
//!     .with_param("host", "db.interne")
//!     .with_secret_ref("keychain://oxyn/base-client");
//! store.connections().save(atelier.id, &connexion)?;
//!
//! // Une commande refusée est journalisée elle aussi.
//! let commande = Command::Execute {
//!     connection: connexion.id,
//!     session: SessionId::new(),
//!     request: Box::new(
//!         ExecRequest::new(QueryLanguage::SQL, "DROP TABLE clients")
//!             .with_intent(StatementIntent::Ddl),
//!     ),
//! };
//! store.journal().append(&JournalRecord::new(
//!     &Actor::Human,
//!     &commande,
//!     &Decision::deny("connexion de production"),
//! ))?;
//!
//! assert_eq!(store.journal().count()?, 1);
//! # Ok::<(), oxyn_store::StoreError>(())
//! ```

pub mod connections;
pub mod conversations;
pub mod documents;
pub mod egress;
pub mod error;
pub mod history;
pub mod journal;
pub mod store;
pub mod workspaces;

pub mod agents;
mod encoding;
pub mod preferences;
pub mod providers;
mod schema;
#[cfg(test)]
mod sentinel_tests;
pub mod sessions;

pub use agents::ExternalAgents;
pub use connections::Connections;
pub use conversations::{
    Conversation, ConversationSummary, Conversations, Destination, DestinationKind, PruneReport,
    RetentionPolicy, ToolCallRecord, ToolCallStatus, Turn, TurnPage, TurnRecord, TurnRole,
    TurnUsage,
};
pub use documents::{Document, Documents};
pub use egress::{Egress, EgressEntry, EgressPage, EgressReach, EgressRecord};
pub use error::{Result, StoreError};
pub use history::{History, HistoryEntry, HistoryRecord, HistoryStatus};
pub use journal::{ActorKind, Journal, JournalEntry, JournalRecord, PolicyOutcome};
pub use providers::Providers;
pub use schema::latest_version as latest_schema_version;
pub use store::{DATABASE_FILE_NAME, Store};
pub use workspaces::{Workspace, Workspaces};

#[cfg(test)]
mod tests {
    use crate::{HistoryRecord, JournalRecord, Store};
    use oxyn_core::{
        Actor, AgentId, AgentSessionId, Command, ConnectionConfig, DriverId, Environment,
        ExecRequest, MutationRisk, PolicyGate, QueryLanguage, SessionId, StatementIntent,
    };

    /// Le trajet que la phase 0 doit rendre exécutable : une commande d'agent
    /// passe par le `PolicyGate` d'`oxyn-core`, sa décision est journalisée
    /// telle quelle, et le journal résiste ensuite à toute tentative de
    /// réécriture.
    #[test]
    fn une_commande_d_agent_refusee_laisse_une_trace_inviolable() {
        let store = Store::open_in_memory().expect("ouverture");
        let atelier = store.workspaces().create("atelier").expect("workspace");

        let connexion = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        store
            .connections()
            .save(atelier.id, &connexion)
            .expect("connexion");

        let politique = oxyn_core::DefaultPolicy::new();
        politique.register(&connexion);

        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
        let commande = Command::Execute {
            connection: connexion.id,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "DELETE FROM clients")
                    .with_intent(StatementIntent::Write)
                    .with_risk(MutationRisk::UnboundedDelete),
            ),
        };

        // Pour un agent, la production est en lecture seule stricte.
        let decision = politique.authorize(&agent, &commande, Environment::Production);
        assert!(decision.is_denied(), "{decision:?}");

        store
            .journal()
            .append(&JournalRecord::new(&agent, &commande, &decision))
            .expect("journalisation");
        store
            .history()
            .record(
                &HistoryRecord::from_command(&agent, &commande)
                    .expect("une Execute produit une entrée d'historique")
                    .denied("connexion de production"),
            )
            .expect("historique");

        // La trace de l'agent survit à tout ce que l'utilisateur peut effacer.
        store.history().clear().expect("purge de l'historique");
        store
            .connections()
            .delete(connexion.id)
            .expect("suppression de la connexion");
        store
            .workspaces()
            .delete(atelier.id)
            .expect("suppression du workspace");

        assert_eq!(store.journal().count().expect("comptage"), 1);
        let trace = store.journal().recent(1).expect("relecture").remove(0);
        assert!(trace.record.actor_kind.is_agent());
        assert_eq!(trace.record.decision, crate::PolicyOutcome::Denied);
        assert_eq!(trace.record.risk, MutationRisk::UnboundedDelete);
        assert_eq!(
            trace.record.statement.as_deref(),
            Some("DELETE FROM clients")
        );
    }

    /// Le fichier reste exploitable au `sqlite3` : c'est ce que promet I-11, et
    /// c'est aussi ce qui rend le déclencheur d'inviolabilité nécessaire.
    #[test]
    fn le_format_reste_ouvert() {
        let store = Store::open_in_memory().expect("ouverture");
        let tables: Vec<String> = store
            .with_connection(|conn| {
                let mut requete = conn.prepare(
                    "SELECT name FROM sqlite_schema WHERE type = 'table' \
                     AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )?;
                let noms = requete.query_map([], |row| row.get(0))?;
                Ok(noms.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .expect("lecture du schéma");

        assert_eq!(
            tables,
            [
                "ai_conversation_nodes",
                "ai_conversation_turns",
                "ai_conversations",
                "ai_egress",
                "ai_providers",
                "app_sessions",
                "audit_journal",
                "connections",
                "documents",
                "external_agents",
                "query_history",
                "schema_version",
                "workspace_preferences",
                "workspaces",
            ]
        );
    }
}
