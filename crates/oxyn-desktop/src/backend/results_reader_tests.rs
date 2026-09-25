//! A retained result is released by its last reader, not by its first.

use super::access_tests::{ids, open, run, runtime, sql_runs};
use super::*;
use crate::ipc::library::RetainedResult;
use oxyn_core::{ExecRequest, QueryLanguage, SqlDialect};

fn opened(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    connection: ConnectionId,
    result: ResultId,
) {
    match runtime
        .block_on(backend.open_retained_result(connection, result))
        .expect("opens")
    {
        RetainedResult::Open { .. } => {}
        RetainedResult::Expired => panic!("the result is still retained"),
    }
}

fn pages(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    connection: ConnectionId,
    result: ResultId,
) -> bool {
    matches!(
        runtime
            .block_on(backend.read_result_page(connection, result, 0, 10))
            .expect("reads"),
        ResultWindow::Page(_)
    )
}

#[test]
fn a_history_tab_keeps_reading_after_its_console_reruns() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let console = ids(&open(&runtime, &backend, "console"));
    let connection = console.0;

    let first = run(&runtime, &backend, console, "SELECT 1 AS first");
    // Two History tabs on the same run.
    opened(&runtime, &backend, connection, first);
    opened(&runtime, &backend, connection, first);

    // The console reruns: it releases the result it was showing.
    let second = run(&runtime, &backend, console, "SELECT 2 AS second");
    backend.forget_result(first);
    let runs = sql_runs(&backend);
    backend.inner.executor.prune_results();
    assert!(pages(&runtime, &backend, connection, first));

    // One tab closes: the other still reads.
    backend.forget_result(first);
    backend.inner.executor.prune_results();
    assert!(pages(&runtime, &backend, connection, first));
    assert_eq!(sql_runs(&backend), runs, "nothing is run again");

    // The last reader releases it at once, and the console's own result is
    // untouched.
    backend.forget_result(first);
    assert!(backend.inner.executor.result(first).is_none());
    assert!(!backend.holds_shown(first));
    assert!(matches!(
        runtime.block_on(backend.read_result_page(connection, first, 0, 10)),
        Ok(ResultWindow::Expired)
    ));
    assert!(pages(&runtime, &backend, connection, second));
}

#[test]
fn a_console_closed_before_its_history_tab_leaves_the_tab_reading() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let console = ids(&open(&runtime, &backend, "console"));
    let connection = console.0;
    let result = run(&runtime, &backend, console, "SELECT 1 AS shown");
    opened(&runtime, &backend, connection, result);

    // The console tab closes, and many other results pass by.
    backend.forget_result(result);
    for index in 0..40 {
        let other = run(&runtime, &backend, console, &format!("SELECT {index} AS n"));
        backend.forget_result(other);
    }
    backend.inner.executor.prune_results();

    assert!(pages(&runtime, &backend, connection, result));
    backend.forget_result(result);
    assert!(backend.inner.executor.result(result).is_none());
}

/// A result the front only opened — an agent's, owned by its conversation —
/// is not deleted by the view that closes: it goes back to retention.
#[test]
fn closing_a_view_of_a_result_it_did_not_run_leaves_it_to_its_owner() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let (connection, session) = ids(&open(&runtime, &backend, "agent"));
    let outcome = runtime
        .block_on(backend.inner.executor.dispatch(
            oxyn_core::Actor::Human,
            Command::Execute {
                connection,
                session,
                request: Box::new(ExecRequest::new(
                    QueryLanguage::Sql(SqlDialect::Sqlite),
                    "SELECT 1 AS owned_elsewhere",
                )),
            },
            &CancelToken::new(),
        ))
        .expect("executes");
    let Outcome::Executed { result, .. } = outcome else {
        panic!("a SELECT executes");
    };

    opened(&runtime, &backend, connection, result);
    assert!(backend.holds_shown(result), "held while the view reads it");
    backend.forget_result(result);

    assert!(!backend.holds_shown(result));
    assert!(
        backend.inner.executor.result(result).is_some(),
        "its owner still holds it"
    );
}
