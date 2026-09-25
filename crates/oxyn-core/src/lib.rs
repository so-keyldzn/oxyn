//! Le vocabulaire du domaine d'Oxyn.
//!
//! `oxyn-core` **ne dépend d'aucune autre crate du workspace**, ni de `tauri`, ni
//! d'un client de base de données. C'est la couche qui se teste sans machine :
//! aucune entrée-sortie, aucun réseau, aucun fichier ouvert. Tout le reste du
//! workspace importe ces types, donc leur nommage est un engagement.
//!
//! # Ce qu'on y trouve
//!
//! | Module | Sujet | Autorité |
//! |---|---|---|
//! | [`ids`] | identifiants d'instance et de driver | — |
//! | [`error`] | [`OxynError`], et la famille à laquelle une erreur appartient | DRIVER-CONTRACT §4 |
//! | [`capabilities`] | ce qu'une session sait faire | ADR-0003 |
//! | [`value`] | valeurs scalaires isolées | DRIVER-CONTRACT §7 |
//! | [`query`] | langage, intention, risque, limites | ADR-0003 |
//! | [`cancel`] | annulation coopérative et hiérarchique | DRIVER-CONTRACT §2 |
//! | [`connection`] | configuration et marquage d'environnement | SECURITY |
//! | [`command`] | [`Command`], [`Actor`] | ADR-0004 |
//! | [`policy`] | [`PolicyGate`], [`DefaultPolicy`] | ADR-0004, SECURITY |
//! | [`ai`] | déclaration d'un fournisseur, provenance d'un texte | ADR-0023 |
//! | [`stats`] | volumétrie et temps d'une exécution | — |
//! | [`transaction`] | l'état de transaction qu'une session a constaté | ADR-0039 |
//! | [`event`] | ce qui remonte vers l'interface | UX-SPEC |
//!
//! # Les trois choix qui gouvernent cette crate
//!
//! **Dans le doute, on protège.** [`StatementIntent::Unknown`] compte pour
//! mutante, [`Environment`] vaut [`Production`](Environment::Production) par
//! défaut, [`ExecLimits`] par défaut interdit l'écriture, et une commande
//! mutante visant une connexion inconnue du `PolicyGate` est refusée. Chacun de
//! ces défauts est le contraire du plus commode ; c'est délibéré.
//!
//! **Rien ne se devine.** Une capacité se déclare, une intention se déclare, une
//! famille d'erreur se déclare. Classer sur un nom d'instruction serait faux :
//! `EXPLAIN ANALYZE` exécute la requête qu'il analyse, `DELETE` compris.
//!
//! **Aucun secret, aucune valeur de la base dans un rendu.** Les `Debug` de
//! [`ConnectionConfig`] et d'[`ExecRequest`] sont écrits à la main pour masquer
//! les valeurs de paramètres et les valeurs liées : un `Debug` dérivé est le
//! mode de fuite le plus fréquent parce qu'il est invisible à la relecture.
//!
//! # Exemple
//!
//! ```
//! use oxyn_core::prelude::*;
//!
//! let politique = DefaultPolicy::new();
//! let connexion = ConnectionConfig::new("caisse", DriverId::postgres())
//!     .with_environment(Environment::Production);
//! politique.register(&connexion);
//!
//! let commande = Command::Execute {
//!     connection: connexion.id,
//!     session: SessionId::new(),
//!     request: Box::new(
//!         ExecRequest::new(QueryLanguage::SQL, "DELETE FROM commandes")
//!             .with_intent(StatementIntent::Write)
//!             .with_risk(MutationRisk::UnboundedDelete),
//!     ),
//! };
//!
//! // Un humain doit confirmer, et la confirmation nomme la connexion.
//! let decision = politique.authorize(&Actor::Human, &commande, Environment::Production);
//! assert!(decision.requires_approval());
//!
//! // Pour un agent, la production est en lecture seule stricte : c'est un refus.
//! let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
//! let decision = politique.authorize(&agent, &commande, Environment::Production);
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
    DriverId, IdParseError, ResultId, SessionId, StatementHandle, WorkspaceId,
};
pub use policy::{ConnectionFacts, Decision, DefaultPolicy, PolicyGate, Preview};
pub use preview::{PreviewShape, PreviewSort};
pub use query::{
    ExecLimits, ExecRequest, MutationRisk, QueryLanguage, SqlDialect, StatementIntent,
};
pub use stats::ExecStats;
pub use transaction::TransactionState;
pub use value::{ParameterParseError, ParameterType, ScalarValue};

/// Ce qu'on importe d'un coup quand on travaille avec le domaine.
///
/// Volontairement large : ces types sont du vocabulaire, et un vocabulaire
/// qu'il faut importer nom par nom finit par être contourné.
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

    /// Le trajet de la porte de sortie de la phase 0, réduit à ce que `core`
    /// peut en éprouver : une commande de lecture émise par un test traverse le
    /// `PolicyGate`.
    #[test]
    fn une_lecture_traverse_le_gate() {
        let politique = DefaultPolicy::new();
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        politique.register(&connexion);

        let commande = Command::Execute {
            connection: connexion.id,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "SELECT 1").with_intent(StatementIntent::Read),
            ),
        };

        let decision = politique.authorize(&Actor::Human, &commande, Environment::Local);
        assert_eq!(decision, Decision::Allow);
    }

    /// Le scénario de SECURITY : l'utilisateur ajoute une connexion à la hâte
    /// sans remplir le champ d'environnement, puis lance un `UPDATE` sans
    /// `WHERE`. Rien ne doit partir sans confirmation.
    #[test]
    fn une_connexion_ajoutee_a_la_hate_est_traitee_comme_de_la_production() {
        let politique = DefaultPolicy::new();
        // Aucun `with_environment` : le défaut s'applique.
        let connexion = ConnectionConfig::new("base client", DriverId::postgres());
        assert!(connexion.is_production());
        politique.register(&connexion);

        let commande = Command::Execute {
            connection: connexion.id,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "UPDATE clients SET actif = false")
                    .with_intent(StatementIntent::Write)
                    .with_risk(MutationRisk::UnboundedUpdate),
            ),
        };

        // Même en annonçant `Local`, l'appelant ne dégrade pas la protection.
        let decision = politique.authorize(&Actor::Human, &commande, Environment::Local);
        assert!(decision.requires_approval(), "{decision:?}");
    }
}
