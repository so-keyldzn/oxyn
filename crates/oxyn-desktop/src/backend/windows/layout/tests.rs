//! What a window's line protects: its consoles written at once, never
//! another window's, a closed window not coming back, and the working copies
//! no window claims reopened once, in the first window.

use oxyn_core::{CommandId, DocumentId, WindowGeometry, WindowLayout};

use crate::backend::{Backend, WindowKey};
use crate::ipc::library::DocumentChange;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

fn placed(backend: &Backend, initial: bool) -> WindowKey {
    let key = backend.reserve_window(initial).expect("a window");
    backend.inner.layouts.place(
        key,
        WindowGeometry {
            x: Some(20.0),
            y: Some(40.0),
            width: 1280.0,
            height: 820.0,
            maximized: false,
        },
    );
    key
}

/// A working copy, written as a console's autosave writes it.
fn working_copy(runtime: &tokio::runtime::Runtime, backend: &Backend) -> DocumentId {
    let document = DocumentId::new();
    runtime
        .block_on(backend.save_query_document(
            CommandId::new(),
            document,
            None,
            DocumentChange {
                document: document.to_string(),
                revision: 1,
                title: "console".into(),
                text: "SELECT 1".into(),
                connection: None,
                named: false,
            },
        ))
        .expect("autosaved");
    document
}

/// What the file holds, read as the next launch reads it.
fn stored(backend: &Backend) -> Vec<WindowLayout> {
    let executor = &backend.inner.executor;
    executor
        .store()
        .windows()
        .adopt(executor.workspace(), backend.inner.workbench.local.session)
        .expect("stored layouts")
}

#[test]
fn a_window_s_consoles_are_written_at_once_and_never_another_s() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let left = placed(&backend, true);
    let right = placed(&backend, false);
    let (mine, theirs) = (
        working_copy(&runtime, &backend),
        working_copy(&runtime, &backend),
    );
    backend
        .inner
        .windows
        .claim_document(right, theirs)
        .expect("the right window writes it");

    runtime
        .block_on(backend.report_window_consoles(left, vec![mine, theirs, mine], Some(theirs)))
        .expect("written");

    let layouts = stored(&backend);
    let left_line = layouts
        .iter()
        .find(|layout| layout.window == left.id())
        .expect("the left window's line");
    assert_eq!(left_line.consoles, vec![mine]);
    assert_eq!(
        left_line.active_document, None,
        "the console in front was another window's"
    );
}

#[test]
fn a_window_closed_while_others_stay_does_not_come_back() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let left = placed(&backend, true);
    let right = placed(&backend, false);
    for window in [left, right] {
        runtime
            .block_on(backend.save_layout(window))
            .expect("written");
    }
    runtime.block_on(backend.remove_layout(right));

    let windows: Vec<_> = stored(&backend)
        .iter()
        .map(|layout| layout.window)
        .collect();
    assert_eq!(windows, vec![left.id()]);
}

#[test]
fn a_window_never_placed_writes_nothing() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let unplaced = backend.reserve_window(true).expect("a window");
    runtime
        .block_on(backend.save_layout(unplaced))
        .expect("nothing to write");
    assert!(stored(&backend).is_empty());
}

/// Copies left open by an earlier version, or by a close not confirmed,
/// belong to no window: the first takes them, once, after its own.
#[test]
fn the_first_window_takes_the_copies_no_window_claims_once() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let first = placed(&backend, true);
    let second = placed(&backend, false);
    let (own, orphan, other) = (
        working_copy(&runtime, &backend),
        working_copy(&runtime, &backend),
        working_copy(&runtime, &backend),
    );
    runtime
        .block_on(backend.report_window_consoles(first, vec![own], None))
        .expect("written");
    runtime
        .block_on(backend.report_window_consoles(second, vec![other], None))
        .expect("written");

    let ids = |entries: Vec<crate::ipc::library::DocumentEntry>| {
        entries
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(runtime
            .block_on(backend.restored_consoles(first))
            .expect("read")),
        vec![own.to_string(), orphan.to_string()]
    );
    assert_eq!(
        ids(runtime
            .block_on(backend.restored_consoles(first))
            .expect("read")),
        vec![own.to_string()],
        "the unclaimed copies are offered once"
    );
    assert_eq!(
        ids(runtime
            .block_on(backend.restored_consoles(second))
            .expect("read")),
        vec![other.to_string()]
    );
}

#[test]
fn a_list_past_the_bound_is_refused() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let window = placed(&backend, true);
    let many = (0..=WindowLayout::MAX_CONSOLES)
        .map(|_| DocumentId::new())
        .collect();
    assert!(
        runtime
            .block_on(backend.report_window_consoles(window, many, None))
            .is_err()
    );
}

/// An agent opens, closes and arranges no window (ADR-0043).
#[test]
fn an_agent_cannot_write_a_layout() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let window = placed(&backend, true);
    let executor = &backend.inner.executor;
    let outcome = runtime
        .block_on(executor.dispatch(
            oxyn_core::Actor::agent(oxyn_core::AgentId::new(), oxyn_core::AgentSessionId::new()),
            oxyn_core::Command::WriteWindowLayout {
                workspace: executor.workspace(),
                change: Box::new(oxyn_core::WindowLayoutChange::Remove(window.id())),
            },
            &oxyn_core::CancelToken::new(),
        ))
        .expect("answered");
    assert!(matches!(outcome, oxyn_exec::Outcome::Denied { .. }));
}
