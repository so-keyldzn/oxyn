//! Independent console sessions over one configured SQLite database.

use std::collections::BTreeMap;

use oxyn_core::Environment;

use super::*;
use crate::ipc::consoles::{ParameterInput, ParameterKind};
use crate::ipc::{ConnectResponse, ConnectionDraft, OpenConnection};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

fn open(runtime: &tokio::runtime::Runtime, backend: &Backend) -> OpenConnection {
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "console isolation".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    };
    match runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    {
        ConnectResponse::Open(open) => open,
        ConnectResponse::Approval { .. } => panic!("a local connection needs no approval"),
    }
}

fn run(target: RunTarget, sql: &str, parameters: Vec<ParameterInput>) -> ConsoleRun {
    ConsoleRun {
        sql: sql.into(),
        target,
        parameters,
        explain: false,
    }
}

#[test]
fn two_consoles_are_two_sessions_and_closing_one_frees_only_its_own() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let catalog: SessionId = open.session.parse().expect("catalog session");
    let first: SessionId = open.console.session.parse().expect("first console");
    let second: SessionId = runtime
        .block_on(backend.open_console(CommandId::new(), connection))
        .expect("second console")
        .session
        .parse()
        .expect("second console id");
    assert_ne!(
        catalog, first,
        "the first console does not share the catalog's session"
    );
    assert_ne!(first, second, "two consoles never share a session");

    runtime
        .block_on(backend.close_console(connection, first))
        .expect("closes");
    let sessions = backend.inner.executor.sessions();
    assert!(
        sessions.get(first).is_none(),
        "the closed console released its session"
    );
    assert!(
        sessions.get(second).is_some(),
        "the sibling keeps its session"
    );
    assert!(
        sessions.get(catalog).is_some(),
        "the catalog keeps its session"
    );
    assert!(matches!(
        runtime.block_on(backend.run_console(
            CommandId::new(),
            connection,
            second,
            run(RunTarget::All, "SELECT 1", Vec::new())
        )),
        Ok(CommandOutcome::Executed { .. })
    ));
    assert!(
        runtime
            .block_on(backend.run_console(
                CommandId::new(),
                connection,
                first,
                run(RunTarget::All, "SELECT 1", Vec::new())
            ))
            .is_err(),
        "a closed console's session runs nothing"
    );
}

#[test]
fn the_statement_under_the_cursor_runs_alone_with_its_bound_values() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let session: SessionId = open.console.session.parse().expect("console");

    // « é » is two UTF-8 bytes and one UTF-16 unit: the cursor is in the second
    // statement only if the offsets are converted.
    let sql = "SELECT 'é' AS a; SELECT ? AS b";
    let cursor = sql.encode_utf16().count() - 2;
    let outcome = runtime
        .block_on(backend.run_console(
            CommandId::new(),
            connection,
            session,
            run(
                RunTarget::Statement { cursor },
                sql,
                vec![ParameterInput {
                    kind: ParameterKind::Int64,
                    text: "42".into(),
                }],
            ),
        ))
        .expect("runs");
    let CommandOutcome::Executed { columns, rows, .. } = outcome else {
        panic!("a SELECT executes, got {outcome:?}");
    };
    assert_eq!(rows, 1);
    assert_eq!(
        columns.first().map(|column| column.name.as_str()),
        Some("b")
    );
}

#[test]
fn a_value_that_does_not_convert_stops_the_run_without_quoting_it() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let session: SessionId = open.console.session.parse().expect("console");
    let secret = "customer-4111-1111";
    let error = runtime
        .block_on(backend.run_console(
            CommandId::new(),
            connection,
            session,
            run(
                RunTarget::All,
                "SELECT ?",
                vec![ParameterInput {
                    kind: ParameterKind::Uuid,
                    text: secret.into(),
                }],
            ),
        ))
        .expect_err("not a UUID");
    assert!(!error.message.contains(secret), "{}", error.message);
    assert!(error.message.contains("parameter 1"), "{}", error.message);
}

#[test]
fn a_cursor_between_statements_runs_nothing() {
    assert!(
        targeted_text(
            "SELECT 1;\n\n\nSELECT 2",
            RunTarget::Statement { cursor: 11 },
            oxyn_core::SqlDialect::Postgres
        )
        .is_err()
    );
    assert_eq!(byte_offset("é", 1), Some(2));
    assert_eq!(byte_offset("😀", 1), None, "inside a surrogate pair");
    assert_eq!(byte_offset("a", 5), None);
}

#[test]
fn a_console_starts_with_the_state_its_session_reported_and_learns_the_next_one() {
    // ADR-0039 §4: the initial state is read, not assumed; the next one
    // arrives as an execution event the front can parse.
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    assert_eq!(
        open.console.transaction_state,
        crate::ipc::TransactionStateView::Idle,
        "a fresh SQLite connection is in autocommit"
    );
    let connection: ConnectionId = open.connection.parse().expect("connection id");
    let session: SessionId = open.console.session.parse().expect("console session");

    let mut events = backend.subscribe();
    let id = CommandId::new();
    runtime
        .block_on(backend.run_console(
            id,
            connection,
            session,
            run(RunTarget::All, "BEGIN", Vec::new()),
        ))
        .expect("BEGIN runs");
    let mut forwarded = Vec::new();
    while let Ok(event) = events.try_recv() {
        if event.command == id
            && let Some(kind) = crate::ipc::ExecutionEventKind::of(&event.event)
        {
            forwarded.push(serde_json::to_value(kind).expect("serialisable"));
        }
    }
    let state = forwarded
        .iter()
        .position(|kind| kind["type"] == "transactionState")
        .expect("the state is forwarded, not dropped by the bridge");
    assert_eq!(forwarded[state]["state"], "open");
    assert_eq!(forwarded[state]["session"], open.console.session.as_str());
    let terminal = forwarded
        .iter()
        .position(|kind| kind["type"] == "completed")
        .expect("completed");
    assert!(
        state < terminal,
        "the state comes before the terminal event"
    );
}
