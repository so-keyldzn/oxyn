//! Oxyn's executor: the **mandatory** passage point of every command.
//!
//! If a path reaches a driver without going through
//! [`Executor::dispatch`], the product's safety architecture is broken. It is
//! not a figure of speech: a second execution path, once created, is never
//! audited like the first — and it is that one the AI will take
//! ([I-01](../../../CLAUDE.md#i-01), [ADR-0004](../../../docs/adr/0004-command-bus.md)).
//!
//! # What is in it
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`executor`] | the complete sequence: reclassify, decide, log, execute | ARCHITECTURE §8, §9 |
//! | [`approval`] | commands awaiting approval, and their expiry | ADR-0004 |
//! | [`cancel`] | executions in progress, and cancellation down to the server | DRIVER-CONTRACT §2 |
//! | [`sessions`] | open sessions, and identifier resolution | SECURITY |
//! | [`events`] | what goes up to the interface | UX-SPEC |
//! | [`sink`] | the agents' door — the same as the interface's | ADR-0004 |
//!
//! # The sequence, in this order and without shortcut
//!
//! 1. **Reclassify** the text with `oxyn-query`. The intent carried by the
//!    command comes from the caller, and an agent is a caller: it is
//!    replaced, never cross-checked (ARCHITECTURE §8, I-07).
//! 2. **Submit to the `PolicyGate`**, with the environment of the target connection.
//! 3. **On `RequireApproval`, execute nothing** and wait for an explicit approval
//!    carrying the command's [`CommandId`](oxyn_core::CommandId).
//! 4. **Log before and after**: the policy decision before any execution, the
//!    result after. A denied command appears there too.
//! 5. **Execute as a stream**, with back-pressure and spill to disk (I-06).
//! 6. **Emit events** to the interface through a channel (I-05).
//!
//! # The three choices that govern this crate
//!
//! **A denial is not a failure.** [`Outcome::Denied`] and
//! [`Outcome::NeedsApproval`] are normal outcomes; an `Err` describes an
//! unreachable server or a timeout. Confusing the two would make "the product
//! does its job" look like an incident, and the user would end up clicking
//! without reading.
//!
//! **The audit trail comes before execution.** A policy decision that cannot
//! be written is not executed. Afterwards, it is the reverse: the command took
//! place, and failing now would suggest otherwise — a logging failure is
//! shouted, not turned into an error.
//!
//! **Agents literally go through the same `dispatch`.** [`ExecutorSink`]
//! calls nothing but the interface's method. There is no second API "for the
//! AI" to audit separately.
//!
//! # Example
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
//! let workshop = store.workspaces().create("workshop")?;
//!
//! let connection = ConnectionConfig::new("customer database", DriverId::postgres())
//!     .with_environment(Environment::Production);
//! store.connections().save(workshop.id, &connection)?;
//!
//! let policy = Arc::new(DefaultPolicy::new());
//! policy.register(&connection);
//!
//! let executor = Executor::builder(Arc::clone(&store), policy)
//!     .with_workspace(workshop.id)
//!     .build();
//! executor.register_connection(&connection);
//!
//! // The agent declares itself read-only. The text says otherwise.
//! let command = Command::Execute {
//!     connection: connection.id,
//!     session: SessionId::new(),
//!     request: Box::new(
//!         ExecRequest::new(QueryLanguage::SQL, "DELETE FROM customers")
//!             .with_intent(StatementIntent::Read),
//!     ),
//! };
//! let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
//!
//! let outcome = futures::executor::block_on(executor.dispatch(
//!     agent,
//!     command,
//!     &CancelToken::new(),
//! ))?;
//!
//! assert!(outcome.is_denied(), "{outcome:?}");
//! // And the denial leaves a trace: that is half of the promise.
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

/// What you import in one go when wiring the executor.
///
/// Including the domain vocabulary: wiring `oxyn-exec` requires
/// [`Actor`](oxyn_core::Actor), [`Command`](oxyn_core::Command) and
/// [`CancelToken`](oxyn_core::CancelToken) at every call.
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
