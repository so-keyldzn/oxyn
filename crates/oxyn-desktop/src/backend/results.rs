//! Results the backend holds: windows of rows, search, value pages, export.
//!
//! A result is never re-run to be read again (ADR-0012, ADR-0017). Rows already
//! in memory are formatted from the buffer, as the GPUI grid reads them; a
//! batch that spilled to disk is brought back by `Command::ReadResultPage`,
//! which checks that the result belongs to the connection. Formatting and disk
//! reads happen on the blocking pool, never on the thread that runs the window
//! ([I-05](../../../../CLAUDE.md#i-05)).

use std::collections::VecDeque;
use std::sync::Arc;

use oxyn_core::{Actor, CancelToken, Command, CommandId, ConnectionId, ExportFormat, ResultId};
use oxyn_data::{BatchIndex, FindOutcome, FormatOptions, ResultBuffer, find_rows, format_cell};
use oxyn_exec::Outcome;
use parking_lot::Mutex;

use super::{Backend, Running};
use crate::ipc::results::{
    ExportFormatChoice, FindAnswer, ResultWindow, ValuePageView, export_formats, matches_in_window,
};
use crate::ipc::{Cell, CommandOutcome, IpcError, ResultColumn, ResultPage};

/// The largest window of rows one page call returns.
///
/// A virtualized grid shows a few dozen rows; 2 000 leaves room for fast
/// scrolling without letting a script ask for a whole result in one call
/// ([I-06](../../../../CLAUDE.md#i-06)).
pub const MAX_PAGE_ROWS: usize = 2_000;

/// The most formatted text one page call returns, in bytes.
///
/// Rows alone do not bound a page: 2 000 rows of 1 000 columns, each cut at
/// `oxyn_data::DEFAULT_MAX_LEN` characters, would cross the IPC bridge as
/// gigabytes. A page stops at the row that crosses this budget — always
/// after at least one row, so that a reader moves forward — and the caller
/// asks again from where it stopped.
pub const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;

/// Results the front is showing that the backend holds open at once.
///
/// `oxyn-exec` prunes a retained result whose buffer nobody holds — in GPUI
/// the grid held the `Arc`, so what was on screen was never idle. A webview
/// holds nothing, so the backend holds it for it. Bounded, or a session of
/// filters and sorts would pin every result it ever produced
/// ([I-06](../../../../CLAUDE.md#i-06)).
const SHOWN_RESULTS: usize = 16;

/// Memory the shown results may hold between them, resident and cached.
const SHOWN_BYTES: usize = 256 * 1024 * 1024;

/// Searches remembered at once. Each holds at most 50 000 row numbers
/// (`oxyn_data::find::MATCH_LIMIT`), so four stay under two megabytes.
const REMEMBERED_FINDS: usize = 4;

/// The longest needle accepted. A search is a word or a value, not a document.
const MAX_NEEDLE_BYTES: usize = 4096;

/// What the results feature keeps between calls: the last searches, so that
/// « next match » does not scan the buffer again.
#[derive(Default)]
pub(crate) struct ResultsState {
    finds: Mutex<VecDeque<RememberedFind>>,
    /// The buffers the front is showing, oldest first.
    shown: Mutex<VecDeque<(ResultId, Arc<ResultBuffer>)>>,
}

// Not derived: the needles are what the user looked for in their data.
impl std::fmt::Debug for ResultsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultsState")
            .field("finds", &self.finds.lock().len())
            .field("shown", &self.shown.lock().len())
            .finish()
    }
}

struct RememberedFind {
    result: ResultId,
    needle: String,
    /// Rows the buffer held when searched: a stream that grew since makes the
    /// answer incomplete, and it is searched again.
    rows: usize,
    outcome: Arc<FindOutcome>,
}

impl Backend {
    /// A bounded window of rows, or `Expired` when retention let it go.
    ///
    /// Batches still in memory are formatted directly; each batch that spilled
    /// to disk is loaded by `Command::ReadResultPage` first, which checks
    /// ownership and never contacts the server (ADR-0012). A failed load is
    /// reported, never retried here.
    pub async fn read_result_page(
        &self,
        connection: ConnectionId,
        result: ResultId,
        offset: usize,
        limit: usize,
    ) -> Result<ResultWindow, IpcError> {
        let Some(buffer) = self.inner.executor.result(result) else {
            return Ok(ResultWindow::Expired);
        };
        for batch in spilled_batches(&buffer, offset, limit.min(MAX_PAGE_ROWS)) {
            let outcome = self
                .inner
                .executor
                .dispatch(
                    Actor::Human,
                    Command::ReadResultPage {
                        connection,
                        result,
                        batch: batch.get(),
                    },
                    &CancelToken::new(),
                )
                .await;
            match outcome {
                Ok(Outcome::ResultPageRead { .. }) => {}
                Ok(Outcome::Denied { reason, .. }) => return Err(IpcError::invalid(reason)),
                Ok(Outcome::NeedsApproval { command, .. }) => {
                    // A local page read waits for nobody: refused and said.
                    self.inner.executor.reject(command);
                    return Err(IpcError::invalid(
                        "The connection policy does not allow automatic local page loading.",
                    ));
                }
                Ok(_) => return Err(IpcError::invalid("Unexpected result page response")),
                Err(_) if self.inner.executor.result(result).is_none() => {
                    return Ok(ResultWindow::Expired);
                }
                Err(error) => return Err(error.into()),
            }
        }
        let options = self.format_options();
        let page = blocking(move || format_window(&buffer, offset, limit, &options)).await??;
        Ok(ResultWindow::Page(page))
    }

    /// Holds a result the front is about to receive, so that retention does
    /// not release what is on screen.
    ///
    /// Only what is **delivered** is held: an outcome that never reaches the
    /// webview is not. Dropping the oldest hold does not delete anything — it
    /// makes that result prunable again, and a read of it then answers
    /// « expired » like any released result.
    pub(crate) fn hold_shown(&self, outcome: &Outcome) {
        let Outcome::Executed { result, buffer, .. } = outcome else {
            return;
        };
        let mut shown = self.inner.results.shown.lock();
        shown.retain(|(held, _)| held != result);
        shown.push_back((*result, Arc::clone(buffer)));
        while shown.len() > SHOWN_RESULTS || held_bytes(&shown) > SHOWN_BYTES {
            // The newest is never dropped: it is the one being shown.
            if shown.len() <= 1 {
                break;
            }
            shown.pop_front();
        }
    }

    /// Whether the backend is holding this result open for the front.
    #[cfg(test)]
    pub(crate) fn holds_shown(&self, result: ResultId) -> bool {
        self.inner
            .results
            .shown
            .lock()
            .iter()
            .any(|(held, _)| *held == result)
    }

    /// The columns of a result, known from its schema — before its first rows.
    ///
    /// Lets the grid draw a result while it streams (UX-SPEC, « États d'une
    /// vue »). `None` when the result is not, or no longer, held.
    #[must_use]
    pub fn result_columns(&self, result: ResultId) -> Option<Vec<ResultColumn>> {
        let buffer = self.inner.executor.result(result)?;
        Some(
            buffer
                .schema()
                .fields()
                .iter()
                .map(|field| ResultColumn {
                    name: field.name().clone(),
                    data_type: field.data_type().to_string(),
                    nullable: field.is_nullable(),
                })
                .collect(),
        )
    }

    /// Releases a result the front no longer shows, and what was searched in it.
    pub fn forget_result(&self, result: ResultId) {
        self.inner
            .results
            .shown
            .lock()
            .retain(|(held, _)| *held != result);
        self.inner
            .results
            .finds
            .lock()
            .retain(|find| find.result != result);
        drop(self.inner.executor.forget_result(result));
    }

    /// Runs `work` on the blocking pool with a handle on the backend.
    ///
    /// For commands whose body waits on a lock, reads a spilled batch or
    /// deletes a file: an `async` command still runs on a runtime worker,
    /// and a worker that blocks stalls every other command
    /// ([I-05](../../../../CLAUDE.md#i-05)).
    pub async fn on_blocking_pool<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Backend) -> Result<T, IpcError> + Send + 'static,
    ) -> Result<T, IpcError> {
        let backend = self.clone();
        blocking(move || work(&backend)).await?
    }

    /// Writes a result to a file the user chose.
    pub async fn export(
        &self,
        id: CommandId,
        connection: ConnectionId,
        result: ResultId,
        format: ExportFormat,
        destination: std::path::PathBuf,
    ) -> Result<CommandOutcome, IpcError> {
        self.run(
            id,
            Command::Export {
                connection,
                result,
                format,
                destination,
            },
        )
        .await
    }

    /// Every export format, with whether it can be written today.
    #[must_use]
    pub fn export_formats(&self) -> Vec<ExportFormatChoice> {
        export_formats()
    }

    /// Where `needle` matches, moving from `from` in one direction.
    ///
    /// Searches the batches in memory only and says how many it skipped
    /// (`oxyn_data::find`); a search never reads the disk nor filters the rows.
    /// `None` when the result has expired.
    pub async fn find_in_result(
        &self,
        result: ResultId,
        needle: String,
        from: usize,
        forward: bool,
    ) -> Result<Option<FindAnswer>, IpcError> {
        let Some(outcome) = self.find(result, needle).await? else {
            return Ok(None);
        };
        Ok(Some(FindAnswer::of(&outcome, from, forward)))
    }

    /// The matching rows inside one window, for the grid to mark.
    pub async fn find_matches_in_window(
        &self,
        result: ResultId,
        needle: String,
        offset: usize,
        limit: usize,
    ) -> Result<Option<Vec<usize>>, IpcError> {
        let Some(outcome) = self.find(result, needle).await? else {
            return Ok(None);
        };
        Ok(Some(matches_in_window(
            &outcome,
            offset,
            limit.min(MAX_PAGE_ROWS),
        )))
    }

    /// One page of one value, through `Command::InspectResultValue`.
    ///
    /// Cancellable under `id`, like any command: closing the inspector while a
    /// page loads cancels it. `None` when the result has expired.
    pub async fn inspect_value(
        &self,
        id: CommandId,
        connection: ConnectionId,
        result: ResultId,
        row: usize,
        column: usize,
        offset: usize,
    ) -> Result<Option<ValuePageView>, IpcError> {
        let inner = &self.inner;
        let Some(buffer) = inner.executor.result(result) else {
            return Ok(None);
        };
        let (name, data_type) = buffer
            .schema()
            .fields()
            .get(column)
            .map(|field| (field.name().clone(), field.data_type().to_string()))
            .ok_or_else(|| IpcError::invalid("This column is not in the result"))?;
        drop(buffer);
        let cancel = self.track(id);
        let _running = Running { inner, id };
        let outcome = inner
            .executor
            .dispatch_as(
                id,
                Actor::Human,
                Command::InspectResultValue {
                    connection,
                    result,
                    row,
                    column,
                    offset,
                },
                &cancel,
            )
            .await;
        match outcome {
            Ok(Outcome::ValueInspected { page }) => {
                Ok(Some(ValuePageView::of(page, name, data_type)))
            }
            Ok(Outcome::Denied { reason, .. }) => Err(IpcError::invalid(reason)),
            Ok(Outcome::NeedsApproval { command, .. }) => {
                inner.executor.reject(command);
                Err(IpcError::invalid(
                    "The connection policy does not allow automatic value inspection.",
                ))
            }
            Ok(_) => Err(IpcError::invalid("Unexpected value inspection response")),
            Err(_) if inner.executor.result(result).is_none() => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// The search outcome for `needle`, computed once per result and size.
    async fn find(
        &self,
        result: ResultId,
        needle: String,
    ) -> Result<Option<Arc<FindOutcome>>, IpcError> {
        if needle.len() > MAX_NEEDLE_BYTES {
            return Err(IpcError::invalid("This search text is too long"));
        }
        let Some(buffer) = self.inner.executor.result(result) else {
            return Ok(None);
        };
        let rows = buffer.row_count();
        if let Some(found) = self
            .inner
            .results
            .finds
            .lock()
            .iter()
            .find(|find| find.result == result && find.needle == needle && find.rows == rows)
        {
            return Ok(Some(Arc::clone(&found.outcome)));
        }
        let searched = needle.clone();
        let options = self.format_options();
        let outcome = blocking(move || find_rows(&buffer, &searched, &options)).await?;
        let outcome = Arc::new(outcome);
        let mut finds = self.inner.results.finds.lock();
        finds.retain(|find| !(find.result == result && find.needle == needle));
        if finds.len() >= REMEMBERED_FINDS {
            finds.pop_front();
        }
        finds.push_back(RememberedFind {
            result,
            needle,
            rows,
            outcome: Arc::clone(&outcome),
        });
        Ok(Some(outcome))
    }
}

/// Runs CPU or disk work on the blocking pool.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, IpcError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| IpcError::invalid("The result worker stopped"))
}

/// What the held buffers weigh in memory, spill files excluded.
fn held_bytes(shown: &VecDeque<(ResultId, Arc<ResultBuffer>)>) -> usize {
    shown.iter().fold(0, |sum, (_, buffer)| {
        sum.saturating_add(buffer.resident_bytes())
            .saturating_add(buffer.cached_bytes())
    })
}

/// What a cell weighs on the IPC bridge, near enough to budget a page.
fn cell_bytes(cell: &Cell) -> usize {
    match cell {
        Cell::Null => 4,
        Cell::Text(text) => text.len(),
        Cell::Truncated { text, .. } => text.len(),
        Cell::Unrenderable { unrenderable } => unrenderable.len(),
    }
}

/// The batches a window touches that are neither resident nor cached.
fn spilled_batches(buffer: &ResultBuffer, offset: usize, limit: usize) -> Vec<BatchIndex> {
    let end = offset.saturating_add(limit).min(buffer.row_count());
    let mut batches = Vec::new();
    let mut row = offset;
    while row < end {
        let Some((position, start_in_batch)) = buffer.locate(row) else {
            break;
        };
        let Some(rows) = buffer.batch_rows(position) else {
            break;
        };
        if buffer.cached_batch(position).is_none() {
            batches.push(position);
        }
        let step = rows.saturating_sub(start_in_batch);
        if step == 0 {
            break;
        }
        row = row.saturating_add(step);
    }
    batches
}

/// Formats `[offset, offset + limit)` of the buffer, bounded by
/// [`MAX_PAGE_ROWS`] and [`MAX_PAGE_BYTES`]. May read the disk: call on the blocking pool only.
fn format_window(
    buffer: &ResultBuffer,
    offset: usize,
    limit: usize,
    options: &FormatOptions,
) -> Result<ResultPage, IpcError> {
    let total_rows = buffer.row_count();
    let end = offset
        .saturating_add(limit.min(MAX_PAGE_ROWS))
        .min(total_rows);
    let mut rows = Vec::with_capacity(end.saturating_sub(offset));
    let mut bytes = 0usize;

    let mut row = offset;
    'window: while row < end {
        let Some((position, start_in_batch)) = buffer.locate(row) else {
            break;
        };
        let batch = match buffer.cached_batch(position) {
            Some(batch) => batch,
            None => buffer
                .batch(position)
                .map_err(|error| IpcError::invalid(format!("reading the result: {error}")))?
                .ok_or_else(|| IpcError::invalid("A batch of this result is missing"))?,
        };
        let available = batch.num_rows().saturating_sub(start_in_batch);
        let take = available.min(end.saturating_sub(row));
        if take == 0 {
            break;
        }
        for local in start_in_batch..start_in_batch.saturating_add(take) {
            let cells: Vec<Cell> = (0..batch.num_columns())
                .map(|column| format_cell(&batch, local, column, options).into())
                .collect();
            bytes = bytes.saturating_add(cells.iter().map(cell_bytes).sum::<usize>());
            rows.push(cells);
            if bytes >= MAX_PAGE_BYTES {
                break 'window;
            }
        }
        row = row.saturating_add(take);
    }

    Ok(ResultPage {
        offset,
        rows,
        total_rows,
        complete: buffer.is_complete(),
    })
}

#[cfg(test)]
mod tests {
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

        let columns = backend.result_columns(result).expect("held");
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
            .block_on(backend.find_in_result(result, "NEEDLE".into(), 0, true))
            .expect("search")
            .expect("held");
        assert_eq!(
            (answer.total, answer.row, answer.ordinal),
            (3, Some(999), Some(1))
        );
        let back = runtime
            .block_on(backend.find_in_result(result, "NEEDLE".into(), 999, false))
            .expect("search")
            .expect("held");
        assert_eq!(back.row, Some(2999), "backward wraps to the last match");
        let window = runtime
            .block_on(backend.find_matches_in_window(result, "needle".into(), 1000, 1000))
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
        assert!(backend.result_columns(result).is_none());
        assert!(matches!(
            runtime.block_on(backend.read_result_page(connection, result, 0, 10)),
            Ok(ResultWindow::Expired)
        ));
        assert!(
            runtime
                .block_on(backend.find_in_result(result, "needle".into(), 0, true))
                .expect("no error")
                .is_none()
        );
    }
}
