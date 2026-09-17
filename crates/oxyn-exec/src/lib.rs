//! L'ordonnanceur d'Oxyn : le point de passage **obligé** de toute commande.
//!
//! Si un chemin permet d'atteindre un driver sans traverser
//! [`Executor::dispatch`], l'architecture de sûreté du produit est cassée. Ce
//! n'est pas une formule : un second chemin d'exécution, une fois créé, n'est
//! jamais audité comme le premier — et c'est celui-là que l'IA empruntera
//! ([I-01](../../../CLAUDE.md#i-01), [ADR-0004](../../../docs/adr/0004-command-bus.md)).
//!
//! # Ce qu'on y trouve
//!
//! | Module | Sujet | Autorité |
//! |---|---|---|
//! | [`executor`] | la séquence complète : reclassifier, décider, journaliser, exécuter | ARCHITECTURE §8, §9 |
//! | [`approval`] | les commandes en attente d'accord, et leur péremption | ADR-0004 |
//! | [`cancel`] | les exécutions en cours, et l'annulation jusqu'au serveur | DRIVER-CONTRACT §2 |
//! | [`sessions`] | les sessions ouvertes, et la résolution des identifiants | SECURITY |
//! | [`events`] | ce qui remonte vers l'interface | UX-SPEC |
//! | [`sink`] | la porte des agents — la même que celle de l'interface | ADR-0004 |
//!
//! # La séquence, dans cet ordre et sans raccourci
//!
//! 1. **Reclassifier** le texte par `oxyn-query`. L'intention portée par la
//!    commande vient de l'appelant, et un agent est un appelant : elle est
//!    remplacée, jamais recoupée (ARCHITECTURE §8, I-07).
//! 2. **Soumettre au `PolicyGate`**, avec l'environnement de la connexion visée.
//! 3. **Sur `RequireApproval`, ne rien exécuter** et attendre un accord explicite
//!    portant le [`CommandId`](oxyn_core::CommandId) de la commande.
//! 4. **Journaliser avant et après** : la décision de politique avant toute
//!    exécution, le résultat après. Une commande refusée y figure aussi.
//! 5. **Exécuter en flux**, avec contre-pression et débordement disque (I-06).
//! 6. **Émettre les événements** vers l'interface par un canal (I-05).
//!
//! # Les trois choix qui gouvernent cette crate
//!
//! **Un refus n'est pas une panne.** [`Outcome::Denied`] et
//! [`Outcome::NeedsApproval`] sont des issues normales ; une `Err` décrit un
//! serveur injoignable ou un délai dépassé. Confondre les deux ferait passer
//! « le produit fait son travail » pour un incident, et l'utilisateur
//! finirait par cliquer sans lire.
//!
//! **La piste d'audit prime sur l'exécution.** Une décision de politique qui ne
//! s'écrit pas ne s'exécute pas. Après coup, c'est l'inverse : la commande a eu
//! lieu, et échouer maintenant laisserait croire le contraire — l'échec de
//! journalisation est crié, pas transformé en erreur.
//!
//! **Les agents passent littéralement par le même `dispatch`.** [`ExecutorSink`]
//! n'appelle rien d'autre que la méthode de l'interface. Il n'y a pas de
//! seconde API « pour l'IA » à auditer séparément.
//!
//! # Exemple
//!
//! ```
//! use std::sync::Arc;
//!
//! use oxyn_core::{
//!     Actor, AgentId, AgentSessionId, CancelToken, Command, ConnectionConfig, DefaultPolicy,
//!     DriverId, Environment, ExecRequest, QueryLanguage, SessionId, StatementIntent,
//! };
//! use oxyn_exec::Executor;
//! use oxyn_store::Store;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store = Arc::new(Store::open_in_memory()?);
//! let atelier = store.workspaces().create("atelier")?;
//!
//! let connexion = ConnectionConfig::new("base client", DriverId::postgres())
//!     .with_environment(Environment::Production);
//! store.connections().save(atelier.id, &connexion)?;
//!
//! let politique = Arc::new(DefaultPolicy::new());
//! politique.register(&connexion);
//!
//! let executeur = Executor::builder(Arc::clone(&store), politique)
//!     .with_workspace(atelier.id)
//!     .build();
//! executeur.register_connection(&connexion);
//!
//! // L'agent s'auto-déclare en lecture seule. Le texte dit autre chose.
//! let commande = Command::Execute {
//!     connection: connexion.id,
//!     session: SessionId::new(),
//!     request: Box::new(
//!         ExecRequest::new(QueryLanguage::SQL, "DELETE FROM clients")
//!             .with_intent(StatementIntent::Read),
//!     ),
//! };
//! let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
//!
//! let issue = futures::executor::block_on(executeur.dispatch(
//!     agent,
//!     commande,
//!     &CancelToken::new(),
//! ))?;
//!
//! assert!(issue.is_denied(), "{issue:?}");
//! // Et le refus laisse une trace : c'est la moitié de la promesse.
//! assert_eq!(store.journal().count()?, 1);
//! # Ok(())
//! # }
//! ```

mod abandon;
pub mod approval;
pub mod cancel;
mod catalog;
pub mod events;
pub mod executor;
pub mod sessions;
pub mod sink;

pub use approval::{ApprovalError, ApprovalRegistry, PendingCommand};
pub use cancel::{CancelRegistry, CancelReport, RunningStatement, ServerCancel};
pub use events::{EventBus, ExecEvent};
pub use executor::{Executor, ExecutorBuilder, Outcome};
pub use sessions::{CredentialResolver, NoCredentials, SessionRegistry, SessionSlot};
pub use sink::{DispatchReport, ExecutorSink};

/// Ce qu'on importe d'un coup quand on câble l'ordonnanceur.
///
/// Y compris le vocabulaire du domaine : câbler `oxyn-exec` demande
/// [`Actor`](oxyn_core::Actor), [`Command`](oxyn_core::Command) et
/// [`CancelToken`](oxyn_core::CancelToken) à chaque appel.
///
/// ```
/// use oxyn_exec::prelude::*;
/// ```
pub mod prelude {
    pub use oxyn_core::{
        Actor, CancelToken, Command, CommandId, ConnectionConfig, ConnectionId, Decision, Event,
        ExecRequest, OxynError, PolicyGate, Result, ResultId, SessionId, StatementHandle,
    };

    pub use crate::approval::{ApprovalError, ApprovalRegistry, PendingCommand};
    pub use crate::cancel::{CancelRegistry, CancelReport, ServerCancel};
    pub use crate::events::{EventBus, ExecEvent};
    pub use crate::executor::{Executor, ExecutorBuilder, Outcome};
    pub use crate::sessions::{CredentialResolver, NoCredentials, SessionRegistry};
    pub use crate::sink::{DispatchReport, ExecutorSink};
}

mod retained;
