use super::*;
use crate::ids::DriverId;
use crate::query::QueryLanguage;

fn execute(intent: StatementIntent, risk: MutationRisk) -> Command {
    Command::Execute {
        connection: ConnectionId::new(),
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::SQL, "SELECT 1")
                .with_intent(intent)
                .with_risk(risk),
        ),
    }
}

#[test]
fn preview_is_a_remote_read_with_its_own_audit_name() {
    let connection = ConnectionId::new();
    let command = Command::PreviewRelation {
        connection,
        session: SessionId::new(),
        catalog: Some("database".into()),
        namespace: Some("schema".into()),
        relation: "table\"; DROP TABLE audit; --".into(),
        limit: 200,
        // A non-trivial shape, so that the serialized round trip really
        // covers it: a `Default` would pass without proving anything.
        shape: crate::preview::PreviewShape {
            sort: vec![crate::preview::PreviewSort::descending("id")],
            predicate: Some("note LIKE '100%'".into()),
            offset: 200,
            columns: Some(vec!["id".into(), "note".into()]),
        },
    };
    assert_eq!(command.intent(), StatementIntent::Read);
    assert_eq!(command.target_connection(), Some(connection));
    assert_eq!(command.name(), "PreviewRelation");
    assert!(command.touches_database());
    assert!(!command.is_mutating());
    assert_eq!(command.statement_text(), None);
    let json = serde_json::to_string(&command).expect("serializable command");
    let decoded: Command = serde_json::from_str(&json).expect("typed round trip");
    assert_eq!(decoded, command);
}

#[test]
fn catalog_scopes_are_reads_for_both_actors_even_in_production() {
    use crate::{DefaultPolicy, Environment, PolicyGate};
    let config = ConnectionConfig::new("fixture", DriverId::sqlite()).read_only();
    let gate = DefaultPolicy::new();
    gate.register(&config);
    for command in [
        Command::RefreshCatalog {
            connection: config.id,
        },
        Command::RefreshCatalogScope {
            connection: config.id,
            scope: CatalogRefreshScope::Root,
        },
        Command::RefreshCatalogScope {
            connection: config.id,
            scope: CatalogRefreshScope::Namespaces { catalog: None },
        },
        Command::RefreshCatalogScope {
            connection: config.id,
            scope: CatalogRefreshScope::Relations {
                catalog: None,
                namespace: None,
            },
        },
        Command::RefreshCatalogScope {
            connection: config.id,
            scope: CatalogRefreshScope::Relation {
                catalog: None,
                namespace: None,
                relation: "users".into(),
            },
        },
        Command::RefreshCatalogScope {
            connection: config.id,
            scope: CatalogRefreshScope::ResolutionPath,
        },
    ] {
        assert_eq!(command.target_connection(), Some(config.id));
        assert!(command.touches_database());
        assert!(!command.is_mutating());
        assert!(command.statement_text().is_none());
        for actor in [
            Actor::Human,
            Actor::agent(AgentId::new(), AgentSessionId::new()),
        ] {
            assert!(
                gate.authorize(&actor, &command, Environment::Production)
                    .is_allowed()
            );
        }
        let json = serde_json::to_string(&command).expect("serializable command");
        assert_eq!(
            serde_json::from_str::<Command>(&json).expect("round trip"),
            command
        );
    }
}

#[test]
fn an_actor_is_recognized() {
    assert!(Actor::Human.is_human());
    assert!(!Actor::Human.is_agent());

    let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
    assert!(agent.is_agent());
    assert!(!agent.is_human());
}

#[test]
fn displaying_an_actor_does_not_show_its_conversation() {
    let session = AgentSessionId::new();
    let agent = Actor::agent(AgentId::new(), session);
    let rendered = agent.to_string();
    assert_eq!(rendered, "agent");
    assert!(!rendered.contains(&session.to_string()));
}

#[test]
fn execute_relays_the_declared_intent_without_guessing_it() {
    assert_eq!(
        execute(StatementIntent::Read, MutationRisk::None).intent(),
        StatementIntent::Read
    );
    assert_eq!(
        execute(StatementIntent::Ddl, MutationRisk::DropObject).intent(),
        StatementIntent::Ddl
    );
}

#[test]
fn an_explain_analyze_declared_as_write_stays_a_write() {
    // The text says "EXPLAIN", the effect is a deletion. It is the
    // declared intent that counts, never the statement's name.
    let cmd = Command::Execute {
        connection: ConnectionId::new(),
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::SQL, "EXPLAIN ANALYZE DELETE FROM events")
                .with_intent(StatementIntent::Write)
                .with_risk(MutationRisk::UnboundedDelete),
        ),
    };
    assert!(cmd.is_mutating());
    assert_eq!(cmd.mutation_risk(), MutationRisk::UnboundedDelete);
}

#[test]
fn an_unqualified_execution_is_mutating() {
    let cmd = Command::Execute {
        connection: ConnectionId::new(),
        session: SessionId::new(),
        request: Box::new(ExecRequest::new(QueryLanguage::SQL, "CALL do_the_thing()")),
    };
    assert_eq!(cmd.intent(), StatementIntent::Unknown);
    assert!(cmd.is_mutating(), "when in doubt, we protect");
}

#[test]
fn read_commands_are_not_mutating() {
    let c = ConnectionId::new();
    for cmd in [
        Command::Connect { connection: c },
        Command::Disconnect { connection: c },
        Command::RefreshCatalog { connection: c },
        Command::Cancel {
            connection: c,
            statement: StatementHandle::new(),
        },
        Command::OpenDocument {
            workspace: WorkspaceId::new(),
            document: DocumentId::new(),
        },
    ] {
        assert!(!cmd.is_mutating(), "{} should be non-mutating", cmd.name());
    }
}

#[test]
fn cancelling_is_never_a_mutation() {
    // A cancellation that must be approved is not a cancellation.
    let cmd = Command::Cancel {
        connection: ConnectionId::new(),
        statement: StatementHandle::new(),
    };
    assert_eq!(cmd.intent(), StatementIntent::Read);
    assert!(!cmd.is_mutating());
}

#[test]
fn connection_management_is_treated_as_ddl() {
    let cfg = ConnectionConfig::new("new", DriverId::postgres());
    let cmd = Command::CreateConnection {
        config: Box::new(cfg.clone()),
    };
    assert_eq!(cmd.intent(), StatementIntent::Ddl);
    assert!(cmd.is_mutating());
    assert_eq!(cmd.target_connection(), Some(cfg.id));
    assert!(
        !cmd.touches_database(),
        "creating a connection reaches no server"
    );
}

#[test]
fn testing_a_connection_is_a_server_read_refused_to_an_agent() {
    use crate::{DefaultPolicy, Environment, PolicyGate};

    let config = ConnectionConfig::new("to test", DriverId::postgres())
        .with_secret_ref("keychain://oxyn/trial");
    let cmd = Command::TestConnection {
        config: Box::new(config.clone()),
    };
    assert_eq!(cmd.intent(), StatementIntent::Read);
    assert!(!cmd.is_mutating(), "nothing is saved or written");
    assert!(cmd.touches_database(), "the server is reached");
    assert_eq!(cmd.target_connection(), Some(config.id));
    assert_eq!(cmd.name(), "TestConnection");
    assert_eq!(cmd.statement_text(), None);

    // The configuration is unknown to the gate, and marked production by
    // default: the human may still try it, since nothing is written.
    let gate = DefaultPolicy::new();
    assert!(
        gate.authorize(&Actor::Human, &cmd, Environment::Production)
            .is_allowed()
    );
    // An agent may not, even on a local connection: choosing the host is
    // the exfiltration channel, whatever the marking.
    let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
    let decision = gate.authorize(&agent, &cmd, Environment::Local);
    assert!(decision.is_denied(), "{decision:?}");
    assert!(!decision.requires_approval(), "a refusal (I-02)");

    // I-03: the reference is not in the `Debug`, and a round trip keeps
    // the command whole.
    assert!(!format!("{cmd:?}").contains("keychain://oxyn/trial"));
    let json = serde_json::to_string(&cmd).expect("serializable command");
    assert_eq!(
        serde_json::from_str::<Command>(&json).expect("typed round trip"),
        cmd
    );
}

#[test]
fn writing_a_document_touches_no_database() {
    let cmd = Command::WriteDocument {
        workspace: WorkspaceId::new(),
        document: DocumentId::new(),
        text: "SELECT 1".into(),
    };
    assert_eq!(cmd.intent(), StatementIntent::Read);
    assert_eq!(cmd.target_connection(), None);
    assert!(!cmd.touches_database());
}

#[test]
fn every_command_targeting_a_server_names_its_connection() {
    // Without it, the PolicyGate could not find the environment marking
    // or the read-only flag.
    let c = ConnectionId::new();
    let commands = [
        Command::Connect { connection: c },
        Command::Disconnect { connection: c },
        Command::RefreshCatalog { connection: c },
        Command::Cancel {
            connection: c,
            statement: StatementHandle::new(),
        },
        Command::Export {
            connection: c,
            result: ResultId::new(),
            format: ExportFormat::Csv,
            destination: PathBuf::from("/tmp/export.csv"),
        },
        execute(StatementIntent::Read, MutationRisk::None),
    ];
    for cmd in commands {
        assert!(cmd.touches_database(), "{}", cmd.name());
        assert!(
            cmd.target_connection().is_some(),
            "{} does not name its connection",
            cmd.name()
        );
    }
}

#[test]
fn the_preview_only_gives_the_written_text() {
    let cmd = execute(StatementIntent::Read, MutationRisk::None);
    assert_eq!(cmd.statement_text(), Some("SELECT 1"));
    assert_eq!(
        Command::Connect {
            connection: ConnectionId::new()
        }
        .statement_text(),
        None
    );
}

#[test]
fn a_command_debug_does_not_show_bound_values() {
    let cmd = Command::Execute {
        connection: ConnectionId::new(),
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::SQL, "INSERT INTO t VALUES ($1)").with_params(vec![
                crate::value::ScalarValue::Text("the-users-secret".into()),
            ]),
        ),
    };
    let rendered = format!("{cmd:?}");
    assert!(
        !rendered.contains("the-users-secret"),
        "bound value leaked: {rendered}"
    );

    // The `Debug` is only one of the six channels. A workspace file is
    // another, and a protection that only holds on the first would be
    // defeated by the first caller that persists a command.
    let written = serde_json::to_string(&cmd).expect("serializable command");
    assert!(
        !written.contains("the-users-secret"),
        "bound value written to a file: {written}"
    );
    let read_back: Command = serde_json::from_str(&written).expect("readable command");
    let Command::Execute { request, .. } = &read_back else {
        panic!("the variant is kept")
    };
    assert!(
        request.params.is_empty(),
        "a command read back comes back without its values, and the server will refuse it"
    );
    assert_eq!(request.text, "INSERT INTO t VALUES ($1)");
}

#[test]
fn declaring_a_provider_reaches_no_database_and_stays_refused_to_an_agent() {
    use crate::ai::{AiProviderConfig, AiProviderKind, ProviderId};
    use crate::{DefaultPolicy, Environment, PolicyGate};

    let config = AiProviderConfig::new(
        ProviderId::ollama(),
        AiProviderKind::OpenAiCompatible,
        "Laptop Ollama",
        "http://localhost:11434/v1",
        "llama3.2",
    );
    let gate = DefaultPolicy::new();
    let agent = Actor::agent(AgentId::new(), AgentSessionId::new());

    for cmd in [
        Command::ListAiProviders,
        Command::SaveAiProvider {
            config: Box::new(config.clone()),
        },
        Command::RemoveAiProvider {
            id: config.id.clone(),
        },
    ] {
        // A provider is declared per machine: no target connection, no
        // server reached (ADR-0023).
        assert_eq!(cmd.target_connection(), None, "{}", cmd.name());
        assert!(!cmd.touches_database(), "{}", cmd.name());
        assert!(!cmd.is_mutating(), "{}", cmd.name());
        assert_eq!(cmd.statement_text(), None, "{}", cmd.name());

        // The human does not have to confirm a local setting: a
        // confirmation that matches nothing ends up clicked without being read.
        assert!(
            gate.authorize(&Actor::Human, &cmd, Environment::Production)
                .is_allowed(),
            "{}",
            cmd.name()
        );

        // The agent, on the other hand, is refused on what **writes** the
        // declaration: the endpoint it speaks through is not chosen by itself.
        let decision = gate.authorize(&agent, &cmd, Environment::Local);
        if matches!(cmd, Command::ListAiProviders) {
            assert!(decision.is_allowed(), "listing declares nothing");
        } else {
            assert!(decision.is_denied(), "{} : {decision:?}", cmd.name());
            assert!(
                !decision.requires_approval(),
                "a refusal, never a stronger confirmation (I-02)"
            );
        }

        let json = serde_json::to_string(&cmd).expect("serializable command");
        assert_eq!(
            serde_json::from_str::<Command>(&json).expect("typed round trip"),
            cmd
        );
    }
}

#[test]
fn a_provider_command_carries_no_key() {
    // I-03: the only field meant for the keychain is a reference, and the
    // command's `Debug` does not render it.
    use crate::ai::{AiProviderConfig, AiProviderKind, ProviderId};

    let cmd = Command::SaveAiProvider {
        config: Box::new(
            AiProviderConfig::new(
                ProviderId::openai(),
                AiProviderKind::OpenAi,
                "OpenAI",
                "https://api.openai.com/v1",
                "gpt-5",
            )
            .with_secret_ref("keychain://oxyn/openai"),
        ),
    };
    let rendered = format!("{cmd:?}");
    assert!(
        !rendered.contains("keychain://oxyn/openai"),
        "reference leaked: {rendered}"
    );
    assert!(rendered.contains("api.openai.com"), "{rendered}");
}

#[test]
fn audit_names_are_unique() {
    let c = ConnectionId::new();
    let names = [
        Command::Connect { connection: c }.name(),
        Command::Disconnect { connection: c }.name(),
        execute(StatementIntent::Read, MutationRisk::None).name(),
        Command::DeleteConnection { connection: c }.name(),
    ];
    let mut sorted: Vec<&str> = names.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names.len(),
        "two commands carry the same name"
    );
}
