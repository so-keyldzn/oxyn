//! What a move protects: the session and its transaction follow the console,
//! the result keeps a reader throughout, another window's console never
//! moves, and a failed build gives the console back.

use oxyn_core::{CommandId, ConnectionId, DocumentId, Environment, ResultId, SessionId};

use crate::backend::{Backend, WindowKey};
use crate::ipc::windows::{ConsoleHandoff, HandedResult, HandoffRequest};
use crate::ipc::{CommandOutcome, ConnectResponse, ConnectionDraft, OpenConnection};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

fn open(runtime: &tokio::runtime::Runtime, backend: &Backend, name: &str) -> OpenConnection {
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: name.into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: std::collections::BTreeMap::new(),
    };
    match runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    {
        ConnectResponse::Open(open) => open,
        ConnectResponse::Approval { .. } => panic!("a local connection opens directly"),
    }
}

fn run(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    (connection, session): (ConnectionId, SessionId),
    sql: &str,
) -> ResultId {
    let outcome = runtime
        .block_on(backend.execute(CommandId::new(), connection, session, sql.into()))
        .expect("executes");
    let CommandOutcome::Executed { result, .. } = outcome else {
        panic!("a SELECT executes");
    };
    result.parse().expect("result id")
}

/// A SQLite connection opened in `window`, as the connection screen opens
/// it: its connection, and its first console's session.
fn opened(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    window: WindowKey,
) -> (ConnectionId, SessionId) {
    let open = open(runtime, backend, "billing");
    let connection: ConnectionId = open.connection.parse().expect("an id");
    let windows = &backend.inner.windows;
    windows.hold_connection(window, connection);
    for session in [&open.session, &open.console.session] {
        windows.claim_session(window, connection, session.parse().expect("an id"));
    }
    (connection, open.console.session.parse().expect("an id"))
}

fn console(
    connection: ConnectionId,
    session: SessionId,
    result: Option<HandedResult>,
) -> HandoffRequest {
    HandoffRequest::Console {
        connection: connection.to_string(),
        session: session.to_string(),
        document: DocumentId::new().to_string(),
        result,
        parameters: Vec::new(),
    }
}

#[test]
fn a_moved_console_keeps_its_session_and_the_new_window_adopts_it_once() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let source = backend.reserve_window(true).expect("a window");
    let (connection, session) = opened(&runtime, &backend, source);
    let target = backend.reserve_window(false).expect("a target");

    runtime
        .block_on(backend.hand_off(source, target, console(connection, session, None)))
        .expect("moved");

    let windows = &backend.inner.windows;
    assert!(windows.check_session(target, session).is_ok());
    assert!(
        windows.check_session(source, session).is_err(),
        "one console, one window"
    );
    assert!(
        windows.holds(source, connection),
        "the source keeps its catalog"
    );
    let Some(ConsoleHandoff::Console { open, .. }) = backend.inner.handoffs.take(target) else {
        panic!("the target adopts a console");
    };
    assert_eq!(
        open.console.session,
        session.to_string(),
        "the same session, not reopened"
    );
    assert_ne!(
        open.session, open.console.session,
        "its own catalog session"
    );
    assert!(
        backend.inner.handoffs.take(target).is_none(),
        "adopted once"
    );
}

/// The target reads the result before the source lets its view go: it is
/// never left without a reader, and nothing runs again (I-06).
#[test]
fn the_result_keeps_a_reader_while_it_moves() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let source = backend.reserve_window(true).expect("a window");
    let (connection, session) = opened(&runtime, &backend, source);
    let result = run(&runtime, &backend, (connection, session), "SELECT 1");
    backend.inner.windows.claim_result(source, result, true);
    let target = backend.reserve_window(false).expect("a target");
    let handed = HandedResult {
        result: result.to_string(),
        rows: 1,
        complete: true,
        truncated: false,
        cancelled: false,
        elapsed_ms: 1,
    };

    runtime
        .block_on(backend.hand_off(source, target, console(connection, session, Some(handed))))
        .expect("moved");
    // The source's console goes: its view is released, as its unmount does.
    backend.inner.windows.forget_result(source, result);
    backend.forget_result(result);

    assert!(backend.inner.windows.check_result(target, result).is_ok());
    assert!(
        backend.inner.executor.result(result).is_some(),
        "the target still reads the same buffer"
    );
}

#[test]
fn another_window_s_console_does_not_move() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let owner = backend.reserve_window(true).expect("a window");
    let intruder = backend.reserve_window(false).expect("another");
    let (connection, session) = opened(&runtime, &backend, owner);

    let refused =
        runtime.block_on(backend.check_handoff(intruder, &console(connection, session, None)));
    assert!(refused.is_err());
    assert!(backend.inner.windows.check_session(owner, session).is_ok());
}

#[test]
fn a_window_that_cannot_be_built_gives_the_console_back() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let source = backend.reserve_window(true).expect("a window");
    let (connection, session) = opened(&runtime, &backend, source);
    let target = backend.reserve_window(false).expect("a target");
    runtime
        .block_on(backend.hand_off(source, target, console(connection, session, None)))
        .expect("moved");

    backend.hand_back(target, source);
    runtime.block_on(backend.release_window(target));

    assert!(backend.inner.windows.check_session(source, session).is_ok());
    assert!(
        backend.inner.executor.sessions().get(session).is_some(),
        "the user's session, and any transaction on it, stay open"
    );
}

/// Checked and marked under one lock: a statement started while the new
/// window opens would answer to a window that no longer holds the console.
#[test]
fn nothing_runs_on_a_console_while_it_moves() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let source = backend.reserve_window(true).expect("a window");
    let (_, session) = opened(&runtime, &backend, source);
    let consoles = &backend.inner.workbench.consoles;

    let moving = consoles.begin_move(session).expect("idle");
    assert!(consoles.run_on(session).is_err());
    assert!(consoles.begin_move(session).is_err(), "one move at a time");
    drop(moving);
    assert!(consoles.run_on(session).is_ok(), "a refused move frees it");
}

/// The console leaves the source's line for the target's: the next launch
/// reopens it in one window, not as a copy no window claims.
#[test]
fn a_moved_console_changes_lines() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let source = backend.reserve_window(true).expect("a window");
    let (connection, session) = opened(&runtime, &backend, source);
    let request = console(connection, session, None);
    let HandoffRequest::Console { document, .. } = &request else {
        unreachable!("built as a console");
    };
    let document: DocumentId = document.parse().expect("an id");
    backend
        .inner
        .layouts
        .move_console(WindowKey::of(oxyn_core::WindowId::new()), source, document);
    let target = backend.reserve_window(false).expect("a target");

    runtime
        .block_on(backend.hand_off(source, target, request))
        .expect("moved");

    let layouts = &backend.inner.layouts;
    assert!(layouts.consoles(source).is_empty());
    assert_eq!(layouts.consoles(target), vec![document]);
}
