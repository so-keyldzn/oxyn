use arrow::array::{Int64Array, RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use oxyn_core::{CommandId, Environment, ExecStats};

use super::*;
use crate::ipc::{ConnectResponse, ConnectionDraft, OpenConnection};

fn buffer(batches: usize, rows: i64) -> ResultBuffer {
    let schema = Arc::new(Schema::new(vec![
        Field::new("n", DataType::Int64, false),
        Field::new("label", DataType::Utf8, true),
    ]));
    let buffer = ResultBuffer::new(schema.clone(), 64 * 1024 * 1024);
    for batch in 0..batches {
        let start = i64::try_from(batch).expect("few batches") * rows;
        let numbers = Int64Array::from_iter_values(start..start + rows);
        let labels = StringArray::from_iter_values(
            (start..start + rows).map(|n| if n % 7 == 0 { "seven" } else { "other" }),
        );
        buffer
            .push(
                RecordBatch::try_new(schema.clone(), vec![Arc::new(numbers), Arc::new(labels)])
                    .expect("batch"),
            )
            .expect("push");
    }
    buffer.mark_complete(ExecStats::default());
    buffer
}

#[test]
fn a_wide_window_stops_at_the_byte_budget_and_still_moves_forward() {
    let columns = 40;
    let schema = Arc::new(Schema::new(
        (0..columns)
            .map(|column| Field::new(format!("c{column}"), DataType::Utf8, false))
            .collect::<Vec<_>>(),
    ));
    let buffer = ResultBuffer::new(schema.clone(), 256 * 1024 * 1024);
    let long = "x".repeat(4096);
    let arrays = (0..columns)
        .map(|_| {
            Arc::new(StringArray::from_iter_values(std::iter::repeat_n(
                &long, 2000,
            ))) as arrow::array::ArrayRef
        })
        .collect();
    buffer
        .push(RecordBatch::try_new(schema, arrays).expect("batch"))
        .expect("push");
    let options = FormatOptions::default();

    let page = format_window(&buffer, 0, MAX_PAGE_ROWS, &options).expect("page");
    let sent: usize = page.rows.iter().flatten().map(cell_bytes).sum();
    assert!(!page.rows.is_empty());
    assert!(
        page.rows.len() < MAX_PAGE_ROWS,
        "the byte budget cut the page"
    );
    assert!(
        sent < MAX_PAGE_BYTES + 40 * 4096,
        "at most one row over budget"
    );

    let tiny = FormatOptions::default().with_max_len(0);
    let one = format_window(&buffer, 1999, 10, &tiny).expect("page");
    assert_eq!(one.rows.len(), 1, "a single heavy row still comes back");
}

#[test]
fn a_window_crosses_batches_and_stays_bounded() {
    let buffer = buffer(3, 1000);
    let options = FormatOptions::default();
    let page = format_window(&buffer, 990, 20, &options).expect("page");
    assert_eq!(page.rows.len(), 20);
    assert_eq!(
        serde_json::to_string(page.rows.get(10).expect("row 1000")).expect("json"),
        r#"["1000","other"]"#
    );
    let huge = format_window(&buffer, 0, usize::MAX, &options).expect("page");
    assert_eq!(huge.rows.len(), MAX_PAGE_ROWS);
    let past = format_window(&buffer, usize::MAX, 10, &options).expect("page");
    assert!(
        past.rows.is_empty(),
        "a window past the end is empty, not a panic"
    );
    assert!(
        spilled_batches(&buffer, 0, usize::MAX).is_empty(),
        "resident batches need no page read"
    );
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

fn open(runtime: &tokio::runtime::Runtime, backend: &Backend) -> OpenConnection {
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "results".into(),
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
        ConnectResponse::Saved { message, .. } => panic!("not opened: {message}"),
        ConnectResponse::Approval { .. } => panic!("a local connection opens directly"),
    }
}

#[test]
fn a_shown_result_survives_retention_until_the_front_forgets_it() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection");
    let session = open.session.parse().expect("session");
    let run = |sql: &str| {
        runtime
            .block_on(backend.execute(CommandId::new(), connection, session, sql.into()))
            .expect("executes")
    };

    let CommandOutcome::Executed { result, .. } = run("SELECT 1 AS shown") else {
        panic!("a SELECT executes");
    };
    let shown: ResultId = result.parse().expect("result id");
    assert!(backend.holds_shown(shown));

    // Far more results than retention keeps idle, each released by the
    // front as it replaces it — then a prune. What is still shown is not
    // idle, because the backend holds it.
    for index in 0..40 {
        let CommandOutcome::Executed { result, .. } = run(&format!("SELECT {index} AS other"))
        else {
            panic!("a SELECT executes");
        };
        backend.forget_result(result.parse().expect("result id"));
    }
    backend.inner.executor.prune_results();

    let ResultWindow::Page(page) = runtime
        .block_on(backend.read_result_page(connection, shown, 0, 10))
        .expect("page")
    else {
        panic!("the shown result is still held, not expired");
    };
    assert_eq!(page.rows.len(), 1);

    backend.forget_result(shown);
    assert!(!backend.holds_shown(shown));
    backend.inner.executor.prune_results();
    assert!(matches!(
        runtime.block_on(backend.read_result_page(connection, shown, 0, 10)),
        Ok(ResultWindow::Expired)
    ));
}

#[test]
fn holding_what_is_shown_stays_bounded() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection");
    let session = open.session.parse().expect("session");
    let mut ids = Vec::new();
    for index in 0..(SHOWN_RESULTS + 4) {
        let outcome = runtime
            .block_on(backend.execute(
                CommandId::new(),
                connection,
                session,
                format!("SELECT {index} AS n"),
            ))
            .expect("executes");
        let CommandOutcome::Executed { result, .. } = outcome else {
            panic!("a SELECT executes");
        };
        ids.push(result.parse::<ResultId>().expect("result id"));
    }
    assert_eq!(backend.inner.results.shown.lock().len(), SHOWN_RESULTS);
    assert!(
        backend.holds_shown(*ids.last().expect("one result")),
        "the newest result is the one on screen"
    );
    assert!(
        !backend.holds_shown(*ids.first().expect("one result")),
        "the oldest hold is released, and its result becomes prunable"
    );
}

#[test]
fn a_result_is_paged_searched_inspected_and_then_expires() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend);
    let connection: ConnectionId = open.connection.parse().expect("connection");
    let session = open.session.parse().expect("session");
    let outcome = runtime
        .block_on(
            backend.execute(
                CommandId::new(),
                connection,
                session,
                "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<3000) \
                 SELECT x, CASE WHEN x % 1000 = 0 THEN 'needle' ELSE 'hay' END AS label, \
                 CASE WHEN x = 1 THEN NULL ELSE 'NULL' END AS literal FROM n"
                    .into(),
            ),
        )
        .expect("executes");
    let CommandOutcome::Executed { result, .. } = outcome else {
        panic!("a SELECT executes, got {outcome:?}");
    };
    let result: ResultId = result.parse().expect("result id");

    let columns = backend
        .result_columns(connection, result)
        .expect("owned")
        .expect("held");
    assert_eq!(
        columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["x", "label", "literal"]
    );

    let ResultWindow::Page(page) = runtime
        .block_on(backend.read_result_page(connection, result, 2990, 50))
        .expect("page")
    else {
        panic!("a held result pages");
    };
    assert_eq!(page.rows.len(), 10);

    let answer = runtime
        .block_on(backend.find_in_result(connection, result, "NEEDLE".into(), 0, true))
        .expect("search")
        .expect("held");
    assert_eq!(
        (answer.total, answer.row, answer.ordinal),
        (3, Some(999), Some(1))
    );
    let back = runtime
        .block_on(backend.find_in_result(connection, result, "NEEDLE".into(), 999, false))
        .expect("search")
        .expect("held");
    assert_eq!(back.row, Some(2999), "backward wraps to the last match");
    let window = runtime
        .block_on(backend.find_matches_in_window(connection, result, "needle".into(), 1000, 1000))
        .expect("search")
        .expect("held");
    assert_eq!(window, vec![1999]);

    let null = runtime
        .block_on(backend.inspect_value(CommandId::new(), connection, result, 0, 2, 0))
        .expect("inspects")
        .expect("held");
    let literal = runtime
        .block_on(backend.inspect_value(CommandId::new(), connection, result, 1, 2, 0))
        .expect("inspects")
        .expect("held");
    assert!(null.is_null);
    assert!(!literal.is_null, "the text NULL is not an absent value");
    assert_eq!(literal.text, "NULL");
    assert_eq!(literal.column, "literal");
    assert!(
        runtime
            .block_on(backend.inspect_value(CommandId::new(), connection, result, 0, 99, 0))
            .is_err()
    );

    backend.forget_result(result);
    assert!(
        backend
            .result_columns(connection, result)
            .expect("owned")
            .is_none()
    );
    assert!(matches!(
        runtime.block_on(backend.read_result_page(connection, result, 0, 10)),
        Ok(ResultWindow::Expired)
    ));
    assert!(
        runtime
            .block_on(backend.find_in_result(connection, result, "needle".into(), 0, true))
            .expect("no error")
            .is_none()
    );
}
