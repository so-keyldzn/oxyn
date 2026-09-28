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
    fn display_name(facts: Option<&ConnectionFacts>) -> String {
        facts.map_or_else(|| "unknown connection".to_owned(), |f| f.name.clone())
    }

    /// Builds the preview of a command.
    fn preview(cmd: &Command, facts: Option<&ConnectionFacts>) -> Option<Preview> {
        let statement = cmd
            .statement_text()
            .map_or_else(|| cmd.name().to_owned(), |text| text.to_owned());
        Some(Preview::new(statement, Self::display_name(facts)))
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
                Self::display_name(facts.as_ref())
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
                    Self::display_name(facts.as_ref())
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
                    Self::display_name(facts.as_ref())
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

    fn name(&self) -> &'static str {
        "DefaultPolicy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, AgentSessionId, DriverId, SessionId, StatementHandle};
    use crate::query::{ExecRequest, MutationRisk, QueryLanguage};

    /// Expected shape of a decision, independently of the reason's text.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Shape {
        Allow,
        Approve,
        Deny,
    }

    impl Shape {
        fn of(decision: &Decision) -> Self {
            match decision {
                Decision::Allow => Self::Allow,
                Decision::RequireApproval { .. } => Self::Approve,
                Decision::Deny { .. } => Self::Deny,
            }
        }
    }

    struct Bench {
        policy: DefaultPolicy,
        open: ConnectionId,
        read_only_connection: ConnectionId,
    }

    impl Bench {
        fn new() -> Self {
            let policy = DefaultPolicy::new();

            // Both connections are marked `Local`: it is the environment
            // passed to `authorize` that drives the matrix. Escalation through
            // the marking has its own test.
            let open = ConnectionConfig::new("workshop", DriverId::postgres())
                .with_environment(Environment::Local);
            let read_only_connection = ConnectionConfig::new("replica", DriverId::postgres())
                .with_environment(Environment::Local)
                .read_only();

            policy.register(&open);
            policy.register(&read_only_connection);

            Self {
                policy,
                open: open.id,
                read_only_connection: read_only_connection.id,
            }
        }

        fn connection(&self, read_only: bool) -> ConnectionId {
            if read_only {
                self.read_only_connection
            } else {
                self.open
            }
        }

        fn execute(&self, read_only: bool, intent: StatementIntent) -> Command {
            Command::Execute {
                connection: self.connection(read_only),
                session: SessionId::new(),
                request: Box::new(
                    ExecRequest::new(QueryLanguage::SQL, "SELECT 1").with_intent(intent),
                ),
            }
        }
    }

    fn human() -> Actor {
        Actor::Human
    }

    fn agent() -> Actor {
        Actor::agent(AgentId::new(), AgentSessionId::new())
    }

    const ENVS: [Environment; 4] = [
        Environment::Local,
        Environment::Development,
        Environment::Staging,
        Environment::Production,
    ];

    /// The complete matrix: actor × intent × environment × read-only.
    ///
    /// Each cell is written by hand. A table computed by an oracle function
    /// would prove nothing: it would copy the implementation.
    #[rustfmt::skip]
    const MATRIX: &[(bool, StatementIntent, Environment, bool, Shape)] = &[
        // ── human, open connection ──────────────────────────────────────────
        (false, StatementIntent::Read,    Environment::Local,       false, Shape::Allow),
        (false, StatementIntent::Read,    Environment::Development, false, Shape::Allow),
        (false, StatementIntent::Read,    Environment::Staging,     false, Shape::Allow),
        (false, StatementIntent::Read,    Environment::Production,  false, Shape::Allow),
        (false, StatementIntent::Write,   Environment::Local,       false, Shape::Allow),
        (false, StatementIntent::Write,   Environment::Development, false, Shape::Allow),
        (false, StatementIntent::Write,   Environment::Staging,     false, Shape::Allow),
        (false, StatementIntent::Write,   Environment::Production,  false, Shape::Approve),
        (false, StatementIntent::Ddl,     Environment::Local,       false, Shape::Allow),
        (false, StatementIntent::Ddl,     Environment::Development, false, Shape::Allow),
        (false, StatementIntent::Ddl,     Environment::Staging,     false, Shape::Allow),
        (false, StatementIntent::Ddl,     Environment::Production,  false, Shape::Approve),
        (false, StatementIntent::Grant,   Environment::Local,       false, Shape::Allow),
        (false, StatementIntent::Grant,   Environment::Development, false, Shape::Allow),
        (false, StatementIntent::Grant,   Environment::Staging,     false, Shape::Allow),
        (false, StatementIntent::Grant,   Environment::Production,  false, Shape::Approve),
        (false, StatementIntent::Unknown, Environment::Local,       false, Shape::Allow),
        (false, StatementIntent::Unknown, Environment::Development, false, Shape::Allow),
        (false, StatementIntent::Unknown, Environment::Staging,     false, Shape::Allow),
        (false, StatementIntent::Unknown, Environment::Production,  false, Shape::Approve),

        // ── human, read-only connection ────────────────────────────────────
        (false, StatementIntent::Read,    Environment::Local,       true,  Shape::Allow),
        (false, StatementIntent::Read,    Environment::Development, true,  Shape::Allow),
        (false, StatementIntent::Read,    Environment::Staging,     true,  Shape::Allow),
        (false, StatementIntent::Read,    Environment::Production,  true,  Shape::Allow),
        (false, StatementIntent::Write,   Environment::Local,       true,  Shape::Deny),
        (false, StatementIntent::Write,   Environment::Development, true,  Shape::Deny),
        (false, StatementIntent::Write,   Environment::Staging,     true,  Shape::Deny),
        (false, StatementIntent::Write,   Environment::Production,  true,  Shape::Deny),
        (false, StatementIntent::Ddl,     Environment::Local,       true,  Shape::Deny),
        (false, StatementIntent::Ddl,     Environment::Development, true,  Shape::Deny),
        (false, StatementIntent::Ddl,     Environment::Staging,     true,  Shape::Deny),
        (false, StatementIntent::Ddl,     Environment::Production,  true,  Shape::Deny),
        (false, StatementIntent::Grant,   Environment::Local,       true,  Shape::Deny),
        (false, StatementIntent::Grant,   Environment::Development, true,  Shape::Deny),
        (false, StatementIntent::Grant,   Environment::Staging,     true,  Shape::Deny),
        (false, StatementIntent::Grant,   Environment::Production,  true,  Shape::Deny),
        (false, StatementIntent::Unknown, Environment::Local,       true,  Shape::Deny),
        (false, StatementIntent::Unknown, Environment::Development, true,  Shape::Deny),
        (false, StatementIntent::Unknown, Environment::Staging,     true,  Shape::Deny),
        (false, StatementIntent::Unknown, Environment::Production,  true,  Shape::Deny),

        // ── agent, open connection ───────────────────────────────────────────
        (true,  StatementIntent::Read,    Environment::Local,       false, Shape::Allow),
        (true,  StatementIntent::Read,    Environment::Development, false, Shape::Allow),
        (true,  StatementIntent::Read,    Environment::Staging,     false, Shape::Allow),
        (true,  StatementIntent::Read,    Environment::Production,  false, Shape::Allow),
        (true,  StatementIntent::Write,   Environment::Local,       false, Shape::Approve),
        (true,  StatementIntent::Write,   Environment::Development, false, Shape::Approve),
        (true,  StatementIntent::Write,   Environment::Staging,     false, Shape::Approve),
        (true,  StatementIntent::Write,   Environment::Production,  false, Shape::Deny),
        (true,  StatementIntent::Ddl,     Environment::Local,       false, Shape::Approve),
        (true,  StatementIntent::Ddl,     Environment::Development, false, Shape::Approve),
        (true,  StatementIntent::Ddl,     Environment::Staging,     false, Shape::Approve),
        (true,  StatementIntent::Ddl,     Environment::Production,  false, Shape::Deny),
        (true,  StatementIntent::Grant,   Environment::Local,       false, Shape::Deny),
        (true,  StatementIntent::Grant,   Environment::Development, false, Shape::Deny),
        (true,  StatementIntent::Grant,   Environment::Staging,     false, Shape::Deny),
        (true,  StatementIntent::Grant,   Environment::Production,  false, Shape::Deny),
        (true,  StatementIntent::Unknown, Environment::Local,       false, Shape::Approve),
        (true,  StatementIntent::Unknown, Environment::Development, false, Shape::Approve),
        (true,  StatementIntent::Unknown, Environment::Staging,     false, Shape::Approve),
        (true,  StatementIntent::Unknown, Environment::Production,  false, Shape::Deny),

        // ── agent, read-only connection ─────────────────────────────────────
        (true,  StatementIntent::Read,    Environment::Local,       true,  Shape::Allow),
        (true,  StatementIntent::Read,    Environment::Development, true,  Shape::Allow),
        (true,  StatementIntent::Read,    Environment::Staging,     true,  Shape::Allow),
        (true,  StatementIntent::Read,    Environment::Production,  true,  Shape::Allow),
        (true,  StatementIntent::Write,   Environment::Local,       true,  Shape::Deny),
        (true,  StatementIntent::Write,   Environment::Development, true,  Shape::Deny),
        (true,  StatementIntent::Write,   Environment::Staging,     true,  Shape::Deny),
        (true,  StatementIntent::Write,   Environment::Production,  true,  Shape::Deny),
        (true,  StatementIntent::Ddl,     Environment::Local,       true,  Shape::Deny),
        (true,  StatementIntent::Ddl,     Environment::Development, true,  Shape::Deny),
        (true,  StatementIntent::Ddl,     Environment::Staging,     true,  Shape::Deny),
        (true,  StatementIntent::Ddl,     Environment::Production,  true,  Shape::Deny),
        (true,  StatementIntent::Grant,   Environment::Local,       true,  Shape::Deny),
        (true,  StatementIntent::Grant,   Environment::Development, true,  Shape::Deny),
        (true,  StatementIntent::Grant,   Environment::Staging,     true,  Shape::Deny),
        (true,  StatementIntent::Grant,   Environment::Production,  true,  Shape::Deny),
        (true,  StatementIntent::Unknown, Environment::Local,       true,  Shape::Deny),
        (true,  StatementIntent::Unknown, Environment::Development, true,  Shape::Deny),
        (true,  StatementIntent::Unknown, Environment::Staging,     true,  Shape::Deny),
        (true,  StatementIntent::Unknown, Environment::Production,  true,  Shape::Deny),
    ];

    #[test]
    fn the_matrix_covers_every_cell() {
        // 2 actors × 5 intents × 4 environments × 2 markings.
        assert_eq!(MATRIX.len(), 80, "a cell is missing from the matrix");

        let mut seen = std::collections::HashSet::new();
        for (is_agent, intent, env, ro, _) in MATRIX {
            assert!(
                seen.insert((*is_agent, *intent, *env, *ro)),
                "duplicated cell: {is_agent} {intent} {env} {ro}"
            );
        }
    }

    #[test]
    fn default_policy_matrix() {
        let bench = Bench::new();

        for (is_agent, intent, env, read_only, expected) in MATRIX {
            let actor = if *is_agent { agent() } else { human() };
            let cmd = bench.execute(*read_only, *intent);
            let decision = bench.policy.authorize(&actor, &cmd, *env);

            assert_eq!(
                Shape::of(&decision),
                *expected,
                "actor={actor} intent={intent} env={env} read_only={read_only} \
                 → {decision:?}"
            );
        }
    }

    #[test]
    fn an_agent_is_never_less_restricted_than_a_human() {
        // The test the `/commande` command asks for: the same command emitted
        // by an agent gets a decision at least as restrictive.
        let bench = Bench::new();
        let agent = agent();

        for intent in [
            StatementIntent::Read,
            StatementIntent::Write,
            StatementIntent::Ddl,
            StatementIntent::Grant,
            StatementIntent::Unknown,
        ] {
            for env in ENVS {
                for read_only in [false, true] {
                    let cmd = bench.execute(read_only, intent);
                    let for_human = bench.policy.authorize(&human(), &cmd, env);
                    let for_agent = bench.policy.authorize(&agent, &cmd, env);
                    assert!(
                        for_agent.restrictiveness() >= for_human.restrictiveness(),
                        "intent={intent} env={env} read_only={read_only}: \
                         agent={for_agent:?} human={for_human:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_unbounded_risk_requires_approval_even_for_a_human_locally() {
        let bench = Bench::new();

        for risk in [
            MutationRisk::UnboundedUpdate,
            MutationRisk::UnboundedDelete,
            MutationRisk::Truncate,
            MutationRisk::DropObject,
        ] {
            let cmd = Command::Execute {
                connection: bench.open,
                session: SessionId::new(),
                request: Box::new(
                    ExecRequest::new(QueryLanguage::SQL, "DELETE FROM events")
                        .with_intent(StatementIntent::Write)
                        .with_risk(risk),
                ),
            };
            let decision = bench.policy.authorize(&human(), &cmd, Environment::Local);
            assert!(
                decision.requires_approval(),
                "{risk:?} should require an approval, got {decision:?}"
            );

            let Decision::RequireApproval { reason, preview } = decision else {
                unreachable!("checked just above");
            };
            assert!(!reason.is_empty());
            let preview = preview.expect("a destructive operation is previewed");
            assert_eq!(
                preview.connection, "workshop",
                "the connection must be named"
            );
            assert_eq!(preview.statement, "DELETE FROM events");
        }
    }

    #[test]
    fn an_unbounded_risk_stays_refused_on_a_read_only_connection() {
        let bench = Bench::new();
        // Intent declared as read, destructive risk: the inconsistency is
        // settled on the cautious side, so the refusal applies.
        let cmd = Command::Execute {
            connection: bench.read_only_connection,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "TRUNCATE audit")
                    .with_intent(StatementIntent::Read)
                    .with_risk(MutationRisk::Truncate),
            ),
        };
        let decision = bench.policy.authorize(&human(), &cmd, Environment::Local);
        assert!(decision.is_denied(), "{decision:?}");
    }

    #[test]
    fn an_unknown_connection_closes_the_door() {
        let policy = DefaultPolicy::new();
        let unknown = ConnectionId::new();

        let write = Command::Execute {
            connection: unknown,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "INSERT INTO t VALUES (1)")
                    .with_intent(StatementIntent::Write),
            ),
        };
        assert!(
            policy
                .authorize(&human(), &write, Environment::Local)
                .is_denied(),
            "the gate cannot decide on what it does not see"
        );

        // A read, on the other hand, does not need the marking.
        let read = Command::Execute {
            connection: unknown,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "SELECT 1").with_intent(StatementIntent::Read),
            ),
        };
        assert!(
            policy
                .authorize(&human(), &read, Environment::Local)
                .is_allowed()
        );
    }

    #[test]
    fn the_connection_marking_prevails_over_a_too_permissive_announced_environment() {
        let policy = DefaultPolicy::new();
        let prod = ConnectionConfig::new("checkout", DriverId::postgres())
            .with_environment(Environment::Production);
        policy.register(&prod);

        let cmd = Command::Execute {
            connection: prod.id,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "UPDATE t SET a = 1 WHERE id = 2")
                    .with_intent(StatementIntent::Write),
            ),
        };

        // The caller announces `Local` — by mistake, or because it was wired wrong.
        let decision = policy.authorize(&human(), &cmd, Environment::Local);
        assert!(
            decision.requires_approval(),
            "the production marking must prevail: {decision:?}"
        );

        let decision = policy.authorize(&agent(), &cmd, Environment::Local);
        assert!(
            decision.is_denied(),
            "an agent stays strictly read-only on a production connection: {decision:?}"
        );
    }

    #[test]
    fn forgetting_a_connection_closes_the_door_again() {
        let bench = Bench::new();
        let cmd = bench.execute(false, StatementIntent::Write);
        assert!(
            bench
                .policy
                .authorize(&human(), &cmd, Environment::Local)
                .is_allowed()
        );

        bench.policy.forget(bench.open);
        assert!(
            bench
                .policy
                .authorize(&human(), &cmd, Environment::Local)
                .is_denied()
        );
    }

    #[test]
    fn cancelling_stays_possible_everywhere() {
        // Refusing a cancellation protects nothing and leaves a query running.
        let bench = Bench::new();
        for read_only in [false, true] {
            for env in ENVS {
                for actor in [human(), agent()] {
                    let cmd = Command::Cancel {
                        connection: bench.connection(read_only),
                        statement: StatementHandle::new(),
                    };
                    assert!(
                        bench.policy.authorize(&actor, &cmd, env).is_allowed(),
                        "cancellation refused: actor={actor} env={env} ro={read_only}"
                    );
                }
            }
        }
    }

    #[test]
    fn reading_the_local_catalog_is_allowed_everywhere_without_approval() {
        // A read of the local cache: no server contacted, no row value.
        // Refusing it in production would leave the agent guessing names;
        // submitting it to approval would teach clicking. What comes out of it
        // is governed by the privacy tier, in `oxyn-ai`.
        let bench = Bench::new();
        for read_only in [false, true] {
            for env in ENVS {
                for actor in [human(), agent()] {
                    let cmd = Command::DescribeCatalog {
                        connection: bench.connection(read_only),
                        focus: Some("customers".to_owned()),
                    };
                    assert!(!cmd.is_mutating());
                    assert!(!cmd.touches_database());
                    assert!(
                        bench.policy.authorize(&actor, &cmd, env).is_allowed(),
                        "catalog read refused: actor={actor} env={env} ro={read_only}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_agent_cannot_create_a_connection_without_approval() {
        // The exfiltration channel: an agent that would declare a connection
        // to the host of its choice.
        let policy = DefaultPolicy::new();
        let cfg = ConnectionConfig::new("elsewhere", DriverId::postgres())
            .with_environment(Environment::Local);
        let cmd = Command::CreateConnection {
            config: Box::new(cfg),
        };

        let decision = policy.authorize(&agent(), &cmd, Environment::Local);
        assert!(
            !decision.is_allowed(),
            "an agent does not create a connection without agreement: {decision:?}"
        );

        // A human, on the other hand, is not hindered: the connection is not
        // registered yet, and the "closed by default" rule only targets what
        // reaches a server.
        assert!(
            policy
                .authorize(&human(), &cmd, Environment::Local)
                .is_allowed()
        );
    }

    #[test]
    fn writing_a_local_document_triggers_no_approval() {
        let policy = DefaultPolicy::new();
        let cmd = Command::WriteDocument {
            workspace: crate::ids::WorkspaceId::new(),
            document: crate::ids::DocumentId::new(),
            text: "SELECT 1".into(),
        };
        for actor in [human(), agent()] {
            assert!(
                policy
                    .authorize(&actor, &cmd, Environment::Production)
                    .is_allowed(),
                "a local document is not a database write"
            );
        }
    }

    #[test]
    fn no_reason_leaks_a_connection_identifier() {
        let bench = Bench::new();
        let identifiers = [
            bench.open.to_string(),
            bench.read_only_connection.to_string(),
        ];

        for (is_agent, intent, env, read_only, _) in MATRIX {
            let actor = if *is_agent { agent() } else { human() };
            let cmd = bench.execute(*read_only, *intent);
            let decision = bench.policy.authorize(&actor, &cmd, *env);

            let text = match &decision {
                Decision::Allow => String::new(),
                Decision::Deny { reason } => reason.clone(),
                Decision::RequireApproval { reason, preview } => {
                    let mut t = reason.clone();
                    if let Some(p) = preview {
                        t.push_str(&p.connection);
                        t.push_str(&p.statement);
                    }
                    t
                }
            };
            for id in &identifiers {
                assert!(
                    !text.contains(id.as_str()),
                    "a connection identifier leaked into a reason: {text}"
                );
            }
        }
    }

    #[test]
    fn a_refusal_says_why() {
        let bench = Bench::new();
        for (is_agent, intent, env, read_only, expected) in MATRIX {
            if *expected != Shape::Deny {
                continue;
            }
            let actor = if *is_agent { agent() } else { human() };
            let cmd = bench.execute(*read_only, *intent);
            let Decision::Deny { reason } = bench.policy.authorize(&actor, &cmd, *env) else {
                panic!("refusal expected");
            };
            assert!(reason.len() > 10, "reason too poor to be shown: {reason}");
        }
    }

    #[test]
    fn the_gate_is_usable_behind_a_trait_object() {
        // It must be shareable between the interface thread, the executor and
        // the agents.
        let gate: std::sync::Arc<dyn PolicyGate> = std::sync::Arc::new(DefaultPolicy::new());
        assert_eq!(gate.name(), "DefaultPolicy");

        let cmd = Command::Connect {
            connection: ConnectionId::new(),
        };
        assert!(
            gate.authorize(&human(), &cmd, Environment::Production)
                .is_allowed()
        );
    }

    #[test]
    fn an_agent_does_not_move_the_session_context() {
        // The gesture neither reads nor writes data: classified `Read`, it
        // passes for a human, including on a read-only connection where
        // switching schema to read elsewhere is precisely the use. For an agent
        // it is a refusal, because the effect bears on the following statements.
        let bench = Bench::new();
        for read_only in [false, true] {
            let cmd = Command::SetSessionContext {
                connection: bench.connection(read_only),
                session: SessionId::new(),
                catalog: None,
                namespace: Some("analytics".to_owned()),
            };
            for env in [
                Environment::Local,
                Environment::Development,
                Environment::Staging,
                Environment::Production,
            ] {
                let for_human = bench.policy.authorize(&human(), &cmd, env);
                assert!(
                    for_human.is_allowed(),
                    "a human changes context: env={env} read_only={read_only} → {for_human:?}"
                );
                let for_agent = bench.policy.authorize(&agent(), &cmd, env);
                assert!(
                    !for_agent.is_allowed(),
                    "an agent never does it: env={env} read_only={read_only}"
                );
            }
        }
    }

    #[test]
    fn an_agent_does_not_drive_the_session_transaction() {
        // `COMMIT` is classified `Read`: it passes for a human everywhere,
        // read-only connection included, where closing a read transaction is
        // the very use. For an agent it is a refusal, not an approval, in every
        // environment: it would settle a transaction the user may have opened.
        let bench = Bench::new();
        for read_only in [false, true] {
            let mut cmd = bench.execute(read_only, StatementIntent::Read);
            if let Command::Execute { request, .. } = &mut cmd {
                request.transaction_control = true;
            }
            assert!(!cmd.is_mutating());
            for env in ENVS {
                let for_human = bench.policy.authorize(&human(), &cmd, env);
                assert!(
                    for_human.is_allowed(),
                    "env={env} read_only={read_only} → {for_human:?}"
                );
                let for_agent = bench.policy.authorize(&agent(), &cmd, env);
                assert!(
                    for_agent.is_denied(),
                    "env={env} read_only={read_only} → {for_agent:?}"
                );
            }
        }
    }

    #[test]
    fn only_the_human_declares_a_reconciled_write() {
        // Local, without a target connection: the human has nothing to confirm.
        // The agent is refused everywhere — not submitted to approval — because
        // it would assert a check of the server nobody made.
        let policy = DefaultPolicy::new();
        let cmd = Command::ReconcileHistoryEntry { entry: 1 };
        for env in ENVS {
            let for_human = policy.authorize(&human(), &cmd, env);
            assert!(for_human.is_allowed(), "env={env} → {for_human:?}");
            let for_agent = policy.authorize(&agent(), &cmd, env);
            assert!(
                matches!(for_agent, Decision::Deny { .. }),
                "env={env} → {for_agent:?}"
            );
        }
    }
}
