//! The `PolicyGate`: the single gateway of the command bus (ADR-0004).
//!
//! Giving AI agents access to production databases is the product's number 1
//! risk. Every command — from the interface, an agent, a plugin — goes through
//! this gate and comes out with a [`Decision`]: `Allow`, `RequireApproval` or
//! `Deny`. There is no second path, and there must never be a duplicated
//! checking `if` in a caller: two places that decide make one place that will
//! forget.
//!
//! # The default policy
//!
//! [`DefaultPolicy`] applies, in this order:
//!
//! | Rule | Effect |
//! |---|---|
//! | agent + `GRANT`/`REVOKE` | `Deny` |
//! | agent + `SetSessionContext` | `Deny` — the effect bears on the following statements |
//! | agent + typed context change (`USE`) | `Deny` — same effect, reached through `Execute`; the user changes it with the context selector |
//! | agent + transaction control (`BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`…) | `Deny` — the effect bears on the session's transaction, perhaps the user's |
//! | mutating command on a connection marked read-only | `Deny`, human included |
//! | mutating command on a connection **unknown** to the gate | `Deny` |
//! | agent + mutating command + production | `Deny` — strict read-only |
//! | unbounded mutation risk (`UPDATE`/`DELETE` without `WHERE`, `TRUNCATE`, `DROP`) | `RequireApproval`, whatever the actor |
//! | agent + mutating command | `RequireApproval` with preview |
//! | human + mutating command + production | `RequireApproval` naming the connection |
//! | the rest | `Allow` |
//!
//! Two points that are not details:
//!
//! * for an agent, a production connection is **strictly read-only** — a
//!   refusal, not a stronger confirmation. The difference matters: a
//!   confirmation ends up being clicked;
//! * the "human + production" rule comes from
//!   [`SECURITY`](../../../docs/SECURITY.md): on a `production` connection,
//!   every write and every DDL require an explicit confirmation that **names
//!   the connection**. It adds to the actor × intent matrix.
//!
//! # Closed by default
//!
//! A mutating command targeting a connection the gate does not know is
//! **refused**. The gate cannot decide on what it does not see: ignoring the
//! environment marking of an unregistered connection would amount to treating
//! production as local. Registering connections with
//! [`DefaultPolicy::register`] is therefore part of the wiring, not optional
//! configuration.

use std::collections::HashMap;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::command::{Actor, Command};
use crate::connection::{ConnectionConfig, Environment};
use crate::ids::ConnectionId;
use crate::query::StatementIntent;

/// What the gate answers.
///
/// **Closed** enumeration: ADR-0004's triad is the bus's contract. A fourth
/// answer would be an ADR decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum Decision {
    /// The command can run.
    Allow,
    /// The command only runs after an explicit agreement from the user.
    RequireApproval {
        /// What the user must decide on, written to be read.
        reason: String,
        /// What is needed to judge without reading the query elsewhere.
        preview: Option<Preview>,
    },
    /// The command will not run. No confirmation unblocks it.
    Deny {
        /// Why, in terms showable to the user.
        reason: String,
    },
}

impl Decision {
    /// Builds a refusal.
    #[must_use]
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }

    /// Builds an approval request.
    #[must_use]
    pub fn approval(reason: impl Into<String>, preview: Option<Preview>) -> Self {
        Self::RequireApproval {
            reason: reason.into(),
            preview,
        }
    }

    /// Can the command run without further formality?
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    /// Is the command refused?
    #[must_use]
    pub const fn is_denied(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }

    /// Is the command waiting for an agreement?
    #[must_use]
    pub const fn requires_approval(&self) -> bool {
        matches!(self, Self::RequireApproval { .. })
    }

    /// Degree of restriction, increasing: `Allow` < `RequireApproval` < `Deny`.
    ///
    /// Used to compare two decisions — notably to check that a decision given
    /// for an agent is at least as restrictive as the one given for a human on
    /// the same command.
    #[must_use]
    pub const fn restrictiveness(&self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::RequireApproval { .. } => 1,
            Self::Deny { .. } => 2,
        }
    }
}

/// What is shown to the user before they decide.
///
/// A confirmation clicked by reflex protects nobody: the preview names the
/// connection and shows the exact statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    /// The exact statement, as the user or the agent wrote it. Without bound
    /// values (I-03).
    pub statement: String,
    /// The connection's **name**, never its identifier.
    pub connection: String,
    /// Estimate of the number of rows touched, when it is known.
    // TODO(2026-12-31): the estimate needs an `EXPLAIN` before review, whose
    // cost and shape differ per driver — that is what unblocks it, and it is a
    // batch identified in IMPLEMENTATION-PLAN.
    //
    // The previous wording said "as long as `oxyn-catalog` does not exist".
    // That crate has existed and been used everywhere for a long time: the
    // marker carried a condition already met, hence an unblocking that would
    // never come. `script/verifier-todo` checks the deadline, not the truth of
    // the reason.
    //
    // An invented figure would be worse than no figure: the production review
    // would read it as a measurement.
    pub estimated_rows: Option<u64>,
}

impl Preview {
    /// Builds a preview without an estimate.
    #[must_use]
    pub fn new(statement: impl Into<String>, connection: impl Into<String>) -> Self {
        Self {
            statement: statement.into(),
            connection: connection.into(),
            estimated_rows: None,
        }
    }
}

/// The single gateway of every command.
///
/// It is a **boundary**, not a speculative abstraction: enterprise policies
/// and test policies are other implementations.
pub trait PolicyGate: Send + Sync {
    /// Returns the decision applicable to this command, for this actor, in this
    /// environment.
    ///
    /// `env` is the environment the caller assigns to the command. An
    /// implementation that knows the real marking of the target connection must
    /// keep **the more restrictive of the two**: a caller that gets it wrong
    /// must not be able to lower the protection.
    fn authorize(&self, actor: &Actor, cmd: &Command, env: Environment) -> Decision;

    /// The environment `authorize` judges `cmd` in, when the caller announces
    /// `announced`.
    ///
    /// What the executor bounds after the decision — a production read goes
    /// out read-only — must rest on this very reckoning, not on a copy fed by
    /// another registry that may lag. The default knows no marking but the
    /// announced one.
    fn retained_environment(&self, cmd: &Command, announced: Environment) -> Environment {
        let _ = cmd;
        announced
    }

    /// Whether `authorize` holds the target of `cmd` to be marked read-only,
    /// when the caller announces `announced`.
    ///
    /// The executor bounds every read on such a connection to a read-only
    /// execution, for the reason it bounds production reads, and on the same
    /// reckoning: an implementation that knows the marking keeps the more
    /// restrictive of the two. The default knows no marking but the announced
    /// one.
    fn retained_read_only(&self, cmd: &Command, announced: bool) -> bool {
        let _ = cmd;
        announced
    }

    /// Name of the policy, for the audit log.
    fn name(&self) -> &'static str {
        "policy"
    }
}

/// What the gate must know about a connection to decide.
///
/// Deliberately reduced: no parameters, no secret reference. The gate does not
/// need to know how to connect, only to know what it protects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionFacts {
    /// The name shown to the user.
    pub name: String,
    /// The environment marking.
    pub environment: Environment,
    /// Is the connection declared read-only?
    pub read_only: bool,
}

impl From<&ConnectionConfig> for ConnectionFacts {
    fn from(cfg: &ConnectionConfig) -> Self {
        Self {
            name: cfg.name.clone(),
            environment: cfg.environment,
            read_only: cfg.read_only,
        }
    }
}

/// Oxyn's default policy.
///
/// The connection registry is internal and protected by a read-write lock: the
/// gate is shared between the interface thread, the executor and the agents,
/// and connections appear and disappear during the program's life.
#[derive(Debug, Default)]
pub struct DefaultPolicy {
    connections: RwLock<HashMap<ConnectionId, ConnectionFacts>>,
}

impl DefaultPolicy {
    /// Creates a policy with no connection registered.
    ///
    /// In this state, every **mutating** command targeting a server is refused:
    /// see the module's "closed by default" note.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or updates what the gate knows about a connection.
    ///
    /// To be called when a connection is created and on every modification: a
    /// registry lagging behind the configuration would make the gate decide on
    /// a stale marking.
    pub fn register(&self, config: &ConnectionConfig) {
        self.connections
            .write()
            .insert(config.id, ConnectionFacts::from(config));
    }

    /// Registers hand-built facts.
    pub fn register_facts(&self, id: ConnectionId, facts: ConnectionFacts) {
        self.connections.write().insert(id, facts);
    }

    /// Forgets a removed connection.
    pub fn forget(&self, id: ConnectionId) {
        self.connections.write().remove(&id);
    }

    /// What the gate knows about a connection.
    #[must_use]
    pub fn facts(&self, id: ConnectionId) -> Option<ConnectionFacts> {
        self.connections.read().get(&id).cloned()
    }

    /// The environment on which the gate judges `cmd`, when the caller
    /// announces `announced`.
    ///
    /// Exposed so that a caller that must know whether a decision bears on
    /// production — ADR-0037's native dialog — reads **this** computation
    /// rather than a copy that would diverge.
    #[must_use]
    pub fn retained_environment(&self, cmd: &Command, announced: Environment) -> Environment {
        let facts = cmd.target_connection().and_then(|id| self.facts(id));
        Self::retain(announced, facts.as_ref())
    }

    /// The more restrictive of the announced environment and the one the
    /// connection is marked with: a caller that gets it wrong must not be able
    /// to lower the protection.
    fn retain(announced: Environment, facts: Option<&ConnectionFacts>) -> Environment {
        facts.map_or(announced, |f| announced.max(f.environment))
    }

    /// Showable name of a connection, or a neutral mention if it is unknown.
    /// **Never** returns the identifier (I-03).
    ///
    /// A connection being created is not registered yet, but its command
    /// carries the name the user typed: the review names what it saves, not
    /// an « unknown connection ».
    fn display_name(cmd: &Command, facts: Option<&ConnectionFacts>) -> String {
        match (facts, cmd) {
            (Some(facts), _) => facts.name.clone(),
            (
                None,
                Command::CreateConnection { config }
                | Command::UpdateConnection { config }
                | Command::TestConnection { config },
            ) => config.name.clone(),
            (None, _) => "unknown connection".to_owned(),
        }
    }

    /// Builds the preview of a command.
    fn preview(cmd: &Command, facts: Option<&ConnectionFacts>) -> Option<Preview> {
        let statement = cmd
            .statement_text()
            .map_or_else(|| cmd.name().to_owned(), |text| text.to_owned());
        Some(Preview::new(statement, Self::display_name(cmd, facts)))
    }
}

impl PolicyGate for DefaultPolicy {
    fn authorize(&self, actor: &Actor, cmd: &Command, env: Environment) -> Decision {
        if actor.is_agent() && matches!(cmd, Command::WriteWorkspacePreferences { .. }) {
            return Decision::Deny {
                reason: "only the human may change workspace display preferences".into(),
            };
        }
        // An agent opens, closes and arranges no window (ADR-0043).
        if actor.is_agent() && matches!(cmd, Command::WriteWindowLayout { .. }) {
            return Decision::Deny {
                reason: "only the human may arrange the windows".into(),
            };
        }
        // An agent does not declare the endpoint it speaks through. A
        // declaration carries a base URL: an agent that could write it would
        // send out of the machine everything entrusted to it afterwards,
        // without any execution appearing in the log. It is a refusal and not
        // a stronger approval — a confirmation ends up being clicked (I-02,
        // ADR-0023).
        if actor.is_agent()
            && matches!(
                cmd,
                Command::SaveAiProvider { .. }
                    | Command::RemoveAiProvider { .. }
                    | Command::SaveExternalAgent { .. }
                    | Command::RemoveExternalAgent { .. }
            )
        {
            return Decision::Deny {
                reason: "only the human may declare or remove an AI provider".into(),
            };
        }
        // Trying a configuration is choosing the host a session opens on: for
        // an agent, the exfiltration channel `CreateConnection` is guarded
        // against, without even a saved connection to show for it.
        if actor.is_agent() && matches!(cmd, Command::TestConnection { .. }) {
            return Decision::deny("only the human may test a connection configuration");
        }
        // A connection's configuration is the human's: the host a session
        // opens on, its markings, and switches like sqlite-vec's, which decides
        // whether bundled C code parses the file (ADR-0054). No tool produces
        // these commands; the gate refuses them anyway, a refusal and not an
        // approval — a confirmation ends up being clicked (I-02).
        if actor.is_agent()
            && matches!(
                cmd,
                Command::CreateConnection { .. } | Command::UpdateConnection { .. }
            )
        {
            return Decision::deny("only the human may create or change a connection");
        }
        // Reconciling means declaring one inspected the server. An agent
        // inspected nothing: accepting it would silence the warning of a write
        // with an unknown outcome that nobody looked at (I-13). A refusal, not
        // a confirmation: a confirmation ends up being clicked (I-02).
        if actor.is_agent() && matches!(cmd, Command::ReconcileHistoryEntry { .. }) {
            return Decision::Deny {
                reason: "only the human may declare an interrupted write reconciled".into(),
            };
        }
        let intent = cmd.intent();
        let mutating = cmd.is_mutating();
        let facts = cmd.target_connection().and_then(|id| self.facts(id));

        let env = Self::retain(env, facts.as_ref());

        // ── Refus ───────────────────────────────────────────────────────────

        // An agent never manages rights. It is not a question of trust in the
        // model: it is the only category of action an agent has no legitimate
        // use for and whose effect outlives the session.
        if actor.is_agent() && intent == StatementIntent::Grant {
            return Decision::deny("an agent may not change privileges (GRANT / REVOKE)");
        }

        // An agent does not move the session context. The command neither
        // reads nor writes data — but its effect outlives the command, and it
        // bears on the meaning of the **following** statements: the user who
        // then writes `DELETE FROM users` would hit a different schema from
        // the one they believe they target, without any confirmation
        // mentioning that move.
        if actor.is_agent() && matches!(cmd, Command::SetSessionContext { .. }) {
            return Decision::deny(
                "an agent may not change the session context: \
                 it changes the meaning of the statements that follow",
            );
        }

        // The same move typed as a statement. `USE other` travels as an
        // `Execute`, so the variant check above never sees it: the flag is
        // what carries it, set by the executor's reclassification from the
        // text and not by the caller. Its intent is `Unknown`, which for an
        // agent on a development database would only ask for an approval —
        // and an approval is what this refusal exists to rule out: a
        // confirmation ends up being clicked (I-02), and the session would
        // not even record the new context.
        if actor.is_agent() && cmd.changes_context() {
            return Decision::deny(
                "an agent may not change the session context by typing it (USE): \
                 it changes the meaning of the statements that follow; \
                 the user changes it with the context selector",
            );
        }

        // Nor does it steer a transaction. `COMMIT` and `ROLLBACK` read and
        // write nothing of their own, but they settle whatever the session
        // holds — on a session shared with the user, the user's own writes,
        // committed or thrown away without a word. The gate does not know
        // whether a transaction is open (ADR-0039 keeps that state out of
        // security decisions), and needs not: an agent has no use for one
        // that it could not begin in a session it may share, and a `BEGIN` of
        // its own would capture the statements the user sends next.
        if actor.is_agent() && cmd.controls_transaction() {
            return Decision::deny(
                "an agent may not begin, commit or roll back a transaction: \
                 the session may hold one the user opened",
            );
        }

        if mutating && cmd.touches_database() {
            match (cmd.target_connection(), facts.as_ref()) {
                (Some(_), None) => {
                    return Decision::deny(
                        "connection unknown to the policy: \
                         its markings cannot be checked",
                    );
                }
                (Some(_), Some(f)) if f.read_only => {
                    return Decision::deny(format!(
                        "connection \"{}\" is marked read-only",
                        f.name
                    ));
                }
                _ => {}
            }
        }

        // For an agent, a production connection is strictly read-only. A
        // refusal, not a stronger confirmation.
        if actor.is_agent() && mutating && env.is_production() {
            return Decision::deny(format!(
                "an agent is strictly read-only on \"{}\" (production)",
                Self::display_name(cmd, facts.as_ref())
            ));
        }

        // ── Approvals ───────────────────────────────────────────────────────

        // The risk comes first, because its reason is the most informative:
        // better to read "DELETE without WHERE" than "write by an agent".
        if let Some(reason) = cmd.mutation_risk().reason() {
            return Decision::approval(reason, Self::preview(cmd, facts.as_ref()));
        }

        if actor.is_agent() && mutating {
            return Decision::approval(
                format!(
                    "an agent is requesting a {intent} operation on \"{}\"",
                    Self::display_name(cmd, facts.as_ref())
                ),
                Self::preview(cmd, facts.as_ref()),
            );
        }

        // SECURITY: on a production connection, every write and every DDL
        // require a confirmation that names the connection.
        if mutating && env.is_production() {
            return Decision::approval(
                format!(
                    "{intent} operation on \"{}\", marked production",
                    Self::display_name(cmd, facts.as_ref())
                ),
                Self::preview(cmd, facts.as_ref()),
            );
        }

        Decision::Allow
    }

    fn retained_environment(&self, cmd: &Command, announced: Environment) -> Environment {
        // The inherent method: one computation, whichever way it is reached.
        Self::retained_environment(self, cmd, announced)
    }

    fn retained_read_only(&self, cmd: &Command, announced: bool) -> bool {
        announced
            || cmd
                .target_connection()
                .and_then(|id| self.facts(id))
                .is_some_and(|facts| facts.read_only)
    }

    fn name(&self) -> &'static str {
        "DefaultPolicy"
    }
}

#[cfg(test)]
mod tests;
