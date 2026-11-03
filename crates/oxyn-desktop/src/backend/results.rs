//! Results the backend holds: windows of rows, search, value pages, export,
//! rows copied as text.
//!
//! A result is never re-run to be read again (ADR-0012, ADR-0017). Rows already
//! in memory are formatted from the buffer, as the grid reads them; a
//! batch that spilled to disk is brought back by `Command::ReadResultPage`,
//! which checks that the result belongs to the connection. Formatting and disk
//! reads happen on the blocking pool, never on the thread that runs the window
//! ([I-05](../../../../CLAUDE.md#i-05)).

mod copy;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use oxyn_core::{Actor, CancelToken, Command, CommandId, ConnectionId, ExportFormat, ResultId};
use oxyn_data::{
    BatchIndex, FindOutcome, FormatOptions, NumberGrouping, ResultBuffer, find_rows, format_cell,
};
use oxyn_exec::Outcome;
use parking_lot::Mutex;

use self::copy::SqlTarget;
use super::Backend;
use super::metadata::qualified_name;
use crate::ipc::results::{
    CopiedRows, CopyRowsFormat, CopySpec, ExportFormatChoice, FindAnswer, ResultWindow,
    ValuePageView, export_formats, matches_in_window,
};
use crate::ipc::{CatalogAddress, Cell, CommandOutcome, IpcError, ResultColumn, ResultPage};

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
/// `oxyn-exec` prunes a retained result whose buffer nobody holds — a native
/// grid held the `Arc` itself, so what was on screen was never idle. A webview
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
    /// The views reading each result: its console, a History tab, an agent's
    /// rows. Without a count, the console that reruns forgets a result a
    /// History tab is still reading (ADR-0017).
    readers: Mutex<HashMap<ResultId, Readers>>,
}

/// Who reads one result.
#[derive(Debug, Clone, Copy, Default)]
struct Readers {
    count: usize,
    /// The front received it from an execution, and its last reader releases
    /// it outright. A result the front only opened — an agent's, whose
    /// conversation owns it — is left to retention instead.
    delivered: bool,
}

// Not derived: the needles are what the user looked for in their data.
impl std::fmt::Debug for ResultsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultsState")
            .field("finds", &self.finds.lock().len())
            .field("shown", &self.shown.lock().len())
            .field("readers", &self.readers.lock().len())
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
    /// Ownership is checked first, the same way whether the rows are in
    /// memory, in the page cache or on disk: only the disk path goes through
    /// a command, and a check left to it would let the other two answer any
    /// connection. Each batch that spilled is then loaded by
    /// `Command::ReadResultPage`, which never contacts the server (ADR-0012).
    /// A failed load is reported, never retried here.
    pub async fn read_result_page(
        &self,
        connection: ConnectionId,
        result: ResultId,
        offset: usize,
        limit: usize,
    ) -> Result<ResultWindow, IpcError> {
        let Some(buffer) = self.inner.executor.result_on(connection, result)? else {
            return Ok(ResultWindow::Expired);
        };
        let spilled = spilled_batches(&buffer, offset, limit.min(MAX_PAGE_ROWS));
        if !self.load_spilled(connection, result, spilled).await? {
            return Ok(ResultWindow::Expired);
        }
        let options = self.format_options();
        let page = blocking(move || format_window(&buffer, offset, limit, &options)).await??;
        Ok(ResultWindow::Page(page))
    }

    /// Rows of a held result as text for the clipboard, in one of the « Copy
    /// rows as » formats.
    ///
    /// Reads the buffer only: never the rest of the cursor, never the query
    /// again (ADR-0017). With `connection`, the result must belong to it —
    /// checked by `Command::ReadResultPage` before anything is composed —,
    /// which also brings back a spilled batch; without it, only rows in
    /// memory copy, and never as SQL. Nothing composed
    /// here runs: `INSERT` and `IN` lists quote identifiers and escape values
    /// by the connection's dialect, and go to the clipboard
    /// ([I-10](../../../../CLAUDE.md#i-10)). Bounded by
    /// `MAX_COPY_ROWS` and `MAX_COPY_BYTES` (`results/copy.rs`); an expired result
    /// is an error here, since there is no window to draw instead.
    pub async fn copy_result_rows(
        &self,
        result: ResultId,
        connection: Option<ConnectionId>,
        address: Option<CatalogAddress>,
        spec: CopySpec,
    ) -> Result<CopiedRows, IpcError> {
        let expired = || IpcError::invalid("This result has expired; run the query again to copy");
        let batches = |buffer: &ResultBuffer| {
            spilled_batches(buffer, spec.offset, spec.count.min(copy::MAX_COPY_ROWS))
        };
        let buffer = if let Some(connection) = connection {
            // The connection chooses the dialect of the literals: a result of
            // another connection would be escaped for the wrong engine — a
            // MySQL cell escaped as SQLite keeps `\'`, and closes its literal
            // once pasted into MySQL (I-10). Ownership is checked first,
            // whether the rows are in memory or on disk.
            let Some(buffer) = self.inner.executor.result_on(connection, result)? else {
                return Err(expired());
            };
            if !self
                .load_spilled(connection, result, batches(&buffer))
                .await?
            {
                return Err(expired());
            }
            buffer
        } else {
            let Some(buffer) = self.inner.executor.result(result) else {
                return Err(expired());
            };
            if !batches(&buffer).is_empty() {
                return Err(IpcError::invalid(
                    "These rows were stored on disk; reading them needs the connection",
                ));
            }
            buffer
        };
        let sql = match (spec.format, connection) {
            (CopyRowsFormat::Insert | CopyRowsFormat::InList, Some(connection)) => {
                // Reads the store: off the runtime worker.
                let language = self
                    .on_blocking_pool(move |backend| {
                        Ok(backend.language_of(&backend.config(connection)?.driver))
                    })
                    .await?;
                // A connection whose consoles speak another language would
                // receive a statement it refuses.
                let Some(dialect) = language.sql_dialect() else {
                    return Err(IpcError::invalid(
                        "This connection does not speak SQL: copy the rows as text, CSV or JSON",
                    ));
                };
                let relation = match (spec.format, address) {
                    (CopyRowsFormat::Insert, Some(address)) => {
                        let path = address.to_path()?;
                        if path.relation().is_none() {
                            return Err(IpcError::invalid(
                                "An INSERT needs a table, not a schema or a database",
                            ));
                        }
                        Some(qualified_name(&path, dialect))
                    }
                    _ => None,
                };
                Some(SqlTarget { dialect, relation })
            }
            _ => None,
        };
        let options = self
            .format_options()
            // A copy is the value whole, and a number that pastes back as a
            // number: no cut, no digit groups — those are reading comforts of
            // the grid.
            .with_max_len(0)
            .with_number_grouping(NumberGrouping::None);
        let rows = spec.count;
        let text =
            blocking(move || copy::compose(&buffer, &spec, sql.as_ref(), &options)).await??;
        Ok(CopiedRows { text, rows })
    }

    /// Brings spilled batches back through `Command::ReadResultPage`, which
    /// checks ownership and never contacts the server (ADR-0012).
    ///
    /// `false` when the result went away meanwhile. A failed load is reported,
    /// never retried here.
    async fn load_spilled(
        &self,
        connection: ConnectionId,
        result: ResultId,
        batches: Vec<BatchIndex>,
    ) -> Result<bool, IpcError> {
        for batch in batches {
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
                Err(_) if self.inner.executor.result(result).is_none() => return Ok(false),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(true)
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
        self.add_reader(*result, buffer, true);
    }

    /// Counts one more view reading `result`, and holds its buffer for it.
    ///
    /// Each call is paired with one `forget_result` from the front. The hold
    /// is bounded like any other: a count keeps the last reader's release
    /// from being preempted, not the buffer from retention's budgets.
    pub(crate) fn add_reader(&self, result: ResultId, buffer: &Arc<ResultBuffer>, delivered: bool) {
        {
            let mut readers = self.inner.results.readers.lock();
            // Bounded by what retention still holds: a count whose result was
            // released elsewhere — a deleted conversation's — reads nothing.
            readers.retain(|id, _| *id == result || self.inner.executor.result(*id).is_some());
            let entry = readers.entry(result).or_default();
            entry.count = entry.count.saturating_add(1);
            entry.delivered |= delivered;
        }
        let mut shown = self.inner.results.shown.lock();
        shown.retain(|(held, _)| *held != result);
        shown.push_back((result, Arc::clone(buffer)));
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
    /// Lets the grid draw a result while it streams (UX-SPEC, « States of a
    /// view »). `None` when the result is not, or no longer, held. Refused,
    /// like a page, when the result belongs to another connection.
    pub fn result_columns(
        &self,
        connection: ConnectionId,
        result: ResultId,
    ) -> Result<Option<Vec<ResultColumn>>, IpcError> {
        let Some(buffer) = self.inner.executor.result_on(connection, result)? else {
            return Ok(None);
        };
        Ok(Some(
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
        ))
    }

    /// One view stops reading `result`.
    ///
    /// Its last reader releases it, and what was searched in it: outright if
    /// the front received it from an execution, by dropping the hold
    /// otherwise — an agent's result belongs to its conversation, and
    /// retention takes it back once nobody holds it.
    pub fn forget_result(&self, result: ResultId) {
        let delivered = {
            let mut readers = self.inner.results.readers.lock();
            match readers.get_mut(&result) {
                Some(entry) if entry.count > 1 => {
                    entry.count = entry.count.saturating_sub(1);
                    return;
                }
                Some(_) => readers.remove(&result).is_some_and(|entry| entry.delivered),
                None => false,
            }
        };
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
        if delivered {
            drop(self.inner.executor.forget_result(result));
        }
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
    /// `None` when the result has expired; refused, like a page, when it
    /// belongs to another connection.
    pub async fn find_in_result(
        &self,
        connection: ConnectionId,
        result: ResultId,
        needle: String,
        from: usize,
        forward: bool,
    ) -> Result<Option<FindAnswer>, IpcError> {
        let Some(outcome) = self.find(connection, result, needle).await? else {
            return Ok(None);
        };
        Ok(Some(FindAnswer::of(&outcome, from, forward)))
    }

    /// The matching rows inside one window, for the grid to mark.
    pub async fn find_matches_in_window(
        &self,
        connection: ConnectionId,
        result: ResultId,
        needle: String,
        offset: usize,
        limit: usize,
    ) -> Result<Option<Vec<usize>>, IpcError> {
        let Some(outcome) = self.find(connection, result, needle).await? else {
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
        let cancel = self.track(id)?;
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
    ///
    /// Ownership is checked before the remembered outcomes are looked at: an
    /// answer kept from the owner's search must not reach another connection.
    async fn find(
        &self,
        connection: ConnectionId,
        result: ResultId,
        needle: String,
    ) -> Result<Option<Arc<FindOutcome>>, IpcError> {
        if needle.len() > MAX_NEEDLE_BYTES {
            return Err(IpcError::invalid("This search text is too long"));
        }
        let Some(buffer) = self.inner.executor.result_on(connection, result)? else {
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
mod tests;

#[cfg(test)]
#[path = "results_access_tests.rs"]
mod access_tests;

#[cfg(test)]
#[path = "results_reader_tests.rs"]
mod reader_tests;
