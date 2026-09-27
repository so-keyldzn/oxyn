//! Oxyn's persistent local state: workspaces, connections, history,
//! **audit journal**, documents.
//!
//! Everything fits in a single SQLite file under the user's data directory.
//! One file, an open format, readable without Oxyn (I-11): what the product
//! writes, the user can recover with the command-line `sqlite3`.
//!
//! # What this crate holds
//!
//! | Module | Table | Authority |
//! |---|---|---|
//! | [`store`] | — | opening, settings, lock |
//! | `schema` (internal) | `schema_version` | numbered migrations |
//! | [`workspaces`] | `workspaces` | — |
//! | [`connections`] | `connections` | SECURITY — never a secret |
//! | [`history`] | `query_history` | — |
//! | [`journal`] | `audit_journal` | ARCHITECTURE §8 — **append-only** |
//! | [`documents`] | `documents` | — |
//! | [`providers`] | `ai_providers` | ADR-0023 — per machine, never a key |
//! | [`egress`] | `ai_egress` | SECURITY — **append-only**, column names, never a value |
//! | [`conversations`] | `ai_conversations`, `ai_conversation_turns` | AI-PROVIDERS — the tier is recorded on the **turn** |
//!
//! # The three choices that govern this crate
//!
//! **The audit trail is a mechanism, not a convention.** The journal
//! ([`journal`]) only exposes [`Journal::append`] — there is neither `update`
//! nor `delete` to call by mistake — and two SQLite triggers abort any
//! `UPDATE` and any `DELETE` on the table, including from another program.
//! The history ([`history`]), on the other hand, can be purged: that is what
//! makes it possible to offer "clear my history" without opening a way to
//! erase an agent's trace.
//!
//! **No secret comes down here.** [`Connections::save`] **refuses** a
//! configuration one of whose parameters carries a secret's name. A comment in
//! the schema prevents nothing; a refusal does (I-03).
//!
//! **What cannot be read back falls on the restrictive side.** An unreadable
//! environment marking counts as `production`, an unreadable intent counts as
//! `Unknown` — hence mutating —, an unreadable policy decision counts as
//! `Denied`. These are the same defaults as `oxyn-core`, applied down to
//! reading back a file a third party may have modified.
//!
//! # What this crate is not
//!
//! **Growth is bounded, and it is bounded by a call.** `query_history` is
//! purged, `ai_conversations` is pruned ([`Conversations::prune`]): neither
//! bounds itself, because a table that erased the user's work without a
//! caller asking for it would be data loss disguised as tidying. The audit
//! journal is not pruned at all.
//!
//! It is **synchronous** and takes a lock. None of its methods may be called
//! from the UI thread (I-05): it is up to `oxyn-exec` to carry them onto the
//! blocking pool.
//!
//! `rusqlite` is also a **public** dependency: [`StoreError::Sqlite`] carries
//! a `rusqlite::Error`, deliberately — the SQLite error code (`SQLITE_BUSY`,
//! constraint violation) is decision information, and hiding it behind a
//! string would destroy it. A caller that needs to read it adds `rusqlite` to
//! its dependencies; it cannot take another version of it anyway, since the
//! `links = "sqlite3"` of `libsqlite3-sys` forbids it (ADR-0010).
//!
//! # Example
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
//! // A refused command is logged too.
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
//!     &Decision::deny("production connection"),
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
pub mod windows;

pub use agents::ExternalAgents;
pub use connections::Connections;
pub use conversations::{
    Conversation, ConversationSummary, Conversations, Destination, DestinationKind,
    OrphanConversation, PruneReport, RetentionPolicy, ToolCallRecord, ToolCallStatus, Turn,
    TurnPage, TurnRecord, TurnRole, TurnUsage,
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

    /// The path phase 0 must make executable: an agent command goes through
    /// `oxyn-core`'s `PolicyGate`, its decision is logged as is, and the
    /// journal then resists any rewrite attempt.
    #[test]
    fn a_refused_agent_command_leaves_a_tamper_proof_trace() {
        let store = Store::open_in_memory().expect("open");
        let atelier = store.workspaces().create("atelier").expect("workspace");

        let connexion = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        store
            .connections()
            .save(atelier.id, &connexion)
            .expect("connection");

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

        // For an agent, production is strictly read-only.
        let decision = politique.authorize(&agent, &commande, Environment::Production);
        assert!(decision.is_denied(), "{decision:?}");

        store
            .journal()
            .append(&JournalRecord::new(&agent, &commande, &decision))
            .expect("logging");
        store
            .history()
            .record(
                &HistoryRecord::from_command(&agent, &commande)
                    .expect("an Execute produces a history entry")
                    .denied("production connection"),
            )
            .expect("history");

        // The agent's trace survives everything the user can erase.
        store.history().clear().expect("purge the history");
        store
            .connections()
            .delete(connexion.id)
            .expect("delete the connection");
        store
            .workspaces()
            .delete(atelier.id)
            .expect("delete the workspace");

        assert_eq!(store.journal().count().expect("count"), 1);
        let trace = store.journal().recent(1).expect("read back").remove(0);
        assert!(trace.record.actor_kind.is_agent());
        assert_eq!(trace.record.decision, crate::PolicyOutcome::Denied);
        assert_eq!(trace.record.risk, MutationRisk::UnboundedDelete);
        assert_eq!(
            trace.record.statement.as_deref(),
            Some("DELETE FROM clients")
        );
    }

    /// The file stays usable with `sqlite3`: that is what I-11 promises, and
    /// it is also what makes the tamper-proofing trigger necessary.
    #[test]
    fn the_format_stays_open() {
        let store = Store::open_in_memory().expect("open");
        let tables: Vec<String> = store
            .with_connection(|conn| {
                let mut requete = conn.prepare(
                    "SELECT name FROM sqlite_schema WHERE type = 'table' \
                     AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )?;
                let noms = requete.query_map([], |row| row.get(0))?;
                Ok(noms.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .expect("read the schema");

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
                "workspace_window_consoles",
                "workspace_windows",
                "workspaces",
            ]
        );
    }
}
