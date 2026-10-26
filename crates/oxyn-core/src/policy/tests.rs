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
            request: Box::new(ExecRequest::new(QueryLanguage::SQL, "SELECT 1").with_intent(intent)),
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

/// The read-only bound rests on the gate's marking: a caller that does not
/// know it cannot lift it, and one that knows it keeps it.
#[test]
fn the_read_only_marking_retained_is_the_more_restrictive() {
    let bench = Bench::new();
    let gate: &dyn PolicyGate = &bench.policy;
    let marked = bench.execute(true, StatementIntent::Read);
    let open = bench.execute(false, StatementIntent::Read);

    assert!(gate.retained_read_only(&marked, false));
    assert!(gate.retained_read_only(&marked, true));
    assert!(!gate.retained_read_only(&open, false));
    assert!(gate.retained_read_only(&open, true));

    // A command without a target keeps what its caller announced.
    let untargeted = Command::ListAiProviders;
    assert!(!gate.retained_read_only(&untargeted, false));
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
    assert_eq!(
        Shape::of(&decision),
        Shape::Deny,
        "an agent does not create a connection, even with agreement: {decision:?}"
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
fn an_agent_cannot_change_a_connection_switch() {
    // Turning sqlite-vec on is what lets bundled C code parse a file
    // (ADR-0054): a connection edit, the human's alone.
    let policy = DefaultPolicy::new();
    let config = ConnectionConfig::new("vectors", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param("path", "/data/semantiq.sqlite");
    policy.register(&config);
    let cmd = Command::UpdateConnection {
        config: Box::new(config.with_param("sqlite_vec", "true")),
    };
    for env in ENVS {
        assert_eq!(
            Shape::of(&policy.authorize(&agent(), &cmd, env)),
            Shape::Deny,
            "{env}"
        );
    }
    assert!(
        policy
            .authorize(&human(), &cmd, Environment::Local)
            .is_allowed()
    );
}

#[test]
fn a_production_creation_is_reviewed_under_the_name_it_saves() {
    let policy = DefaultPolicy::new();
    let cmd = Command::CreateConnection {
        config: Box::new(ConnectionConfig::new("billing", DriverId::postgres())),
    };

    let Decision::RequireApproval { reason, preview } =
        policy.authorize(&human(), &cmd, Environment::Production)
    else {
        panic!("a production connection is saved only on a decision");
    };
    assert!(reason.contains("\"billing\""), "{reason}");
    assert!(!reason.contains("unknown connection"), "{reason}");
    assert_eq!(
        preview.map(|preview| preview.connection),
        Some("billing".to_owned())
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

/// An `Execute` whose reclassified text moves the session's context, as
/// the executor hands it to the gate for `USE other`.
fn typed_context_change(bench: &Bench, read_only: bool) -> Command {
    let mut cmd = bench.execute(read_only, StatementIntent::Unknown);
    if let Command::Execute { request, .. } = &mut cmd {
        request.context_change = true;
    }
    cmd
}

#[test]
fn an_agent_does_not_type_a_context_change() {
    // `USE other` is `Unknown`, so on a development database an agent's
    // statement would only ask for an approval — and an approval ends up
    // being clicked (I-02). The flag turns it into a refusal in every
    // environment, and that refusal prevails over the production and
    // read-only ones: the reason must send the user to the selector.
    let bench = Bench::new();
    for read_only in [false, true] {
        let cmd = typed_context_change(&bench, read_only);
        assert!(cmd.changes_context());
        for env in ENVS {
            let decision = bench.policy.authorize(&agent(), &cmd, env);
            let Decision::Deny { reason } = &decision else {
                panic!("a refusal is expected: env={env} read_only={read_only} → {decision:?}");
            };
            assert!(
                reason.contains("context selector"),
                "env={env} read_only={read_only}: the reason must point to the selector: \
                     {reason}"
            );
            assert!(
                !reason.contains("read-only"),
                "env={env} read_only={read_only}: the read-only or production reason won \
                     over the context one: {reason}"
            );
        }
    }

    // The control: without the flag, the same agent statement on a
    // development database only asks for an approval. Without this, the
    // loop above could pass for another reason.
    let plain = bench.execute(false, StatementIntent::Unknown);
    assert!(!plain.changes_context());
    let decision = bench
        .policy
        .authorize(&agent(), &plain, Environment::Development);
    assert!(decision.requires_approval(), "{decision:?}");
}

#[test]
fn a_typed_context_change_is_not_refused_to_the_user() {
    // The refusal is the agent's. The user types `USE other` in their own
    // console; being `Unknown`, it is a write for the gate: free off
    // production, and on production it asks for the confirmation that
    // names the connection (I-02).
    let bench = Bench::new();

    let open = typed_context_change(&bench, false);
    for env in [
        Environment::Local,
        Environment::Development,
        Environment::Staging,
    ] {
        let decision = bench.policy.authorize(&human(), &open, env);
        assert!(!decision.is_denied(), "env={env} → {decision:?}");
    }
    let decision = bench
        .policy
        .authorize(&human(), &open, Environment::Production);
    let Decision::RequireApproval { reason, .. } = &decision else {
        panic!("an approval is expected on production: {decision:?}");
    };
    assert!(
        reason.contains("workshop"),
        "the confirmation must name the connection: {reason}"
    );

    // On a read-only connection the statement stays refused to the user as
    // a write, and for the read-only reason, not for the context one: this
    // is what changes for a human now that `USE` is no longer a `Read`.
    let read_only = typed_context_change(&bench, true);
    for env in ENVS {
        let decision = bench.policy.authorize(&human(), &read_only, env);
        let Decision::Deny { reason } = &decision else {
            panic!("a write on a read-only connection is refused: env={env} → {decision:?}");
        };
        assert!(reason.contains("marked read-only"), "env={env}: {reason}");
        assert!(
            !reason.contains("context selector"),
            "env={env}: the agent's reason leaked to the user: {reason}"
        );
    }
}

#[test]
fn a_typed_context_change_keeps_the_agent_at_least_as_restricted_as_the_human() {
    // The test `/commande` asks for, on the one flag the generic matrix
    // does not vary.
    let bench = Bench::new();
    for read_only in [false, true] {
        let cmd = typed_context_change(&bench, read_only);
        for env in ENVS {
            let for_human = bench.policy.authorize(&human(), &cmd, env);
            let for_agent = bench.policy.authorize(&agent(), &cmd, env);
            assert!(
                for_agent.restrictiveness() >= for_human.restrictiveness(),
                "env={env} read_only={read_only}: agent={for_agent:?} human={for_human:?}"
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
