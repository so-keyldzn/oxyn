//! Who may read a retained result, and for how long.

use super::*;
use crate::ipc::{ConnectResponse, ConnectionDraft, OpenConnection};
use oxyn_core::{Environment, SessionId};

pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

pub(super) fn open(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    name: &str,
) -> OpenConnection {
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

pub(super) fn ids(open: &OpenConnection) -> (ConnectionId, SessionId) {
    (
        open.connection.parse().expect("connection id"),
        open.session.parse().expect("session id"),
    )
}

pub(super) fn run(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    (connection, session): (ConnectionId, SessionId),
    sql: &str,
) -> ResultId {
    let outcome = runtime
        .block_on(backend.execute(CommandId::new(), connection, session, sql.into()))
        .expect("executes");
    let CommandOutcome::Executed { result, .. } = outcome else {
        panic!("a SELECT executes, got {outcome:?}");
    };
    result.parse().expect("result id")
}

pub(super) fn sql_runs(backend: &Backend) -> u64 {
    backend
        .inner
        .executor
        .store()
        .history()
        .count()
        .expect("SQL history")
}

/// Twenty SQLite batches of 8 192 integers, 64 KiB each. Under a 512 KiB
/// budget the first ones stay in memory, a quarter of it caches the pages
/// read back, and the rest spills to disk.
const ROWS: usize = 20 * 8192;
const BUDGET: usize = 512 * 1024;

#[test]
fn another_connection_is_refused_alike_in_memory_in_the_cache_and_on_disk() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary_spilling(BUDGET).expect("temporary backend");
    let owner = ids(&open(&runtime, &backend, "owner"));
    let (other, _) = ids(&open(&runtime, &backend, "other"));
    // Through the executor: a console run stops at `ExecLimits`' default
    // row limit, far below the batches this test needs.
    let outcome = runtime
        .block_on(
            backend.inner.executor.dispatch(
                oxyn_core::Actor::Human,
                Command::Execute {
                    connection: owner.0,
                    session: owner.1,
                    request: Box::new(
                        oxyn_core::ExecRequest::new(
                            oxyn_core::QueryLanguage::Sql(oxyn_core::SqlDialect::Sqlite),
                            format!(
                                "WITH RECURSIVE n(x) AS (SELECT 0 UNION ALL SELECT x+1 FROM n \
                             WHERE x<{}) SELECT x FROM n",
                                ROWS - 1
                            ),
                        )
                        .with_limits(oxyn_core::ExecLimits::default().with_max_rows(ROWS)),
                    ),
                },
                &CancelToken::new(),
            ),
        )
        .expect("executes");
    let Outcome::Executed { result, .. } = outcome else {
        panic!("a SELECT executes, got {outcome:?}");
    };
    let runs = sql_runs(&backend);
    let buffer = backend.inner.executor.result(result).expect("retained");
    let batch_of = |row: usize| buffer.locate(row).expect("a row of the result").0;

    let in_memory = 0;
    let cached = ROWS - 1;
    let on_disk = ROWS - 8192 * 2;
    assert!(buffer.is_resident(batch_of(in_memory)));
    // The owner reads the last page once: it now answers from the cache.
    let ResultWindow::Page(_) = runtime
        .block_on(backend.read_result_page(owner.0, result, cached, 1))
        .expect("the owner reads a page from disk")
    else {
        panic!("a retained result pages");
    };
    assert!(!buffer.is_resident(batch_of(cached)));
    assert!(buffer.cached_batch(batch_of(cached)).is_some());
    assert!(!buffer.is_resident(batch_of(on_disk)));
    assert!(buffer.cached_batch(batch_of(on_disk)).is_none());

    let refusals: Vec<String> = [in_memory, cached, on_disk]
        .into_iter()
        .map(
            |row| match runtime.block_on(backend.read_result_page(other, result, row, 10)) {
                Err(error) => error.message,
                Ok(window) => panic!("row {row} was read by another connection: {window:?}"),
            },
        )
        .collect();
    assert!(
        refusals.iter().all(|refusal| *refusal == refusals[0]),
        "one refusal, wherever the rows are: {refusals:?}"
    );
    assert!(refusals[0].contains("does not belong to this connection"));

    for row in [in_memory, cached, on_disk] {
        let ResultWindow::Page(page) = runtime
            .block_on(backend.read_result_page(owner.0, result, row, 1))
            .expect("the owner still reads")
        else {
            panic!("a retained result pages");
        };
        assert_eq!(
            serde_json::to_string(&page.rows[0]).expect("serializable"),
            format!(r#"["{row}"]"#)
        );
    }
    assert_eq!(sql_runs(&backend), runs, "reading a page runs no SQL");
}
