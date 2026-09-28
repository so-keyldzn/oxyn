//! Oxyn's domain vocabulary.
//!
//! `oxyn-core` **depends on no other crate of the workspace**, nor on `tauri`, nor
//! on a database client. It is the layer that is tested without a machine: no
//! input-output, no network, no open file. The rest of the workspace imports
//! these types, so their naming is a commitment.
//!
//! # What it contains
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`ids`] | instance and driver identifiers | — |
//! | [`error`] | [`OxynError`], and the family an error belongs to | DRIVER-CONTRACT §4 |
//! | [`capabilities`] | what a session can do | ADR-0003 |
//! | [`value`] | isolated scalar values | DRIVER-CONTRACT §7 |
//! | [`query`] | language, intent, risk, limits | ADR-0003 |
//! | [`cancel`] | cooperative, hierarchical cancellation | DRIVER-CONTRACT §2 |
//! | [`connection`] | configuration and environment marking | SECURITY |
//! | [`command`] | [`Command`], [`Actor`] | ADR-0004 |
//! | [`policy`] | [`PolicyGate`], [`DefaultPolicy`] | ADR-0004, SECURITY |
//! | [`ai`] | a provider's declaration, a text's provenance | ADR-0023 |
//! | [`stats`] | volume and time of an execution | — |
//! | [`transaction`] | the transaction state a session observed | ADR-0039 |
//! | [`event`] | what goes up to the interface | UX-SPEC |
//!
//! # The three choices that govern this crate
//!
//! **When in doubt, protect.** [`StatementIntent::Unknown`] counts as
//! mutating, [`Environment`] defaults to [`Production`](Environment::Production),
//! the default [`ExecLimits`] forbid writing, and a mutating command aimed at a
//! connection unknown to the `PolicyGate` is refused. Each of these defaults is
//! the opposite of the most convenient one; that is deliberate.
//!
//! **Nothing is guessed.** A capability is declared, an intent is declared, an
//! error family is declared. Classifying on a statement name would be wrong:
//! `EXPLAIN ANALYZE` runs the query it analyzes, `DELETE` included.
//!
//! **No secret, no database value in a rendering.** The `Debug` of
//! [`ConnectionConfig`] and of [`ExecRequest`] are written by hand to mask
//! parameter values and bound values: a derived `Debug` is the most frequent
//! leak because it is invisible in review.
//!
//! # Example
//!
//! ```
//! use oxyn_core::prelude::*;
//!
//! let policy = DefaultPolicy::new();
//! let connection = ConnectionConfig::new("checkout", DriverId::postgres())
//!     .with_environment(Environment::Production);
//! policy.register(&connection);
//!
//! let command = Command::Execute {
//!     connection: connection.id,
//!     session: SessionId::new(),
//!     request: Box::new(
//!         ExecRequest::new(QueryLanguage::SQL, "DELETE FROM orders")
//!             .with_intent(StatementIntent::Write)
//!             .with_risk(MutationRisk::UnboundedDelete),
//!     ),
//! };
//!
//! // A human must confirm, and the confirmation names the connection.
//! let decision = policy.authorize(&Actor::Human, &command, Environment::Production);
//! assert!(decision.requires_approval());
//!
//! // For an agent, production is strictly read-only: it is a refusal.
//! let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
//! let decision = policy.authorize(&agent, &command, Environment::Production);
//! assert!(decision.is_denied());
//! ```

pub mod ai;
pub mod cancel;
pub mod capabilities;
pub mod command;
pub mod connection;
pub mod error;
pub mod event;
pub mod ids;
pub mod library;
pub use library::{
    DocumentFilter, HistoryConnectionFilter, HistoryFilter, HistoryStatusFilter,
    MAX_QUERY_DOCUMENT_BYTES, QueryDocumentUpdate,
};
pub mod policy;
pub mod preferences;
pub mod preview;
pub use preferences::{
    Appearance, BinaryPreference, ObjectLocation, ObjectSection, PreferencesSnapshot,
    ReadingDensity, WorkspacePreferences,
};
pub mod query;
pub mod stats;
pub mod transaction;
pub mod value;
pub mod window_layout;

pub use ai::{
    AiProviderConfig, AiProviderKind, ExternalAgentConfig, MAX_PROVENANCE_BYTES, Provenance,
    ProviderId, ReasoningBlock, Role, StopReason,
};
pub use cancel::CancelToken;
pub use capabilities::Capabilities;
pub use command::{Actor, CatalogRefreshScope, Command, ExportFormat, MAX_CATALOG_FOCUS_BYTES};
pub use connection::{ConnectionConfig, Environment, PrivacyTier};
pub use error::{ErrorClass, OxynError, Result};
pub use event::Event;
pub use ids::{
    AgentId, AgentSessionId, AppSessionId, CommandId, ConnectionId, ConversationId, DocumentId,
    DriverId, IdParseError, ResultId, SessionId, StatementHandle, WindowId, WorkspaceId,
};
pub use policy::{ConnectionFacts, Decision, DefaultPolicy, PolicyGate, Preview};
pub use preview::{PreviewShape, PreviewSort};
pub use query::{
    ExecLimits, ExecRequest, MutationRisk, QueryLanguage, SqlDialect, StatementIntent,
};
pub use stats::ExecStats;
pub use transaction::TransactionState;
pub use value::{ParameterParseError, ParameterType, ScalarValue};
pub use window_layout::{WindowGeometry, WindowLayout, WindowLayoutChange};

/// What one imports in one go when working with the domain.
///
/// Deliberately broad: these types are vocabulary, and a vocabulary that must
/// be imported name by name ends up being bypassed.
///
/// ```
/// use oxyn_core::prelude::*;
/// ```
pub mod prelude {
    pub use crate::cancel::CancelToken;
    pub use crate::capabilities::Capabilities;
    pub use crate::command::{Actor, Command, ExportFormat};
    pub use crate::connection::{ConnectionConfig, Environment, PrivacyTier};
    pub use crate::error::{ErrorClass, OxynError, Result};
    pub use crate::event::Event;
    pub use crate::ids::{
        AgentId, AgentSessionId, CommandId, ConnectionId, DocumentId, DriverId, ResultId,
        SessionId, StatementHandle, WorkspaceId,
    };
    pub use crate::policy::{Decision, DefaultPolicy, PolicyGate, Preview};
    pub use crate::query::{
        ExecLimits, ExecRequest, MutationRisk, QueryLanguage, SqlDialect, StatementIntent,
    };
    pub use crate::stats::ExecStats;
    pub use crate::transaction::TransactionState;
    pub use crate::value::{ParameterParseError, ParameterType, ScalarValue};
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    /// The phase 0 exit-gate path, reduced to what `core` can exercise: a read
    /// command emitted by a test goes through the `PolicyGate`.
    #[test]
    fn a_read_goes_through_the_gate() {
        let policy = DefaultPolicy::new();
        let connection = ConnectionConfig::new("workshop", DriverId::sqlite())
            .with_environment(Environment::Local);
        policy.register(&connection);

        let command = Command::Execute {
            connection: connection.id,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "SELECT 1").with_intent(StatementIntent::Read),
            ),
        };

        let decision = policy.authorize(&Actor::Human, &command, Environment::Local);
        assert_eq!(decision, Decision::Allow);
    }

    /// The SECURITY scenario: the user hastily adds a connection without
    /// filling in the environment field, then runs an `UPDATE` without
    /// `WHERE`. Nothing must leave without confirmation.
    #[test]
    fn a_hastily_added_connection_is_treated_as_production() {
        let policy = DefaultPolicy::new();
        // No `with_environment`: the default applies.
        let connection = ConnectionConfig::new("customer database", DriverId::postgres());
        assert!(connection.is_production());
        policy.register(&connection);

        let command = Command::Execute {
            connection: connection.id,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "UPDATE customers SET active = false")
                    .with_intent(StatementIntent::Write)
                    .with_risk(MutationRisk::UnboundedUpdate),
            ),
        };

        // Even by announcing `Local`, the caller does not lower the protection.
        let decision = policy.authorize(&Actor::Human, &command, Environment::Local);
        assert!(decision.requires_approval(), "{decision:?}");
    }
}
